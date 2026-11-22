use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use browser_core::staleness::Staleness;
use browser_core::tabs::custom_title;
use engine::nav_policy::{self, Decision, Source};
use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{Either, select};
use gpui::{Context, PromptLevel, Window, actions};
use shield::ShieldLevel;
use store::{SavedWindow, Tab, TabKind, WebEngine};
use url::Url;

#[cfg(feature = "chromium")]
use crate::blocking::Offer;
use crate::browser::{Browser, TabKey};
use crate::memory::Cost;

actions!(tarn, [RenameTab, ResetTabTitle, ReturnToPinnedUrl, SetPinnedUrl, SleepOtherTabs, ReloadInChromium, ReloadInWebKit, CloseDuplicateTabs]);

pub(crate) const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const HEAVY_TABS: usize = 3;

/// Set once the user has read `GOOGLE_NOTICE`.
const GOOGLE_NOTICE_SEEN: &str = "chromium_notice_seen";
const GOOGLE_NOTICE: &str = "Chromium makes requests of its own to Google: security data (certificate revocation and the root store) and installing and updating Chrome extensions. Google sees your IP address for these requests.";

/// The label of a tab command named by the tab's state, alike in the tab menu, the menu bar and
/// the command bar: `on` is whether the tab is pinned, its site muted or the tab in a split.
pub(crate) fn state_label(action: &dyn gpui::Action, on: bool) -> Option<&'static str> {
    let is = |a: &dyn gpui::Action| action.partial_eq(a);
    Some(if is(&crate::TogglePin) {
        if on { "Unpin Tab" } else { "Pin Tab" }
    } else if is(&crate::menubar::ToggleMute) {
        if on { "Unmute Site" } else { "Mute Site" }
    } else if is(&crate::split::ToggleSplitView) {
        if on { "Close Split View" } else { "Open in Split View" }
    } else {
        return None;
    })
}

pub fn web_engine_name(engine: WebEngine) -> &'static str {
    match engine {
        WebEngine::WebKit => "WebKit",
        WebEngine::Chromium => "Chromium",
    }
}

/// The name a tab shows: the user's own, the page title, an opened file's name, the host or
/// scheme, or "New Tab".
pub fn display_title(tab: &Tab) -> String {
    browser_core::command::display_title(&tab.custom_title, &tab.title, &tab.url)
}

impl Browser {
    /// Shows this window's burner tab: a throwaway space at Strict whose non-persistent data
    /// store is its own; leaving it, closing its tab or the window burns it.
    pub fn open_burner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.model.burner_shown() {
            self.burn(cx);
            let back = self.model.current_space().map(|s| (s.private, s.space.id));
            let Some(ix) = self.model.open_burner(&self.core) else { return };
            self.burner_return = back;
            self.switch_space(ix, window, cx);
        }
        if self.model.tabs.is_empty() {
            self.open_command(crate::browser::CommandMode::NewTab, window, cx);
        }
    }

    /// Goes back to the space the burner was opened from, which burns it.
    pub(crate) fn leave_burner(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let back = self.burner_return.take();
        let burner = self.model.burner;
        let ix = self.model.spaces.iter().position(|s| Some((s.private, s.space.id)) == back && Some(s.space.id) != burner);
        let ix = ix.or_else(|| self.model.spaces.iter().position(|s| !s.private)).unwrap_or(0);
        self.switch_space(ix, window, cx);
    }

    /// Drops the burner's views, page state, peek and site icons, then its space and store.
    pub(crate) fn burn(&mut self, cx: &mut Context<Self>) {
        for key in self.model.burner_tabs(&self.core) {
            self.discard_tab(key, cx);
        }
        let Some(profile) = self.model.burn(&self.core) else { return };
        if self.link_peek_profile() == Some((true, profile)) {
            self.close_link_peek(cx);
        }
        self.forget_favicons((true, profile));
    }

    /// The pinned URL of a pinned tab or tile that now shows another page.
    pub fn pinned_drift(&self, key: TabKey) -> Option<Url> {
        self.model.pinned_drift(key, &self.tab_url(key)?)
    }

    pub fn return_to_pinned(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.return_to_pinned(key, window, cx)) {
            return;
        }
        let Some(url) = self.pinned_drift(key).filter(|u| nav_policy::decide(u, Source::Sync) == Decision::Allow) else { return };
        self.activate(key, window, cx);
        if self.views.contains_key(&key) {
            self.navigate(&url, window, cx);
        }
    }

    pub fn set_pinned_url(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if let Some(url) = self.tab_url(key) {
            self.store(key.private).set_pinned_url(key.id, &url).ok();
            self.reload_space();
            cx.notify();
        }
    }

    pub(crate) fn save_tab_title(&self, id: store::Id, typed: &str) -> Option<()> {
        let tab = self.model.tabs.iter().find(|t| t.id == id)?;
        self.store(self.model.private()).set_tab_custom_title(id, &custom_title(typed, &tab.title)).ok()
    }

    pub fn reset_tab_title(&mut self, key: TabKey, cx: &mut Context<Self>) {
        self.store(key.private).set_tab_custom_title(key.id, "").ok();
        self.reload_space();
        cx.notify();
    }

    /// A tab without a live web view: never opened this session, or hibernated.
    pub fn sleeping(&self, key: TabKey) -> bool {
        self.model.active != Some(key) && !self.views.contains_key(&key)
    }

    /// The current space's unpinned tabs whose page another of its tabs shows.
    pub fn duplicate_tabs(&self) -> Vec<TabKey> {
        let private = self.model.private();
        let url = |t: &Tab| self.tab_url(TabKey { private, id: t.id }).unwrap_or_else(|| t.url.clone());
        browser_core::tabs::duplicates(&self.model.tabs, self.model.active.map(|k| k.id), url).into_iter().map(|id| TabKey { private, id }).collect()
    }

    /// Hibernates the live tabs among `keys` except the active one and the split's panes; their pages come back from saved state.
    pub fn sleep_tabs(&mut self, keys: Vec<TabKey>, cx: &mut Context<Self>) {
        let live: Vec<TabKey> = keys.into_iter().filter(|k| !self.sleeping(*k) && self.model.active != Some(*k) && !self.in_split(*k)).collect();
        for key in live {
            self.sleep_keeping_scroll(key, false, cx);
        }
    }

    /// Hibernates a background tab once its page has said what sleep would lose: the scroll offset
    /// comes back when the page loads again, and with `spare_input` a page with typed input stays
    /// awake. A page that does not answer sleeps.
    fn sleep_keeping_scroll(&mut self, key: TabKey, spare_input: bool, cx: &mut Context<Self>) {
        let Some(view) = self.views.get(&key) else { return };
        let (weak, (tx, mut rx)) = (Rc::downgrade(view), mpsc::unbounded());
        view.page_keep(move |keep| drop(tx.unbounded_send(keep)));
        cx.spawn(async move |this, cx| {
            // Sleep Tab waits less on a page that does not answer, the one most worth sleeping.
            let wait = if spare_input { PROBE_TIMEOUT } else { Duration::from_secs(2) };
            let timeout = std::pin::pin!(cx.background_executor().timer(wait));
            let keep = match select(rx.next(), timeout).await {
                Either::Left((Some(keep), _)) => keep,
                _ => Default::default(),
            };
            this.update(cx, |b, cx| {
                if !b.same_view(key, &weak) || b.model.active == Some(key) || b.in_split(key) {
                    return;
                }
                if spare_input && keep.edited {
                    b.typed_in.insert(key);
                    return;
                }
                let url = b.views.get(&key).and_then(|v| v.url()).map(String::from).filter(|_| keep.scroll != (0., 0.));
                b.pages.entry(key).or_default().scroll_back = url.map(|url| (url, keep.scroll));
                b.hibernate(key, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Hibernates a background tab unless its page is playing video or audio. Until the page has
    /// answered the tab stays live, as the least recently used.
    pub(crate) fn sleep_unless_playing(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(view) = self.views.get(&key) else { return };
        self.model.lifecycle.keep(key);
        let (weak, (tx, mut rx)) = (Rc::downgrade(view), mpsc::unbounded());
        view.media_playing(move |playing| drop(tx.unbounded_send(playing)));
        cx.spawn(async move |this, cx| {
            // A page that does not answer, such as one with a hung process, sleeps.
            let timeout = std::pin::pin!(cx.background_executor().timer(PROBE_TIMEOUT));
            let answer = match select(rx.next(), timeout).await {
                Either::Left((answer, _)) => answer,
                Either::Right(_) => Some(false),
            };
            if answer == Some(false) {
                this.update(cx, |b, cx| {
                    let same = b.same_view(key, &weak);
                    if same && b.model.active != Some(key) && !b.in_split(key) {
                        b.sleep_keeping_scroll(key, true, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Keeps `seen` even when sleeping is Never, since the heavy marker ranks by it. Unless sleeping
    /// is Never, background tabs over their window's equal part of the memory budget sleep too, the
    /// heaviest first; pinned tabs only for the budget, once unused for `PINNED_IDLE`.
    pub(crate) fn sleep_idle_tabs(&mut self, cx: &mut Context<Self>) {
        // Short of memory, a tab left for five minutes sleeps; the web processes are what holds the memory.
        let max_idle = browser_core::settings::max_idle(crate::prefs::sleep_after(&self.core.disk), platform::memory::pressure());
        let (now, keys) = (Instant::now(), self.views.keys().copied().collect::<Vec<_>>());
        self.seen.retain(|k, _| keys.contains(k));
        self.typed_in.retain(|k| keys.contains(k));
        let mut background = Vec::new();
        for key in keys {
            let pinned = self.store(key.private).tab(key.id).is_ok_and(|t| t.kind != TabKind::Ephemeral);
            let on_screen = self.model.active == Some(key) || self.in_split(key);
            let seen = self.seen.entry(key).or_insert(now);
            if on_screen {
                *seen = now;
                self.typed_in.remove(&key);
                continue;
            }
            let idle = now.duration_since(*seen);
            background.push((key, engine::lifecycle::budget_idle(idle, pinned)));
            if !pinned && !self.typed_in.contains(&key) && max_idle.is_some_and(|max| idle >= max) {
                self.sleep_unless_playing(key, cx);
            }
        }
        self.sample_processes(cx);
        if max_idle.is_some() {
            let costs = self.tab_costs();
            // A tab kept for typed input counts like a playing one, so the budget sleeps others instead.
            let awake = background.into_iter().map(|(k, idle)| (k, share(&costs, k), self.playing.contains(&k) || self.typed_in.contains(&k), idle));
            let windows = crate::shared::browser_windows(cx).max(1) as u64;
            for key in engine::lifecycle::over_budget(awake, engine::lifecycle::BACKGROUND_BUDGET / windows) {
                self.sleep_unless_playing(key, cx);
            }
        }
        self.probe_heavy(cx);
    }

    /// Asks each awake background tab, in every space for the memory budget, whether it is playing,
    /// then ranks the space's for the sidebar's heavy marker.
    fn probe_heavy(&mut self, cx: &mut Context<Self>) {
        self.playing.retain(|k| self.views.contains_key(k));
        let keys: Vec<TabKey> = self.views.keys().copied().filter(|k| self.background(*k)).collect();
        for key in keys {
            let Some(view) = self.views.get(&key) else { continue };
            let (weak, (tx, mut rx)) = (Rc::downgrade(view), mpsc::unbounded());
            view.media_playing(move |playing| drop(tx.unbounded_send(playing)));
            cx.spawn(async move |this, cx| {
                let Some(playing) = rx.next().await else { return };
                this.update(cx, |b, cx| {
                    if !b.same_view(key, &weak) {
                        return;
                    }
                    let changed = if playing { b.playing.insert(key) } else { b.playing.remove(&key) };
                    if changed && playing {
                        b.ask_media(key, cx);
                    }
                    if changed {
                        b.rank_heavy();
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        self.rank_heavy();
    }

    fn background(&self, key: TabKey) -> bool {
        self.views.contains_key(&key) && self.model.active != Some(key) && !self.in_split(key)
    }

    fn rank_heavy(&mut self) {
        let (now, costs) = (Instant::now(), self.tab_costs());
        let awake =
            self.model.tabs.iter().map(|t| (self.model.key(t), t.kind != TabKind::Ephemeral)).filter(|(k, _)| self.background(*k)).map(|(k, pinned)| {
                let idle = self.seen.get(&k).map_or(Duration::ZERO, |s| now.duration_since(*s));
                (k, share(&costs, k), self.playing.contains(&k), engine::lifecycle::budget_idle(idle, pinned))
            });
        self.heavy = engine::lifecycle::heaviest(awake, HEAVY_TABS);
    }

    /// Awake, outside the active tab and split, ranked by `rank_heavy`, and not left since the last ranking.
    pub fn heavy(&self, key: TabKey) -> bool {
        let used = !self.playing.contains(&key) && self.seen.get(&key).is_some_and(|s| s.elapsed() < engine::lifecycle::HEAVY_IDLE);
        self.background(key) && self.heavy.contains(&key) && !used
    }

    /// The engine the tab's page runs in, or for a sleeping tab the one it asks for.
    pub(crate) fn tab_web_engine(&self, key: TabKey) -> WebEngine {
        if !cfg!(feature = "chromium") {
            return WebEngine::WebKit;
        }
        match self.views.get(&key) {
            Some(view) if view.is_chromium() => WebEngine::Chromium,
            Some(_) => WebEngine::WebKit,
            None if key.private => WebEngine::WebKit,
            None => self.model.tab(key).map_or(WebEngine::WebKit, |t| {
                browser_core::tabs::sleeping_engine(t, key.private, || self.level_for(&t.url), self.core.default_web_engine.get())
            }),
        }
    }

    /// The engine "Reload in …" would move the tab to; `None` where it cannot move.
    pub(crate) fn web_engine_switch(&self, key: TabKey) -> Option<WebEngine> {
        if !cfg!(feature = "chromium") || key.private {
            return None;
        }
        match self.tab_web_engine(key) {
            WebEngine::Chromium => Some(WebEngine::WebKit),
            WebEngine::WebKit => {
                self.tab_url(key).filter(|u| self.level_for(u) != ShieldLevel::Lockdown && self.chromium_available(key)).map(|_| WebEngine::Chromium)
            }
        }
    }

    /// Whether Chromium can open for the tab's profile now; only one profile owns it, and a wipe
    /// keeps it shut until the next launch.
    #[cfg(feature = "chromium")]
    fn chromium_available(&self, key: TabKey) -> bool {
        let own_store = self.profile_of(key).and_then(|id| self.store(key.private).profile(id).ok()).map(|p| p.data_store_uuid);
        !key.private
            && self.chromium_closed().is_none()
            && own_store.is_some_and(|s| engine::chromium::available(&crate::browser::data_dir().join("Chromium"), &s))
    }

    #[cfg(not(feature = "chromium"))]
    fn chromium_available(&self, _key: TabKey) -> bool {
        false
    }

    pub(crate) fn chromium_closed(&self) -> Option<&'static str> {
        let staleness = self.core.chromium_staleness.get();
        matches!(staleness, Staleness::Unavailable(_)).then(|| staleness.message())
    }

    /// Checks Chromium's age now; an unavailable Chromium leaves `key` a notice with a way to WebKit.
    #[cfg(feature = "chromium")]
    pub(crate) fn chromium_current(&mut self, key: TabKey, cx: &mut Context<Self>) -> bool {
        crate::shared::check_chromium(cx);
        let Some(message) = self.chromium_closed() else { return true };
        self.notify_offer(key, message.to_owned(), Some(Offer::SwitchToWebKit));
        false
    }

    /// Closes every Chromium view of this window once Chromium is unavailable; nothing loads in
    /// WebKit until the user asks, since that could take a page outside an approved proxy.
    #[cfg(feature = "chromium")]
    pub(crate) fn close_stale_chromium(&mut self, cx: &mut Context<Self>) {
        let Some(message) = self.chromium_closed() else { return };
        if self.page_fade.as_ref().is_some_and(|(view, _)| view.is_chromium()) {
            self.page_fade = None;
        }
        let keys: Vec<TabKey> = self.views.iter().filter(|(_, v)| v.is_chromium()).map(|(k, _)| *k).collect();
        for key in keys {
            let active = self.model.active;
            self.hibernate(key, cx);
            self.model.active = active;
            self.notify_offer(key, message.to_owned(), Some(Offer::SwitchToWebKit));
            if let Some(page) = self.pages.get_mut(&key) {
                page.loading = false;
            }
        }
        cx.notify();
    }

    /// Reopens the tab's page in `engine`; its back list and unsaved input are lost.
    pub fn switch_web_engine(&mut self, key: TabKey, engine: WebEngine, window: &mut Window, cx: &mut Context<Self>) {
        self.reopen_in(key, engine, true, window, cx);
    }

    /// `switch_web_engine`; without `pin` the tab keeps following the default engine, which must be `engine`.
    fn reopen_in(&mut self, key: TabKey, engine: WebEngine, pin: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.tab(key).is_none() || self.web_engine_switch(key) != Some(engine) || (!pin && self.core.default_web_engine.get() != engine) {
            return;
        }
        if engine == WebEngine::Chromium && !self.google_notice_seen() {
            return self.show_google_notice(window, cx, move |b, window, cx| b.reopen_in(key, engine, pin, window, cx));
        }
        if browser_core::tabs::set_web_engine(&self.core, key, pin.then_some(engine)).is_err() {
            return;
        }
        self.reload_space();
        let Some(url) = self.tab_url(key).and_then(|u| Url::parse(&u).ok()) else { return cx.notify() };
        // Storage follows a user's switch only, and never for a Lockdown site.
        let old = self.views.get(&key).cloned().filter(|_| self.level_for_tab(key, url.as_str()) != ShieldLevel::Lockdown);
        let Some(old) = old else {
            self.rebuild_view(key, &url, cx);
            return cx.notify();
        };
        let (events, weak) = (self.events.clone(), std::rc::Rc::downgrade(&old));
        let (late, late_weak) = (events.clone(), weak.clone());
        old.read_storage(move |storage| drop(events.unbounded_send(crate::browser::Msg::Handover(key, weak, storage))));
        // A busy page may never answer; the switch then goes ahead without its storage.
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
            late.unbounded_send(crate::browser::Msg::Handover(key, late_weak, None)).ok();
        })
        .detach();
    }

    /// Applies to tabs without an engine of their own as they are next created or restored.
    pub fn set_default_web_engine(&mut self, engine: WebEngine, window: &mut Window, cx: &mut Context<Self>) {
        // Chromium's security fixes reach users only through automatic updates.
        if engine == WebEngine::Chromium && (!self.core.chromium_allowed() || self.chromium_closed().is_some()) {
            return;
        }
        if engine == WebEngine::Chromium && !self.google_notice_seen() {
            return self.show_google_notice(window, cx, move |b, window, cx| b.set_default_web_engine(engine, window, cx));
        }
        if self.core.disk.set_setting(browser_core::site::DEFAULT_WEB_ENGINE, engine.as_str()).is_ok() {
            self.core.default_web_engine.set(engine);
            if engine == WebEngine::Chromium {
                self.offer_reload_in_chromium(window, cx);
            }
        }
        cx.refresh_windows();
    }

    /// Asks whether this window's open WebKit pages without an engine of their own move to Chromium;
    /// sleeping tabs already wake in the default.
    fn offer_reload_in_chromium(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<TabKey> = self
            .views
            .keys()
            .copied()
            .filter(|k| self.model.tab(*k).is_some_and(|t| t.web_engine.is_none()) && self.web_engine_switch(*k) == Some(WebEngine::Chromium))
            .collect();
        if keys.is_empty() {
            return;
        }
        let detail = format!(
            "{} open {} still in WebKit. Reloading moves {} to Chromium; back history and unsaved input are lost.",
            keys.len(),
            if keys.len() == 1 { "tab is" } else { "tabs are" },
            if keys.len() == 1 { "it" } else { "them" }
        );
        let answer = window.prompt(PromptLevel::Info, "Reload open tabs in Chromium?", Some(&detail), &["Reload Tabs", "Not Now"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |b, window, cx| keys.into_iter().for_each(|key| b.reopen_in(key, WebEngine::Chromium, false, window, cx))).ok();
            }
        })
        .detach();
    }

    /// A second window whose new tabs are Chromium tabs.
    pub fn new_chromium_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "chromium")]
        if let Some(message) = self.chromium_closed() {
            if let Some(key) = self.model.active.filter(|k| self.pages.get(k).is_none_or(|p| p.notice.is_none())) {
                self.notify_offer(key, message.to_owned(), None);
                cx.notify();
            }
            return;
        }
        if !self.google_notice_seen() {
            return self.show_google_notice(window, cx, |b, window, cx| b.new_chromium_window(window, cx));
        }
        let [x, y, width, height] = crate::shared::window_frame(window);
        let row = SavedWindow { web_engine: Some(WebEngine::Chromium), x: x + 30., y: y + 30., width, height, ..Default::default() };
        if let Ok(row) = self.core.disk.create_window(&row) {
            let display = window.display(cx).map(|d| d.id());
            cx.defer(move |cx| {
                crate::shared::open_window(row, display, None, crate::browser::WindowStart::Saved, cx);
            });
        }
    }

    fn google_notice_seen(&self) -> bool {
        self.core.disk.setting(GOOGLE_NOTICE_SEEN).ok().flatten().is_some()
    }

    /// Shown the first time Chromium is chosen anywhere; `then` runs once the user accepts it.
    fn show_google_notice(&mut self, window: &mut Window, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        let answer = window.prompt(PromptLevel::Info, "Chromium connects to Google", Some(GOOGLE_NOTICE), &["Use Chromium", "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |b, window, cx| {
                    if b.core.disk.set_setting(GOOGLE_NOTICE_SEEN, "1").is_ok() {
                        then(b, window, cx);
                    }
                })
                .ok();
            }
        })
        .detach();
    }
}

pub fn listen(root: gpui::Div, cx: &mut Context<Browser>) -> gpui::Div {
    use gpui::InteractiveElement;
    root.on_action(cx.listener(|b, _: &RenameTab, w, cx| {
        if let Some(key) = b.model.active.filter(|k| b.model.tab(*k).is_some_and(|t| t.kind != TabKind::Tile)) {
            b.begin_rename(crate::groups::RenameTarget::Tab(key.id), w, cx);
        }
    }))
    .on_action(cx.listener(|b, _: &ResetTabTitle, _, cx| {
        if let Some(key) = b.model.active {
            b.reset_tab_title(key, cx);
        }
    }))
    .on_action(cx.listener(|b, _: &ReturnToPinnedUrl, w, cx| {
        if let Some(key) = b.model.active {
            b.return_to_pinned(key, w, cx);
        }
    }))
    .on_action(cx.listener(|b, _: &SetPinnedUrl, _, cx| {
        if let Some(key) = b.model.active {
            b.set_pinned_url(key, cx);
        }
    }))
    .on_action(cx.listener(|b, _: &SleepOtherTabs, _, cx| {
        if let Some(key) = b.model.active {
            let others = b.model.other_tabs(key);
            b.sleep_tabs(others, cx);
        }
    }))
    .on_action(cx.listener(|b, _: &CloseDuplicateTabs, w, cx| {
        let keys = b.duplicate_tabs();
        b.close_tabs(keys, w, cx);
    }))
    .on_action(cx.listener(|b, _: &ReloadInChromium, w, cx| {
        if let Some(key) = b.model.active {
            b.switch_web_engine(key, WebEngine::Chromium, w, cx);
        }
    }))
    .on_action(cx.listener(|b, _: &ReloadInWebKit, w, cx| {
        if let Some(key) = b.model.active {
            b.switch_web_engine(key, WebEngine::WebKit, w, cx);
        }
    }))
}

/// A tab's part of its process's measured memory, or 0 when unmeasured.
fn share(costs: &HashMap<TabKey, Cost>, key: TabKey) -> u64 {
    costs.get(&key).map_or(0, |c| c.footprint / c.sharing as u64)
}

impl Browser {
    /// Pings the shown page every `PING_EVERY` while the window lives.
    pub(crate) fn watch_hangs(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(engine::lifecycle::PING_EVERY).await;
                if this.update(cx, |b, cx| b.ping_page(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn ping_page(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let was = self.hang.hung(now);
        match self.model.active.and_then(|k| Some((k, self.views.get(&k)?.clone()))) {
            Some((key, view)) if self.hang.ping(key, now) => {
                let (tx, mut rx) = mpsc::unbounded();
                view.selected_text(move |_| drop(tx.unbounded_send(())));
                cx.spawn(async move |this, cx| {
                    if rx.next().await.is_some() {
                        this.update(cx, |b, cx| {
                            let was = b.hang.hung(Instant::now());
                            b.hang.answered(key);
                            if was.is_some() {
                                cx.notify();
                            }
                        })
                        .ok();
                    }
                })
                .detach();
            }
            Some(_) => {}
            None => self.hang.forget(),
        }
        if was != self.hang.hung(now) {
            cx.notify();
        }
    }

    pub(crate) fn wait_for_page(&mut self, cx: &mut Context<Self>) {
        self.hang.wait(Instant::now());
        cx.notify();
    }

    /// Reload on the hung-page card: a new view, so WebKit and Chromium may give it a new process.
    pub(crate) fn reload_hung(&mut self, key: TabKey, cx: &mut Context<Self>) {
        self.hang.forget();
        match self.tab_url(key).and_then(|u| Url::parse(&u).ok()) {
            Some(url) => self.rebuild_view(key, &url, cx),
            None => self.reload(cx),
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(title: &str, custom: &str, url: &str) -> Tab {
        Tab {
            id: 1,
            space_id: 1,
            folder_id: None,
            kind: TabKind::Pinned,
            url: url.into(),
            title: title.into(),
            position: 0,
            last_active: 0,
            group_id: None,
            pinned_url: None,
            custom_title: custom.into(),
            web_engine: None,
        }
    }

    #[test]
    fn stateful_tab_commands_are_named_by_state() {
        assert_eq!(state_label(&crate::TogglePin, true), Some("Unpin Tab"));
        assert_eq!(state_label(&crate::TogglePin, false), Some("Pin Tab"));
        assert_eq!(state_label(&crate::menubar::ToggleMute, true), Some("Unmute Site"));
        assert_eq!(state_label(&crate::split::ToggleSplitView, false), Some("Open in Split View"));
        assert_eq!(state_label(&crate::BookmarkTab, true), None);
    }

    #[test]
    fn a_custom_title_wins_until_cleared() {
        assert_eq!(display_title(&tab("Inbox (3)", "Mail", "https://mail.test/")), "Mail");
        assert_eq!(display_title(&tab("Inbox (3)", "", "https://mail.test/")), "Inbox (3)");
        assert_eq!(display_title(&tab("", " ", "https://Mail.test/")), "mail.test");
        assert_eq!(display_title(&tab("", "", "about:blank")), "about:blank");
        assert_eq!(display_title(&tab("", "", "blob:https://Evil.test/0b1c")), "evil.test");
        assert_eq!(display_title(&tab("", "", "data:text/html,bank.test")), "data:");
        assert_eq!(display_title(&tab("", "", "")), "New Tab");
        let file = if cfg!(windows) { "file:///C:/d/doc.pdf" } else { "file:///tmp/doc.pdf" };
        assert_eq!(display_title(&tab("", "", file)), "doc.pdf", "an opened file without a title");
        assert_eq!(display_title(&tab("x", "evil\u{202e}name", "https://a.test/")), ui::text::bounded_title("evil\u{202e}name"));
    }

    #[test]
    fn renaming_to_the_page_title_follows_the_page_again() {
        assert_eq!(custom_title("Inbox", "Inbox"), "");
        assert_eq!(custom_title("Mail", "Inbox"), "Mail");
    }
}
