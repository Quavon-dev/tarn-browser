use super::*;

impl Browser {
    pub(crate) fn store(&self, private: bool) -> &Store {
        self.core.store(private)
    }

    pub fn page(&self, key: TabKey) -> PageState {
        self.pages.get(&key).cloned().unwrap_or_default()
    }

    pub fn loading(&self, key: TabKey) -> bool {
        self.pages.get(&key).is_some_and(|p| p.loading)
    }

    /// Books the redraw of the spinners' next step while a spinner of this space turns.
    pub(crate) fn keep_spinning(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reloading = self.reload_spin.is_some_and(|(_, at)| at.elapsed() < crate::toolbar::SPIN);
        if self.spinning || self.still() || !reloading && !self.model.tabs.iter().any(|t| self.loading(self.model.key(t))) {
            return;
        }
        self.spinning = true;
        spin_step(window, cx.entity().downgrade(), Instant::now());
    }

    /// Whether spinners stand still: motion is reduced or Tarn is in the background.
    pub(crate) fn still(&self) -> bool {
        !engine::app_active() || self.pref(crate::prefs::Pref::ReduceMotion)
    }

    pub(crate) fn reload_spaces(&mut self) {
        self.model.reload_spaces(&self.core);
    }

    pub(crate) fn reload_space(&mut self) {
        let old = self.model.reload_space(&self.core);
        self.space_shown(&old);
    }

    /// Catches the shell up with the current space's lists: its profile's site icons, the tabs
    /// that fade in and its bookmarks.
    pub(crate) fn space_shown(&mut self, old: &[Tab]) {
        self.note_new_tabs(old);
        let profile = self.model.profile();
        if let Some((private, profile_id)) = profile {
            self.load_favicons(private, profile_id);
        }
        if profile != self.bookmarks_of {
            self.bookmarks_of = profile;
            self.set_bookmarks(profile.map(|(private, id)| self.store(private).bookmarks(id).unwrap_or_default()).unwrap_or_default());
        }
    }

    /// A space's tabs, folders and groups as stored, with its profile's site icons loaded.
    pub(crate) fn space_lists(&mut self, ix: usize) -> browser_core::tabs::Lists {
        if let Some(space) = self.model.spaces.get(ix) {
            self.load_favicons(space.private, space.space.profile_id);
        }
        self.model.space_lists(&self.core, ix)
    }

    /// Stored icons are small 64 px PNGs, so a profile's set is read once and kept in memory.
    pub(super) fn load_favicons(&mut self, private: bool, profile_id: Id) {
        let store = self.core.store(private);
        self.favicons.load(&(private, profile_id), || {
            let icons = store.favicons(profile_id).unwrap_or_default();
            let icons = icons.into_iter().map(|(host, png)| (host, Arc::new(Image::from_bytes(ImageFormat::Png, png)))).collect();
            (icons, store.stale_favicons(profile_id, now() - favicon::MAX_AGE).unwrap_or_default())
        });
        self.favicon_memo.take();
    }

    pub(super) fn runtime(&mut self, private: bool, profile_id: Id) -> Option<Ref<'_, ProfileRuntime>> {
        let downloads = || -> Rc<dyn Fn(Event)> {
            let events = self.core.downloads.clone();
            Rc::new(move |event| drop(events.unbounded_send((private, profile_id, event))))
        };
        if self.core.open_profile(private, profile_id, downloads)? {
            let profiles = self.core.profiles.borrow();
            let data = &profiles[&(private, profile_id)].data;
            self.core.apply_blocking_to(data);
            data.set_sort_downloads(self.pref(crate::prefs::Pref::SortDownloads));
            drop(profiles);
            self.core.apply_extensions(private, profile_id);
            if !private {
                crate::tarn_ext::opened(&self.core, profile_id);
            }
            self.apply_site_styles(private, profile_id);
            if !private {
                self.apply_cert_exceptions(profile_id);
            }
            crate::zaps::apply(&self.core, private, profile_id);
            // Chromium refuses a profile's requests until its Shield choices arrive.
            #[cfg(windows)]
            self.core.sync_shield();
        }
        self.loaded(private, profile_id)
    }

    /// The runtime of a profile already opened; the borrow ends with the returned guard.
    pub(crate) fn loaded(&self, private: bool, profile_id: Id) -> Option<Ref<'_, ProfileRuntime>> {
        Ref::filter_map(self.core.profiles.borrow(), |p| p.get(&(private, profile_id))).ok()
    }

    pub fn set_filter_updates(&mut self, on: bool, cx: &mut Context<Self>) {
        match self.core.disk.set_setting(filters::SETTING, if on { "on" } else { "off" }) {
            Ok(()) => {
                self.core.filters.borrow_mut().updates = on;
                if self.warning.as_deref().is_some_and(|w| w.starts_with(FILTER_SETTING_WARNING)) {
                    self.warning = None;
                }
            }
            Err(e) => self.warning = Some(format!("{FILTER_SETTING_WARNING} ({e}).")),
        }
        cx.notify();
    }

    pub(super) fn current_runtime(&mut self) -> Option<Ref<'_, ProfileRuntime>> {
        let space = self.model.current_space()?;
        let (private, profile_id) = (space.private, space.space.profile_id);
        self.runtime(private, profile_id)
    }

    pub fn level_for(&self, url: &str) -> ShieldLevel {
        let Some((private, profile_id)) = self.model.profile() else { return ShieldLevel::default() };
        self.core.level(private, profile_id, Url::parse(url).ok().as_ref()).unwrap_or_default()
    }

    pub fn has_override(&self, host: &str) -> bool {
        self.model.profile().is_some_and(|(private, profile_id)| self.core.has_override(private, profile_id, host))
    }

    /// Events are tagged with the view that sent them, so a replaced view's late events are dropped.
    pub(crate) fn sink(&mut self, key: TabKey) -> Rc<dyn Fn(Event)> {
        self.next_generation += 1;
        let generation = self.next_generation;
        self.view_generation.insert(key, generation);
        let events = self.events.clone();
        Rc::new(move |event| {
            events.unbounded_send(Msg::Tab(key, generation, event)).ok();
        })
    }

    /// Events of the link peek's view, tagged so a closed peek's late events are dropped.
    pub(crate) fn peek_sink(&mut self) -> (u64, Rc<dyn Fn(Event)>) {
        self.next_generation += 1;
        let (generation, events) = (self.next_generation, self.events.clone());
        (generation, Rc::new(move |event| drop(events.unbounded_send(Msg::Peek(generation, event)))))
    }

    #[cfg(feature = "chromium")]
    pub(crate) fn web_engine_for(&mut self, key: TabKey, url: &str) -> WebEngine {
        let tab = self.model.tab(key).and_then(|t| t.web_engine);
        let Some((private, profile_id)) = self.model.profile().filter(|_| self.model.agent_profile.is_none()) else { return WebEngine::WebKit };
        self.runtime(private, profile_id);
        self.core.web_engine(private, profile_id, tab, Url::parse(url).ok().as_ref()).unwrap_or(WebEngine::WebKit)
    }

    /// A web view of the profile's data store with the Shield rules for `url`.
    pub(crate) fn web_view(&mut self, host: &Host, private: bool, profile_id: Id, url: &str, sink: Rc<dyn Fn(Event)>) -> Option<WebView> {
        let developer = self.developer;
        let sets = self.core.sets.clone();
        let extensions = self.model.agent_profile.is_none();
        let rt = self.runtime(private, profile_id)?;
        let site_for = browser_core::site::site_for(rt.profile.clone(), rt.overrides.clone(), rt.rules.clone(), sets);
        let site = Url::parse(url).map_or(nav_policy::Site::new(rt.profile.shield_level), |u| site_for(&u));
        let inspectable = (developer || rt.profile.is_dev) && site.level != ShieldLevel::Lockdown;
        let features = engine::Features { filtered: site.filtered, extensions };
        Some(WebView::new(host, &rt.data, site.level, features, inspectable, site_for, sink))
    }

    #[cfg_attr(not(feature = "chromium"), allow(unused_variables))]
    pub(crate) fn create_view(&mut self, key: TabKey, url: &str, cx: &mut Context<Self>) -> Option<Rc<WebView>> {
        let host = self.host.clone()?;
        let sink = self.sink(key);
        let space = self.model.current_space()?;
        let (private, profile_id) = (space.private, space.space.profile_id);
        if self.core.awaits_extensions(private, profile_id) {
            return None;
        }
        #[cfg(feature = "chromium")]
        let wants_chromium = !private && self.web_engine_for(key, url) == WebEngine::Chromium;
        #[cfg(feature = "chromium")]
        if wants_chromium && !self.chromium_current(key, cx) {
            return None;
        }
        #[cfg(feature = "chromium")]
        let developer = self.developer;
        #[cfg(feature = "chromium")]
        let chromium = wants_chromium
            .then(|| self.runtime(private, profile_id))
            .flatten()
            .and_then(|rt| WebView::chromium(&host, &rt.data, &data_dir().join("Chromium"), &rt.own_store, developer || rt.profile.is_dev, sink.clone()));
        #[cfg(not(feature = "chromium"))]
        let chromium = None;
        #[cfg(feature = "chromium")]
        if chromium.is_some() {
            if self.core.chromium_profile.get() != Some(profile_id) {
                crate::shared::count_chromium_blocks(&self.core);
            }
            let owner_changed = self.core.chromium_profile.replace(Some(profile_id)) != Some(profile_id);
            self.core.sync_shield();
            if owner_changed {
                crate::tarn_ext::chromium_owner_changed(&self.core, profile_id);
            }
            let ageing = browser_core::staleness::Staleness::Ageing;
            if self.core.chromium_staleness.get() == ageing && self.pages.get(&key).is_none_or(|p| p.notice.is_none()) {
                self.notify_offer(key, ageing.message().to_owned(), None);
            }
        }
        let view = Rc::new(match chromium {
            Some(view) => view,
            None => self.web_view(&host, private, profile_id, url, sink)?,
        });
        view.set_corner_radius(page_corner());
        view.set_dark(!ui::theme::Theme::is_light());
        self.view_prefs(&view);
        crate::agent::gate_view(key, &view, cx);
        self.views.insert(key, view.clone());
        self.attach_tarn(key);
        Some(view)
    }

    /// Typed and incoming URLs are user-initiated, so stripping tracking parameters here keeps
    /// the request's context; links from pages are left alone.
    pub fn clean_user_url(&self, url: Url) -> Url {
        self.core.clean_user_url(self.model.profile(), url)
    }

    /// `clean_user_url` for a load the user starts now; removed parameters count on the current
    /// profile's Shield card.
    pub(super) fn clean_user_load(&mut self, url: Url, cx: &mut Context<Self>) -> Url {
        let clean = self.clean_user_url(url.clone());
        if let Some((private, profile_id)) = self.model.current_space().map(|s| (s.private, s.space.profile_id)) {
            self.count_shield(private, profile_id, Counter::TrackingParams, shield::url_clean::removed_params(&url, &clean), clean.host_str(), cx);
        }
        clean
    }

    /// Counts one event for the site the tab shows, as the page that caused it.
    pub(super) fn count_on_page(&mut self, key: TabKey, counter: Counter, cx: &mut Context<Self>) {
        let host = self.tab_url(key).and_then(|u| Url::parse(&u).ok()).and_then(|u| page_host(&u));
        if let Some(profile_id) = self.profile_of(key) {
            self.count_shield(key.private, profile_id, counter, 1, host.as_deref(), cx);
        }
    }

    /// Adds to a Shield card counter and repaints every window, so an open card follows.
    pub(crate) fn count_shield(&mut self, private: bool, profile_id: Id, counter: Counter, n: i64, host: Option<&str>, cx: &mut Context<Self>) {
        if n <= 0 || self.store(private).count(profile_id, counter, n, host).is_err() {
            return;
        }
        crate::shield_card::touch();
        crate::shared::others(cx, |_, _, cx| cx.notify());
        cx.notify();
    }

    /// Shows how a download ended over the active tab: a passing toast in the window in front, or a
    /// warning in every window, held until dismissed (S11) and, while the window shows another
    /// profile's space, waiting for that profile's space.
    pub(crate) fn notify_download(
        &mut self,
        private: bool,
        profile_id: Id,
        result: &Result<(std::path::PathBuf, bool), String>,
        front: bool,
        cx: &mut Context<Self>,
    ) {
        let here = self.model.current_space().is_some_and(|s| (s.private, s.space.profile_id) == (private, profile_id));
        let mark = if cfg!(windows) { "the mark of the web" } else { "macOS quarantine" };
        let (tone, text) = match result {
            Ok((path, quarantined)) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                browser_core::toast::download(&ui::text::bounded_title(&name), Ok((*quarantined, engine::downloads::is_dangerous(&name))), mark)
            }
            Err(message) => browser_core::toast::download("", Err(&ui::text::bounded_title(message)), mark),
        };
        if tone == browser_core::toast::Tone::Warning {
            self.hold_toast(text, self.model.active.filter(|_| here), (private, profile_id), cx);
        } else if here && front {
            let reveal = result.as_ref().ok().map(|(path, _)| crate::toast::Action::Reveal(path.clone()));
            self.show_tab_toast(tone, text, reveal, (private, profile_id), cx);
        }
    }

    pub fn active_view(&self) -> Option<Rc<WebView>> {
        self.views.get(&self.model.active?).cloned()
    }

    pub fn activate(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.tab(key).is_none() {
            return;
        }
        let switched = self.model.active != Some(key);
        let _span = switched.then(|| engine::startup::span(engine::startup::TAB_SWITCH)).flatten();
        if switched {
            let now = Instant::now();
            let leaving: Vec<TabKey> = self.views.keys().copied().filter(|k| self.model.active == Some(*k) || self.in_split(*k)).collect();
            leaving.into_iter().for_each(|k| self.back_forward.shown(k, now));
            self.leave_page_fullscreen(window);
        }
        let mut fading = false;
        if switched
            && let Some((outgoing, _)) = self.page_fade.take()
            && !self.views.get(&key).is_some_and(|v| Rc::ptr_eq(v, &outgoing))
        {
            outgoing.set_visible(false);
        }
        let partners = self.split_partners(key, cx);
        if let Some(prev) = self.model.active.filter(|p| *p != key && !partners.contains(p)).and_then(|p| self.views.get(&p)).cloned() {
            fading = partners.is_empty() && !self.pref(crate::prefs::Pref::ReduceMotion) && !self.overlay_open() && self.active_json().is_none();
            if fading {
                self.page_fade = Some((prev, Instant::now()));
            } else {
                prev.set_visible(false);
            }
        }
        if let Some(prev) = self.model.active.filter(|p| *p != key) {
            self.ask_media(prev, cx);
        }
        let left = Instant::now();
        self.seen.extend(self.model.active.into_iter().chain([key]).map(|k| (k, left)));
        self.typed_in.remove(&key);
        let evicted = self.model.lifecycle.activate_with(&partners, key);
        for evicted in evicted {
            if evicted == key || partners.contains(&evicted) || self.typed_in.contains(&evicted) {
                // The split's panes, and tabs with typed input, stay awake above the live-tab limit.
                self.model.lifecycle.keep(evicted);
            } else {
                self.sleep_unless_playing(evicted, cx);
            }
        }
        // A view taken over from another window keeps its page; only a new one loads it.
        let loads = self.sleeping(key) && self.adopt_view(key, cx).is_none();
        if switched && self.find_open {
            if let Some(prev) = self.model.active.and_then(|p| self.views.get(&p)) {
                prev.clear_find();
            }
            self.find_count = None;
            self.find_typing = None;
            self.find_after_load = None;
        }
        let view = self.wake(key, cx);
        self.model.active = Some(key);
        if switched && self.find_open && self.active_json().is_none() && !self.reader_shown() {
            if loads || self.pages.get(&key).is_some_and(|p| p.loading) {
                self.find_after_load = Some(key);
            } else {
                self.find(false, cx);
            }
        }
        if let Some(view) = view {
            if fading {
                view.raise();
                view.set_alpha(0.);
            } else if switched {
                view.set_alpha(1.);
            }
            view.set_visible(!self.overlay_open());
            if !self.overlay_open() {
                view.focus();
            }
        }
        if !partners.is_empty() {
            // A page still fading out would show between the panes.
            if let Some((outgoing, _)) = self.page_fade.take() {
                outgoing.set_visible(false);
            }
            for pane in &partners {
                if let Some(view) = self.wake(*pane, cx) {
                    view.set_alpha(1.);
                    view.set_visible(!self.overlay_open());
                }
            }
        }
        self.model.touch(&self.core, key, now());
        if !self.overlay_open() {
            window.focus(&self.focus);
        }
        if switched {
            self.layout_changed();
            if self.active_prompt().is_some() {
                self.prepare_prompt(window, cx);
            }
        }
        cx.notify();
    }

    /// The tab's view: this window's, else one taken over from another window, else a new one
    /// that restores the tab's saved state or loads its page.
    pub(super) fn wake(&mut self, key: TabKey, cx: &mut Context<Self>) -> Option<Rc<WebView>> {
        if let Some(view) = self.views.get(&key).cloned().or_else(|| self.adopt_view(key, cx)) {
            return Some(view);
        }
        let tab = self.model.tab(key).cloned()?;
        self.create_view(key, &tab.url, cx).inspect(|view| {
            let restorable = self.level_for(&tab.url).policy().save_interaction_state;
            let saved = self.store(key.private).interaction_state(key.id).ok().flatten();
            // Windows saves Tarn's own format; anything else loads the tab's page instead.
            let ours = |state: &&[u8]| !cfg!(windows) || engine::view_policy::SavedHistory::from_bytes(state).is_some();
            let url = Url::parse(&tab.url).ok();
            // A restored history is never an Open File (S6), so an opened file loads afresh.
            let opened = url.as_ref().is_some_and(|u| self.opened_file(key, u));
            match saved.as_deref().filter(|state| !opened && restorable && !view.is_chromium() && ours(state)) {
                Some(state) => view.restore_interaction_state(state),
                None => {
                    if let Some(url) = url {
                        self.load_tab_url(key, view, url);
                    }
                }
            }
        })
    }

    pub(super) fn opened_file(&self, key: TabKey, url: &Url) -> bool {
        self.pages.get(&key).and_then(|p| p.opened_file.as_ref()) == Some(url)
    }

    pub(super) fn load_tab_url(&self, key: TabKey, view: &WebView, url: Url) {
        match self.link_loads.get(&key) {
            _ if self.opened_file(key, &url) => view.load_file(&url),
            // Says why a file tab no Open File vouches for stays empty.
            _ if url.scheme() == "file" => view.load(&url),
            _ if nav_policy::decide(&url, Source::Sync) != Decision::Allow => {}
            Some(from) => view.load_link(&url, from.as_ref()),
            None => view.load(&url),
        }
    }

    pub(super) fn save_state(&self, key: TabKey) {
        let Some(view) = self.views.get(&key) else { return };
        let url = view.url().map(String::from).unwrap_or_default();
        let state = self.level_for_tab(key, &url).policy().save_interaction_state.then(|| view.interaction_state()).flatten();
        self.store(key.private).set_interaction_state(key.id, state.as_deref()).ok();
    }

    /// Whether `key` still shows the view a request was sent to; tab ids are reused after a close.
    pub(crate) fn same_view(&self, key: TabKey, asked: &std::rc::Weak<WebView>) -> bool {
        asked.upgrade().is_some_and(|v| self.views.get(&key).is_some_and(|live| Rc::ptr_eq(live, &v)))
    }

    /// Drops this window's view of `key`, and the fade still showing it.
    pub(super) fn take_view(&mut self, key: TabKey) -> Option<Rc<WebView>> {
        self.typed_in.remove(&key);
        self.forget_media(key);
        let view = self.views.remove(&key)?;
        if self.page_fade.as_ref().is_some_and(|(fading, _)| Rc::ptr_eq(fading, &view)) {
            self.page_fade = None;
        }
        Some(view)
    }

    pub(crate) fn hibernate(&mut self, key: TabKey, cx: &mut Context<Self>) {
        self.save_state(key);
        self.take_view(key);
        self.json_pages.remove(&key);
        self.drop_reader(key);
        self.view_generation.remove(&key);
        self.drop_prompts(key, cx);
        self.model.lifecycle.remove(key);
        if let Some(page) = self.pages.get_mut(&key) {
            page.loading = false;
            page.retry = None;
        }
        if self.model.active == Some(key) {
            self.model.active = None;
        }
    }

    pub(crate) fn save_all(&self) {
        for key in self.views.keys() {
            self.save_state(*key);
        }
    }

    pub fn new_tab(&mut self, url: &Url, window: &mut Window, cx: &mut Context<Self>) {
        self.new_tab_in(url, None, Opened::Typed, window, cx);
    }

    /// A new tab whose `tabs.web_engine` is `web_engine`, as for a copy or a pop-up of a tab,
    /// placed as `opened` says.
    pub(crate) fn new_tab_in(&mut self, url: &Url, web_engine: Option<WebEngine>, opened: Opened, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.burner_shown()
            && let Some(key) = self.model.tabs.first().map(|t| self.model.key(t))
        {
            self.activate(key, window, cx);
            return self.navigate(url, window, cx);
        }
        let Some(key) = self.model.open_tab(&self.core, url.as_str(), web_engine.or(self.saved.web_engine), opened) else { return };
        self.reload_space();
        self.activate(key, window, cx);
    }

    pub fn navigate(&mut self, url: &Url, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.model.active {
            self.link_loads.remove(&key);
            let page = self.pages.entry(key).or_default();
            page.retry = Some(url.clone());
            page.loads += 1;
        }
        match (self.active_view(), self.model.active) {
            (Some(view), Some(key)) if view.is_chromium() && self.level_for(url.as_str()) == ShieldLevel::Lockdown => self.rebuild_view(key, url, cx),
            (Some(view), _) => view.load(url),
            (None, Some(key)) => {
                self.store(key.private).update_tab_page(key.id, url.as_str(), "").ok();
                self.reload_space();
                self.activate(key, window, cx);
            }
            (None, None) => self.new_tab(url, window, cx),
        }
    }

    pub fn close_tab(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tabs(vec![key], window, cx);
    }

    /// Unloads a pinned tab, or closes an unpinned one and returns what reopening it needs. The
    /// caller reloads the space and picks the next active tab.
    pub(super) fn close_one(&mut self, key: TabKey, cx: &mut Context<Self>) -> Option<ClosedTab> {
        let tab = self.model.tab(key).cloned()?;
        self.splits.leave(key);
        if tab.kind != TabKind::Ephemeral {
            self.hibernate(key, cx);
            return None;
        }
        let closed = self.closed_tab(&tab, key);
        self.tab_leaving(&tab, cx);
        if !key.private {
            self.core.tarn.borrow().send(crate::tarn_ext::Msg::TabClosed(key.id as u64));
        }
        self.take_view(key);
        self.link_loads.remove(&key);
        self.json_pages.remove(&key);
        self.drop_reader(key);
        self.view_generation.remove(&key);
        self.drop_prompts(key, cx);
        self.model.lifecycle.remove(key);
        self.pages.remove(&key);
        self.store(key.private).delete_tab(key.id).ok();
        if self.model.active == Some(key) {
            self.model.active = None;
        }
        Some(closed)
    }

    pub(crate) fn pick_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let adopting = self.adopting;
        match self.model.pick(|k| Some(k) != adopting && crate::shared::live_elsewhere(k, cx)) {
            Some(key) => self.activate(key, window, cx),
            None => {
                self.model.active = None;
                cx.notify();
            }
        }
    }

    pub fn move_tab(&mut self, key: TabKey, kind: TabKind, folder: Option<Id>, index: usize, cx: &mut Context<Self>) {
        if self.model.burner_shown() && kind != TabKind::Ephemeral {
            return;
        }
        if self.store(key.private).move_tab(key.id, kind, folder, index).is_ok() {
            self.reload_space();
            cx.notify();
        }
    }

    /// The `n`th visible tab, counted from 1; 9 is the last, as in other browsers.
    pub fn select_tab(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.model.slot(n) {
            self.switch_to(key, window, cx);
        }
    }

    pub fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.model.cycled(forward) {
            self.switch_to(key, window, cx);
        }
    }

    /// A tab the user picks; an open address field or launcher closes so the tab shows.
    pub fn switch_to(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.switch_to(key, window, cx)) {
            return;
        }
        if self.command_mode.is_some() {
            self.close_command(window, cx);
        }
        self.activate(key, window, cx);
    }

    pub fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.model.active_tab().map(|t| self.model.key(t)) {
            self.toggle_pin_tab(key, cx);
        }
    }

    pub fn new_folder(&mut self, parent: Option<Id>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self.model.new_folder(&self.core, parent) else { return };
        self.reload_space();
        self.begin_rename(crate::groups::RenameTarget::Folder(folder.id), window, cx);
    }

    pub fn move_folder(&mut self, id: Id, parent: Option<Id>, cx: &mut Context<Self>) {
        if self.model.move_folder(&self.core, id, parent) {
            self.reload_space();
            cx.notify();
        }
    }

    pub fn toggle_folder(&mut self, id: Id, cx: &mut Context<Self>) {
        self.model.toggle_folder(id);
        cx.notify();
    }

    pub fn switch_space(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.abandoned || ix >= self.model.spaces.len() || self.in_main(window, cx, move |b, window, cx| b.switch_space(ix, window, cx)) {
            return;
        }
        let _span = (ix != self.model.space_ix).then(|| engine::startup::span(engine::startup::SPACE_SWITCH)).flatten();
        self.commit_rename(cx);
        self.hide_hover_card(cx);
        self.tab_switcher = None;
        if self.command_mode.is_some() {
            self.close_command(window, cx);
        }
        if let Some(view) = self.active_view() {
            view.set_visible(false);
        }
        self.motion = Default::default();
        self.floating = Default::default();
        let burning = self.model.burner_shown();
        let Some((from, old)) = self.model.switch_space(&self.core, ix) else { return };
        self.motion.forget_pointer();
        if from != ix {
            self.space_switched_at = Some(now());
        }
        self.current_runtime();
        if self.responsive.is_some() && !self.devtools_on() {
            self.set_responsive(None, cx);
        }
        self.space_shown(&[]);
        self.slide_from(from, old);
        self.pick_active(window, cx);
        if burning && !self.model.burner_shown() {
            self.burn(cx);
        }
        cx.notify();
    }

    pub fn open_private_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.model.private_space(&self.core) {
            self.switch_space(ix, window, cx);
        }
    }
}
