use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use engine::nav_policy::{PopupRule, Site};
use engine::request_filter::Shield;
use shield::domains::DomainSet;
use shield::matcher::Matcher;
use shield::{Category, ShieldLevel};
use store::{Id, Profile, WebEngine};
use url::Url;

pub type Overrides = Rc<RefCell<HashMap<String, ShieldLevel>>>;
pub type Hosts = Rc<RefCell<HashSet<String>>>;
/// Pop-up rules by page origin, as stored in the `permissions` table under kind `popups`.
pub type PopupRules = Rc<RefCell<HashMap<String, PopupRule>>>;
pub const POPUPS: &str = "popups";
pub const DEFAULT_WEB_ENGINE: &str = "default_web_engine";

/// Domain sets of the switched-on categories; `None` while off or not installed.
#[derive(Default, Clone)]
pub struct Sets {
    pub malware: Option<Arc<DomainSet>>,
    pub adult: Option<Arc<DomainSet>>,
    pub popups: Option<Arc<DomainSet>>,
}

impl Sets {
    /// The most severe category naming `host`; `excepted` skips only the malware set, so a malware
    /// exception never lifts an adult or pop-up block.
    pub fn lookup(&self, host: &str, excepted: bool) -> Option<Category> {
        [(Category::Malware, &self.malware), (Category::Adult, &self.adult), (Category::Popups, &self.popups)]
            .into_iter()
            .filter(|(c, _)| !(excepted && *c == Category::Malware))
            .find(|(_, set)| set.as_ref().is_some_and(|s| s.contains(host)))
            .map(|(c, _)| c)
    }
}

/// One profile's per-site blocking choices, shared with its views' `SiteFor` closures.
#[derive(Clone, Default)]
pub struct SiteRules {
    pub paused: Hosts,
    pub popups: PopupRules,
    /// Malware hosts the user chose to visit anyway; in memory, for this session only.
    pub exceptions: Hosts,
}

impl SiteRules {
    pub fn load(store: &store::Store, profile_id: Id) -> Self {
        let paused = store.paused_sites(profile_id).unwrap_or_default().into_iter().collect();
        let popups = store
            .permissions(profile_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, kind, _)| kind == POPUPS)
            .map(|(origin, _, decision)| (origin, if decision == "allow" { PopupRule::Allow } else { PopupRule::Block }))
            .collect();
        SiteRules { paused: Rc::new(RefCell::new(paused)), popups: Rc::new(RefCell::new(popups)), exceptions: Rc::default() }
    }
}

/// The host every blocking choice is keyed by: `example.com.` and `example.com` are one site.
pub fn site_host(url: &Url) -> Option<&str> {
    url.host_str().map(|h| h.trim_end_matches('.')).filter(|h| !h.is_empty())
}

/// The page origin pop-up rules are stored under, with the same trailing-dot rule as `site_host`.
pub fn popup_origin(url: &Url) -> String {
    let mut plain = url.clone();
    if let Some(host) = site_host(url).map(str::to_owned) {
        plain.set_host(Some(&host)).ok();
    }
    plain.origin().ascii_serialization()
}

pub fn effective_level(profile: &Profile, overrides: &Overrides, url: Option<&Url>) -> ShieldLevel {
    level_in(profile.shield_level, &overrides.borrow(), url)
}

fn level_in(profile: ShieldLevel, overrides: &HashMap<String, ShieldLevel>, url: Option<&Url>) -> ShieldLevel {
    // `example.com.` is the same site as `example.com` and must not escape its override.
    let site = url.and_then(|u| u.host_str()).and_then(|h| overrides.get(h.trim_end_matches('.')).copied());
    ShieldLevel::effective(profile, site)
}

/// The engine for a tab of `profile` showing `url`, Lockdown forces WebKit, then the tab's choice, then `default`.
pub fn web_engine(profile: &Profile, overrides: &Overrides, tab: Option<WebEngine>, default: WebEngine, url: Option<&Url>) -> WebEngine {
    WebEngine::resolve(profile.shield_level, effective_level(profile, overrides, url), tab, None, Some(default))
}

pub fn site(level: ShieldLevel, rules: &SiteRules, sets: &RefCell<Sets>, url: &Url) -> Site {
    verdict(level, &rules.paused.borrow(), &rules.popups.borrow(), &rules.exceptions.borrow(), &sets.borrow(), url)
}

fn verdict(level: ShieldLevel, paused: &HashSet<String>, popups: &HashMap<String, PopupRule>, exceptions: &HashSet<String>, sets: &Sets, url: &Url) -> Site {
    let host = site_host(url).unwrap_or_default();
    let blocked = if host.is_empty() { None } else { sets.lookup(host, exceptions.contains(host)) };
    Site {
        level,
        filtered: host.is_empty() || !paused.contains(host),
        blocked,
        malware: !host.is_empty() && sets.lookup(host, false) == Some(Category::Malware),
        popups: popups.get(&popup_origin(url)).copied().unwrap_or_default(),
    }
}

pub fn site_for(profile: Profile, overrides: Overrides, rules: SiteRules, sets: Rc<RefCell<Sets>>) -> engine::SiteFor {
    Rc::new(move |url: &Url| site(effective_level(&profile, &overrides, Some(url)), &rules, &sets, url))
}

/// A copy of the profile's choices for a filter that runs off the main thread; it does not follow
/// later changes, so the shell builds a new one after each.
pub fn shield(profile: &Profile, overrides: &Overrides, rules: &SiteRules, sets: &Sets, matcher: Matcher) -> Shield {
    let level = profile.shield_level;
    let overrides = overrides.borrow().clone();
    let (paused, popups, exceptions) = (rules.paused.borrow().clone(), rules.popups.borrow().clone(), rules.exceptions.borrow().clone());
    let sets = sets.clone();
    let site_for = move |url: &Url| verdict(level_in(level, &overrides, Some(url)), &paused, &popups, &exceptions, &sets, url);
    Shield { site_for: Box::new(site_for), matcher }
}

pub fn saves_mute(muted: bool, level: ShieldLevel) -> bool {
    !muted || level != ShieldLevel::Lockdown
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::Store;

    fn sets() -> RefCell<Sets> {
        let set = |d: &str| DomainSet::from_bytes(&DomainSet::encode([d])).map(Arc::new);
        RefCell::new(Sets { malware: set("evil.example"), adult: set("adult.example"), popups: set("pop.example") })
    }

    #[test]
    fn only_a_lockdown_mute_goes_unsaved() {
        assert!(saves_mute(true, ShieldLevel::Strict) && saves_mute(false, ShieldLevel::Lockdown));
        assert!(!saves_mute(true, ShieldLevel::Lockdown));
    }

    #[test]
    fn verdict_follows_sets_pause_popups_and_exceptions() {
        let rules = SiteRules::default();
        let sets = sets();
        let url = |u: &str| Url::parse(u).unwrap();
        let s = |u: &str| site(ShieldLevel::Strict, &rules, &sets, &url(u));
        assert_eq!(s("https://www.evil.example/x").blocked, Some(Category::Malware));
        assert_eq!(s("https://adult.example/").blocked, Some(Category::Adult));
        assert_eq!(s("https://pop.example/").blocked, Some(Category::Popups));
        assert_eq!(s("https://fine.example/").blocked, None);
        assert_eq!(s("about:blank").blocked, None);
        assert!(s("about:blank").filtered && s("https://fine.example/").filtered);
        rules.paused.borrow_mut().insert("fine.example".into());
        rules.exceptions.borrow_mut().insert("www.evil.example".into());
        rules.exceptions.borrow_mut().insert("adult.example".into());
        rules.popups.borrow_mut().insert("https://fine.example".into(), PopupRule::Block);
        assert!(!s("https://fine.example/").filtered);
        assert!(!s("https://fine.example./").filtered, "a trailing dot must not escape a pause");
        assert_eq!(s("https://fine.example./").popups, PopupRule::Block, "or a pop-up rule");
        assert_eq!(popup_origin(&url("https://fine.example.:8443/x")), "https://fine.example:8443");
        assert_eq!(s("https://www.evil.example./").blocked, None, "or an exception");
        assert_eq!(s("https://fine.example/").popups, PopupRule::Block);
        assert_eq!(s("http://fine.example/").popups, PopupRule::Default);
        assert_eq!(s("https://www.evil.example/").blocked, None);
        assert!(s("https://www.evil.example/").malware, "an exception still warns before a download");
        assert!(!s("https://adult.example/").malware && !s("about:blank").malware);
        assert_eq!(s("https://evil.example/").blocked, Some(Category::Malware));
        assert_eq!(s("https://adult.example/").blocked, Some(Category::Adult));
        assert_eq!(s("https://fine.example/").level, ShieldLevel::Strict);
    }

    #[test]
    fn malware_wins_over_other_categories() {
        let set = || DomainSet::from_bytes(&DomainSet::encode(["both.example"])).map(Arc::new);
        let both = Sets { malware: set(), adult: set(), popups: set() };
        assert_eq!(both.lookup("both.example", false), Some(Category::Malware));
        assert_eq!(both.lookup("both.example", true), Some(Category::Adult), "a malware exception must not lift the adult block");
        assert_eq!(Sets::default().lookup("both.example", false), None);
    }

    #[test]
    fn shield_copies_the_choices_at_the_time() {
        use engine::request_filter::{RequestAction, decide};
        let store = Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "uuid", false).unwrap();
        let overrides: Overrides = Rc::default();
        overrides.borrow_mut().insert("strict.test".into(), ShieldLevel::Strict);
        let rules = SiteRules::default();
        rules.paused.borrow_mut().insert("paused.test".into());
        let matcher = Matcher::new(&["||ads.example^\n"], &[], &[]);
        let shield = shield(&profile, &overrides, &rules, &sets().borrow(), matcher);
        rules.paused.borrow_mut().clear();
        let url = |u: &str| Url::parse(u).unwrap();
        let ad = url("https://ads.example/a.js");
        assert_eq!(decide(&shield, &ad, "script", Some(&url("https://news.test/")), None, None, false), RequestAction::Cancel);
        assert_ne!(decide(&shield, &ad, "script", Some(&url("https://paused.test/")), None, None, false), RequestAction::Cancel, "a paused site is exempt");
        assert_eq!(
            decide(&shield, &url("https://www.evil.example/x.js"), "script", Some(&url("https://paused.test/")), None, None, false),
            RequestAction::Cancel
        );
        assert_eq!((shield.site_for)(&url("https://strict.test./")).level, ShieldLevel::Strict);
        assert_eq!((shield.site_for)(&url("https://other.test/")).level, ShieldLevel::Standard);
    }

    #[test]
    fn profile_shield_change_applies_to_effective_level() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        store.set_profile_shield_level(p.id, ShieldLevel::Strict).unwrap();
        let profile = store.profile(p.id).unwrap();
        let url = Url::parse("https://a.test/").unwrap();
        let overrides: Overrides = Rc::default();
        assert_eq!(effective_level(&profile, &overrides, Some(&url)), ShieldLevel::Strict);
        overrides.borrow_mut().insert("a.test".into(), ShieldLevel::Standard);
        assert_eq!(effective_level(&profile, &overrides, Some(&url)), ShieldLevel::Standard);
        let dotted = Url::parse("https://a.test./").unwrap();
        assert_eq!(effective_level(&profile, &overrides, Some(&dotted)), ShieldLevel::Standard);
    }

    #[test]
    fn web_engine_follows_lockdown_tab_and_default() {
        use WebEngine::{Chromium, WebKit};
        let store = Store::open_in_memory().unwrap();
        let mut profile = store.create_profile("P", "uuid", false).unwrap();
        let overrides: Overrides = Rc::default();
        overrides.borrow_mut().insert("locked.test".into(), ShieldLevel::Lockdown);
        let cases = [
            (None, WebKit, "https://a.test/", WebKit),
            (Some(Chromium), WebKit, "https://a.test/", Chromium),
            (None, Chromium, "https://a.test/", Chromium),
            (Some(WebKit), Chromium, "https://a.test/", WebKit),
            (None, Chromium, "about:blank", Chromium),
            (Some(Chromium), Chromium, "https://locked.test/", WebKit),
            (None, Chromium, "https://locked.test./x", WebKit),
        ];
        for (tab, default, url, expected) in cases {
            assert_eq!(web_engine(&profile, &overrides, tab, default, Url::parse(url).ok().as_ref()), expected, "{tab:?} {default:?} {url}");
        }
        profile.shield_level = ShieldLevel::Lockdown;
        assert_eq!(web_engine(&profile, &overrides, Some(Chromium), Chromium, Url::parse("https://a.test/").ok().as_ref()), WebKit);
    }
}
