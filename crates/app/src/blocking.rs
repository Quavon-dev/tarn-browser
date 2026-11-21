//! Ad, tracker, cookie-banner, malware, adult-content and pop-up blocking: the switches, the per-site
//! choices and the verdict the navigation gate asks for on every navigation.

use std::time::Instant;

use engine::nav_policy::PopupRule;
use engine::{DataStore, FilterLists};
use gpui::{App, Context, FontWeight, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px};
use shield::{Category, youtube};
use store::Id;
use ui::controls::{secure_button, secure_surface};
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
        }
        // Every profile and the private space have a Chromium profile of their own on Windows.
        #[cfg(windows)]
        for rt in self.profiles.borrow().values() {
            let matcher = self.matcher.borrow().only(self.pref(Pref::BlockAds), self.pref(Pref::BlockTrackers), self.pref(Pref::BlockAnnoyances));
            rt.data.set_shield(browser_core::site::shield(&rt.profile, &rt.overrides, &rt.rules, &self.sets.borrow(), matcher));
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
                    self.new_tab_in(&url, engine, window, cx);
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
