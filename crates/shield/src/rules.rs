//! WebKit content rule lists that Shield builds itself, independent of filter lists.

use std::net::Ipv4Addr;

use serde_json::{Value, json};
use url::{Host, Url};

use crate::Policy;
use crate::domains;
use crate::lists::MAX_RULES_PER_LIST;

/// Hosts in loopback, private, CGNAT, link-local and unspecified ranges, each matched up to its
/// end so `127.example.com` is not local. WebKit's rule regex has no alternation, so each range is
/// its own pattern.
const LOCAL_HOSTS: &[&str] = &[
    r"127\.[0-9.]*",
    r"10\.[0-9.]*",
    r"192\.168\.[0-9.]*",
    r"172\.1[6-9]\.[0-9.]*",
    r"172\.2[0-9]\.[0-9.]*",
    r"172\.3[01]\.[0-9.]*",
    r"169\.254\.[0-9.]*",
    r"100\.6[4-9]\.[0-9.]*",
    r"100\.[7-9][0-9]\.[0-9.]*",
    r"100\.1[01][0-9]\.[0-9.]*",
    r"100\.12[0-7]\.[0-9.]*",
    r"0\.0\.0\.0\.?",
    r"localhost\.?",
    r"[-_a-z0-9.]*\.localhost\.?",
    r"\[::1\]",
    r"\[::\]",
    r"\[f[cd][0-9a-f:]*\]",
    r"\[fe[89ab][0-9a-f:]*\]",
];

/// IPv6 forms that embed any IPv4 address (mapped, NAT64): blocked as targets, but never a local
/// top page, since `[::ffff:808:808]` reaches a public server.
const EMBEDDED_V4_HOSTS: &[&str] = &[r"\[::ffff:[0-9a-f:.]*\]", r"\[64:ff9b::[0-9a-f:.]*\]"];

/// A host ends at the path or at a numeric port, never at a password (`127.0.0.1:x@evil.com`).
const HOST_ENDS: [&str; 2] = ["/", ":[0-9]+/"];

/// Anchored patterns for local URLs; `target` adds the `http://user@10.0.0.1/` forms and IPv6
/// forms embedding IPv4. The top-page exception leaves those out, so a URL with credentials or
/// an embedded public IPv4 address never counts as local.
fn local_url_patterns(target: bool) -> Vec<String> {
    let prefixes: &[&str] = if target { &["^[a-z]+://", "^[a-z]+://[^/]*@"] } else { &["^[a-z]+://"] };
    let embedded = if target { EMBEDDED_V4_HOSTS } else { &[] };
    LOCAL_HOSTS
        .iter()
        .chain(embedded)
        .flat_map(|h| HOST_ENDS.iter().map(move |end| format!("{h}{end}")))
        .flat_map(|host| prefixes.iter().map(move |p| format!("{p}{host}")))
        .collect()
}

/// `local_url_patterns` for an engine that checks each request itself: `target` counts
/// credentials and IPv6 forms embedding IPv4 as local, a top page with either is never local.
pub fn is_local(url: &Url, target: bool) -> bool {
    if !target && (!url.username().is_empty() || url.password().is_some()) {
        return false;
    }
    match url.host() {
        Some(Host::Domain(d)) => {
            let d = d.trim_end_matches('.');
            d == "localhost" || d.ends_with(".localhost")
        }
        Some(Host::Ipv4(ip)) => local_v4(ip),
        Some(Host::Ipv6(ip)) => {
            let s = ip.segments();
            let embeds_v4 = ip.to_ipv4_mapped().is_some() || s[..6] == [0x64, 0xff9b, 0, 0, 0, 0];
            if embeds_v4 { target } else { ip.is_loopback() || ip.is_unspecified() || s[0] & 0xfe00 == 0xfc00 || s[0] & 0xffc0 == 0xfe80 }
        }
        None => false,
    }
}

/// Host names under suffixes that never resolve publicly (mDNS, RFC 8375, ICANN's reserved and
/// undelegated ones), which `is_local` cannot see as addresses. Registrable suffixes such as
/// `.box` and single-label names (intranet go-links) are left out on purpose. For navigation
/// checks only; the content rules cannot match names that are not addresses.
pub fn home_network_name(url: &Url) -> bool {
    const SUFFIXES: [&str; 6] = [".local", ".lan", ".home", ".home.arpa", ".internal", ".localdomain"];
    let Some(Host::Domain(host)) = url.host() else { return false };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    SUFFIXES.iter().any(|s| host.ends_with(s))
}

/// A navigation target S10 keeps public pages away from: a local address, a home-network name or
/// a router name that is public in the DNS but answered by the router itself (`fritz.box`). Such
/// a name is a target only: a page under it is never treated as local, since `.box` is registrable.
pub fn local_target(url: &Url) -> bool {
    const ROUTER_NAMES: [&str; 1] = ["fritz.box"];
    let router = matches!(url.host(), Some(Host::Domain(h)) if ROUTER_NAMES.contains(&h.trim_end_matches('.').to_ascii_lowercase().as_str()));
    is_local(url, true) || home_network_name(url) || router
}

/// This computer, without credentials: `localhost`, 127.0.0.0/8, `0.0.0.0` (which macOS connects
/// here), `::1` and loopback mapped into IPv6. Subdomains of `localhost` are left out: the system
/// resolver, not a rule, decides where they go.
pub fn is_loopback(url: &Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    match url.host() {
        Some(Host::Domain(d)) => d.trim_end_matches('.').eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Some(Host::Ipv6(ip)) => ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
        None => false,
    }
}

fn local_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || (a == 100 && b & 0xc0 == 64)
}

/// Blocks requests to local addresses unless the top-level page is local itself (S10).
fn private_network_rules() -> Vec<Value> {
    let unless_top = local_url_patterns(false);
    local_url_patterns(true)
        .into_iter()
        .map(|filter| {
            json!({
                "trigger": { "url-filter": filter, "unless-top-url": unless_top },
                "action": { "type": "block" }
            })
        })
        .collect()
}

fn https_upgrade_rules() -> Vec<Value> {
    vec![json!({ "trigger": { "url-filter": "^http://" }, "action": { "type": "make-https" } })]
}

/// The built-in rule list for a policy, as WebKit content-blocker JSON.
pub fn builtin_rules(policy: &Policy) -> String {
    let mut rules = Vec::new();
    if policy.block_private_network {
        rules.extend(private_network_rules());
    }
    if policy.https_only {
        rules.extend(https_upgrade_rules());
    }
    Value::Array(rules).to_string()
}

/// Every resource type except `document`: a listed host cannot load into a page, while top-level
/// navigations to it reach the navigation gate and its warning page instead of a WebKit error.
const SUBRESOURCES: &[&str] = &["image", "style-sheet", "script", "font", "raw", "svg-document", "media", "popup", "ping", "fetch", "websocket", "other"];

/// Part of the malware group's key; bump it when `domain_rules` changes its output.
pub const DOMAIN_RULES_VERSION: &str = "\0domain-rules-2";

/// Block rules for the hosts in `text` (one per line) and their subdomains, chunked under WebKit's
/// limit. Lines are checked again with `domains::entry`, so the bundle cannot widen them.
pub fn domain_rules(text: &str) -> Vec<String> {
    let rules: Vec<Value> = text
        .lines()
        .filter_map(domains::entry)
        .map(|d| {
            json!({
                "trigger": { "url-filter": format!("^[^:]+://+([^/]*@)?([^:/]+\\.)?{}\\.?[:/]", d.replace('.', "\\.")), "resource-type": SUBRESOURCES },
                "action": { "type": "block" }
            })
        })
        .collect();
    rules.chunks(MAX_RULES_PER_LIST).map(|c| Value::Array(c.to_vec()).to_string()).collect()
}

/// YouTube's ad containers, the anti-adblock notice and the player's ad overlay. Written for Tarn,
/// on with the YouTube switch beside `youtube::script`.
const YOUTUBE_SELECTORS: &[&str] = &[
    "#masthead-ad",
    "#player-ads",
    "ytd-ad-slot-renderer",
    "ytd-in-feed-ad-layout-renderer",
    "ytd-banner-promo-renderer",
    "ytd-statement-banner-renderer",
    "ytd-display-ad-renderer",
    "ytd-promoted-sparkles-web-renderer",
    "ytd-promoted-video-renderer",
    "ytd-player-legacy-desktop-watch-ads-renderer",
    "ytd-engagement-panel-section-list-renderer[target-id=\"engagement-panel-ads\"]",
    "ytd-enforcement-message-view-model",
    ".ytp-ad-overlay-container",
    ".ytp-ad-image-overlay",
    "ad-slot-renderer",
    "ytm-promoted-sparkles-web-renderer",
    "ytm-companion-ad-renderer",
];

pub fn youtube_rules() -> String {
    let domains = ["*youtube.com", "*youtube-nocookie.com", "*youtubekids.com"];
    let rules: Vec<Value> = YOUTUBE_SELECTORS
        .iter()
        .map(|s| json!({ "trigger": { "url-filter": ".*", "if-domain": domains }, "action": { "type": "css-display-none", "selector": s } }))
        .collect();
    Value::Array(rules).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ShieldLevel;

    #[test]
    fn domain_rules_skip_documents_and_revalidate() {
        let chunks = domain_rules("evil.example\ncom\ngoogle.com\nnot a host\n203.0.113.7\n");
        assert_eq!(chunks.len(), 1);
        let rules: Vec<Value> = serde_json::from_str(&chunks[0]).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["trigger"]["url-filter"], "^[^:]+://+([^/]*@)?([^:/]+\\.)?evil\\.example\\.?[:/]");
        assert!(rules.iter().all(|r| !r["trigger"]["resource-type"].as_array().unwrap().contains(&json!("document"))));
        assert!(domain_rules("").is_empty());
    }

    #[test]
    fn youtube_rules_stay_on_youtube() {
        let rules: Vec<Value> = serde_json::from_str(&youtube_rules()).unwrap();
        assert_eq!(rules.len(), YOUTUBE_SELECTORS.len());
        assert!(rules.iter().all(|r| r["trigger"]["if-domain"][0] == "*youtube.com" && r["action"]["type"] == "css-display-none"));
    }

    #[test]
    fn standard_blocks_local_network_without_https_upgrade() {
        let rules: Vec<Value> = serde_json::from_str(&builtin_rules(&ShieldLevel::Standard.policy())).unwrap();
        assert_eq!(rules.len(), (LOCAL_HOSTS.len() + EMBEDDED_V4_HOSTS.len()) * HOST_ENDS.len() * 2);
        assert!(rules.iter().all(|r| r["action"]["type"] == "block"));
        assert!(rules.iter().all(|r| !r["trigger"]["url-filter"].as_str().unwrap().contains('|')));
    }

    fn matches(patterns: &[String], url: &str) -> bool {
        patterns.iter().any(|p| regex::Regex::new(&format!("(?i){p}")).unwrap().is_match(url))
    }

    #[test]
    fn local_patterns_stop_at_the_host() {
        let blocked = local_url_patterns(true);
        let top = local_url_patterns(false);
        for url in [
            "http://127.0.0.1/",
            "http://127.0.0.1:8080/x",
            "http://10.1.2.3./",
            "http://user:pw@192.168.1.1/",
            "http://100.100.100.100/",
            "http://0.0.0.0:3000/",
            "http://localhost:3000/",
            "http://app.localhost/",
            "http://my_app.localhost:8080/",
            "http://[::1]:8080/",
            "http://[::ffff:7f00:1]/",
            "http://[fd12::1]/",
            "http://[fe9a::1]/",
            "http://[64:ff9b::a00:1]/",
        ] {
            assert!(matches(&blocked, url), "{url} should be local");
            assert!(is_local(&Url::parse(url).unwrap(), true), "{url} should be local");
        }
        for url in ["http://127.0.0.1/", "http://localhost:3000/", "http://[::1]/"] {
            assert!(matches(&top, url), "{url} should be a local top page");
            assert!(is_local(&Url::parse(url).unwrap(), false), "{url}");
        }
        for url in [
            "https://127.attacker.com/",
            "https://10.1.attacker.com/",
            "https://192.168.attacker.com/",
            "https://0.0.0.0.attacker.com/",
            "https://100.64.example/",
            "https://100.128.0.1/",
            "https://localhost.attacker.com/",
            "https://localhost:x@evil.com/",
            "https://a.localhost:1@evil.com/",
            "https://127.0.0.1:80@evil.com/",
            "https://127.0.0.1@evil.com/",
            "https://evil.com/?http://127.0.0.1/",
            "http://[::ffff:808:808]/",
            "http://[64:ff9b::808:808]/",
        ] {
            assert!(!matches(&top, url), "{url} must not lift the private-network block");
            assert!(!is_local(&Url::parse(url).unwrap(), false), "{url} must not lift the private-network block");
        }
    }

    #[test]
    fn strict_adds_https_upgrade() {
        let rules: Vec<Value> = serde_json::from_str(&builtin_rules(&ShieldLevel::Strict.policy())).unwrap();
        assert!(rules.iter().any(|r| r["action"]["type"] == "make-https"));
    }
}
