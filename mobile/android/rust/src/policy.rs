use std::sync::{OnceLock, RwLock};

use engine::content_rules::{self, Request, ResourceKind, RuleList, Verdict};
use engine::nav_policy::{self, Decision, LevelAction, Site, Source};
use engine::view_policy;
use shield::ShieldLevel;
use shield::matcher::Matcher;
use url::Url;

pub const ALLOW: i32 = 0;
pub const PROMPT: i32 = 1;
pub const BLOCK: i32 = 2;

pub const REBUILD: i32 = 0;
pub const CONTINUE_JS: i32 = 1;
pub const CONTINUE_NO_JS: i32 = 2;
pub const BLOCK_INSECURE: i32 = 3;

pub const VERDICT_ALLOW: i32 = 0;
pub const VERDICT_BLOCK: i32 = 1;
pub const VERDICT_MAKE_HTTPS: i32 = 2;

const SOURCES: [Source; 4] = [Source::Typed, Source::Link, Source::Popup, Source::FileOpen];

const KINDS: [(ResourceKind, &str); 11] = [
    (ResourceKind::Document, "main_frame"),
    (ResourceKind::Subdocument, "sub_frame"),
    (ResourceKind::Image, "image"),
    (ResourceKind::StyleSheet, "stylesheet"),
    (ResourceKind::Script, "script"),
    (ResourceKind::Font, "font"),
    (ResourceKind::Media, "media"),
    (ResourceKind::Fetch, "xmlhttprequest"),
    (ResourceKind::WebSocket, "websocket"),
    (ResourceKind::Ping, "ping"),
    (ResourceKind::Other, "other"),
];

static FILTERS: RwLock<Option<Matcher>> = RwLock::new(None);

pub fn level(index: i32) -> ShieldLevel {
    usize::try_from(index).ok().and_then(|i| ShieldLevel::ALL.get(i).copied()).unwrap_or(ShieldLevel::Lockdown)
}

fn builtin(level: ShieldLevel) -> &'static RuleList {
    static LISTS: [OnceLock<RuleList>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    LISTS[level as usize].get_or_init(|| RuleList::parse(&shield::rules::builtin_rules(&level.policy())).unwrap_or_default())
}

pub fn warm() -> usize {
    ShieldLevel::ALL.iter().map(|&l| builtin(l).len()).sum()
}

pub fn install_filters(matcher: Option<Matcher>) {
    *FILTERS.write().unwrap_or_else(|e| e.into_inner()) = matcher;
}

fn filters() -> Option<Matcher> {
    FILTERS.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn decide(url: &str, source: i32) -> i32 {
    let source = usize::try_from(source).ok().and_then(|i| SOURCES.get(i).copied()).unwrap_or(Source::Link);
    match Url::parse(url).map_or(Decision::Block, |url| nav_policy::decide(&url, source)) {
        Decision::Allow => ALLOW,
        Decision::Prompt => PROMPT,
        Decision::Block => BLOCK,
    }
}

pub fn level_action(url: &str, site: i32, view: i32, from_shell: bool) -> i32 {
    let Ok(url) = Url::parse(url) else {
        return BLOCK_INSECURE;
    };
    match nav_policy::level_action(Site::new(level(site)), level(view), true, from_shell, &url) {
        LevelAction::Rebuild => REBUILD,
        LevelAction::Continue { javascript: true } => CONTINUE_JS,
        LevelAction::Continue { javascript: false } => CONTINUE_NO_JS,
        LevelAction::BlockInsecure => BLOCK_INSECURE,
    }
}

pub fn into_private_network(target: &str, from: Option<&str>, user_started: bool, level_index: i32) -> bool {
    let Ok(target) = Url::parse(target) else {
        return true;
    };
    let from = from.and_then(|f| Url::parse(f).ok());
    nav_policy::into_private_network(&target, from.as_ref(), user_started, level(level_index).policy().block_private_network)
}

pub fn may_ask_credentials(challenge: &str, page: Option<&str>, failures: i32) -> bool {
    let (Ok(challenge), Ok(failures)) = (Url::parse(challenge), u32::try_from(failures)) else {
        return false;
    };
    let page = page.and_then(|p| Url::parse(p).ok());
    view_policy::may_ask_credentials(&challenge, page.as_ref(), failures)
}

fn kind_names(kind: i32) -> (ResourceKind, &'static str) {
    usize::try_from(kind).ok().and_then(|i| KINDS.get(i).copied()).unwrap_or((ResourceKind::Other, "other"))
}

pub(crate) fn resource_kind(kind: i32) -> ResourceKind {
    kind_names(kind).0
}

pub fn verdict(url: &str, top: Option<&str>, kind: i32, level_index: i32) -> i32 {
    let Ok(parsed) = Url::parse(url) else {
        return VERDICT_BLOCK;
    };
    let top_url = top.and_then(|t| Url::parse(t).ok());
    let (kind, adblock_kind) = kind_names(kind);
    let request = Request { url: &parsed, top: top_url.as_ref(), kind };
    let verdict = content_rules::evaluate([builtin(level(level_index))], &request);
    if verdict == Verdict::Block {
        return VERDICT_BLOCK;
    }
    if adblock_kind != "main_frame" && filters().is_some_and(|m| m.blocks(url, top.unwrap_or_default(), adblock_kind)) {
        return VERDICT_BLOCK;
    }
    match verdict {
        Verdict::Allow => VERDICT_ALLOW,
        Verdict::Block => VERDICT_BLOCK,
        Verdict::MakeHttps => VERDICT_MAKE_HTTPS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: i32 = 0;
    const SUB_FRAME: i32 = 1;
    const IMAGE: i32 = 2;
    const SCRIPT: i32 = 4;
    const FETCH: i32 = 7;

    #[test]
    fn every_builtin_rule_is_understood() {
        for l in ShieldLevel::ALL {
            let json = shield::rules::builtin_rules(&l.policy());
            let rules: Vec<serde_json::Value> = serde_json::from_str(&json).expect("built-in rules are JSON");
            assert_eq!(builtin(l).len(), rules.len(), "{l:?}");
        }
        assert!(warm() > 0);
    }

    #[test]
    fn credentials_only_for_the_page_origin() {
        let page = Some("https://a.test/x");
        assert!(may_ask_credentials("https://a.test/", page, 0));
        assert!(!may_ask_credentials("https://a.test/", page, 3));
        assert!(!may_ask_credentials("https://b.test/", page, 0));
        assert!(!may_ask_credentials("https://a.test/", None, 0));
        assert!(!may_ask_credentials("https://a.test/", page, -1));
        assert!(!may_ask_credentials("not a url", page, 0));
    }

    #[test]
    fn navigation_codes() {
        assert_eq!(decide("https://example.com/", 1), ALLOW);
        assert_eq!(decide("mailto:a@example.com", 1), PROMPT);
        assert_eq!(decide("javascript:alert(1)", 1), BLOCK);
        assert_eq!(decide("data:text/html,x", 1), BLOCK);
        assert_eq!(decide("intent://x#Intent;end", 2), PROMPT);
        assert_eq!(decide("not a url", 1), BLOCK);
        assert_eq!(decide("https://example.com/", 99), ALLOW);
        assert_eq!(decide("file:///data/opened/0/a.html", 3), ALLOW);
        for source in [0, 1, 2, 99] {
            assert_eq!(decide("file:///data/opened/0/a.html", source), BLOCK);
        }
    }

    #[test]
    fn level_codes() {
        assert_eq!(level_action("https://a.test/", 0, 0, false), CONTINUE_JS);
        assert_eq!(level_action("https://a.test/", 2, 2, false), CONTINUE_NO_JS);
        assert_eq!(level_action("https://a.test/", 2, 0, false), CONTINUE_NO_JS, "a page-initiated load keeps the stricter level");
        assert_eq!(level_action("https://a.test/", 2, 0, true), REBUILD);
        assert_eq!(level_action("http://a.test/", 1, 0, false), BLOCK_INSECURE);
        assert_eq!(level_action("not a url", 0, 0, true), BLOCK_INSECURE);
        assert_eq!(level(-1), ShieldLevel::Lockdown, "an unknown level is the strictest");
        assert_eq!(level(7), ShieldLevel::Lockdown);
    }

    #[test]
    fn page_initiated_loads_cannot_enter_the_local_network() {
        assert!(into_private_network("http://192.168.1.1/", Some("https://public.test/"), false, 0));
        assert!(into_private_network("http://localhost:8771/", None, false, 0));
        assert!(!into_private_network("http://localhost:8771/", Some("https://public.test/"), true, 0), "the user may");
        assert!(!into_private_network("http://localhost:8771/b", Some("http://localhost:8771/a"), false, 0));
        assert!(!into_private_network("https://public.test/", Some("https://other.test/"), false, 0));
        assert!(into_private_network("not a url", None, true, 0));
    }

    #[test]
    fn shield_codes() {
        let public = Some("https://public.test/");
        assert_eq!(verdict("http://127.0.0.1:8765/pixel", public, IMAGE, 0), VERDICT_BLOCK);
        assert_eq!(verdict("http://localhost:8765/pixel", public, IMAGE, 0), VERDICT_BLOCK);
        assert_eq!(verdict("http://localhost:8765/pixel", Some("http://localhost:8765/"), IMAGE, 0), VERDICT_ALLOW);
        assert_eq!(verdict("http://localhost:8765/", Some("http://localhost:8765/"), MAIN, 0), VERDICT_ALLOW);
        assert_eq!(verdict("http://example.com/", Some("http://example.com/"), MAIN, 1), VERDICT_MAKE_HTTPS);
        assert_eq!(verdict("http://example.com/", Some("http://example.com/"), MAIN, 0), VERDICT_ALLOW);
        assert_eq!(verdict("http://example.com/", None, SUB_FRAME, 7), VERDICT_MAKE_HTTPS);
        assert_eq!(verdict("not a url", public, IMAGE, 0), VERDICT_BLOCK);
    }

    #[test]
    fn filter_lists_block_subresources_but_not_pages() {
        install_filters(Some(Matcher::new(&["||ads.test^\n"], &[], &[])));
        let page = Some("http://public.test/filter");
        assert_eq!(verdict("http://ads.test/banner.js", page, SCRIPT, 0), VERDICT_BLOCK);
        assert_eq!(verdict("http://ads.test/", Some("http://ads.test/"), MAIN, 0), VERDICT_ALLOW, "the navigation gate judges pages");
        assert_eq!(verdict("http://ok.test/app.js", page, SCRIPT, 0), VERDICT_ALLOW);
        install_filters(Some(Matcher::new(&[], &["||tracker.test^$third-party\n"], &[])));
        assert_eq!(
            verdict("http://tracker.test/collect", None, FETCH, 0),
            VERDICT_BLOCK,
            "a service worker's request has no top page and counts as third-party"
        );
        install_filters(None);
        assert_eq!(verdict("http://ads.test/banner.js", page, SCRIPT, 0), VERDICT_ALLOW);
    }
}
