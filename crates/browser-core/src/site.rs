use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine::nav_policy::{PopupRule, Site};
use engine::request_filter::Shield;
use shield::bundle::{Kind, VerifiedBundle};
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
/// A site's pop-up choices, as labelled.
pub const POPUP_RULES: [(PopupRule, &str); 3] = [(PopupRule::Default, "Default"), (PopupRule::Allow, "Allow"), (PopupRule::Block, "Block")];
pub const DEFAULT_WEB_ENGINE: &str = "default_web_engine";
pub use crate::incoming::LOCALHOST_FROM_APPS;

/// The domain-set categories, the setting that switches each and whether it is on by default.
pub const SET_SWITCHES: [(Category, &str, bool); 3] =
    [(Category::Malware, "block_malware", true), (Category::Adult, "block_adult", false), (Category::Popups, "block_popups", true)];

/// "Visit Anyway", a notice's offer and a permission prompt's Allow ignore clicks this long after
/// any of them appears or moves, so a page cannot time a click to land on them.
pub const BYPASS_DELAY: Duration = Duration::from_millis(700);

pub fn stable(since: Instant, now: Instant) -> bool {
    now.saturating_duration_since(since) >= BYPASS_DELAY
}

/// The categories whose switch is on, read from settings stored as `on` / `off`.
pub fn set_categories(setting: impl Fn(&str) -> Option<String>) -> Vec<Category> {
    SET_SWITCHES.iter().filter(|(_, key, default)| setting(key).map_or(*default, |v| v != "off")).map(|(c, ..)| *c).collect()
}

pub fn warning_heading(category: Category) -> &'static str {
    match category {
        Category::Malware => "Dangerous site blocked",
        Category::Adult => "Adult site blocked",
        _ => "Site blocked",
    }
}

/// Why the warning page stopped the site; `computer` names the device at risk.
pub fn warning_reason(category: Category, computer: &str) -> String {
    match category {
        Category::Malware => format!(
            "This site is on a list of sites that spread malware, phishing or scams. Visiting it may put your passwords, payments or {computer} at risk."
        ),
        Category::Adult => {
            "This site is on a list of adult websites, and adult content blocking is on. Turn it off in Settings › Privacy to visit such sites.".into()
        }
        _ => "This site is blocked.".into(),
    }
}

pub fn warning_source(category: Category) -> String {
    format!("Listed in Tarn's {} list. Lists are updated daily.", category.as_str())
}

/// Domain sets of the switched-on categories; `None` while off or not installed.
#[derive(Default, Clone)]
pub struct Sets {
    pub malware: Option<Arc<DomainSet>>,
    pub adult: Option<Arc<DomainSet>>,
    pub popups: Option<Arc<DomainSet>>,
}

impl Sets {
    /// Domain sets of `categories`; a set that fails its own checks is left out.
    pub fn from_bundle(bundle: &VerifiedBundle, categories: &[Category]) -> Self {
        let load = |c: Category| {
            categories.contains(&c).then(|| bundle.of(c.as_str(), Kind::Set).next().and_then(|l| DomainSet::from_bytes(&l.data)).map(Arc::new)).flatten()
        };
        Sets { malware: load(Category::Malware), adult: load(Category::Adult), popups: load(Category::Popups) }
    }

    /// The most severe category naming `host`; `excepted` skips only the malware set, so a malware
    /// exception never lifts an adult or pop-up block.
    pub fn lookup(&self, host: &str, excepted: bool) -> Option<Category> {
        [(Category::Malware, &self.malware), (Category::Adult, &self.adult), (Category::Popups, &self.popups)]
            .into_iter()
            .filter(|(c, _)| !(excepted && *c == Category::Malware))
            .find(|(_, set)| set.as_ref().is_some_and(|s| s.contains(host)))
            .map(|(c, _)| c)
    }

    /// Installs the sets that arrived for categories in `keep` and drops the others. Sets read
    /// before a switch changed can therefore neither bring back one the user turned off nor remove
    /// one a later read installed.
    pub fn install(&mut self, incoming: Sets, keep: &[Category]) {
        let Sets { malware, adult, popups } = incoming;
        for (category, current, new) in
            [(Category::Malware, &mut self.malware, malware), (Category::Adult, &mut self.adult, adult), (Category::Popups, &mut self.popups, popups)]
        {
            if !keep.contains(&category) {
                *current = None;
            } else if new.is_some() {
                *current = new;
            }
        }
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

    pub fn clear(&self) {
        self.paused.borrow_mut().clear();
        self.popups.borrow_mut().clear();
        self.exceptions.borrow_mut().clear();
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
    fn only_switched_on_sets_load_from_a_bundle() {
        use shield::bundle::VerifiedList;
        let list = |category: &str, data: Vec<u8>| VerifiedList { name: format!("{category}.x"), category: category.into(), kind: Kind::Set, count: 1, data };
        let set = DomainSet::encode(["evil.example"]);
        let bundle = VerifiedBundle {
            version: 1,
            created: String::new(),
            lists: vec![list("malware", set.clone()), list("adult", set), list("popups", b"not a set".to_vec())],
        };
        let malware = Sets::from_bundle(&bundle, &[Category::Malware]);
        assert!(malware.malware.as_ref().is_some_and(|s| s.contains("evil.example")) && malware.adult.is_none() && malware.popups.is_none());
        let all = Sets::from_bundle(&bundle, &[Category::Malware, Category::Adult, Category::Popups]);
        assert!(all.adult.is_some() && all.popups.is_none(), "a set that fails its checks is left out");
        let site = |u: &str| site(ShieldLevel::Standard, &SiteRules::default(), &RefCell::new(malware.clone()), &Url::parse(u).unwrap());
        let urls = ["https://ok.example/f.zip", "https://evil.example./page"].map(|u| Url::parse(u).ok());
        assert_eq!(engine::downloads::flagged_source(urls, |u| site(u.as_str())).map(|u| u.to_string()), Some("https://evil.example./page".to_owned()));
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
    fn sets_load_switched_on_categories_and_install_only_those_still_on() {
        use shield::bundle::VerifiedList;
        let list = |category: &str, data: Vec<u8>| VerifiedList { name: format!("{category}.set"), category: category.into(), kind: Kind::Set, count: 1, data };
        let bundle = VerifiedBundle {
            version: 1,
            created: String::new(),
            lists: vec![
                list("malware", DomainSet::encode(["evil.example"])),
                list("adult", DomainSet::encode(["adult.example"])),
                list("popups", vec![1, 2, 3]),
            ],
        };
        let loaded = Sets::from_bundle(&bundle, &[Category::Malware, Category::Popups]);
        assert!(loaded.malware.is_some() && loaded.adult.is_none(), "a category switched off is not loaded");
        assert!(loaded.popups.is_none(), "a set that fails its checks is left out");
        let mut sets = Sets { adult: Sets::from_bundle(&bundle, &[Category::Adult]).adult, ..Sets::default() };
        sets.install(loaded.clone(), &[Category::Malware, Category::Adult]);
        assert!(sets.malware.is_some() && sets.adult.is_some(), "a read without the adult set keeps the installed one");
        sets.install(loaded, &[Category::Adult]);
        assert!(sets.malware.is_none(), "a set read before its switch went off is dropped");
    }

    #[test]
    fn set_switches_default_malware_and_popups_on() {
        assert_eq!(set_categories(|_| None), [Category::Malware, Category::Popups]);
        let set = |k: &str| match k {
            "block_adult" => Some("on".to_owned()),
            "block_popups" => Some("off".to_owned()),
            _ => None,
        };
        assert_eq!(set_categories(set), [Category::Malware, Category::Adult]);
    }

    #[test]
    fn guarded_clicks_wait_for_a_stable_layout() {
        let since = Instant::now();
        assert!(!stable(since, since));
        assert!(!stable(since, since + BYPASS_DELAY - Duration::from_millis(1)));
        assert!(stable(since, since + BYPASS_DELAY));
        assert!(!stable(since + BYPASS_DELAY, since), "a change after `now` is not stable");
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
