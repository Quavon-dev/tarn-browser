use std::time::Instant;

use browser_core::permissions::{PermissionRow, permission_rows};
use browser_core::site::{POPUPS, SET_SWITCHES, SiteRules, popup_origin, set_categories, site_host, stable};
use browser_core::tabs::TabKey;
use engine::nav_policy::{self, PopupRule};
use serde_json::{Map, Value, json};
use shield::Category;
use store::Id;
use url::Url;

use super::{Browser, Notice};

/// The warning page shown in place of a tab whose navigation the gate stopped.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockedPage {
    pub url: String,
    pub category: Category,
}

impl Browser {
    fn rules_of(&self, private: bool, profile: Id) -> Option<SiteRules> {
        self.core.profiles.borrow().get(&(private, profile)).map(|rt| rt.rules.clone())
    }

    fn current_rules(&self) -> Option<(bool, Id, SiteRules)> {
        let (private, profile) = self.model.profile()?;
        Some((private, profile, self.rules_of(private, profile)?))
    }

    pub fn set_categories(&self) -> Vec<Category> {
        set_categories(|k| self.core.disk.setting(k).ok().flatten())
    }

    pub fn set_on(&self, category: Category) -> bool {
        self.set_categories().contains(&category)
    }

    pub fn toggle_set(&mut self, category: Category) {
        if let Some((_, key, _)) = SET_SWITCHES.iter().find(|(c, ..)| *c == category) {
            let on = self.set_on(category);
            self.core.disk.set_setting(key, if on { "off" } else { "on" }).ok();
        }
    }

    /// What a tab's web view needs to judge each navigation: its profile's per-site choices and the
    /// switched-on domain sets.
    pub fn blocking(&self, key: TabKey) -> Value {
        let sets: Vec<&str> = self.set_categories().into_iter().map(Category::as_str).collect();
        let Some(rules) = self.model.profile_of(&self.core, key).and_then(|p| self.rules_of(key.private, p)) else { return json!({ "sets": sets }) };
        let sorted = |hosts: &std::collections::HashSet<String>| {
            let mut list: Vec<String> = hosts.iter().cloned().collect();
            list.sort();
            list
        };
        let popups: Map<String, Value> = rules
            .popups
            .borrow()
            .iter()
            .filter_map(|(origin, rule)| match rule {
                PopupRule::Allow => Some((origin.clone(), Value::from("allow"))),
                PopupRule::Block => Some((origin.clone(), Value::from("block"))),
                PopupRule::Default => None,
            })
            .collect();
        json!({ "paused": sorted(&rules.paused.borrow()), "exceptions": sorted(&rules.exceptions.borrow()), "popups": popups, "sets": sets })
    }

    /// Whether ad, tracker and cookie-banner blocking applies to `url` in the tab's profile.
    pub fn filtered(&self, key: TabKey, url: &str) -> bool {
        let host = Url::parse(url).ok().and_then(|u| site_host(&u).map(str::to_owned));
        let rules = self.model.profile_of(&self.core, key).and_then(|p| self.rules_of(key.private, p));
        !host.zip(rules).is_some_and(|(h, r)| r.paused.borrow().contains(&h))
    }

    pub fn site_paused(&self, host: &str) -> bool {
        self.current_rules().is_some_and(|(_, _, r)| r.paused.borrow().contains(host))
    }

    /// Turns ad, tracker and cookie-banner blocking off or on for `host` in the current profile and
    /// reloads its pages.
    pub fn set_site_paused(&mut self, host: &str, paused: bool) {
        let Some((private, profile, rules)) = self.current_rules() else { return };
        if self.core.store(private).set_site_paused(profile, host, paused).is_err() {
            return;
        }
        if paused {
            rules.paused.borrow_mut().insert(host.to_owned());
        } else {
            rules.paused.borrow_mut().remove(host);
        }
        let keys: Vec<TabKey> = self
            .pages
            .iter()
            .filter(|(k, p)| {
                k.private == private
                    && self.model.profile_of(&self.core, **k) == Some(profile)
                    && Url::parse(&p.url).ok().as_ref().and_then(site_host) == Some(host)
            })
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            let url = self.pages[&key].url.clone();
            self.load(key, &url);
        }
    }

    pub fn paused_sites(&self) -> Vec<String> {
        let mut hosts: Vec<String> = self.current_rules().map(|(_, _, r)| r.paused.borrow().iter().cloned().collect()).unwrap_or_default();
        hosts.sort();
        hosts
    }

    fn site_url(&self) -> Option<(TabKey, Url)> {
        let key = self.model.active?;
        let url = Url::parse(&self.pages.get(&key)?.url).ok()?;
        url.host_str()?;
        Some((key, url))
    }

    pub fn site_popups(&self) -> PopupRule {
        let Some((_, url)) = self.site_url() else { return PopupRule::Default };
        self.current_rules().and_then(|(_, _, r)| r.popups.borrow().get(&popup_origin(&url)).copied()).unwrap_or_default()
    }

    pub fn set_site_popups(&mut self, rule: PopupRule) {
        let (Some((_, url)), Some((private, profile))) = (self.site_url(), self.model.profile()) else { return };
        let allow = match rule {
            PopupRule::Default => None,
            PopupRule::Allow => Some(true),
            PopupRule::Block => Some(false),
        };
        self.core.set_permission(private, profile, &popup_origin(&url), POPUPS, allow).ok();
    }

    /// The active page's remembered permissions, its own first, then those of frames in it.
    pub fn site_permissions(&self) -> Vec<PermissionRow> {
        let (Some((key, url)), Some(profile)) = (self.site_url(), self.model.active.and_then(|k| self.model.profile_of(&self.core, k))) else {
            return Vec::new();
        };
        let stored = self.core.store(key.private).permissions(profile).unwrap_or_default();
        permission_rows(stored.into_iter().filter(|(_, kind, _)| kind != POPUPS).collect(), &url.origin().ascii_serialization())
    }

    pub fn set_site_permission(&mut self, origin: &str, kind: &str, decision: Option<&str>) {
        let Some(key) = self.model.active else { return };
        let Some(profile) = self.model.profile_of(&self.core, key) else { return };
        let store = self.core.store(key.private);
        match decision {
            Some(decision) => store.set_permission(profile, origin, kind, decision),
            None => store.delete_permission(profile, origin, kind),
        }
        .ok();
    }

    /// Forgets the active tab's zapped elements of `host`; its stored data is removed by the
    /// engine, which `site_cleared` reports.
    pub fn clear_site(&mut self, host: &str) -> Option<crate::data::StoreKey> {
        let key = self.model.active?;
        if let Some(profile) = self.model.profile_of(&self.core, key).filter(|_| !key.private) {
            self.core.disk.unzap_element(profile, host, None).ok();
        }
        crate::data::StoreKey::parse(&self.store_prop(key))
    }

    pub fn site_cleared(&mut self, host: &str, count: usize) {
        let message = if count == 0 { format!("{host} had no stored data.") } else { format!("Cleared stored data for {host}.") };
        self.notice = Some(Notice::Message(message));
        if let Some(key) = self.model.active {
            self.outbox.push((key, "reload", Value::Null));
        }
    }

    pub(super) fn on_listed(&mut self, key: TabKey, url: &str, category: &str) {
        let (Some(category), Ok(parsed)) = (Category::parse(category), Url::parse(url)) else { return };
        if category.warns() {
            let page = self.pages.entry(key).or_default();
            page.blocked = Some(BlockedPage { url: parsed.to_string(), category });
            if self.model.active == Some(key) {
                browser_core::prompts::restart(&mut self.guard_since, Instant::now());
            }
        } else {
            self.notice = Some(Notice::Open { key, url: parsed.to_string(), message: nav_policy::blocked_message(category, &parsed) });
        }
    }

    /// The sidebar slides the page area and the downloads panel beside it for `slide`; their
    /// guarded buttons wait until it has ended.
    pub fn sidebar_moved(&mut self, slide: std::time::Duration) {
        let rest = Instant::now() + slide;
        browser_core::prompts::restart(&mut self.guard_since, rest);
        browser_core::prompts::restart(&mut self.downloads_moved, rest);
    }

    /// "Go Back": the navigation never happened, so the tab still shows the page it had.
    pub fn leave_blocked(&mut self) {
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)) {
            page.blocked = None;
        }
    }

    /// "Visit Anyway" on a malware warning: lets this exact host through in this profile until
    /// quit, then loads it as the user's own navigation. Clicks in the first moments after the page
    /// showed are ignored.
    pub fn visit_anyway(&mut self) {
        self.visit_anyway_at(Instant::now());
    }

    pub(super) fn visit_anyway_at(&mut self, now: Instant) {
        let Some(key) = self.model.active else { return };
        let Some(blocked) = self.pages.get(&key).and_then(|p| p.blocked.clone()) else { return };
        if blocked.category != Category::Malware || !stable(self.guard_since, now) {
            return;
        }
        let Some(rules) = self.model.profile_of(&self.core, key).and_then(|p| self.rules_of(key.private, p)) else { return };
        let Some(host) = Url::parse(&blocked.url).ok().and_then(|u| site_host(&u).map(str::to_owned)) else { return };
        rules.exceptions.borrow_mut().insert(host);
        self.load(key, &blocked.url);
    }
}
