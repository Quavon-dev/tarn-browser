//! Shield for an engine that asks about each request instead of loading WebKit rule lists:
//! the same verdicts, read off the main thread from a snapshot the shell replaces on every change.

use shield::matcher::Matcher;
use shield::rules::is_local;
use shield::{Category, ShieldLevel};
use url::Url;

use crate::nav_policy::Site;

pub struct Shield {
    pub site_for: Box<dyn Fn(&Url) -> Site + Send + Sync>,
    /// The switched-on groups only.
    pub matcher: Matcher,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestAction {
    Cancel,
    Continue { upgrade: Option<Url>, strip_referrer: bool },
}

/// `kind` is the request type in adblock's names. `top` is the main frame's page, `None` when the
/// engine names none; for a main-frame load it is the page being left. `first_party` is the
/// engine's record of the request's top-level site, which cannot lag behind a navigation as a main
/// frame's URL can, so a local page just left does not exempt the next one; without it (a
/// cross-site frame) no page counts as local. A Lockdown site loads only in a `lockdown_view`, a view
/// made without JavaScript (Windows has no WebKit for it); any other view cancels its requests.
pub fn decide(
    shield: &Shield,
    url: &Url,
    kind: &str,
    top: Option<&Url>,
    first_party: Option<&Url>,
    referrer: Option<&Url>,
    lockdown_view: bool,
) -> RequestAction {
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return RequestAction::Continue { upgrade: None, strip_referrer: false };
    }
    let shown = top;
    let top = if kind == "main_frame" { Some(url) } else { top };
    let target = (shield.site_for)(url);
    let page = top.map_or(target, |t| (shield.site_for)(t));
    let policy = page.level.policy();
    let lockdown = !lockdown_view && (target.level == ShieldLevel::Lockdown || page.level == ShieldLevel::Lockdown);
    let local_page = top.is_some_and(|t| is_local(t, false)) && (kind == "main_frame" || first_party.is_some_and(|f| is_local(f, false)));
    let local = policy.block_private_network && is_local(url, true) && !local_page;
    let listed = match kind {
        "main_frame" => false,
        "sub_frame" => target.blocked.is_some(),
        _ => target.blocked == Some(Category::Malware),
    };
    let filtered = page.filtered && shield.matcher.blocks(url.as_str(), top.map_or("", Url::as_str), kind);
    if lockdown || local || listed || filtered {
        return RequestAction::Cancel;
    }
    // As WebKit's referrer policy and `make-https` rule, which belong to the page shown, the
    // page a link leaves also decides, so a link from a Strict page is upgraded and trimmed.
    let sender = if kind == "main_frame" { shown.map_or(page, |p| (shield.site_for)(p)) } else { page };
    let secure = match url.scheme() {
        "http" => "https",
        "ws" => "wss",
        _ => "",
    };
    // A URL with an explicit port is left alone, as by `make-https`.
    let https_only = policy.https_only || sender.level.policy().https_only;
    let upgrade = (https_only && url.port().is_none() && !secure.is_empty())
        .then(|| {
            let mut upgraded = url.clone();
            upgraded.set_scheme(secure).ok().map(|_| upgraded)
        })
        .flatten();
    let destination = upgrade.as_ref().unwrap_or(url);
    let strip_referrer = sender.level.policy().trim_referrer && referrer.is_some_and(|r| r.origin() != destination.origin());
    RequestAction::Continue { upgrade, strip_referrer }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav_policy::PopupRule;

    fn shield() -> Shield {
        let site_for = |url: &Url| {
            let host = url.host_str().unwrap_or_default();
            Site {
                level: match host {
                    "strict.test" => ShieldLevel::Strict,
                    "locked.test" => ShieldLevel::Lockdown,
                    _ => ShieldLevel::Standard,
                },
                filtered: host != "paused.test",
                blocked: match host {
                    "evil.test" => Some(Category::Malware),
                    "adult.test" => Some(Category::Adult),
                    _ => None,
                },
                malware: host == "evil.test",
                popups: PopupRule::Default,
            }
        };
        Shield { site_for: Box::new(site_for), matcher: Matcher::new(&["||ads.example^\n"], &["||tracker.example^\n"], &[]) }
    }

    #[test]
    fn lockdown_sites_load_only_in_a_lockdown_view() {
        let parse = |u: &str| Url::parse(u).unwrap();
        let (page, image) = (parse("https://locked.test/"), parse("http://cdn.test/a.png"));
        assert_eq!(decide(&shield(), &page, "main_frame", None, None, None, false), RequestAction::Cancel);
        assert_eq!(decide(&shield(), &page, "main_frame", None, None, None, true), RequestAction::Continue { upgrade: None, strip_referrer: false });
        let upgraded = decide(&shield(), &image, "image", Some(&page), Some(&page), None, true);
        assert!(
            matches!(upgraded, RequestAction::Continue { upgrade: Some(ref u), .. } if u.scheme() == "https"),
            "Lockdown keeps Strict's HTTPS: {upgraded:?}"
        );
    }

    fn go(top: &str, url: &str, kind: &str) -> RequestAction {
        let parse = |u: &str| Url::parse(u).unwrap();
        decide(&shield(), &parse(url), kind, Some(&parse(top)), Some(&parse(top)), Some(&parse(top)), false)
    }

    #[test]
    fn is_send_and_sync() {
        fn check<T: Send + Sync>() {}
        check::<Shield>();
    }

    #[test]
    fn request_table() {
        use RequestAction::*;
        let pass = Continue { upgrade: None, strip_referrer: false };
        let upgraded = |u: &str| Continue { upgrade: Some(Url::parse(u).unwrap()), strip_referrer: true };
        let cases = [
            ("https://news.test/", "https://tracker.example/t.js", "script", Cancel),
            ("https://news.test/", "https://ads.example/feed", "xhr", Cancel),
            ("https://news.test/", "wss://tracker.example/live", "websocket", Cancel),
            ("https://news.test/", "https://news.test/app.js", "script", pass.clone()),
            ("https://paused.test/", "https://tracker.example/t.js", "script", pass.clone()),
            ("https://news.test/", "https://evil.test/x.js", "image", Cancel),
            ("https://paused.test/", "https://evil.test/x.js", "image", Cancel),
            ("https://news.test/", "https://adult.test/x.png", "image", pass.clone()),
            ("https://news.test/", "https://adult.test/", "sub_frame", Cancel),
            ("https://news.test/", "https://evil.test/", "main_frame", pass.clone()),
            ("https://news.test/", "http://127.0.0.1:8801/count", "image", Cancel),
            ("https://news.test/", "http://192.168.1.1/", "sub_frame", Cancel),
            ("https://news.test/", "ws://localhost:9000/", "websocket", Cancel),
            ("https://paused.test/", "http://10.0.0.1/", "xhr", Cancel),
            ("http://127.0.0.1:8801/", "http://127.0.0.1:8802/count", "image", pass.clone()),
            ("https://news.test/", "http://127.0.0.1:8801/", "main_frame", pass.clone()),
            ("https://news.test/", "https://locked.test/x.js", "script", Cancel),
            ("https://locked.test/", "https://news.test/x.js", "script", Cancel),
            ("https://news.test/", "https://locked.test/", "main_frame", Cancel),
            ("https://strict.test/", "http://cdn.test/x.js", "script", upgraded("https://cdn.test/x.js")),
            ("https://strict.test/", "ws://cdn.test/live", "websocket", upgraded("wss://cdn.test/live")),
            (
                "https://news.test/",
                "http://strict.test/",
                "main_frame",
                Continue { upgrade: Some(Url::parse("https://strict.test/").unwrap()), strip_referrer: false },
            ),
            ("https://strict.test/", "https://news.test/", "main_frame", Continue { upgrade: None, strip_referrer: true }),
            ("https://strict.test/", "http://news.test/", "main_frame", upgraded("https://news.test/")),
            (
                "https://strict.test/",
                "http://strict.test/x.js",
                "script",
                Continue { upgrade: Some(Url::parse("https://strict.test/x.js").unwrap()), strip_referrer: false },
            ),
            ("https://strict.test/", "http://cdn.test:8080/x.js", "script", Continue { upgrade: None, strip_referrer: true }),
            ("https://strict.test/", "https://strict.test/x.js", "script", pass.clone()),
            ("https://news.test/", "http://cdn.test/x.js", "script", pass.clone()),
            ("https://news.test/", "data:text/plain,x", "other", pass.clone()),
        ];
        for (top, url, kind, expected) in cases {
            assert_eq!(go(top, url, kind), expected, "{url} on {top} as {kind}");
        }
    }

    #[test]
    fn a_public_first_party_overrides_a_local_main_frame_still_showing() {
        let s = shield();
        let url = |u: &str| Url::parse(u).unwrap();
        let (local, public) = (url("http://127.0.0.1:8801/"), url("https://news.test/"));
        let target = url("http://127.0.0.1:8802/count");
        assert_eq!(decide(&s, &target, "image", Some(&local), Some(&public), None, false), RequestAction::Cancel);
        assert_ne!(decide(&s, &target, "image", Some(&local), Some(&url("http://127.0.0.1/")), None, false), RequestAction::Cancel);
        assert_eq!(decide(&s, &target, "image", Some(&public), Some(&local), None, false), RequestAction::Cancel);
        assert_eq!(decide(&s, &target, "image", Some(&local), None, None, false), RequestAction::Cancel, "a cross-site frame has no first party");
        assert_eq!(
            decide(&s, &url("http://news.test/"), "main_frame", Some(&url("https://strict.test/")), None, None, false),
            RequestAction::Continue { upgrade: Some(url("https://news.test/")), strip_referrer: false },
            "a Strict page's link is upgraded without a referrer"
        );
    }

    #[test]
    fn workers_without_a_page_get_the_target_level_and_no_local_access() {
        let s = shield();
        let url = |u: &str| Url::parse(u).unwrap();
        assert_eq!(decide(&s, &url("https://tracker.example/sw"), "xhr", None, None, None, false), RequestAction::Cancel);
        assert_eq!(decide(&s, &url("http://127.0.0.1/"), "xhr", None, None, None, false), RequestAction::Cancel);
        assert_eq!(
            decide(&s, &url("http://strict.test/a"), "xhr", None, None, None, false),
            RequestAction::Continue { upgrade: Some(url("https://strict.test/a")), strip_referrer: false }
        );
    }
}
