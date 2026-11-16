use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::nav_policy::{self, Decision, LevelAction, Site, Source};
use shield::ShieldLevel;
use url::Url;

pub const PROMPT_BURST: usize = 5;
pub const POPUP_BURST: usize = 3;
/// Downloads one tab may start per `BURST_WINDOW`; a popup is a tab of its own, and pages open
/// at most `POPUP_BURST` of them per window.
pub const DOWNLOAD_BURST: usize = 10;
pub const MAX_AUTH_ATTEMPTS: u32 = 3;
pub const MAX_TITLE: usize = 1024;
pub const BURST_WINDOW: Duration = Duration::from_secs(10);
pub const DEFAULT_LEVEL_KEY: &str = "*";
pub const MAX_PROMPTS: usize = 8;
pub const MIN_PAGE_ZOOM: f64 = 0.5;
pub const MAX_PAGE_ZOOM: f64 = 3.0;

pub fn zoom_factor(factor: f64) -> f64 {
    if factor.is_finite() { factor.clamp(MIN_PAGE_ZOOM, MAX_PAGE_ZOOM) } else { 1.0 }
}

#[derive(Default)]
pub struct Burst(VecDeque<Instant>);

impl Burst {
    pub fn admit(&mut self, max: usize, now: Instant) -> bool {
        admit(&mut self.0, max, now)
    }
}

pub fn admit(recent: &mut VecDeque<Instant>, max: usize, now: Instant) -> bool {
    recent.retain(|t| now.duration_since(*t) < BURST_WINDOW);
    let ok = recent.len() < max;
    if ok {
        recent.push_back(now);
    }
    ok
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Levels {
    pub default: ShieldLevel,
    pub sites: HashMap<String, ShieldLevel>,
}

impl Levels {
    pub fn parse(value: &serde_json::Value, fallback: ShieldLevel) -> Self {
        let mut levels = Levels { default: fallback, sites: HashMap::new() };
        for (host, level) in value.as_object().into_iter().flatten() {
            let level = level.as_str().and_then(ShieldLevel::parse).unwrap_or(ShieldLevel::Lockdown);
            if host == DEFAULT_LEVEL_KEY {
                levels.default = level;
            } else {
                levels.sites.insert(host.trim_end_matches('.').to_ascii_lowercase(), level);
            }
        }
        levels
    }

    pub fn level(&self, url: &Url) -> ShieldLevel {
        url.host_str().map(|h| h.trim_end_matches('.')).and_then(|h| self.sites.get(h)).copied().unwrap_or(self.default)
    }

    pub fn site(&self, url: &Url) -> Site {
        Site::new(self.level(url))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow { javascript: bool },
    Blocked(&'static str),
    External,
    Upgrade(Url),
    Rebuild,
}

pub struct Navigation<'a> {
    pub url: &'a Url,
    pub from_shell: bool,
    pub user_link: bool,
    pub user_step: bool,
    pub page: Option<&'a Url>,
    pub upgraded: bool,
    pub likely_subframe: bool,
}

pub fn navigation(levels: &Levels, view: ShieldLevel, nav: &Navigation) -> Verdict {
    let url = nav.url;
    if nav.likely_subframe && matches!(url.scheme(), "data" | "about") {
        return Verdict::Allow { javascript: view.policy().javascript };
    }
    let source = if nav.from_shell { Source::Typed } else { Source::Link };
    match nav_policy::decide(url, source) {
        Decision::Allow => {}
        Decision::Prompt => return Verdict::External,
        Decision::Block => return Verdict::Blocked("nav_policy"),
    }
    let site = levels.site(url);
    let page_level = nav.page.map_or(view, |p| levels.level(p));
    let user_started = nav.from_shell || nav.user_step;
    if nav_policy::into_private_network(url, nav.page, user_started, page_level.policy().block_private_network) {
        return Verdict::Blocked("local_network");
    }
    if url.scheme() == "http" && site.level.policy().https_only {
        return match https(url) {
            Some(upgraded) if !nav.upgraded && (user_started || nav.user_link) => Verdict::Upgrade(upgraded),
            _ => Verdict::Blocked("https_only"),
        };
    }
    let view_javascript = view.policy().javascript;
    match nav_policy::level_action(site, view, true, nav.from_shell, url) {
        LevelAction::Rebuild => Verdict::Rebuild,
        LevelAction::BlockInsecure => Verdict::Blocked("https_only"),
        LevelAction::Continue { .. } if nav.user_link && needs_stricter_view(levels, view, url) => Verdict::Rebuild,
        LevelAction::Continue { .. } => Verdict::Allow { javascript: view_javascript && site.level.policy().javascript },
    }
}

pub fn needs_stricter_view(levels: &Levels, view: ShieldLevel, url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && view.policy().javascript && !levels.level(url).policy().javascript
}

pub fn javascript_restored(levels: &Levels, view: ShieldLevel, url: &Url, enabled: bool) -> bool {
    matches!(url.scheme(), "http" | "https") && !enabled && view.policy().javascript && levels.level(url).policy().javascript
}

pub fn popup(levels: &Levels, view: ShieldLevel, url: &Url, page: Option<&Url>) -> Verdict {
    match nav_policy::decide(url, Source::Popup) {
        Decision::Allow => {}
        Decision::Prompt => return Verdict::External,
        Decision::Block => return Verdict::Blocked("nav_policy"),
    }
    let page_level = page.map_or(view, |p| levels.level(p));
    if nav_policy::into_private_network(url, page, false, page_level.policy().block_private_network) {
        return Verdict::Blocked("shield");
    }
    Verdict::Allow { javascript: view.policy().javascript }
}

pub fn https(url: &Url) -> Option<Url> {
    let mut upgraded = url.clone();
    let port = upgraded.port();
    upgraded.set_scheme("https").ok()?;
    if port == Some(443) {
        upgraded.set_port(None).ok()?;
    }
    Some(upgraded)
}

pub fn origin(url: &Url) -> Option<String> {
    let origin = url.origin();
    origin.is_tuple().then(|| origin.ascii_serialization())
}

pub fn without_credentials(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else { return String::new() };
    if parsed.set_username("").is_err() || parsed.set_password(None).is_err() {
        return String::new();
    }
    parsed.to_string()
}

pub fn same_origin(challenge: &Url, page: Option<&Url>) -> bool {
    page.and_then(origin).is_some_and(|p| origin(challenge).is_some_and(|c| c == p))
}

/// Sign-in requests are shown only for the page's own origin and only for a few failed tries.
pub fn may_ask_credentials(challenge: &Url, page: Option<&Url>, failures: u32) -> bool {
    failures < MAX_AUTH_ATTEMPTS && same_origin(challenge, page)
}

pub fn title(raw: &str) -> String {
    match raw.char_indices().nth(MAX_TITLE) {
        Some((end, _)) => raw[..end].to_owned(),
        None => raw.to_owned(),
    }
}

/// Entries kept on each side of a saved back list.
pub const MAX_SAVED_HISTORY: usize = 50;

/// A tab's back and forward lists, saved for an engine that cannot restore its own (Chromium):
/// `back` oldest first, `forward` nearest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedHistory {
    pub back: Vec<Url>,
    pub current: Url,
    pub forward: Vec<Url>,
}

impl SavedHistory {
    /// Only web pages are kept, the nearest `MAX_SAVED_HISTORY` on each side.
    pub fn new(back: Vec<Url>, current: Url, forward: Vec<Url>) -> Option<Self> {
        let web = |u: &Url| nav_policy::decide(u, Source::Sync) == Decision::Allow && u.as_str() != "about:blank";
        if !web(&current) {
            return None;
        }
        let mut back: Vec<Url> = back.into_iter().filter(web).collect();
        back.drain(..back.len().saturating_sub(MAX_SAVED_HISTORY));
        let forward = forward.into_iter().filter(web).take(MAX_SAVED_HISTORY).collect();
        Some(SavedHistory { back, current, forward })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let list = |urls: &[Url]| urls.iter().map(|u| u.to_string()).collect::<Vec<_>>();
        serde_json::json!({ "tarn-history": 1, "back": list(&self.back), "current": self.current.as_str(), "forward": list(&self.forward) })
            .to_string()
            .into_bytes()
    }

    /// Reads what `to_bytes` wrote; anything else, or a list that is not web pages, is refused.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        if value.get("tarn-history")?.as_u64()? != 1 {
            return None;
        }
        let list = |key: &str| -> Option<Vec<Url>> { value.get(key)?.as_array()?.iter().map(|u| Url::parse(u.as_str()?).ok()).collect() };
        SavedHistory::new(list("back")?, Url::parse(value.get("current")?.as_str()?).ok()?, list("forward")?)
    }
}

/// The back and forward lists of a restored tab past the engine's own, which cannot take them:
/// `back` oldest first, `forward` nearest first. A step loads its page in place of the current one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoredHistory {
    pub back: Vec<Url>,
    pub forward: Vec<Url>,
}

impl RestoredHistory {
    /// The page a step back (`true`) or forward would load.
    pub fn next(&self, back: bool) -> Option<&Url> {
        if back { self.back.last() } else { self.forward.first() }
    }

    /// A step to `next(back)` committed, leaving `left`, which joins the other side.
    pub fn stepped(&mut self, back: bool, left: Url) {
        if back {
            self.back.pop();
            self.forward.insert(0, left);
        } else if !self.forward.is_empty() {
            self.forward.remove(0);
            self.back.push(left);
        }
    }

    /// A new page ends the forward list, as it ends the engine's own.
    pub fn new_page(&mut self) {
        self.forward.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restored_history_steps_both_ways_and_a_new_page_ends_its_forward_list() {
        let u = |s: &str| Url::parse(&format!("https://{s}.test/")).unwrap();
        let mut h = RestoredHistory { back: vec![u("a")], forward: vec![u("c")] };
        assert_eq!(h.next(true), Some(&u("a")));
        h.stepped(true, u("b"));
        assert_eq!(h, RestoredHistory { back: vec![], forward: vec![u("b"), u("c")] });
        assert_eq!((h.next(true), h.next(false)), (None, Some(&u("b"))));
        h.stepped(false, u("a"));
        assert_eq!(h, RestoredHistory { back: vec![u("a")], forward: vec![u("c")] });
        h.new_page();
        assert_eq!(h, RestoredHistory { back: vec![u("a")], forward: vec![] });
        h.stepped(false, u("x"));
        assert_eq!(h.back, [u("a")], "no forward entry, nothing moves");
    }

    #[test]
    fn a_saved_history_keeps_web_pages_and_reads_back_what_it_wrote() {
        let u = |s: &str| Url::parse(s).unwrap();
        let saved = SavedHistory::new(
            vec![u("https://a.test/"), u("file:///c:/x"), u("about:blank")],
            u("https://b.test/"),
            vec![u("https://c.test/"), u("javascript:alert(1)")],
        )
        .unwrap();
        assert_eq!(saved.back, [u("https://a.test/")]);
        assert_eq!(saved.forward, [u("https://c.test/")]);
        assert_eq!(SavedHistory::from_bytes(&saved.to_bytes()), Some(saved));
        assert_eq!(SavedHistory::new(Vec::new(), u("about:blank"), Vec::new()), None);
        assert_eq!(SavedHistory::from_bytes(b"bplist00"), None, "WebKit's own state is not ours");
        assert_eq!(
            SavedHistory::from_bytes(br#"{"tarn-history":1,"back":["file:///etc/passwd"],"current":"https://a.test/","forward":[]}"#).map(|s| s.back.len()),
            Some(0)
        );
        let long: Vec<Url> = (0..80).map(|i| u(&format!("https://a.test/{i}"))).collect();
        let kept = SavedHistory::new(long.clone(), u("https://b.test/"), long).unwrap();
        assert_eq!((kept.back.len(), kept.back[0].path(), kept.forward.len(), kept.forward[0].path()), (MAX_SAVED_HISTORY, "/30", MAX_SAVED_HISTORY, "/0"));
    }

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn nav(url: &Url) -> Navigation<'_> {
        Navigation { url, from_shell: false, user_link: false, user_step: false, page: None, upgraded: false, likely_subframe: false }
    }

    fn levels(default: ShieldLevel) -> Levels {
        Levels { default, sites: HashMap::new() }
    }

    #[test]
    fn https_only_upgrades_user_loads_on_any_port_and_blocks_the_rest() {
        let strict = levels(ShieldLevel::Strict);
        let local = u("http://localhost:8765/insecure");
        let typed = Navigation { from_shell: true, ..nav(&local) };
        assert_eq!(navigation(&strict, ShieldLevel::Strict, &typed), Verdict::Upgrade(u("https://localhost:8765/insecure")));
        let public = u("http://example.com/");
        let clicked = Navigation { user_link: true, page: Some(&public), ..nav(&public) };
        assert_eq!(navigation(&strict, ShieldLevel::Strict, &clicked), Verdict::Upgrade(u("https://example.com/")));
        assert_eq!(navigation(&strict, ShieldLevel::Strict, &nav(&public)), Verdict::Blocked("https_only"), "scripted");
        let again = Navigation { upgraded: true, ..typed };
        assert_eq!(navigation(&strict, ShieldLevel::Strict, &again), Verdict::Blocked("https_only"), "no upgrade loop");
        let standard = levels(ShieldLevel::Standard);
        assert_eq!(navigation(&standard, ShieldLevel::Standard, &typed), Verdict::Allow { javascript: true });
    }

    #[test]
    fn a_public_page_cannot_reach_the_local_network() {
        let any = levels(ShieldLevel::Standard);
        let page = u("https://public.test/");
        let target = u("http://192.168.1.1/");
        assert_eq!(navigation(&any, ShieldLevel::Standard, &Navigation { page: Some(&page), ..nav(&target) }), Verdict::Blocked("local_network"));
        let typed = Navigation { from_shell: true, page: Some(&page), ..nav(&target) };
        assert_eq!(navigation(&any, ShieldLevel::Standard, &typed), Verdict::Allow { javascript: true });
        let clicked = Navigation { user_link: true, page: Some(&page), ..nav(&target) };
        assert_eq!(navigation(&any, ShieldLevel::Standard, &clicked), Verdict::Blocked("local_network"), "a page can fake a click");
        let back = Navigation { user_step: true, page: Some(&page), ..nav(&target) };
        assert_eq!(navigation(&any, ShieldLevel::Standard, &back), Verdict::Allow { javascript: true }, "the user's history step");
    }

    #[test]
    fn schemes_follow_nav_policy() {
        let any = levels(ShieldLevel::Standard);
        let page = u("https://public.test/");
        let data = u("data:text/html,x");
        assert_eq!(navigation(&any, ShieldLevel::Standard, &Navigation { page: Some(&page), user_link: true, ..nav(&data) }), Verdict::Blocked("nav_policy"));
        assert_eq!(navigation(&any, ShieldLevel::Standard, &Navigation { likely_subframe: true, ..nav(&data) }), Verdict::Allow { javascript: true });
        assert_eq!(navigation(&any, ShieldLevel::Standard, &nav(&u("mailto:a@example.com"))), Verdict::External);
        assert_eq!(navigation(&any, ShieldLevel::Standard, &nav(&u("file:///etc/passwd"))), Verdict::Blocked("nav_policy"));
        let public = u("https://public.test/");
        assert_eq!(popup(&any, ShieldLevel::Standard, &u("https://a.test/"), Some(&public)), Verdict::Allow { javascript: true });
        assert_eq!(popup(&any, ShieldLevel::Standard, &u("javascript:1"), Some(&public)), Verdict::Blocked("nav_policy"));
        assert_eq!(
            popup(&any, ShieldLevel::Standard, &u("http://192.168.1.1/"), Some(&public)),
            Verdict::Blocked("shield"),
            "a popup is never the user's own load"
        );
        let local = u("http://localhost:3000/");
        assert_eq!(popup(&any, ShieldLevel::Standard, &u("http://192.168.1.1/"), Some(&local)), Verdict::Allow { javascript: true });
    }

    #[test]
    fn site_levels_rebuild_shell_loads_and_set_javascript_for_links() {
        let value = serde_json::json!({ "*": "standard", "locked.test": "lockdown", "bad": 3 });
        let levels = Levels::parse(&value, ShieldLevel::Strict);
        assert_eq!(levels.default, ShieldLevel::Standard);
        assert_eq!(levels.level(&u("https://bad/")), ShieldLevel::Lockdown, "an unknown level fails closed");
        assert_eq!(
            Levels::parse(&serde_json::json!({ "a.test": "strict" }), ShieldLevel::Lockdown).level(&u("https://b.test/")),
            ShieldLevel::Lockdown,
            "without \"*\" the view's level is the default"
        );
        let locked = u("https://locked.test./");
        assert_eq!(levels.level(&locked), ShieldLevel::Lockdown);
        let typed = Navigation { from_shell: true, ..nav(&locked) };
        assert_eq!(navigation(&levels, ShieldLevel::Standard, &typed), Verdict::Rebuild);
        let clicked = Navigation { user_link: true, ..nav(&locked) };
        assert_eq!(navigation(&levels, ShieldLevel::Standard, &clicked), Verdict::Rebuild, "a clicked link into Lockdown gets a Lockdown view");
        assert_eq!(
            navigation(&levels, ShieldLevel::Standard, &nav(&locked)),
            Verdict::Allow { javascript: false },
            "a script's navigation turns JavaScript off first"
        );
        assert!(needs_stricter_view(&levels, ShieldLevel::Standard, &locked));
        assert!(!needs_stricter_view(&levels, ShieldLevel::Lockdown, &locked));
        assert!(!needs_stricter_view(&levels, ShieldLevel::Standard, &u("https://open.test/")));
        assert!(!needs_stricter_view(&levels, ShieldLevel::Standard, &u("blob:https://locked.test/1")), "no host to judge");
        assert!(javascript_restored(&levels, ShieldLevel::Standard, &u("https://open.test/"), false));
        assert!(!javascript_restored(&levels, ShieldLevel::Standard, &u("https://open.test/"), true));
        assert!(!javascript_restored(&levels, ShieldLevel::Standard, &locked, false));
    }

    #[test]
    fn zoom_stays_in_range() {
        assert_eq!(zoom_factor(1.25), 1.25);
        assert_eq!(zoom_factor(9.0), MAX_PAGE_ZOOM);
        assert_eq!(zoom_factor(0.1), MIN_PAGE_ZOOM);
        assert_eq!(zoom_factor(f64::NAN), 1.0);
    }

    #[test]
    fn bursts_admit_a_few_per_window() {
        let mut burst = Burst::default();
        let t = Instant::now();
        assert!((0..3).all(|_| burst.admit(3, t)));
        assert!(!burst.admit(3, t));
        assert!(burst.admit(3, t + BURST_WINDOW));
    }

    #[test]
    fn credentials_are_asked_only_by_the_page_origin() {
        let page = u("https://a.test/x");
        assert!(same_origin(&u("https://a.test/login"), Some(&page)));
        assert!(!same_origin(&u("https://a.test:8443/"), Some(&page)));
        assert!(!same_origin(&u("https://b.test/"), Some(&page)));
        assert!(!same_origin(&u("https://a.test/"), None));
        assert!(may_ask_credentials(&u("https://a.test:443/"), Some(&page), MAX_AUTH_ATTEMPTS - 1));
        assert!(!may_ask_credentials(&u("https://a.test/"), Some(&page), MAX_AUTH_ATTEMPTS));
        assert!(!may_ask_credentials(&u("http://a.test/"), Some(&page), 0), "the challenge's own scheme counts");
        assert!(!may_ask_credentials(&u("https://ads.test/"), Some(&page), 0));
        assert_eq!(title(&"x".repeat(5000)).len(), MAX_TITLE);
        assert_eq!(without_credentials("https://u:p@a.test/f?x"), "https://a.test/f?x");
        assert_eq!(without_credentials("not a url"), "");
    }
}
