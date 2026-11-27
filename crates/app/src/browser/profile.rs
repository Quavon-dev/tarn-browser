use super::*;

impl Browser {
    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.tab_placement == browser_core::layout::TabPlacement::Top {
            return self.toggle_tabs_on_top(cx);
        }
        self.set_sidebar_open(!self.sidebar_open, cx);
    }

    /// Tabs on top hide this window's sidebar without forgetting whether it was open.
    pub fn toggle_tabs_on_top(&mut self, cx: &mut Context<Self>) {
        self.tab_placement = self.tab_placement.toggled(&self.core.disk);
        let open = self.tab_placement == browser_core::layout::TabPlacement::Side && crate::prefs::sidebar_open(&self.core.disk);
        self.slide_sidebar(open);
        self.close_peek(cx);
        self.hide_hover_card(cx);
        cx.notify();
    }

    /// Puts this window's tabs in the sidebar or on top, as Settings picks for every window.
    pub fn set_tabs_on_top(&mut self, top: bool, cx: &mut Context<Self>) {
        if (self.tab_placement == browser_core::layout::TabPlacement::Top) != top {
            self.toggle_tabs_on_top(cx);
        }
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, |b, window, cx| b.open_settings(window, cx)) {
            return;
        }
        if let Some(handle) = self.settings_window
            && handle.update(cx, |_, settings_window, _| settings_window.activate_window()).is_ok()
        {
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>() else { return };
        let browser = cx.entity().downgrade();
        // The new window renders from the browser at once, so it must open after this update ends.
        cx.defer(move |cx| {
            let handle = SettingsWindow::open(browser.clone(), main, cx);
            browser.update(cx, |b, _| b.settings_window = handle).ok();
        });
    }

    pub fn open_settings_pane(&mut self, pane: crate::settings::SettingsPane, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.open_settings_pane(pane, window, cx)) {
            return;
        }
        self.open_settings(window, cx);
        let browser = cx.entity().downgrade();
        cx.defer(move |cx| {
            if let Some(handle) = browser.upgrade().and_then(|b| b.read(cx).settings_window) {
                handle.update(cx, |settings, _, cx| settings.set_pane(pane, cx)).ok();
            }
        });
    }

    pub fn current_profile(&self) -> Option<Profile> {
        self.current_loaded().map(|rt| rt.profile.clone())
    }

    /// The current profile without copying it, for checks that run every frame.
    pub(crate) fn current_loaded(&self) -> Option<Ref<'_, ProfileRuntime>> {
        let space = self.model.current_space()?;
        self.loaded(space.private, space.space.profile_id)
    }

    pub fn site_overrides(&self) -> Vec<(String, ShieldLevel)> {
        let Some(space) = self.model.current_space() else { return Vec::new() };
        let Some(rt) = self.loaded(space.private, space.space.profile_id) else { return Vec::new() };
        let mut list: Vec<_> = rt.overrides.borrow().iter().map(|(h, l)| (h.clone(), *l)).collect();
        list.sort();
        list
    }

    pub fn all_profiles(&self) -> Vec<Profile> {
        let burners = self.core.burners.borrow();
        [false, true]
            .into_iter()
            .flat_map(|private| self.store(private).profiles().unwrap_or_default())
            .filter(|p| !(p.is_private && burners.contains(&p.id)))
            .collect()
    }

    pub(crate) fn shield_wipes_data(&self, level: ShieldLevel) -> bool {
        self.current_loaded().is_some_and(|rt| browser_core::profiles::shield_wipes_data(level, self.model.private(), rt.profile.shield_level))
    }

    pub(crate) fn shield_target(&self) -> Option<(bool, Id)> {
        self.model.current_space().map(|space| (space.private, space.space.profile_id))
    }

    pub fn set_profile_shield(&mut self, level: ShieldLevel, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.shield_target().filter(|(private, id)| !self.model.shield_fixed(*private, *id)) else { return };
        if !self.shield_wipes_data(level) {
            return self.apply_profile_shield(target, level, window, cx);
        }
        let name = self.current_profile().map(|p| ui::text::bounded_title(&p.name)).unwrap_or_default();
        let question = crate::settings::lockdown_question(&name);
        let answer = window.prompt(gpui::PromptLevel::Warning, &question, Some(crate::settings::LOCKDOWN_WIPE), &["Use Lockdown", "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |b, window, cx| b.apply_profile_shield(target, level, window, cx)).ok();
            }
        })
        .detach();
    }

    pub(crate) fn apply_profile_shield(&mut self, (private, profile_id): (bool, Id), level: ShieldLevel, window: &mut Window, cx: &mut Context<Self>) {
        if self.store(private).set_profile_shield_level(profile_id, level).is_err() {
            return;
        }
        let new_store = match self.core.profiles.borrow_mut().get_mut(&(private, profile_id)) {
            Some(rt) => {
                let crossing = !private && (rt.profile.shield_level == ShieldLevel::Lockdown) != (level == ShieldLevel::Lockdown);
                rt.profile.shield_level = level;
                crossing
            }
            None => false,
        };
        if !new_store {
            self.core.apply_extensions(private, profile_id);
        }
        if !private {
            crate::tarn_ext::level_changed(&self.core, profile_id, cx);
        }
        self.restart_profile_views(private, profile_id, None, new_store, window, cx);
        #[cfg(feature = "chromium")]
        if new_store
            && level == ShieldLevel::Lockdown
            && let Some(own_store) = self.loaded(private, profile_id).map(|rt| rt.own_store.clone())
        {
            self.wipe_chromium(&own_store, &own_store);
        }
    }

    pub fn remove_site_override(&mut self, host: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.set_site_shield(host, None, window, cx);
    }

    /// Content rules are fixed when a web view is created, so a level change needs new views, in
    /// every window. `new_store` also replaces the profile's data store, for a move into or out of
    /// Lockdown.
    pub(crate) fn restart_profile_views(
        &mut self,
        private: bool,
        profile_id: Id,
        host: Option<&str>,
        new_store: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.core.sync_shield();
        self.restart_views_here(private, profile_id, host, new_store, window, cx);
        crate::shared::others(cx, |b, window, cx| b.restart_views_here(private, profile_id, host, false, window, cx));
    }

    pub(crate) fn restart_views_here(
        &mut self,
        private: bool,
        profile_id: Id,
        host: Option<&str>,
        new_store: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on_host = |view: &WebView| host.is_none_or(|h| view.url().is_some_and(|u| u.host_str().map(|s| s.trim_end_matches('.')) == Some(h)));
        let keys: Vec<TabKey> =
            self.views.iter().filter(|(k, v)| k.private == private && self.profile_of(**k) == Some(profile_id) && on_host(v)).map(|(k, _)| *k).collect();
        self.remake_views(keys, window, cx, |b| {
            #[cfg(windows)]
            if new_store {
                crate::shared::count_chromium_blocks(&b.core);
            }
            let old = if new_store { b.core.profiles.borrow_mut().remove(&(private, profile_id)) } else { None };
            if let Some(old) = old {
                // WebKit holds a download's delegate weakly, so the old store outlives its downloads.
                // shortcut: kept until quit; crossings are rare, prune by active downloads if that changes.
                old.data.set_extensions(&[], &[], old.profile.shield_level);
                b.core.retired_stores.borrow_mut().push(old.data);
            }
            b.runtime(private, profile_id);
        });
    }

    /// Makes the views of `profiles` again so their pages load with the profile's new style
    /// script; with `chromium_only`, only Chromium views, which keep the script they were created
    /// with, while WebKit views take it on their next load. Windows never asks: its Chromium
    /// views are restyled in place.
    pub(crate) fn restart_views_of(&mut self, profiles: &[(bool, Id)], chromium_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<TabKey> = self
            .views
            .iter()
            .filter(|(k, v)| (!chromium_only || v.is_chromium()) && self.profile_of(**k).is_some_and(|id| profiles.contains(&(k.private, id))))
            .map(|(k, _)| *k)
            .collect();
        if !keys.is_empty() {
            self.remake_views(keys, window, cx, |_| {});
        }
    }

    /// Drops the views of `keys` (their pages reload when shown), runs `between`, then shows the
    /// active tab and split again.
    pub(super) fn remake_views(&mut self, keys: Vec<TabKey>, window: &mut Window, cx: &mut Context<Self>, between: impl FnOnce(&mut Self)) {
        let active = self.model.active;
        let split = self.shown_split().is_some_and(|s| s.panes().iter().any(|p| keys.contains(p)));
        for key in keys {
            self.hibernate(key, cx);
        }
        between(self);
        if let Some(key) = active.filter(|_| split || self.model.active.is_none()) {
            self.activate(key, window, cx);
        }
        cx.notify();
    }

    /// Tabs of the profile stop renaming visits that were just deleted.
    pub(super) fn forget_visits(&mut self, private: bool, profile_id: Id) {
        let keys: Vec<TabKey> = self.pages.keys().copied().filter(|k| k.private == private && self.profile_of(*k) == Some(profile_id)).collect();
        for key in keys {
            if let Some(page) = self.pages.get_mut(&key) {
                page.visit = None;
            }
        }
    }

    pub fn clear_history(&mut self, private: bool, profile_id: Id, range: browser_core::library::HistoryRange, cx: &mut Context<Self>) {
        browser_core::library::clear_history(self.store(private), profile_id, range, now()).ok();
        let all = range == browser_core::library::HistoryRange::All;
        if all {
            crate::zaps::apply(&self.core, private, profile_id);
        }
        let forget = move |b: &mut Self, cx: &mut Context<Self>| {
            b.forget_history(private, profile_id);
            if all {
                b.reapply_site_zooms(private, profile_id, None);
                b.set_site_muted(private, profile_id, None, false, false);
            }
            cx.notify();
        };
        forget(self, cx);
        crate::shared::others(cx, |b, _, cx| forget(b, cx));
    }

    pub fn remove_history_url(&mut self, url: &str, cx: &mut Context<Self>) {
        let Some((private, profile_id)) = self.model.profile() else { return };
        self.store(private).delete_history_url(profile_id, url).ok();
        let forget = move |b: &mut Self, cx: &mut Context<Self>| {
            b.forget_visits(private, profile_id);
            b.history_cache.clear();
            b.forget_text_matches();
            b.menu.history_cleared();
            cx.notify();
        };
        forget(self, cx);
        crate::shared::others(cx, |b, _, cx| forget(b, cx));
    }

    /// Forgets what this window keeps in memory of a profile's history: visits, suggestions,
    /// recently closed tabs and site icons.
    pub(crate) fn forget_history(&mut self, private: bool, profile_id: Id) {
        self.forget_visits(private, profile_id);
        self.history_cache.clear();
        self.forget_text_matches();
        self.menu.history_cleared();
        self.model.closed_tabs.clear_profile(private, profile_id);
        self.forget_favicons((private, profile_id));
    }

    /// The private profile is never written to disk.
    pub fn export_profile(&mut self, private: bool, profile_id: Id, cx: &mut Context<Self>) {
        let notice = match self.write_export(private, profile_id) {
            Ok(path) => format!("Exported to {}.", path.display()),
            Err(e) => format!("Export failed: {e}."),
        };
        self.privacy_notice = Some(((private, profile_id), notice));
        cx.notify();
    }

    pub(super) fn write_export(&self, private: bool, profile_id: Id) -> Result<PathBuf, String> {
        use std::io::Write;
        #[cfg(unix)]
        use std::os::unix::fs::OpenOptionsExt;
        if private {
            return Err("private data is not saved".into());
        }
        let store = self.store(private);
        let name = store.profile(profile_id).map_err(|e| e.to_string())?.name;
        let json = store.export_profile(profile_id).map_err(|e| e.to_string())?;
        let dir = std::env::home_dir().ok_or("no home folder")?.join("Downloads");
        let file = browser_core::profile_data::export_name(&name);
        let path = engine::downloads::free_path(&dir, &file, |_| false);
        let mut options = std::fs::OpenOptions::new();
        #[cfg(unix)]
        options.mode(0o600);
        options.write(true).create_new(true).open(&path).and_then(|mut f| f.write_all(json.as_bytes())).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Files already saved to Downloads stay; the profile row (name, Shield level) stays.
    pub fn delete_profile_data(&mut self, private: bool, profile_id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let target = (private, profile_id);
        self.forget_visits(private, profile_id);
        let keys: Vec<TabKey> = self.views.keys().filter(|k| k.private == private && self.profile_of(**k) == Some(profile_id)).copied().collect();
        let folders: Vec<Id> = self
            .model
            .spaces
            .iter()
            .filter(|s| s.private == private && s.space.profile_id == profile_id)
            .flat_map(|s| self.store(private).folders(s.space.id).unwrap_or_default())
            .map(|f| f.id)
            .collect();
        let reset = match self.core.reset_profile_data(private, profile_id) {
            Ok(reset) => reset,
            Err(e) => {
                self.privacy_notice = Some((target, format!("Deleting failed ({e}).")));
                cx.notify();
                return;
            }
        };
        let space = reset.space;
        if !private {
            self.apply_cert_exceptions(profile_id);
        }
        Self::sync_other_windows(cx);
        crate::shared::others(cx, |b, _, cx| {
            b.forget_history(private, profile_id);
            b.toast.forget(target);
            b.forget_assistant(profile_id);
            cx.notify();
        });
        self.forget_assistant(profile_id);
        for key in keys {
            self.hibernate(key, cx);
        }
        self.model.forget_folders(private, &folders);
        self.toast.forget(target);
        self.pending_external = None;
        self.refocus |= std::mem::take(&mut self.find_open);
        self.suggestions.clear();
        let store = self.core.store(private);
        self.pages.retain(|k, _| k.private != private || store.tab(k.id).is_ok());
        self.history_cache.clear();
        self.forget_text_matches();
        self.model.closed_tabs.clear_profile(private, profile_id);
        self.forget_favicons(target);
        self.privacy_notice = Some((target, "Deleting website data…".into()));
        let own_store = reset.own_store;
        // WebKit's data removal leaves its tracking-prevention and site-security databases behind,
        // so a profile with a store of its own moves to a new one and the old one goes as a whole.
        if reset.fresh_store.is_some() {
            #[cfg(feature = "chromium")]
            self.wipe_chromium(&own_store, reset.fresh_store.as_deref().unwrap_or_default());
            #[cfg(windows)]
            crate::shared::count_chromium_blocks(&self.core);
            if let Some(old) = self.core.profiles.borrow_mut().remove(&(false, profile_id)) {
                old.data.set_extensions(&[], &[], old.profile.shield_level);
                if self.downloads_popover.active.get(&target).is_some_and(|n| *n > 0) {
                    self.core.retired_stores.borrow_mut().push(old.data);
                }
            }
            self.core.extensions.borrow_mut().forget_running(false, profile_id);
            DataStore::wipe_persistent(&own_store);
            DataStore::wipe_extensions(&own_store);
            crate::zaps::forget(&own_store);
            self.retire_store(&own_store, cx);
            self.runtime(false, profile_id);
            self.core.sync_shield();
            self.privacy_notice = Some((target, "All data of this profile was deleted.".into()));
            self.reload_spaces();
            let ix = self.model.spaces.iter().position(|s| !s.private && s.space.id == space.id).unwrap_or(0);
            self.switch_space(ix, window, cx);
            return;
        }
        #[cfg(feature = "chromium")]
        if !private {
            self.wipe_chromium(&own_store, &own_store);
        }
        let (core, app_cx) = (self.core.clone(), cx.to_async());
        let reached = match self.runtime(private, profile_id) {
            Some(rt) => {
                rt.overrides.borrow_mut().clear();
                rt.data.set_site_styles(&crate::themes::page_sheets(&core, private, profile_id));
                if !private {
                    crate::zaps::forget(&own_store);
                    rt.data.set_rule_lists("zaps", None, false);
                    #[cfg(windows)]
                    rt.data.set_hidden_elements(&[]);
                }
                rt.rules.clear();
                if !private && rt.profile.shield_level == ShieldLevel::Lockdown {
                    DataStore::wipe_persistent(&rt.profile.data_store_uuid);
                    DataStore::wipe_extensions(&own_store);
                }
                let this = cx.entity().downgrade();
                let mut async_cx = cx.to_async();
                rt.data.remove_extension_data(move || {
                    core.extensions.borrow_mut().forget_running(private, profile_id);
                    if core.apply_extensions(private, profile_id) {
                        core.sync_shield();
                        app_cx.update(|cx| crate::shared::each(cx, |b, window, cx| b.restart_views_here(private, profile_id, None, false, window, cx))).ok();
                    }
                });
                rt.data.remove_all_data(move || {
                    this.update(&mut async_cx, |b, cx| {
                        b.privacy_notice = Some((target, "All data of this profile was deleted.".into()));
                        cx.notify();
                    })
                    .ok();
                });
                true
            }
            None => false,
        };
        if !reached {
            self.privacy_notice = Some((target, "Stored data was deleted; website data could not be reached.".into()));
        }
        self.core.sync_shield();
        self.reload_spaces();
        let ix = self.model.spaces.iter().position(|s| s.private == private && s.space.id == space.id).unwrap_or(0);
        self.switch_space(ix, window, cx);
    }

    pub(crate) fn forget_favicons(&mut self, profile: (bool, Id)) {
        self.favicons.forget(&profile);
        self.favicon_memo.take();
        #[cfg(feature = "translation")]
        self.translation_offered.retain(|(p, id, _)| (*p, *id) != profile);
    }

    /// The host's icon, else the nearest parent's within its site, so a subdomain never visited
    /// still shows its site's icon.
    /// Every tab row asks once per frame, so answers are kept until an icon arrives or goes.
    pub fn favicon(&self, url: &str) -> Option<Arc<Image>> {
        let space = self.model.current_space()?;
        let profile = (space.private, space.space.profile_id);
        let mut memo = self.favicon_memo.borrow_mut();
        // shortcut: a full memo starts over, so URLs a long session visited do not pile up.
        if memo.as_ref().is_none_or(|(p, urls)| *p != profile || urls.len() >= 4096) {
            *memo = Some((profile, HashMap::new()));
        }
        let urls = &mut memo.as_mut()?.1;
        if let Some(found) = urls.get(url) {
            return found.clone();
        }
        let found = self.find_favicon(profile, url);
        urls.insert(url.to_owned(), found.clone());
        found
    }

    pub(super) fn find_favicon(&self, profile: (bool, Id), url: &str) -> Option<Arc<Image>> {
        self.favicons.find(&profile, &tab_host(url)?).cloned()
    }

    /// Lockdown pages get no extra requests.
    pub(super) fn request_favicon(&mut self, key: TabKey, url: &Url) {
        let lockdown = self.level_for_tab(key, url.as_str()) == ShieldLevel::Lockdown;
        let (Some(profile_id), Some(host)) = (self.profile_of(key).filter(|_| self.model.agent_profile.is_none()), favicon::icon_host(url, lockdown)) else {
            return;
        };
        self.load_favicons(key.private, profile_id);
        let Some(view) = self.views.get(&key) else { return };
        let profile = (key.private, profile_id);
        let Some(ticket) = self.favicons.request(&profile, &host) else { return };
        let icon = (profile, host, ticket);
        let (events, page) = (self.events.clone(), url.clone());
        view.icon_links(url, move |links| {
            events.unbounded_send(Msg::IconLinks(key, icon.clone(), page.clone(), links)).ok();
        });
    }

    pub(super) fn fetch_favicon(&mut self, key: TabKey, icon: IconKey, page: &Url, links: &str) {
        let urls = favicon::candidates(page, links);
        let still_there = |v: &&Rc<WebView>| v.url().is_some_and(|u| u.origin() == page.origin());
        let view = self.views.get(&key).filter(still_there).filter(|_| !urls.is_empty() && self.favicons.is_requested(&icon.0, &icon.1, icon.2));
        let Some(view) = view else {
            self.favicons.withdraw(&icon.0, &icon.1, icon.2);
            return;
        };
        let events = self.events.clone();
        view.fetch_icon(page, &urls, favicon::MAX_BYTES, move |data| {
            events.unbounded_send(Msg::IconData(icon.clone(), data)).ok();
        });
    }

    pub(super) fn store_favicon(&mut self, icon: IconKey, data: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let png = cx.background_spawn(async move { favicon::normalize(&data) }).await;
            this.update(cx, |b, cx| {
                let (profile, host, ticket) = icon;
                // A history clear while the icon was loading withdraws the request.
                let wanted = b.favicons.is_requested(&profile, &host, ticket);
                if let Some(png) = png.as_ref().filter(|_| wanted && b.core.keeps_records(profile.0, profile.1)) {
                    b.store(profile.0).set_favicon(profile.1, &host, png).ok();
                }
                if b.favicons.accept(&profile, &host, ticket, png.map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)))) {
                    b.favicon_memo.take();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The zoom is kept per host and profile and shared by its open tabs; Lockdown pages keep none.
    pub fn zoom(&mut self, factor: Option<f64>, cx: &mut Context<Self>) {
        if self.responsive_active() {
            return self.show_toast("Zoom is off while Responsive Size is on", None, cx);
        }
        let (Some(key), Some(v)) = (self.model.active, self.active_view()) else { return };
        v.set_zoom(factor.map_or(1.0, |f| v.zoom() * f));
        self.show_toast(format!("Zoom {}%", (v.zoom() * 100.).round()), None, cx);
        let (Some(profile_id), Some(url)) = (self.profile_of(key), v.url()) else { return };
        let Some(host) = url.host_str() else { return };
        let Some(zoom) = zoom_to_save(v.zoom(), self.level_for_tab(key, url.as_str()) == ShieldLevel::Lockdown) else { return };
        if self.store(key.private).set_site_zoom(profile_id, host, zoom).is_ok() {
            self.reapply_site_zooms(key.private, profile_id, Some(host));
        }
        cx.notify();
    }

    /// Sets the host's saved zoom; without one, `reset` returns the page to 100% and otherwise leaves it.
    pub(super) fn apply_site_zoom(&mut self, key: TabKey, url: Option<&Url>, reset: bool) {
        let Some(view) = self.views.get(&key).cloned() else { return };
        let saved = self.profile_of(key).zip(url.and_then(Url::host_str)).and_then(|(p, host)| self.store(key.private).site_zoom(p, host).ok().flatten());
        let Some(zoom) = saved.or(reset.then_some(1.0)) else { return };
        if self.responsive_active() {
            self.zoom_before_responsive.insert(key, zoom);
        } else {
            view.set_zoom(zoom);
        }
    }

    /// Re-applies saved zooms to the profile's open pages on `host`, or on every host.
    pub(super) fn reapply_site_zooms(&mut self, private: bool, profile_id: Id, host: Option<&str>) {
        let views: Vec<(TabKey, Option<Url>)> = self
            .views
            .iter()
            .filter(|(k, _)| k.private == private && self.profile_of(**k) == Some(profile_id))
            .map(|(k, v)| (*k, v.url()))
            .filter(|(_, url)| host.is_none() || url.as_ref().and_then(Url::host_str) == host)
            .collect();
        for (key, url) in views {
            self.apply_site_zoom(key, url.as_ref(), true);
        }
    }

    pub(super) fn saved_mute(&self, key: TabKey, host: &str) -> bool {
        self.profile_of(key).is_some_and(|p| self.store(key.private).muted_sites(p).unwrap_or_default().iter().any(|h| h == host))
    }

    /// A page starts muted on a muted host; like zoom, an unsaved Lockdown mute lasts until the tab leaves the host.
    pub(super) fn apply_site_mute(&self, key: TabKey, url: &Url, left_host: bool) {
        let saved = url.host_str().is_some_and(|host| self.saved_mute(key, host));
        if let Some(view) = self.views.get(&key).filter(|v| saved || left_host || v.is_muted()) {
            view.set_muted(saved || !left_host);
        }
    }

    pub fn tab_muted(&self, key: TabKey) -> bool {
        match self.views.get(&key) {
            Some(view) => view.is_muted(),
            None => self.tab_url(key).and_then(|u| tab_host(&u)).is_some_and(|host| self.saved_mute(key, &host)),
        }
    }

    /// Mutes or unmutes the tab's host in its profile, for its open tabs and later visits; Lockdown saves nothing.
    pub fn toggle_site_muted(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(url) = self.tab_url(key).and_then(|u| Url::parse(&u).ok()) else { return };
        let (Some(profile_id), Some(host)) = (self.profile_of(key), url.host_str()) else { return };
        let muted = !self.tab_muted(key);
        let save = browser_core::site::saves_mute(muted, self.level_for_tab(key, url.as_str()));
        self.set_site_muted(key.private, profile_id, Some(host), muted, save);
        let host = crate::toolbar::host_tail(host.trim_start_matches("www."), 40);
        self.show_toast(if muted { format!("Muted {host}") } else { format!("Unmuted {host}") }, None, cx);
    }

    /// Applies a mute to the profile's open pages on `host`, or on every host, saving it when `save` is set.
    pub fn set_site_muted(&mut self, private: bool, profile_id: Id, host: Option<&str>, muted: bool, save: bool) {
        if let Some(host) = host.filter(|_| save) {
            self.store(private).set_site_muted(profile_id, host, muted).ok();
        }
        for (key, view) in &self.views {
            let on_host = host.is_none() || view.url().as_ref().and_then(Url::host_str) == host;
            if key.private == private && on_host && self.profile_of(*key) == Some(profile_id) {
                view.set_muted(muted);
            }
        }
    }

    pub fn muted_sites(&self) -> Vec<String> {
        let Some(space) = self.model.current_space() else { return Vec::new() };
        self.store(space.private).muted_sites(space.space.profile_id).unwrap_or_default()
    }

    pub fn site_zooms(&self) -> Vec<(String, f64)> {
        let Some(space) = self.model.current_space() else { return Vec::new() };
        self.store(space.private).site_zooms(space.space.profile_id).unwrap_or_default()
    }

    pub fn remove_site_zoom(&mut self, private: bool, profile_id: Id, host: Option<&str>, cx: &mut Context<Self>) {
        let store = self.store(private);
        match host {
            Some(host) => store.set_site_zoom(profile_id, host, None),
            None => store.clear_site_zooms(profile_id),
        }
        .ok();
        self.reapply_site_zooms(private, profile_id, host);
        cx.notify();
    }
}
