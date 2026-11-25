use super::*;

impl Browser {
    pub(super) fn on_event(&mut self, key: TabKey, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        // A local-network refusal is counted once, as `Event::Protected`.
        match &event {
            Event::Blocked { category: Some(_), .. } => self.count_on_page(key, Counter::ListedStopped, cx),
            Event::PopupBlocked(url) if !url.as_ref().is_some_and(shield::rules::local_target) => self.count_on_page(key, Counter::PopupsBlocked, cx),
            _ => {}
        }
        match event {
            Event::Loading(on) => {
                self.pages.entry(key).or_default().loading = on;
                if !on {
                    self.find_loaded(key, cx);
                }
            }
            Event::LinkHovered(link) => return self.link_hovered(key, link.as_deref(), cx),
            Event::AssistantSelection(command) => return self.ask_selection(key, command, cx),
            Event::Committed { url, title, can_go_back, can_go_forward } => {
                self.link_hovered(key, None, cx);
                engine::startup::mark("first page commit");
                // A link stays the page's until it shows a page: a view slept or never made loads it as a link again.
                if url.as_str() != "about:blank" {
                    self.link_loads.remove(&key);
                    self.pages.entry(key).or_default().retry = None;
                }
                self.drop_prompts(key, cx);
                self.tarn_committed(key, &url, window, cx);
                self.json_pages.remove(&key);
                let previous = self.store(key.private).tab(key.id).ok().and_then(|t| tab_host(&t.url));
                self.page_changed(key, &url, &title, can_go_back, can_go_forward, true);
                self.sync_assistant_menu(key);
                // An Open Anyway offer belongs to the page that was refused, not to the next one; the
                // blank document a new view starts with is no page.
                if let Some(page) = self.pages.get_mut(&key).filter(|p| url.as_str() != "about:blank" && matches!(p.offer, Some(Offer::Open(_)))) {
                    (page.notice, page.offer) = (None, None);
                }
                let left_host = previous.as_deref() != url.host_str();
                self.apply_site_zoom(key, Some(&url), left_host);
                self.apply_site_mute(key, &url, left_host);
                let page = self.pages.entry(key).or_default();
                page.blocked = None;
                page.visit = None;
                (page.local_counted, page.https_counted) = (false, false);
                self.forget_article(key, window, cx);
                let unfiltered = self.views.get(&key).is_some_and(|v| !v.is_chromium() && !v.filtered());
                if unfiltered && !crate::blocking::site_host(&url).is_some_and(|h| self.site_paused_in(key, h)) {
                    let message = "Ad, tracker and cookie-banner blocking is off in this tab since it left a site where you paused it.".to_owned();
                    self.notify_offer(key, message, Some(Offer::Reload));
                }
            }
            Event::Finished { url, title, can_go_back, can_go_forward } => {
                self.page_changed(key, &url, &title, can_go_back, can_go_forward, false);
                let page = self.pages.entry(key).or_default();
                page.crashed = false;
                if let Some((_, offset)) = page.scroll_back.take().filter(|(slept, _)| slept == url.as_str())
                    && let Some(view) = self.views.get(&key)
                {
                    view.scroll_to(offset);
                }
                let typed = suggest::typed_visit(&mut page.typed, url.as_str());
                self.record_visit(key, &url, &title, typed);
                self.request_page_text(key, &url);
                self.request_favicon(key, &url);
                #[cfg(feature = "translation")]
                self.offer_translation(key, &url, cx);
                self.probe_article(key, &url);
                self.save_state(key);
                self.after_load_devtools(key, &url);
                self.find_loaded(key, cx);
            }
            Event::Blocked { url, message, category } => {
                suggest::typed_visit(&mut self.pages.entry(key).or_default().typed, url.as_str());
                match category {
                    Some(category) if category.warns() => self.show_blocked(key, url, category),
                    Some(_) => self.notify_offer(key, message, Some(Offer::Open(url))),
                    // Open Anyway loads it as the user's own navigation, which S10 lets through; the
                    // notice names the address, since the page chose it.
                    None if message == nav_policy::LOCAL_NETWORK_BLOCKED => match nav_policy::open_anyway_notice(&url) {
                        Some(notice) => self.notify_offer(key, notice, Some(Offer::Open(url))),
                        None => self.notify_offer(key, message, None),
                    },
                    None => self.notify_offer(key, message, None),
                }
            }
            Event::StoreInstall(id) => self.store_link(key, id, window, cx),
            Event::PopupBlocked(url) => {
                let message = nav_policy::popup_blocked_notice(url.as_ref());
                let offer = url.filter(|u| nav_policy::decide(u, Source::Typed) == Decision::Allow).map(Offer::NewTab);
                self.notify_offer(key, message, offer);
            }
            Event::PopupOffered(url) => {
                let message = nav_policy::popup_blocked_notice(Some(&url));
                let offer = (nav_policy::decide(&url, Source::Typed) == Decision::Allow).then_some(Offer::NewTab(url));
                self.notify_offer(key, message, offer);
            }
            Event::WebEngineOutdated => {
                #[cfg(any(feature = "chromium", windows))]
                crate::shared::check_chromium(cx);
                #[cfg(windows)]
                if let Some(message) = self.chromium_closed() {
                    self.notify_offer(key, message.to_owned(), None);
                }
            }
            Event::WebEngineRequired(url) => {
                let host = |u: &Url| ui::text::bounded_origin(u.host_str().unwrap_or_default());
                let target = host(&url);
                let message = match self.views.get(&key).and_then(|v| v.url()).filter(|u| u.host_str().is_some()) {
                    Some(from) => format!("{} asked to open {target}. This site is set to Lockdown and opens in WebKit.", host(&from)),
                    None => format!("{target} is set to Lockdown and opens in WebKit."),
                };
                self.notify_offer(key, message, Some(Offer::WebKit(url)));
            }
            Event::Failed { url, code, message } => {
                // The latches stay: a page can make a load fail at will while it keeps showing, and an
                // undercount after a failed upgrade beats a counter a page can run up.
                let page = self.pages.entry(key).or_default();
                page.loading = false;
                if let Some(url) = url {
                    suggest::typed_visit(&mut page.typed, url.as_str());
                }
                if !IGNORED_LOAD_ERRORS.contains(&code) {
                    page.error = Some(message);
                }
                self.find_loaded(key, cx);
            }
            Event::TitleChanged(title) => {
                let unchanged = self.model.tab(key).is_some_and(|t| t.title == title);
                if !unchanged {
                    self.store(key.private).set_tab_title(key.id, &title).ok();
                }
                // WebKit can report an empty title for the outgoing page just before the next one commits.
                if let (false, Some(profile_id), Some((visit, url))) =
                    (title.is_empty(), self.profile_of(key), self.pages.get(&key).and_then(|p| p.visit.clone()))
                {
                    self.store(key.private).retitle_visit(profile_id, visit, &url, &title).ok();
                }
                if unchanged {
                    return;
                }
                if self.private_matches(key)
                    && let Some(t) = self.model.tabs.iter_mut().find(|t| t.id == key.id)
                {
                    t.title = title;
                }
            }
            Event::UrlChanged { url, can_go_back, can_go_forward } => {
                let page = self.pages.entry(key).or_default();
                page.can_back = can_go_back;
                page.can_forward = can_go_forward;
                if let Ok(tab) = self.store(key.private).tab(key.id)
                    && tab.url != url.as_str()
                    && nav_policy::decide(&url, Source::Link) == Decision::Allow
                {
                    self.store(key.private).update_tab_page(key.id, url.as_str(), &tab.title).ok();
                    if self.private_matches(key)
                        && let Some(t) = self.model.tabs.iter_mut().find(|t| t.id == key.id)
                    {
                        t.url = url.to_string();
                    }
                    // A same-document navigation (pushState) is a visit like a page load, but pages
                    // rewrite their URL constantly (maps, anchors, search as you type): fragment
                    // changes, very long URLs and more than one visit per tab per interval are not
                    // recorded.
                    let mut previous = Url::parse(&tab.url).ok();
                    if let Some(p) = previous.as_mut() {
                        p.set_fragment(None);
                    }
                    let mut target = url.clone();
                    target.set_fragment(None);
                    // A page that rewrites its own address shows another article, or none; it is asked
                    // again once it has had a moment to render, as no load finishes to tell.
                    if previous.as_ref() != Some(&target) {
                        self.forget_article(key, window, cx);
                        let url = url.clone();
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                            this.update(cx, |b, _| {
                                if b.views.get(&key).and_then(|v| v.url()).as_ref() == Some(&url) {
                                    b.probe_article(key, &url);
                                }
                            })
                            .ok();
                        })
                        .detach();
                    }
                    let page = self.pages.entry(key).or_default();
                    let due = page.last_visit.is_none_or(|at| at.elapsed() >= SAME_DOCUMENT_VISIT_INTERVAL);
                    if previous.as_ref() != Some(&target) && url.as_str().len() <= MAX_SAME_DOCUMENT_URL && due {
                        page.last_visit = Some(Instant::now());
                        self.record_visit(key, &url, &tab.title, false);
                    } else {
                        page.visit = None;
                    }
                }
            }
            Event::AgentHeld(url) => crate::agent::held_navigation(key, url, cx),
            Event::Prompt(prompt)
                if self.model.agent_profile.is_some() && (prompt.kind.is_permission() || matches!(prompt.kind, PromptKind::ClientCertificate { .. })) =>
            {
                drop(prompt)
            }
            Event::Prompt(prompt) => {
                // A prompt shows with the shell's origin chip, never over a full-screen page.
                if self.model.active == Some(key) {
                    self.leave_page_fullscreen(window);
                }
                self.queue_prompt(key, prompt, window, cx);
            }
            Event::Popup(popup) => self.adopt_popup(key, popup, window, cx),
            Event::ShieldLevelChanged(url) => self.rebuild_view(key, &url, cx),
            Event::RebuildForLink(url) => self.rebuild_view_for_link(key, &url, cx),
            Event::PeekLink(url, from) => {
                if self.model.active == Some(key) && self.model.agent_profile.is_none() && crate::agent::gated(key, cx).is_none() {
                    self.open_link_peek(key, url, from, window, cx);
                }
            }
            // As a pop-up's tab: never in the burner, and its first load is the page's link, so S10 judges every hop.
            Event::OpenTab { url, from, background, split } => {
                let Some(opener) = self.model.tab(key).filter(|_| !self.model.burner_shown() && self.model.agent_profile.is_none()) else { return cx.notify() };
                let gate = crate::agent::gated(key, cx);
                if gate.as_ref().is_some_and(|gate| !crate::agent::may_open(gate, &url, cx)) {
                    return cx.notify();
                }
                let engine = opener.web_engine.or(self.saved.web_engine);
                let routed = if gate.is_some() { None } else { self.routed_space(&url) };
                if let Some(ix) = routed {
                    self.switch_space(ix, window, cx);
                }
                if let Some(tab) = self.model.open_tab(&self.core, url.as_str(), engine, Opened::Link(key)) {
                    if gate.is_some() {
                        crate::agent::touch(tab, cx);
                    }
                    self.reload_space();
                    self.link_loads.insert(tab, from.clone());
                    if split && routed.is_none() {
                        self.split_link(key, tab, window, cx);
                    } else if !background || routed.is_some() {
                        self.activate(tab, window, cx);
                    } else if let Some(view) = self.create_view(tab, url.as_str(), cx) {
                        // Loaded now, as a pop-up is: a tab restored later would load its URL as the user's own.
                        view.set_visible(false);
                        view.load_link(&url, from.as_ref());
                    }
                }
            }
            Event::ExternalScheme { url, origin } => {
                let showing = self.pending_external.as_ref().is_some_and(|p| p.tab.is_none_or(|t| t == key));
                if self.model.active == Some(key) && !showing {
                    self.pending_external = Some(PendingExternal { tab: Some(key), origin, url });
                    self.layout_changed();
                }
            }
            event @ (Event::DownloadFinished { .. }
            | Event::DownloadFailed { .. }
            | Event::DownloadCancelled(_)
            | Event::DownloadsActive(_)
            | Event::DownloadProgress { .. }) => {
                if let Some(profile_id) = self.profile_of(key) {
                    self.core.downloads.unbounded_send((key.private, profile_id, event)).ok();
                }
                return;
            }
            Event::Protected(protection, target) => {
                let page = self.pages.entry(key).or_default();
                let (latch, counter) = match protection {
                    engine::Protection::HttpsUpgrade => (&mut page.https_counted, Counter::HttpsUpgrades),
                    engine::Protection::LocalNetwork => (&mut page.local_counted, Counter::LocalBlocked),
                };
                let counter = (!std::mem::replace(latch, true)).then_some(counter);
                // An upgrade is the upgraded site's; a refusal is the page's that tried it, unless the
                // tab shows nothing but the refused address (a link from another app).
                let page = self.tab_url(key).and_then(|u| Url::parse(&u).ok()).filter(|u| !shield::rules::local_target(u));
                let host = target.or_else(|| page.as_ref().and_then(page_host));
                if let (Some(counter), Some(profile_id)) = (counter, self.profile_of(key)) {
                    self.count_shield(key.private, profile_id, counter, 1, host.as_deref(), cx);
                }
            }
            Event::FullScreen(on) => self.set_page_fullscreen(key, on, window),
            Event::ExtensionMessage { world, frame, origin, json } => {
                if let Some(profile) = self.profile_of(key).filter(|_| !key.private) {
                    self.core.tarn.borrow().send(crate::tarn_ext::Msg::Tab { profile, tab: key.id as u64, world, frame, origin, json });
                }
                return;
            }
            Event::ContentProcessTerminated => {
                self.link_hovered(key, None, cx);
                if self.model.active == Some(key) {
                    self.leave_page_fullscreen(window);
                }
                self.drop_prompts(key, cx);
                let page = self.pages.entry(key).or_default();
                if page.crashed {
                    page.loading = false;
                    page.error = Some("The page crashed again and was not reloaded.".into());
                } else if let Some(view) = self.views.get(&key) {
                    page.crashed = true;
                    view.recover();
                }
            }
        }
        cx.notify();
    }

    /// The active tab's page alone fills the window, in full screen, while it shows an element
    /// full screen; leaving restores the window as it was.
    pub(super) fn set_page_fullscreen(&mut self, key: TabKey, on: bool, window: &mut Window) {
        if self.model.active != Some(key) {
            return;
        }
        if !on {
            return self.leave_page_fullscreen(window);
        }
        let Some(view) = self.views.get(&key).cloned() else { return };
        // Nothing the shell must show may be hidden: a pending prompt keeps the toolbar and its origin.
        let Some(entered) = self.page_fullscreen.enter(self.active_prompt().is_some(), window.is_fullscreen()) else { return };
        if entered {
            window.toggle_fullscreen();
        }
        view.set_corner_radius(0.);
        view.raise();
        self.layout_changed();
    }

    /// Whether the window may show the full-screen page alone: no prompt, overlay, address field,
    /// find bar or notice of the shell waits to be seen, and the page is still the active one.
    pub(crate) fn page_fullscreen_holds(&self) -> bool {
        browser_core::fullscreen::Shell {
            page: self.active_view().is_some(),
            prompt: self.active_prompt().is_some(),
            overlay: self.overlay_open(),
            address: self.command_mode.is_some(),
            find: self.find_open,
            notice: self.model.active.and_then(|k| self.pages.get(&k)).is_some_and(|p| p.notice.is_some() || p.blocked.is_some()),
        }
        .lets_page_fill_window()
    }

    /// Gives the window back its sidebar, toolbar and size: when the page leaves full screen, and
    /// when a prompt, another tab or a crash needs the shell. The page itself may stay full screen
    /// inside the tab until it is put away.
    pub(crate) fn leave_page_fullscreen(&mut self, window: &mut Window) {
        let Some(entered) = self.page_fullscreen.leave() else { return };
        if entered && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        if let Some(view) = self.active_view() {
            view.set_corner_radius(page_corner());
        }
        self.layout_changed();
    }

    pub(super) fn record_visit(&mut self, key: TabKey, url: &Url, title: &str, typed: bool) {
        if let Some(profile_id) = self.profile_of(key).filter(|_| self.model.agent_profile.is_none())
            && matches!(url.scheme(), "http" | "https")
            && self.core.keeps_records(key.private, profile_id)
        {
            let visit = self.store(key.private).add_history(profile_id, url.as_str(), title, now(), typed).ok().map(|id| (id, url.to_string()));
            self.menu.visited(url.as_str());
            self.pages.entry(key).or_default().visit = visit;
        }
    }

    /// Only after a full load: a same-document visit would read the previous route's text.
    pub(super) fn request_page_text(&mut self, key: TabKey, url: &Url) {
        let visit = self.pages.get(&key).and_then(|p| p.visit.clone()).filter(|(_, u)| u == url.as_str());
        let (Some(profile_id), Some((visit, page)), Some(view)) = (self.profile_of(key), visit, self.views.get(&key)) else { return };
        if key.private || !self.pref(crate::prefs::Pref::HistoryText) || self.level_for_tab(key, url.as_str()) == ShieldLevel::Lockdown {
            return;
        }
        let events = self.events.clone();
        view.page_text(url, store::MAX_HISTORY_TEXT, move |_, text| {
            events.unbounded_send(Msg::PageText(profile_id, visit, page.clone(), text)).ok();
        });
    }

    /// Only the disk store keeps page text, so the private space never does.
    pub(super) fn store_page_text(&mut self, profile_id: Id, visit: Id, url: &str, text: String) {
        if text.is_empty() || !self.pref(crate::prefs::Pref::HistoryText) {
            return;
        }
        self.core.disk.set_history_text(profile_id, visit, url, store::clip_history_text(&text)).ok();
    }

    /// Replaces the tab's view with one built for `url`'s Shield level and rules, then loads it.
    pub(crate) fn rebuild_view(&mut self, key: TabKey, url: &Url, cx: &mut Context<Self>) {
        self.rebuild_view_with(key, url, None, cx);
    }

    /// Rebuilds the tab's view for a page's navigation, which the new view judges as the page's link.
    pub(super) fn rebuild_view_for_link(&mut self, key: TabKey, url: &Url, cx: &mut Context<Self>) {
        let opener = self.link_loads.get(&key).cloned().unwrap_or_else(|| self.views.get(&key).and_then(|v| v.url()));
        self.rebuild_view_loading(key, url, None, Some(opener), cx);
    }

    /// Rebuilds the tab's view, handing `storage` to the new one before its first load.
    pub(crate) fn rebuild_view_with(&mut self, key: TabKey, url: &Url, storage: Option<engine::handover::Storage>, cx: &mut Context<Self>) {
        self.rebuild_view_loading(key, url, storage, None, cx);
    }

    /// With `link`, the URL loads as a link from that page (or from none), not as typed.
    pub(super) fn rebuild_view_loading(
        &mut self,
        key: TabKey,
        url: &Url,
        storage: Option<engine::handover::Storage>,
        link: Option<Option<Url>>,
        cx: &mut Context<Self>,
    ) {
        if self.model.tab(key).is_some() && self.views.contains_key(&key) {
            self.forget_media(key);
            self.views.remove(&key);
            // The old view's last events are dropped; the new one says when its load starts.
            let page = self.pages.entry(key).or_default();
            page.loading = false;
            page.retry = link.is_none().then(|| url.clone());
            self.drop_prompts(key, cx);
            let active = self.model.active == Some(key);
            let visible = (active || self.in_split(key)) && !self.overlay_open();
            if let Some(view) = self.create_view(key, url.as_str(), cx) {
                if let Some(storage) = storage {
                    view.hand_over(storage);
                }
                match &link {
                    None if self.pages.get(&key).and_then(|p| p.opened_file.as_ref()) == Some(url) => view.load_file(url),
                    Some(opener) => view.load_link(url, opener.as_ref()),
                    None => view.load(url),
                }
                view.set_visible(visible);
                if visible && active {
                    view.focus();
                }
            }
        }
    }

    pub(super) fn site_paused_in(&self, key: TabKey, host: &str) -> bool {
        self.profile_of(key).and_then(|p| self.loaded(key.private, p)).is_some_and(|rt| rt.rules.paused.borrow().contains(host))
    }

    /// Shield level for `url` under the profile that owns `key`, which may not be the current space's.
    pub fn level_for_tab(&self, key: TabKey, url: &str) -> ShieldLevel {
        self.profile_of(key).and_then(|p| self.core.level(key.private, p, Url::parse(url).ok().as_ref())).unwrap_or(ShieldLevel::Lockdown)
    }

    pub(crate) fn queue_prompt(&mut self, key: TabKey, prompt: Prompt, window: &mut Window, cx: &mut Context<Self>) {
        if !self.views.contains_key(&key) {
            return;
        }
        if let Some(permission) = prompt.kind.permission_key() {
            let page = prompt.top_origin.as_deref().unwrap_or(&prompt.origin);
            let level = self.level_for_tab(key, page).max(self.level_for_tab(key, &prompt.origin));
            if level == ShieldLevel::Lockdown {
                return;
            }
            let stored = (level == ShieldLevel::Standard).then(|| self.profile_of(key)).flatten().and_then(|p| {
                self.store(key.private).permission(p, &permission_origin(&prompt.origin, prompt.top_origin.as_deref()), permission).ok().flatten()
            });
            match stored.as_deref() {
                Some("allow") => return prompt.accept("", None),
                Some("deny") => return,
                _ => {}
            }
        }
        let queue = self.prompts.entry(key).or_default();
        if queue.len() >= MAX_QUEUED_PROMPTS {
            return;
        }
        queue.push_back(prompt);
        if queue.len() == 1 && self.model.active == Some(key) {
            self.prepare_prompt(window, cx);
        }
    }

    /// Discards a tab's prompts (each declines on drop) and its external-app request, and clears what
    /// the user typed into them.
    pub(super) fn drop_prompts(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if self.pending_external.as_ref().is_some_and(|p| p.tab == Some(key)) {
            self.pending_external = None;
        }
        if self.prompts.remove(&key).is_none_or(|q| q.is_empty()) || self.model.active != Some(key) {
            return;
        }
        for input in [&self.prompt_input, &self.auth_user, &self.auth_pass] {
            input.update(cx, |i, cx| i.set_text("", cx));
        }
        self.refocus = true;
        if self.model.active == Some(key)
            && let Some(view) = self.views.get(&key)
        {
            view.focus();
        }
    }

    /// The find, prompt or command field has focus while the bar or card holding it is not drawn.
    pub(crate) fn hidden_field_focused(&self, window: &Window, cx: &App) -> bool {
        let focused = |input: &Entity<TextInput>| input.focus_handle(cx).is_focused(window);
        let card = !self.overlay_open();
        (focused(&self.find_input) && !(card && self.find_open && self.active_json().is_none()))
            || ([&self.prompt_input, &self.auth_user, &self.auth_pass].into_iter().any(focused) && !(card && self.active_prompt().is_some()))
            || (focused(&self.command) && self.command_mode.is_none())
    }

    pub fn active_prompt(&self) -> Option<&Prompt> {
        self.prompts.get(&self.model.active?)?.front()
    }

    pub(super) fn prepare_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_open() {
            return;
        }
        let Some(kind) = self.active_prompt().map(|p| p.kind.clone()) else { return };
        self.remember_permission = false;
        self.layout_changed();
        if let Some(host) = &self.host {
            host.focus();
        }
        let focus = match kind {
            PromptKind::Text { default } => {
                self.prompt_input.update(cx, |i, cx| i.set_text(default, cx));
                self.prompt_input.focus_handle(cx)
            }
            PromptKind::Credentials { .. } => {
                self.auth_user.update(cx, |i, cx| i.set_text("", cx));
                self.auth_pass.update(cx, |i, cx| i.set_text("", cx));
                self.auth_user.focus_handle(cx)
            }
            _ => self.focus.clone(),
        };
        window.focus(&focus);
        cx.notify();
    }

    pub fn answer_prompt(&mut self, accept: bool, silence: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        if accept && self.prompt_guarded() {
            return;
        }
        let Some(prompt) = self.prompts.get_mut(&key).and_then(VecDeque::pop_front) else { return };
        let page = prompt.top_origin.clone().unwrap_or_else(|| prompt.origin.clone());
        if matches!(prompt.kind, PromptKind::ExtensionAccess { .. }) {
            match (self.remember_permission, silence) {
                (true, false) => prompt.remember(accept),
                (false, false) if accept => prompt.accept("", None),
                _ => prompt.decline(),
            }
            return self.after_prompt(window, cx);
        }
        if let Some(permission) = prompt.kind.permission_key()
            && self.remember_permission
            && self.level_for_tab(key, &page).max(self.level_for_tab(key, &prompt.origin)) == ShieldLevel::Standard
            && let Some(profile_id) = self.profile_of(key)
        {
            let decision = if accept { "allow" } else { "deny" };
            self.store(key.private).set_permission(profile_id, &permission_origin(&prompt.origin, prompt.top_origin.as_deref()), permission, decision).ok();
        }
        match (accept, silence) {
            (_, true) => {
                prompt.decline_and_silence_page();
                let queue = self.prompts.get_mut(&key);
                if queue.is_none_or(|q| {
                    q.retain(|p| p.kind == PromptKind::ResendForm);
                    q.is_empty()
                }) {
                    self.drop_prompts(key, cx);
                }
            }
            (true, _) => {
                let text = self.prompt_input.read(cx).text().to_owned();
                let user = self.auth_user.read(cx).text().to_owned();
                let pass = self.auth_pass.read(cx).text().to_owned();
                let credentials = matches!(prompt.kind, PromptKind::Credentials { .. }).then_some((user.as_str(), pass.as_str()));
                prompt.accept(&text, credentials);
            }
            (false, _) => prompt.decline(),
        }
        self.after_prompt(window, cx);
    }

    pub(crate) fn choose_certificate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        if self.prompt_guarded() {
            return;
        }
        let Some(prompt) = self.prompts.get_mut(&key).and_then(VecDeque::pop_front) else { return };
        prompt.choose(index);
        self.after_prompt(window, cx);
    }

    /// Esc declines the prompt in front, as Cancel or Deny does, without remembering the answer.
    pub(crate) fn dismiss_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_permission = false;
        self.answer_prompt(false, false, window, cx);
    }

    /// Enter answers an alert or confirm whose card holds the focus; permission and download
    /// warnings stay mouse-only, behind their click guard.
    pub(crate) fn prompt_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let simple = self.active_prompt().is_some_and(|p| matches!(p.kind, PromptKind::Alert | PromptKind::Confirm));
        let plain = !event.keystroke.modifiers.modified();
        if event.keystroke.key == "enter" && !event.is_held && plain && simple && !self.overlay_open() && self.focus.is_focused(window) {
            cx.stop_propagation();
            self.answer_prompt(true, false, window, cx);
        }
    }

    pub(super) fn after_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.prompt_input, &self.auth_user, &self.auth_pass] {
            input.update(cx, |i, cx| i.set_text("", cx));
        }
        if self.active_prompt().is_some() {
            self.prepare_prompt(window, cx);
        } else {
            window.focus(&self.focus);
            if let Some(view) = self.active_view() {
                view.focus();
            }
            cx.notify();
        }
    }

    /// A permission prompt's Allow and Remember, Keep Anyway on a malware warning, Send Again on a form resend and a certificate choice ignore clicks for a
    /// moment after they appear, so a page cannot time a click to land on them.
    pub(crate) fn prompt_guarded(&self) -> bool {
        self.active_prompt().is_some_and(|p| {
            p.kind.is_permission() || matches!(p.kind, PromptKind::MaliciousDownload { .. } | PromptKind::ResendForm | PromptKind::ClientCertificate { .. })
        }) && !self.layout_stable()
    }

    pub(super) fn on_prompt_input_event(&mut self, input: &Entity<TextInput>, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Submit if input == &self.auth_user => {
                let focus = self.auth_pass.focus_handle(cx);
                window.focus(&focus);
            }
            TextInputEvent::Submit => self.answer_prompt(true, false, window, cx),
            TextInputEvent::Cancel => self.answer_prompt(false, false, window, cx),
            _ => {}
        }
    }

    /// In the reader, finds on the page it was made from, which the reader then gives way to.
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find(true, window, cx);
    }

    /// Opens the find bar; `search` runs its kept text again when the bar was closed.
    pub(crate) fn show_find(&mut self, search: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_view().is_none() || self.card_open() || self.active_json().is_some() {
            return;
        }
        if self.reader_shown() {
            self.toggle_reader(window, cx);
        }
        let reopened = !std::mem::replace(&mut self.find_open, true);
        self.find_count = None;
        let text = self.find_input.read(cx).text().to_owned();
        self.find_input.update(cx, |i, cx| i.set_text(text, cx));
        if reopened && search {
            self.find(false, cx);
        }
        if let Some(host) = &self.host {
            host.focus();
        }
        let focus = self.find_input.focus_handle(cx);
        window.focus(&focus);
        cx.notify();
    }

    /// Runs the search `find_after_load` held for `key` once its load has ended, from the top.
    fn find_loaded(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if self.find_after_load.take_if(|k| *k == key).is_some()
            && self.find_open
            && let Some(view) = self.active_view().filter(|_| self.model.active == Some(key))
        {
            view.clear_find();
            self.find(false, cx);
        }
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        self.find_typing = None;
        window.focus(&self.focus);
        if let Some(view) = self.active_view() {
            view.clear_find();
            view.focus();
        }
        cx.notify();
    }

    pub fn find(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let text = self.find_input.read(cx).text().to_owned();
        let (Some(key), Some(view)) = (self.model.active, self.active_view().filter(|_| !text.is_empty())) else { return };
        let events = self.events.clone();
        view.find(&text, backwards, move |found, current| {
            events.unbounded_send(Msg::Found(key, found, current)).ok();
        });
    }

    pub(super) fn on_find_event(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Submit | TextInputEvent::Down => {
                self.find_typing = None;
                self.find(false, cx)
            }
            TextInputEvent::Up => {
                self.find_typing = None;
                self.find(true, cx)
            }
            TextInputEvent::Cancel => self.close_find(window, cx),
            TextInputEvent::Changed => {
                self.find_count = None;
                if self.find_input.read(cx).text().is_empty() {
                    self.find_typing = None;
                    if let Some(view) = self.active_view() {
                        view.clear_find();
                    }
                } else {
                    self.find_typing = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(browser_core::find::TYPING_DELAY).await;
                        this.update(cx, |b, cx| {
                            if b.find_open {
                                b.find(false, cx);
                            }
                        })
                        .ok();
                    }));
                }
                cx.notify();
            }
            TextInputEvent::MenuClosed | TextInputEvent::PastEnd => {}
        }
    }

    pub fn print(&mut self) {
        if let Some(view) = self.active_view() {
            view.print();
        }
    }

    pub(super) fn adopt_popup(&mut self, opener: TabKey, popup: Popup, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else { return };
        if self.model.tab(opener).is_none() || self.model.burner_shown() || self.model.agent_profile.is_some() {
            return;
        }
        let gate = crate::agent::gated(opener, cx);
        if let Some(gate) = &gate
            && !popup.url.as_ref().is_some_and(|url| crate::agent::may_open(gate, url, cx))
        {
            return;
        }
        // A script's window.open keeps its opener, which sign-in and payment pop-ups need, so only links are routed.
        if gate.is_none() && popup.link && popup.url.clone().is_some_and(|url| self.open_routed(&url, window, cx)) {
            return;
        }
        let Some(space) = self.model.current_space() else { return };
        let (private, space_id) = (space.private, space.space.id);
        let url = popup.url.as_ref().map_or("about:blank".to_owned(), Url::to_string);
        let Ok(tab) = self.store(private).create_tab(space_id, TabKind::Ephemeral, None, &url, "") else {
            return;
        };
        let key = TabKey { private, id: tab.id };
        if gate.is_some() {
            crate::agent::touch(key, cx);
        }
        // On macOS only WebKit views adopt pop-ups, so an opener with an engine of its own ran in WebKit.
        if cfg!(target_os = "macos") {
            self.store(private).set_tab_web_engine(tab.id, self.model.tab(opener).and_then(|t| t.web_engine).map(|_| WebEngine::WebKit)).ok();
        }
        let inspectable = self.developer || self.is_dev_tab(opener);
        let Some(view) = WebView::adopt(popup, &host, inspectable, self.sink(key)) else {
            self.store(private).delete_tab(tab.id).ok();
            return;
        };
        view.set_corner_radius(page_corner());
        view.set_dark(!ui::theme::Theme::is_light());
        self.view_prefs(&view);
        crate::agent::gate_view(key, &view, cx);
        self.views.insert(key, Rc::new(view));
        self.link_loads.insert(key, None);
        self.attach_tarn(key);
        self.reload_space();
        self.activate(key, window, cx);
    }

    pub(super) fn page_changed(&mut self, key: TabKey, url: &Url, title: &str, back: bool, forward: bool, loading: bool) {
        let page = self.pages.entry(key).or_default();
        page.can_back = back;
        page.can_forward = forward;
        page.loading = loading;
        page.error = None;
        self.store(key.private).update_tab_page(key.id, url.as_str(), title).ok();
        if self.private_matches(key)
            && let Some(t) = self.model.tabs.iter_mut().find(|t| t.id == key.id)
        {
            t.url = url.to_string();
            t.title = title.to_owned();
        }
    }

    pub(super) fn private_matches(&self, key: TabKey) -> bool {
        key.private == self.model.private()
    }

    pub(crate) fn profile_of(&self, key: TabKey) -> Option<Id> {
        self.model.profile_of(&self.core, key)
    }

    pub fn back(&mut self) {
        if let Some(v) = self.active_view() {
            v.back();
        }
    }

    pub fn forward(&mut self) {
        if let Some(v) = self.active_view() {
            v.forward();
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let dev = self.model.active.is_some_and(|k| self.is_dev_tab(k));
        self.reload_page(dev, cx);
    }

    pub fn reload_page(&mut self, from_origin: bool, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        if !self.reload_view(key, from_origin) {
            return;
        }
        self.reload_spin = Some((key, Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(crate::toolbar::SPIN).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn reload_view(&self, key: TabKey, from_origin: bool) -> bool {
        let Some(v) = self.views.get(&key) else { return false };
        let page = self.pages.get(&key);
        let unloaded = (v.url().is_none() && !page.is_some_and(|p| p.loading))
            .then(|| page.and_then(|p| p.retry.clone()).or_else(|| self.model.tab(key).and_then(|t| Url::parse(&t.url).ok())))
            .flatten();
        if let Some(url) = unloaded {
            self.load_tab_url(key, v, url)
        } else if from_origin {
            v.reload_from_origin()
        } else {
            v.reload()
        }
        true
    }

    pub fn stop_loading(&mut self, cx: &mut Context<Self>) {
        self.reload_spin = None;
        if let Some(view) = self.active_view() {
            view.stop();
        }
        cx.notify();
    }

    pub(super) fn is_dev_tab(&self, key: TabKey) -> bool {
        self.profile_of(key).and_then(|p| self.loaded(key.private, p)).is_some_and(|rt| rt.profile.is_dev)
    }

    pub(super) fn after_load_devtools(&mut self, key: TabKey, url: &Url) {
        let dev = self.is_dev_tab(key);
        if dev && let Some(rt) = self.profile_of(key).and_then(|p| self.loaded(key.private, p)) {
            rt.data.clear_cache();
            #[cfg(feature = "chromium")]
            engine::chromium::clear_cache(&data_dir().join("Chromium"), &rt.own_store);
        }
        if !(self.developer || dev) {
            return;
        }
        let Some(view) = self.views.get(&key) else { return };
        let (events, page) = (self.events.clone(), url.clone());
        view.json_text(url, devkit::json_view::MAX_BYTES, move |text| {
            if !text.is_empty() {
                events.unbounded_send(Msg::Json(key, page.clone(), text)).ok();
            }
        });
    }

    pub(super) fn show_json(&mut self, key: TabKey, url: Url, text: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let page = cx.background_spawn(async move { JsonPage::parse(url, &text) }).await;
            this.update(cx, |b, cx| {
                let current = b.views.get(&key).and_then(|v| v.url());
                if let Ok(page) = page
                    && current.as_ref() == Some(&page.url)
                {
                    b.json_pages.insert(key, page);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn set_json_raw(&mut self, key: TabKey, raw: bool, cx: &mut Context<Self>) {
        if let Some(page) = self.json_pages.get_mut(&key) {
            page.set_raw(raw);
            cx.notify();
        }
    }

    pub fn toggle_json_node(&mut self, key: TabKey, path: Vec<usize>, cx: &mut Context<Self>) {
        if let Some(page) = self.json_pages.get_mut(&key) {
            page.toggle(path);
            cx.notify();
        }
    }

    pub fn active_json(&self) -> Option<(TabKey, &JsonPage)> {
        let key = self.model.active?;
        self.json_pages.get(&key).map(|p| (key, p))
    }

    pub fn current_is_dev(&self) -> bool {
        self.current_loaded().is_some_and(|rt| rt.profile.is_dev)
    }

    pub fn devtools_on(&self) -> bool {
        self.developer || self.current_is_dev()
    }

    /// Web Inspector availability is fixed when a view is created, so every live view is rebuilt.
    pub fn set_developer(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.core.disk.set_setting(DEVELOPER_SETTING, if on { "on" } else { "off" }).is_err() {
            return;
        }
        self.apply_developer(on, cx);
        crate::shared::others(cx, |b, _, cx| b.apply_developer(on, cx));
        let profiles: Vec<(bool, Id)> = self.core.profiles.borrow().iter().filter(|(_, rt)| !rt.profile.is_dev).map(|(k, _)| *k).collect();
        for (private, id) in profiles {
            self.restart_profile_views(private, id, None, false, window, cx);
        }
        cx.notify();
    }

    pub(super) fn apply_developer(&mut self, on: bool, cx: &mut Context<Self>) {
        self.developer = on;
        if on {
            self.refresh_local_servers(cx);
        } else if !self.current_is_dev() {
            self.set_responsive(None, cx);
        }
        cx.notify();
    }

    pub fn create_dev_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.core.disk.profiles().unwrap_or_default().into_iter().find(|p| p.is_dev);
        let space = match existing {
            Some(p) => self.core.disk.spaces(p.id).ok().and_then(|s| s.into_iter().next()),
            None => self.core.disk.create_dev_profile(&uuid::Uuid::new_v4().to_string()).ok().map(|(_, space)| space),
        };
        let Some(space) = space else { return };
        self.refresh_local_servers(cx);
        self.reload_spaces();
        let ix = self.model.spaces.iter().position(|s| !s.private && s.space.id == space.id).unwrap_or(0);
        self.switch_space(ix, window, cx);
    }

    pub fn refresh_local_servers(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let ports = cx.background_spawn(async { devkit::localhost::open_dev_ports() }).await;
            this.update(cx, |b, cx| {
                b.local_servers = Some(ports);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_local_server(&mut self, port: u16, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(url) = Url::parse(&format!("http://localhost:{port}/")) {
            self.new_tab(&url, window, cx);
        }
    }

    pub fn cycle_responsive(&mut self, cx: &mut Context<Self>) {
        let next = match self.responsive {
            None => Some(0),
            Some(i) if i + 1 < devkit::responsive::PRESETS.len() => Some(i + 1),
            Some(_) => None,
        };
        self.set_responsive(next, cx);
    }

    pub(super) fn set_responsive(&mut self, preset: Option<usize>, cx: &mut Context<Self>) {
        match (self.responsive.is_some(), preset.is_some()) {
            (false, true) => self.zoom_before_responsive = self.views.iter().map(|(k, v)| (*k, v.zoom())).collect(),
            (true, false) => {
                for (key, view) in &self.views {
                    view.set_zoom(self.zoom_before_responsive.get(key).copied().unwrap_or(1.0));
                }
                self.zoom_before_responsive.clear();
            }
            _ => {}
        }
        self.responsive = preset;
        cx.notify();
    }

    /// The preset applies only while the developer tools that control it are shown.
    pub fn responsive_preset(&self) -> Option<devkit::responsive::Preset> {
        self.responsive.filter(|_| self.devtools_on()).and_then(|i| devkit::responsive::PRESETS.get(i)).copied()
    }

    pub(super) fn responsive_active(&self) -> bool {
        self.responsive_preset().is_some()
    }

    /// The first click asks; a second click on the same tab and host clears.
    pub fn clear_site_data(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let Some(host) = self.active_view().and_then(|v| v.url()).and_then(|u| u.host_str().map(str::to_owned)) else { return };
        if self.clear_site_armed.take() != Some((key, host.clone())) {
            self.clear_site_armed = Some((key, host.clone()));
            self.notify_offer(key, format!("Click Clear Site Data again to remove cookies, storage and caches of {host}."), None);
            cx.notify();
            return;
        }
        self.clear_site(key, host, cx);
    }

    /// Removes every WebKit data type of `host` in the tab's profile, then reloads the tab.
    pub(crate) fn clear_site(&mut self, key: TabKey, host: String, cx: &mut Context<Self>) {
        if let Some(profile_id) = self.profile_of(key).filter(|_| !key.private)
            && self.core.disk.unzap_element(profile_id, &host, None).is_ok()
        {
            crate::zaps::apply(&self.core, false, profile_id);
        }
        let Some(rt) = self.profile_of(key).and_then(|p| self.loaded(key.private, p)) else { return };
        #[cfg(feature = "chromium")]
        self.clear_chromium_site(&rt.own_store, &host);
        let this = cx.entity().downgrade();
        let mut async_cx = cx.to_async();
        rt.data.clear_site_data(&host.clone(), move |count| {
            this.update(&mut async_cx, |b, cx| {
                let message = if count == 0 { format!("{host} had no stored data.") } else { format!("Cleared stored data for {host}.") };
                b.notify_offer(key, message, None);
                b.reload_view(key, false);
                cx.notify();
            })
            .ok();
        });
    }
}
