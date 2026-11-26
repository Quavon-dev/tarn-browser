//! Ad, tracker, cookie-banner, malware, adult-content and pop-up blocking: the switches, the per-site
//! choices and the verdict the navigation gate asks for on every navigation.

use std::time::Instant;

use browser_core::cert_error::{CertDetails, CertError, CertPage, Problem, Step};
use engine::nav_policy::PopupRule;
use engine::{DataStore, FilterLists};
use futures::channel::oneshot;
use gpui::{App, Context, FontWeight, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px};
use gpui::{ClipboardItem, InteractiveElement};
use shield::{Category, youtube};
use store::Id;
use ui::controls::{secure_button, secure_ghost_button, secure_surface, tab_stop};
use ui::icon::{ICON_LARGE, Icon, icon};
use ui::text::OneLine;
use ui::theme::{Elevation, Secure, TEXT, TEXT_SMALL, TEXT_TITLE, Theme};
use url::Url;

use crate::browser::{Browser, TabKey};
use crate::prefs::Pref;
use crate::shared::AppCore;
pub(crate) use browser_core::site::{POPUPS, Sets, SiteRules, popup_origin, site_host, stable};

/// Rule-list groups: the switch that turns each on, and whether pausing a site turns it off.
pub const GROUPS: [(&str, Pref, bool); 6] = [
    ("ads", Pref::BlockAds, true),
    ("cosmetic", Pref::BlockAds, true),
    ("trackers", Pref::BlockTrackers, true),
    ("annoyances", Pref::BlockAnnoyances, true),
    ("malware", Pref::BlockMalware, false),
    (crate::filters::YOUTUBE_GROUP, Pref::YouTubeAds, true),
];

/// A page shown in place of a tab whose navigation the gate stopped.
#[derive(Clone)]
pub struct BlockedPage {
    pub url: Url,
    pub category: Category,
}

/// What a blocking notice offers besides "Dismiss".
#[derive(Clone, PartialEq)]
pub enum Offer {
    /// Load the URL in this tab as the user's own navigation.
    Open(Url),
    /// Open the URL in a new tab.
    NewTab(Url),
    /// Load the URL in this tab as the user's own navigation, which puts a Lockdown site in WebKit.
    WebKit(Url),
    /// Reload the tab with blocking on after a paused site left it off.
    Reload,
    /// Reopen the tab in WebKit after its Chromium view closed as outdated.
    #[cfg(feature = "chromium")]
    SwitchToWebKit,
    /// Translate this page from the language on this Mac.
    #[cfg(feature = "translation")]
    Translate(String, Url),
    /// Reload a translated page in its own language.
    #[cfg(feature = "translation")]
    ShowOriginal,
}

impl AppCore {
    /// Categories whose domain sets the switches ask for.
    pub fn set_categories(&self) -> Vec<Category> {
        browser_core::site::set_categories(|key| Pref::ALL.into_iter().find(|p| p.key() == key).map(|p| if self.pref(p) { "on" } else { "off" }.to_owned()))
    }

    pub fn set_youtube_config(&self, config: &youtube::Config) {
        *self.youtube_script.borrow_mut() = youtube::script(config);
        for rt in self.profiles.borrow().values() {
            rt.data.set_page_script(self.pref(Pref::YouTubeAds).then_some(self.youtube_script.borrow().as_str()));
        }
    }

    pub fn set_rule_lists(&self, group: &'static str, lists: FilterLists) {
        self.rule_lists.borrow_mut().insert(group, lists);
        for rt in self.profiles.borrow().values() {
            self.apply_group(&rt.data, group);
        }
    }

    fn apply_group(&self, data: &DataStore, group: &str) {
        if let Some((group, pref, pausable)) = GROUPS.into_iter().find(|(g, ..)| *g == group) {
            data.set_rule_lists(group, self.rule_lists.borrow().get(group).filter(|_| self.pref(pref)), pausable);
        }
    }

    /// Applies the switches to one profile's store: rule lists, the YouTube script and WebKit's
    /// fraudulent-website check, which goes with the malware switch.
    pub(crate) fn apply_blocking_to(&self, data: &DataStore) {
        for (group, ..) in GROUPS {
            self.apply_group(data, group);
        }
        data.set_page_script(self.pref(Pref::YouTubeAds).then_some(self.youtube_script.borrow().as_str()));
        data.set_safe_browsing(self.pref(Pref::BlockMalware));
    }

    pub(crate) fn apply_blocking(&self) {
        for rt in self.profiles.borrow().values() {
            self.apply_blocking_to(&rt.data);
        }
        self.sync_shield();
    }

    /// Hands Chromium's request filter a fresh copy of its profile's choices; call after any of
    /// them, a switch, the domain sets or the matcher changed.
    pub(crate) fn sync_shield(&self) {
        #[cfg(feature = "chromium")]
        if let Some(rt) = self
            .chromium_profile
            .get()
            .and_then(|id| self.profiles.borrow().get(&(false, id)).map(|rt| (rt.profile.clone(), rt.overrides.clone(), rt.rules.clone())))
        {
            let matcher = self.matcher.borrow().only(self.pref(Pref::BlockAds), self.pref(Pref::BlockTrackers), self.pref(Pref::BlockAnnoyances));
            engine::chromium::set_shield(browser_core::site::shield(&rt.0, &rt.1, &rt.2, &self.sets.borrow(), matcher));
            // Chromium remembers each Continue; forgetting them and their connections with every
            // Shield change keeps a site that moved to Lockdown from reusing one.
            #[cfg(target_os = "macos")]
            engine::chromium::clear_cert_decisions();
        }
        // Every profile and the private space have a Chromium profile of their own on Windows.
        #[cfg(windows)]
        for rt in self.profiles.borrow().values() {
            let matcher = self.matcher.borrow().only(self.pref(Pref::BlockAds), self.pref(Pref::BlockTrackers), self.pref(Pref::BlockAnnoyances));
            rt.data.set_shield(browser_core::site::shield(&rt.profile, &rt.overrides, &rt.rules, &self.sets.borrow(), matcher));
            rt.data.clear_cert_decisions();
        }
    }

    /// Installs the sets that arrived for categories still switched on and drops those switched
    /// off. Sets read before a switch changed can therefore neither bring back one the user turned
    /// off nor remove one a later read installed.
    fn set_sets(&self, incoming: Sets) {
        let keep = self.set_categories();
        self.sets.borrow_mut().install(incoming, &keep);
        self.sync_shield();
    }
}

/// Installs `incoming` (see `AppCore::set_sets`) and checks every open page in every window, so
/// tabs restored before the sets were read get their warning page too.
pub fn set_sets(incoming: Sets, cx: &mut App) {
    crate::shared::core(cx).set_sets(incoming);
    crate::shared::each(cx, |b, _, cx| b.check_blocked(cx));
}

impl Browser {
    fn check_blocked(&mut self, cx: &mut Context<Self>) {
        let stopped: Vec<(TabKey, Url, Category)> = self
            .views
            .iter()
            .filter_map(|(key, view)| {
                let url = view.url()?;
                let category = self.core.sets.borrow().lookup(site_host(&url)?, self.excepted(*key, &url))?;
                category.warns().then_some((*key, url, category))
            })
            .collect();
        for (key, url, category) in stopped {
            self.show_blocked(key, url, category);
        }
        cx.notify();
    }

    /// Forgets the profile's in-memory choices after its stored data was deleted.
    fn excepted(&self, key: TabKey, url: &Url) -> bool {
        let rules = self.profile_of(key).and_then(|p| self.loaded(key.private, p).map(|rt| rt.rules.clone()));
        rules.zip(site_host(url)).is_some_and(|(r, h)| r.exceptions.borrow().contains(h))
    }

    pub(crate) fn show_blocked(&mut self, key: TabKey, url: Url, category: Category) {
        if let Some(view) = self.views.get(&key) {
            view.stop();
        }
        let page = self.pages.entry(key).or_default();
        let moved = page.blocked.as_ref().is_none_or(|b| b.url != url || b.category != category);
        page.blocked = Some(BlockedPage { url, category });
        page.cert = None;
        page.loading = false;
        if moved {
            self.changed_in(key);
        }
    }

    /// After a blocking switch changed: rule lists follow at once, domain sets are read again.
    pub(crate) fn blocking_changed(&mut self, pref: Pref, cx: &mut Context<Self>) {
        self.core.apply_blocking();
        if matches!(pref, Pref::BlockMalware | Pref::BlockAdult | Pref::BlockPopups) {
            self.core.set_sets(Sets::default());
            self.check_blocked(cx);
            crate::shared::others(cx, |b, _, cx| b.check_blocked(cx));
            if self.pref(pref) {
                crate::filters::reload_sets(self.core.clone(), cx);
            }
        }
    }

    fn current_rules(&self) -> Option<(bool, Id, SiteRules)> {
        let space = self.model.current_space()?;
        let (private, profile_id) = (space.private, space.space.profile_id);
        Some((private, profile_id, self.loaded(private, profile_id)?.rules.clone()))
    }

    pub fn site_paused(&self, host: &str) -> bool {
        self.current_rules().is_some_and(|(_, _, r)| r.paused.borrow().contains(host))
    }

    /// Turns ad, tracker and cookie-banner blocking off or on for `host` in the current profile and reloads its pages.
    pub fn set_site_paused(&mut self, host: &str, paused: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((private, profile_id, rules)) = self.current_rules() else { return };
        if self.store(private).set_site_paused(profile_id, host, paused).is_err() {
            return;
        }
        if paused {
            rules.paused.borrow_mut().insert(host.to_owned());
        } else {
            rules.paused.borrow_mut().remove(host);
        }
        self.restart_profile_views(private, profile_id, Some(host), false, window, cx);
    }

    pub fn paused_sites(&self) -> Vec<String> {
        let mut hosts: Vec<String> = self.current_rules().map(|(_, _, r)| r.paused.borrow().iter().cloned().collect()).unwrap_or_default();
        hosts.sort();
        hosts
    }

    pub fn site_popups(&self, origin: &str) -> PopupRule {
        self.current_rules().and_then(|(_, _, r)| r.popups.borrow().get(origin).copied()).unwrap_or_default()
    }

    pub fn set_site_popups(&mut self, origin: &str, rule: PopupRule, cx: &mut Context<Self>) {
        let Some((private, profile_id, rules)) = self.current_rules() else { return };
        let store = self.store(private);
        let saved = match rule {
            PopupRule::Default => store.delete_permission(profile_id, origin, POPUPS),
            PopupRule::Allow => store.set_permission(profile_id, origin, POPUPS, "allow"),
            PopupRule::Block => store.set_permission(profile_id, origin, POPUPS, "deny"),
        };
        if saved.is_ok() {
            match rule {
                PopupRule::Default => rules.popups.borrow_mut().remove(origin),
                rule => rules.popups.borrow_mut().insert(origin.to_owned(), rule),
            };
            self.core.sync_shield();
        }
        cx.notify();
    }

    /// "Go Back" on the warning page: the navigation never happened, so the tab still shows the
    /// page it had, or nothing in a new tab.
    pub fn leave_blocked(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)) {
            page.blocked = None;
            page.cert = None;
        }
        cx.notify();
    }

    /// "Visit Anyway" on a malware warning: lets this exact host through in this profile until
    /// quit, then loads it as the user's own navigation.
    pub fn visit_anyway(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let Some(blocked) = self.pages.get(&key).and_then(|p| p.blocked.clone()) else { return };
        if blocked.category != Category::Malware || !self.layout_stable() {
            return;
        }
        let Some(rules) = self.profile_of(key).and_then(|p| self.loaded(key.private, p).map(|rt| rt.rules.clone())) else { return };
        let Some(host) = site_host(&blocked.url) else { return };
        rules.exceptions.borrow_mut().insert(host.to_owned());
        self.core.sync_shield();
        if let Some(page) = self.pages.get_mut(&key) {
            page.blocked = None;
        }
        self.navigate(&blocked.url, window, cx);
    }

    pub fn set_cert_step(&mut self, step: Step, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let may_continue = self.cert_may_continue(key);
        if let Some(page) = self.pages.get_mut(&key).and_then(|p| p.cert.as_mut()) {
            page.go(step, may_continue);
        }
        if step == Step::Confirm {
            self.layout_changed();
        }
        cx.notify();
    }

    pub fn toggle_cert_viewer(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)).and_then(|p| p.cert.as_mut()) {
            page.toggle_viewer();
        }
        cx.notify();
    }

    fn cert_may_continue(&self, key: TabKey) -> bool {
        self.pages.get(&key).and_then(|p| p.cert.as_ref()).is_some_and(|c| c.error.may_continue(key.private, self.level_for_tab(key, c.error.url.as_str())))
    }

    /// "Trust Certificate": the owner confirms with Touch ID or their password, then this exact
    /// certificate is kept for the site in this profile and the page loads.
    pub fn trust_certificate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let Some(page) = self.pages.get(&key).and_then(|p| p.cert.clone()) else { return };
        if !self.layout_stable() || page.trusted(key.private, self.level_for_tab(key, page.error.url.as_str())).is_none() {
            return;
        }
        let (tx, rx) = oneshot::channel();
        engine::confirm_owner(&page.error.owner_reason(), move |ok| {
            let _ = tx.send(ok);
        });
        cx.spawn_in(window, async move |this, cx| {
            let confirmed = rx.await == Ok(true);
            this.update_in(cx, |b, window, cx| {
                if confirmed {
                    b.keep_cert_exception(key, &page, window, cx);
                } else {
                    b.notify_offer(key, "Tarn could not confirm it is you, so the certificate was not trusted.".to_owned(), None);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn keep_cert_exception(&mut self, key: TabKey, page: &CertPage, window: &mut Window, cx: &mut Context<Self>) {
        if self.pages.get(&key).and_then(|p| p.cert.as_ref()) != Some(page) {
            return;
        }
        let (Some(profile), Some(exception)) = (self.profile_of(key), page.trusted(key.private, self.level_for_tab(key, page.error.url.as_str()))) else {
            return;
        };
        if let Err(e) = self.core.disk.set_cert_exception(profile, &exception.host, exception.port, &exception.fingerprint) {
            self.warning = Some(format!("The certificate could not be saved ({e})."));
            cx.notify();
            return;
        }
        self.apply_cert_exceptions(profile);
        if let Some(state) = self.pages.get_mut(&key) {
            state.cert = None;
        }
        // A load the user started during the owner check wins over the certificate's page.
        if self.model.active == Some(key) && !self.pages.get(&key).is_some_and(|p| p.loading) {
            self.navigate(&page.error.url, window, cx);
        }
        cx.notify();
    }

    /// The site popover's Remove: the tab's page asks again on its next load.
    pub fn forget_cert_exception(&mut self, key: TabKey, url: &Url, cx: &mut Context<Self>) {
        if let (false, Some(profile), Some((host, port))) = (key.private, self.profile_of(key), browser_core::cert_error::host_port(url)) {
            self.remove_cert_exception(profile, &host, port, cx);
        }
    }

    pub fn remove_cert_exception(&mut self, profile: Id, host: &str, port: u16, cx: &mut Context<Self>) {
        match self.core.disk.delete_cert_exception(profile, host, port) {
            Ok(()) => self.apply_cert_exceptions(profile),
            Err(e) => self.warning = Some(format!("The trusted certificate could not be removed ({e}).")),
        }
        cx.notify();
    }

    /// Hands the profile's stored exceptions to its open views; private profiles never have any.
    pub(crate) fn apply_cert_exceptions(&self, profile: Id) {
        if let Some(rt) = self.loaded(false, profile) {
            rt.data.cert_exceptions().set(browser_core::cert_error::exceptions(&self.core.disk, profile));
            #[cfg(windows)]
            rt.data.clear_cert_decisions();
        }
        #[cfg(all(target_os = "macos", feature = "chromium"))]
        if self.core.chromium_profile.get() == Some(profile) {
            engine::chromium::clear_cert_decisions();
        }
    }

    /// Whether the tab's page loads under a certificate exception, so it is shown as not secure.
    pub(crate) fn cert_excepted(&self, key: TabKey, url: &Url) -> bool {
        !key.private && self.profile_of(key).and_then(|p| self.loaded(false, p)).is_some_and(|rt| rt.data.cert_exceptions().covers(url))
    }

    pub fn take_offer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        if !self.layout_stable() {
            return;
        }
        let Some(page) = self.pages.get_mut(&key) else { return };
        page.notice = None;
        let Some(offer) = page.offer.take() else { return };
        match offer {
            Offer::Open(url) | Offer::WebKit(url) => self.navigate(&url, window, cx),
            Offer::NewTab(url) => {
                if !self.open_routed(&url, window, cx) {
                    let engine = self.model.tab(key).and_then(|t| t.web_engine);
                    self.new_tab_in(&url, engine, browser_core::tabs::Opened::Link(key), window, cx);
                }
            }
            Offer::Reload => {
                if let Some(url) = self.views.get(&key).and_then(|v| v.url()) {
                    self.rebuild_view(key, &url, cx);
                }
            }
            #[cfg(feature = "translation")]
            Offer::Translate(source, page) => self.translate_page(key, source, page, cx),
            #[cfg(feature = "translation")]
            Offer::ShowOriginal => self.reload(cx),
            #[cfg(feature = "chromium")]
            Offer::SwitchToWebKit => {
                self.switch_web_engine(key, store::WebEngine::WebKit, window, cx);
                if !self.views.contains_key(&key) {
                    self.activate(key, window, cx);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn layout_changed(&mut self) {
        self.layout_moving(std::time::Duration::ZERO);
    }

    /// Like `layout_changed`, for a move that lasts `motion`: guarded controls wait until it ends.
    pub(crate) fn layout_moving(&mut self, motion: std::time::Duration) {
        browser_core::prompts::restart(&mut self.layout_stable_since, Instant::now() + motion);
    }

    /// A background tab's controls wait for the switch to it, and a notice repeated exactly does not
    /// move anything, so neither can keep the active tab's controls waiting.
    fn changed_in(&mut self, key: TabKey) {
        if self.model.active == Some(key) {
            self.layout_changed();
        }
    }

    pub(crate) fn layout_stable(&self) -> bool {
        stable(self.layout_stable_since, Instant::now())
    }

    pub(crate) fn notify_offer(&mut self, key: TabKey, message: String, offer: Option<Offer>) {
        let page = self.pages.entry(key).or_default();
        let moved = page.notice.as_ref() != Some(&message) || page.offer != offer;
        page.notice = Some(message);
        page.offer = offer;
        if moved {
            self.changed_in(key);
        }
    }
}

/// The warning page, drawn by the shell in place of the web view; nothing of the site is loaded.
pub fn render_blocked(page: &BlockedPage, cx: &mut Context<Browser>) -> gpui::AnyElement {
    let host = ui::text::bounded_origin(&page.url.origin().ascii_serialization());
    let malware = page.category == Category::Malware;
    let color = if malware { Theme::danger() } else { Theme::warning() };
    let card = secure_surface(Theme::radius_card())
        .w(px(520.))
        .max_w_full()
        .p_6()
        .flex()
        .flex_col()
        .gap_3()
        .text_size(px(TEXT))
        .shadow(Elevation::Modal.shadow())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(icon(Icon::Shield, ICON_LARGE, color))
                .child(div().text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).child(browser_core::site::warning_heading(page.category))),
        )
        .child(div().font_weight(FontWeight::SEMIBOLD).one_line(host))
        .child(div().text_color(Secure::muted()).child(browser_core::site::warning_reason(page.category, if cfg!(windows) { "PC" } else { "Mac" })))
        .child(div().text_size(px(TEXT_SMALL)).text_color(Secure::faint()).child(browser_core::site::warning_source(page.category)))
        .child(
            div()
                .pt_2()
                .flex()
                .items_center()
                .child(secure_button("blocked-back", "Go Back", true).on_click(cx.listener(|b, _, _, cx| b.leave_blocked(cx))))
                .child(div().flex_1())
                .when(malware, |d| d.child(secure_button("blocked-visit", "Visit Anyway", false).on_click(cx.listener(|b, _, w, cx| b.visit_anyway(w, cx))))),
        );
    div().flex_1().flex().flex_col().items_center().pt(gpui::relative(0.18)).px_4().child(card).into_any_element()
}

/// The certificate error page of `key`'s tab, if it shows one.
pub fn render_cert_page(b: &Browser, key: TabKey, cx: &mut Context<Browser>) -> Option<gpui::AnyElement> {
    let page = b.pages.get(&key)?.cert.as_ref()?;
    let may_continue = page.error.may_continue(key.private, b.level_for_tab(key, page.error.url.as_str()));
    Some(render_cert_error(&page.error, page.shown(may_continue), page.viewer, may_continue, cx))
}

/// Drawn by the shell in place of the web view, as the warning page is; nothing of the site loaded.
pub fn render_cert_error(cert: &CertError, step: Step, viewer: bool, may_continue: bool, cx: &mut Context<Browser>) -> gpui::AnyElement {
    let revoked = cert.problem == Problem::Revoked;
    let color = if revoked { Theme::danger() } else { Theme::warning() };
    let secure = |id: &'static str, text: &'static str, primary: bool| tab_stop(secure_button(id, text, primary));
    let actions = if step == Step::Confirm {
        div()
            .pt_2()
            .flex()
            .items_center()
            .gap_2()
            .child(secure("cert-cancel", "Cancel", true).on_click(cx.listener(|b, _, _, cx| b.set_cert_step(Step::Details, cx))))
            .child(div().flex_1())
            .child(secure("cert-trust", "Trust Certificate", false).on_click(cx.listener(|b, _, w, cx| b.trust_certificate(w, cx))))
    } else {
        let details = if step == Step::Details { "Hide Details" } else { "Details" };
        let next = if step == Step::Details { Step::Summary } else { Step::Details };
        div()
            .pt_2()
            .flex()
            .items_center()
            .gap_2()
            .child(secure("cert-back", "Go Back", true).on_click(cx.listener(|b, _, _, cx| b.leave_blocked(cx))))
            .child(div().flex_1())
            .child(
                secure("cert-show", if viewer { "Hide Certificate" } else { "Show Certificate" }, false)
                    .on_click(cx.listener(|b, _, _, cx| b.toggle_cert_viewer(cx))),
            )
            .when(may_continue, |d| d.child(secure("cert-details", details, false).on_click(cx.listener(move |b, _, _, cx| b.set_cert_step(next, cx)))))
    };
    let way_through = match step {
        Step::Summary => None,
        Step::Details => Some(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_2()
                .pt_1()
                .child(div().text_color(Secure::muted()).child(
                    "If you know this site and its certificate, such as a router, NAS or server on your own network, you can trust this exact certificate in this profile.",
                ))
                .child(tab_stop(secure_ghost_button("cert-continue", cert.continue_label())).on_click(cx.listener(|b, _, _, cx| b.set_cert_step(Step::Confirm, cx)))),
        ),
        Step::Confirm => Some(
            div()
                .p_3()
                .rounded(px(Theme::radius_card() / 2.))
                .border_1()
                .border_color(color)
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(color).child(format!("Trust the certificate of {}?", cert.site())))
                .child(div().child(cert.confirm_message()))
                .when(cfg!(target_os = "macos"), |d| d.child(div().text_size(px(TEXT_SMALL)).text_color(Secure::faint()).child("Tarn asks for Touch ID or your password next."))),
        ),
    };
    let card = secure_surface(Theme::radius_card())
        .flex_none()
        .w(px(560.))
        .max_w_full()
        .p_6()
        .flex()
        .flex_col()
        .gap_3()
        .text_size(px(TEXT))
        .shadow(Elevation::Modal.shadow())
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(icon(Icon::TriangleAlert, ICON_LARGE, color))
                .child(div().text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).child(cert.heading())),
        )
        .child(div().font_weight(FontWeight::SEMIBOLD).one_line(cert.site()))
        .child(div().text_color(Secure::muted()).child(cert.reason(if cfg!(windows) { "PC" } else { "Mac" })))
        .when(!revoked, |d| d.child(div().text_size(px(TEXT_SMALL)).text_color(Secure::faint()).child(CertError::WARNING)))
        .children(way_through)
        .when(viewer, |d| d.child(cert_viewer(cert)))
        .child(actions);
    div().id("cert-page").flex_1().flex().flex_col().items_center().pt(gpui::relative(0.12)).pb_6().px_4().overflow_y_scroll().child(card).into_any_element()
}

/// The server's certificates, leaf first, as the viewer lists them.
fn cert_viewer(cert: &CertError) -> gpui::Div {
    let row = |label: &'static str, value: String| {
        div().flex().gap_3().child(div().w(px(72.)).flex_none().text_color(Secure::faint()).child(label)).child(div().flex_1().min_w_0().child(value))
    };
    let list = cert.details();
    let count = list.len();
    let entries = list.into_iter().enumerate().map(|(ix, d): (usize, CertDetails)| {
        let fingerprint = d.fingerprint.clone();
        let copy = tab_stop(secure_ghost_button(("cert-copy", ix), "Copy SHA-256"))
            .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(fingerprint.clone())));
        div()
            .pt_3()
            .when(ix > 0, |d| d.border_t_1().border_color(Secure::hairline()))
            .flex()
            .flex_col()
            .gap_1()
            .text_size(px(TEXT_SMALL))
            .child(div().text_size(px(TEXT)).font_weight(FontWeight::SEMIBOLD).one_line(d.subject.clone()))
            .when_some(d.organization.clone(), |el, o| el.child(row("Organization", o)))
            .child(row("Issued by", d.issuer.clone()))
            .child(row("Valid", format!("{} to {}", browser_core::shield_card::date(d.not_before), browser_core::shield_card::date(d.not_after))))
            .when(!d.names.is_empty(), |el| el.child(row("Names", d.names.join(", "))))
            .child(row("SHA-256", d.fingerprint.clone()))
            .child(div().pt_1().flex().child(copy))
    });
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p_3()
        .rounded(px(Theme::radius_card() / 2.))
        .bg(Secure::control())
        .child(div().font_weight(FontWeight::SEMIBOLD).child(match count {
            0 => "The site sent no certificate Tarn could read.".to_owned(),
            1 => "Certificate".to_owned(),
            n => format!("Certificate chain ({n})"),
        }))
        .children(entries)
}
