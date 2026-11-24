//! The single gate for top-level navigations and URLs entering the browser (S6).

use std::rc::Rc;

use shield::{Category, ShieldLevel};
use url::Url;

/// The shell's verdict for a URL, asked on each navigation so per-site choices and domain sets apply.
pub type SiteFor = Rc<dyn Fn(&Url) -> Site>;

/// What a new view takes from its profile besides the Shield level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Features {
    /// The rule set with ad, tracker and cookie-banner blocking, off for paused sites.
    pub filtered: bool,
    /// The profile's web extensions, never at Lockdown; off in an agent's window (S22).
    pub extensions: bool,
}

/// What the shell decided about a site, asked on every navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    pub level: ShieldLevel,
    /// Ad, tracker and element-hiding rules and the YouTube script apply; false when paused.
    pub filtered: bool,
    /// The first switched-on domain set naming the host, unless the user let it through.
    pub blocked: Option<Category>,
    /// The switched-on malware set names the host, even when the user let it through; a download
    /// from it asks first.
    pub malware: bool,
    pub popups: PopupRule,
}

impl Site {
    pub fn new(level: ShieldLevel) -> Self {
        Site { level, filtered: true, blocked: None, malware: false, popups: PopupRule::Default }
    }
}

/// The user's pop-up choice for a site, as the opener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopupRule {
    #[default]
    Default,
    Allow,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateAction {
    Allow,
    /// Cancel and show the warning page for the category in place of the tab's page.
    Warn(Category),
    /// Cancel, keep the current page and say what was stopped, with a way to open it anyway.
    Notice(Category),
    /// Cancel without a word: a frame or download from a listed host.
    Cancel,
}

/// The navigation gate for any document load. `blocked` is the target's verdict; the shell's own
/// loads only skip the pop-up set, which exists to stop page-driven redirects, not typed visits.
pub fn gate(blocked: Option<Category>, main_frame: bool, from_shell: bool, download: bool) -> GateAction {
    match blocked {
        None => GateAction::Allow,
        Some(_) if download || !main_frame => GateAction::Cancel,
        Some(Category::Popups) if from_shell => GateAction::Allow,
        Some(c) if c.warns() => GateAction::Warn(c),
        Some(c) => GateAction::Notice(c),
    }
}

/// Whether a page may open `target` in a new window. WebKit already refuses pop-ups without a user
/// gesture; this adds the site's choice and the domain sets.
pub fn popup_allowed(opener: Site, target: Option<Site>) -> bool {
    let blocked = target.and_then(|t| t.blocked);
    match opener.popups {
        PopupRule::Block => false,
        PopupRule::Allow => !blocked.is_some_and(Category::warns),
        PopupRule::Default => blocked.is_none(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Typed in the command bar.
    Typed,
    Paste,
    Drop,
    /// URL event from another app.
    ExternalApp,
    /// Main-frame navigation started by web content.
    Link,
    /// `window.open` or `target=_blank` from web content.
    Popup,
    /// Explicit "Open File" by the user.
    FileOpen,
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Hand off to another app only after the user confirms.
    Prompt,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelAction {
    /// Rebuild the web view for the target's level and load it there.
    Rebuild,
    /// Continue in this view with JavaScript as given.
    Continue { javascript: bool },
    /// Plain HTTP into an HTTPS-only site that this view's rules would not upgrade.
    BlockInsecure,
}

/// What to do when a main-frame navigation goes to `site` and the view was built for `view` level
/// with `filtered` rules. Only loads the shell started may be rebuilt: reissuing a page-initiated
/// navigation would make it look typed by the user.
pub fn level_action(site: Site, view: ShieldLevel, filtered: bool, from_shell: bool, url: &Url) -> LevelAction {
    let site_level = site.level;
    if (site_level != view || site.filtered != filtered) && from_shell {
        LevelAction::Rebuild
    } else if url.scheme() == "http" && site_level.policy().https_only && !view.policy().https_only {
        LevelAction::BlockInsecure
    } else {
        LevelAction::Continue { javascript: site_level.max(view).policy().javascript }
    }
}

pub fn decide(url: &Url, source: Source) -> Decision {
    match url.scheme() {
        "http" | "https" => Decision::Allow,
        "about" if url.path() == "blank" => Decision::Allow,
        "blob" => match source {
            Source::Link | Source::Popup => Decision::Allow,
            _ => Decision::Block,
        },
        "file" if source == Source::FileOpen => Decision::Allow,
        "about" | "data" | "file" | "javascript" | "vbscript" | "tarn" | "view-source" | "webkit" | "x-apple-internal" => Decision::Block,
        _ => match source {
            Source::Typed | Source::Link | Source::Popup => Decision::Prompt,
            _ => Decision::Block,
        },
    }
}

/// Whether a main-frame navigation is an Open File the user chose: `vouched` when the view
/// showed `url` through its own `load_file` and this is that load or the user's Back, Forward or
/// Reload onto it. Such a navigation is decided with `Source::FileOpen`; no other way, a typed
/// address, a restored history or a page's link included, reaches `file:`.
pub fn file_open(url: &Url, vouched: bool) -> bool {
    url.scheme() == "file" && vouched
}

/// What a refused `file:` load says: only Open File shows a local file.
pub const FILE_NOT_OPENED: &str = "Local files open only with Open File.";

/// Whether a page may load `url` as a subresource: a `file:` one only from what the opened file
/// it belongs to may read (`file_read_access`), symlinks resolved, for engines whose own sandbox
/// does not limit it. The path is checked as written before the disk is asked, and a `file:` URL
/// naming a host is refused unasked, since resolving it would reach that host (SMB on Windows).
pub fn file_subresource_allowed(url: &Url, top: Option<&Url>) -> bool {
    if url.scheme() != "file" {
        return true;
    }
    let (Some(path), Some(grant)) = (local_path(url), top.and_then(file_read_access)) else { return false };
    path.starts_with(&grant) && matches!((path.canonicalize(), grant.canonicalize()), (Ok(path), Ok(grant)) if path.starts_with(&grant))
}

/// What an engine may read for an opened file: the file's own folder, so its images and styles
/// load; only the file itself when that folder is a root or holds a home folder, which would
/// open the user's other files to the page. File pages cannot read other files with scripts.
pub fn file_read_access(url: &Url) -> Option<std::path::PathBuf> {
    let file = local_path(url)?;
    let folder = file.parent()?;
    let real = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
    let home = std::env::home_dir().into_iter().flat_map(|home| [home.canonicalize().ok(), Some(home)]).flatten();
    let holds_home = |home: std::path::PathBuf| home.ancestors().any(|a| a == folder || a == real || same_file(a, &real));
    let broad = volume_root(&real) || home.into_iter().any(holds_home);
    Some(if broad { file } else { folder.to_path_buf() })
}

/// A filesystem or drive root, or the top of a mounted volume (`/Volumes/USB`, `/media/u/USB`).
fn volume_root(folder: &std::path::Path) -> bool {
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;
    match folder.parent() {
        None => true,
        #[cfg(unix)]
        Some(parent) => std::fs::metadata(folder).ok().zip(std::fs::metadata(parent).ok()).is_some_and(|(f, p)| f.dev() != p.dev()),
        #[cfg(not(unix))]
        Some(_) => false,
    }
}

/// The same file or folder under two paths, such as a firmlink (`/System/Volumes/Data/Users`).
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(a).ok().zip(std::fs::metadata(b).ok()).is_some_and(|(a, b)| (a.dev(), a.ino()) == (b.dev(), b.ino()))
    }
    #[cfg(not(unix))]
    {
        a == b
    }
}

/// A local path without a host, and on Windows without an alternate data stream (`a.html:s`).
fn local_path(url: &Url) -> Option<std::path::PathBuf> {
    let path = (url.scheme() == "file" && url.host().is_none()).then(|| url.to_file_path().ok()).flatten()?;
    let stream = cfg!(windows) && path.components().any(|c| matches!(c, std::path::Component::Normal(name) if name.to_string_lossy().contains(':')));
    (!stream).then_some(path)
}

/// Whether a main-frame navigation is the user's: a load or history step the shell started, or a
/// server redirect of the load it started for the user. A link the shell loads for the page (a
/// peek) and anything else that happens while the user's load is pending are the page's own.
pub fn user_started(shell_load: bool, link: bool, history_step: bool, redirect: bool, pending_user_load: bool) -> bool {
    (shell_load && !link) || history_step || (redirect && pending_user_load)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StoreNav {
    pub redirect: bool,
    pub shell_load: bool,
    pub page_link: bool,
    pub clicked: bool,
}

pub fn store_link_asks(nav: StoreNav) -> bool {
    !nav.redirect && !nav.page_link && (nav.shell_load || nav.clicked)
}

pub fn chromium_store_link(redirect: bool, shell_load: bool, link_load: bool, user_gesture: bool) -> bool {
    store_link_asks(StoreNav { redirect, shell_load, page_link: link_load, clicked: user_gesture })
}

/// Whether a main-frame navigation away from the page `from` into a local address is refused (a
/// router's admin page, a local service): a script may not take a tab from the public web onto the
/// local network; the user may, by navigations the caller judges as theirs (typed, history steps the shell started; in Chromium also a click with a user gesture), also through redirects. A `blob:`
/// page counts as its inner origin; any page without an http(s) origin (`about:blank`, none) counts
/// as public, so callers pass the opener's origin for a popup that has not committed one.
pub fn into_private_network(target: &Url, from: Option<&Url>, user_started: bool, block_private_network: bool) -> bool {
    !user_started && block_private_network && shield::rules::local_target(target) && !from.is_some_and(local_page)
}

/// The notice for a navigation `into_private_network` refused; shells offer to open it anyway.
pub const LOCAL_NETWORK_BLOCKED: &str = "An address on your local network was blocked.";

pub fn open_anyway_notice(url: &Url) -> Option<String> {
    if !url.username().is_empty() || url.password().is_some() || !shield::rules::local_target(url) {
        return None;
    }
    let host = url.host_str().unwrap_or_default();
    let host = url.port().map_or_else(|| host.to_owned(), |p| format!("{host}:{p}"));
    let path: String = url.path().chars().take(40).collect();
    let elided = url.query().is_some() || url.fragment().is_some() || url.path().chars().count() > 40;
    let shown = format!("{host}{}{}", path.trim_end_matches('/'), if elided { "…" } else { "" });
    (shown.chars().count() <= 70).then(|| format!("{}: {shown}", LOCAL_NETWORK_BLOCKED.trim_end_matches('.')))
}

/// What a notice says about a navigation a domain set stopped.
pub fn blocked_message(category: Category, url: &Url) -> String {
    let host = url.host_str().unwrap_or_default();
    match category {
        Category::Malware => format!("{host} is listed as malicious."),
        Category::Adult => format!("{host} is listed as adult content."),
        Category::Popups => format!("Stopped a redirect to {host}, a known pop-up ad site."),
        Category::Ads | Category::Trackers | Category::Annoyances => format!("{host} is blocked."),
    }
}

pub fn popup_blocked_notice(url: Option<&Url>) -> String {
    match url.and_then(Url::host_str) {
        Some(host) => format!("Blocked a pop-up window to {host}."),
        None => "Blocked a pop-up window.".to_owned(),
    }
}

/// Whether a link another app hands over loads as if the user typed it: only a loopback address
/// (this computer's own services), and only while the user allows it. Other local addresses stay
/// links, so S10 still refuses them.
pub fn app_link_typed(url: &Url, loopback_allowed: bool) -> bool {
    loopback_allowed && shield::rules::is_loopback(url)
}

/// Whether the Strict rules load this plain-HTTP URL over HTTPS; `make-https` and the request
/// filter both leave a URL with an explicit port alone.
pub fn upgrades_to_https(url: &Url, https_only: bool) -> bool {
    https_only && url.scheme() == "http" && url.port().is_none()
}

/// Whether a page counts as local, so it may send a tab into the local network; never by a
/// target-only router name.
pub fn local_page(page: &Url) -> bool {
    let inner = (page.scheme() == "blob").then(|| Url::parse(page.path()).ok()).flatten();
    let page = inner.as_ref().unwrap_or(page);
    matches!(page.scheme(), "http" | "https") && (shield::rules::is_local(page, false) || shield::rules::home_network_name(page))
}

/// Chromium's built-in PDF viewer, which shows a PDF a page loads in a frame of its own.
const PDF_VIEWER: &str = "mhjfbmdgcfjbbpaeojofohoefgiehjai";

/// What a frame inside a Chromium page may load: web content and Chromium's PDF viewer, never a
/// URL handed to another app or another browser page.
pub fn chromium_subframe_allowed(url: &Url) -> bool {
    match url.scheme() {
        "http" | "https" | "blob" | "data" => true,
        "about" => matches!(url.path(), "blank" | "srcdoc"),
        "chrome-extension" => cfg!(windows) && url.host_str() == Some(PDF_VIEWER),
        _ => false,
    }
}

/// Chromium views load web pages only; every other scheme is blocked, never handed to an app.
pub fn decide_chromium(url: &Url, source: Source) -> Decision {
    match url.scheme() {
        "http" | "https" => Decision::Allow,
        "about" if url.path() == "blank" => Decision::Allow,
        "blob" if matches!(source, Source::Link | Source::Popup) => Decision::Allow,
        _ => Decision::Block,
    }
}

/// A link from a Chromium page to another app's scheme (`mailto:`, `tel:`), which the shell
/// offers to hand off as WebKit tabs do; Chromium's own schemes are never one.
pub fn chromium_external(url: &Url, source: Source) -> bool {
    !matches!(
        url.scheme(),
        "chrome"
            | "chrome-error"
            | "chrome-extension"
            | "chrome-native"
            | "chrome-untrusted"
            | "chrome-search"
            | "devtools"
            | "edge"
            | "isolated-app"
            | "blob"
            | "filesystem"
            | "ws"
            | "wss"
            | "ftp"
    ) && decide(url, source) == Decision::Prompt
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_users_own_navigation_asks_for_a_store_install() {
        use super::{StoreNav, chromium_store_link, store_link_asks};
        let asks = |redirect, shell_load, page_link, clicked| store_link_asks(StoreNav { redirect, shell_load, page_link, clicked });
        assert!(asks(false, true, false, false), "typed");
        assert!(asks(false, false, false, true), "a click the engine credits to the user");
        assert!(!asks(false, false, false, false), "a script's navigation without a gesture");
        assert!(!asks(true, true, false, false) && !asks(true, false, false, true), "a redirect never");
        assert!(!asks(false, true, true, false), "a link the shell loads for a page (peek, pop-up tab)");
        assert!(chromium_store_link(false, false, false, true), "Chromium: a click with a gesture");
        assert!(chromium_store_link(false, true, false, true), "Chromium: the shell's own load, which CEF reports as a gesture");
        assert!(!chromium_store_link(true, false, false, true), "Chromium: the redirect of a gesture's navigation");
        assert!(!chromium_store_link(false, true, true, true), "Chromium: a peek or pop-up the shell loads for the page");
        assert!(!chromium_store_link(false, false, false, false), "Chromium: a script navigation without a gesture");
    }

    #[test]
    fn chromium_hands_off_only_app_schemes() {
        use super::Source::*;
        let external = |s: &str, source| super::chromium_external(&url::Url::parse(s).unwrap(), source);
        assert!(external("mailto:a@b.test", Link));
        assert!(external("tel:+491234", Popup));
        assert!(external("zoommtg://zoom.us/join", Typed));
        assert!(!external("mailto:a@b.test", Sync));
        for blocked in [
            "https://a.test/",
            "chrome://settings",
            "chrome-error://chromewebdata/",
            "wss://a.test/",
            "chrome-extension://abc/x",
            "devtools://devtools/x",
            "file:///c:/x",
            "javascript:alert(1)",
            "data:text/html,x",
            "view-source:https://a.test/",
            "blob:https://a.test/x",
        ] {
            assert!(!external(blocked, Link), "{blocked}");
        }
    }

    #[test]
    fn only_portless_plain_http_is_upgraded() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert!(super::upgrades_to_https(&u("http://a.test/x"), true));
        assert!(!super::upgrades_to_https(&u("http://a.test/x"), false));
        assert!(!super::upgrades_to_https(&u("http://a.test:8080/"), true));
        assert!(!super::upgrades_to_https(&u("https://a.test/"), true));
    }

    #[test]
    fn open_anyway_names_the_address_and_skips_credentials_and_long_ones() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        assert_eq!(super::open_anyway_notice(&u("http://192.168.1.1/")).as_deref(), Some("An address on your local network was blocked: 192.168.1.1"));
        assert_eq!(
            super::open_anyway_notice(&u("http://localhost:3000/a/?q=1")).as_deref(),
            Some("An address on your local network was blocked: localhost:3000/a…")
        );
        assert_eq!(super::open_anyway_notice(&u("http://user@10.0.0.2/")), None);
        assert_eq!(super::open_anyway_notice(&u("http://u:p@10.0.0.2/")), None);
        let long = format!("http://{}.local/", "a".repeat(80));
        assert_eq!(super::open_anyway_notice(&u(&long)), None);
        assert_eq!(super::open_anyway_notice(&u("https://tracker.example/pixel")), None);
    }

    #[test]
    fn only_loopback_links_from_apps_load_as_typed_and_only_when_allowed() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        for typed in [
            "http://localhost:3000/",
            "http://LOCALHOST./",
            "http://127.0.0.1:8080/",
            "http://127.9.9.9/",
            "http://0.0.0.0:8000/",
            "http://[::1]:5173/",
            "http://[::ffff:127.0.0.1]/",
        ] {
            assert!(super::app_link_typed(&u(typed), true), "{typed}");
            assert!(!super::app_link_typed(&u(typed), false), "{typed}");
        }
        for link in [
            "http://192.168.1.1/",
            "http://fritz.box/",
            "http://printer.local/",
            "http://10.0.0.2/",
            "http://app.localhost/",
            "http://[::ffff:192.168.1.1]/",
            "http://user@localhost/",
            "https://example.com/",
        ] {
            assert!(!super::app_link_typed(&u(link), true), "{link}");
        }
    }

    #[test]
    fn a_public_page_cannot_take_a_tab_onto_the_local_network() {
        let u = |s: &str| url::Url::parse(s).unwrap();
        let public = u("https://news.test/");
        assert!(super::into_private_network(&u("http://192.168.1.1/admin"), Some(&public), false, true));
        assert!(super::into_private_network(&u("http://localhost:3000/"), Some(&public), false, true));
        assert!(!super::into_private_network(&u("http://192.168.1.1/"), Some(&public), true, true), "started by the user");
        assert!(!super::into_private_network(&u("http://192.168.1.1/"), Some(&u("http://localhost:8080/")), false, true), "from a local page");
        assert!(super::into_private_network(&u("http://192.168.1.1/"), Some(&u("about:blank")), false, true), "popup without an opener");
        assert!(super::into_private_network(&u("http://192.168.1.1/"), None, false, true), "no page");
        assert!(super::into_private_network(&u("http://192.168.1.1/"), Some(&u("blob:https://news.test/1")), false, true), "public blob");
        assert!(super::into_private_network(&u("http://192.168.1.1/"), Some(&u("data:text/html,x")), false, true));
        assert!(super::into_private_network(&u("http://192.168.1.1/"), Some(&u("blob:blob:http://localhost/1")), false, true));
        assert!(!super::into_private_network(&u("http://192.168.1.1/"), Some(&u("blob:http://localhost:8080/1")), false, true), "local blob");
        assert!(!super::into_private_network(&u("http://192.168.1.1/"), None, true, true), "typed into an empty tab");
        assert!(!super::into_private_network(&u("http://192.168.1.1/"), Some(&public), false, false), "level without the rule");
        assert!(!super::into_private_network(&u("https://other.test/"), Some(&public), false, true));
        for name in ["http://printer.local./", "http://nas.home.arpa/", "http://router.lan/"] {
            assert!(super::into_private_network(&u(name), Some(&public), false, true), "{name}");
        }
        assert!(super::into_private_network(&u("http://fritz.box/"), Some(&public), false, true), "a router's own name");
        assert!(!super::local_page(&u("http://fritz.box/")) && super::local_page(&u("http://nas.local/")), "a router name is a target only");
        assert!(super::into_private_network(&u("http://192.168.1.1/"), Some(&u("http://fritz.box/")), false, true));
        for public_name in ["https://shop.box/", "http://go/x", "https://box.com/"] {
            assert!(!super::into_private_network(&u(public_name), Some(&public), false, true), "{public_name}");
        }
        assert!(super::into_private_network(&u("http://192.168.1.1/"), Some(&u("https://evil.box/")), false, true), "a registrable suffix is public");
        assert!(!super::into_private_network(&u("http://nas.local/settings"), Some(&u("http://nas.local/")), false, true), "within a home site");
        assert!(!super::into_private_network(&u("http://192.168.1.1/"), Some(&u("http://nas.local/")), false, true), "from a home site");
    }
    #[test]
    fn chromium_subframes_load_web_content_only() {
        let allowed = |u: &str| super::chromium_subframe_allowed(&url::Url::parse(u).unwrap());
        for ok in ["https://a.test/", "http://a.test/", "blob:https://a.test/x", "data:text/html,x", "about:blank", "about:srcdoc"] {
            assert!(allowed(ok), "{ok}");
        }
        assert_eq!(allowed("chrome-extension://mhjfbmdgcfjbbpaeojofohoefgiehjai/index.html"), cfg!(windows));
        for no in ["mailto:a@b.test", "chrome://settings", "about:version", "file:///etc/hosts", "javascript:1", "zoommtg://x", "chrome-extension://abc/x"] {
            assert!(!allowed(no), "{no}");
        }
    }

    use super::Decision::*;
    use super::Source::*;
    use super::*;

    fn decide_str(input: &str, source: Source) -> Decision {
        Url::parse(input).map_or(Decision::Block, |url| decide(&url, source))
    }

    #[test]
    fn only_shell_loads_history_steps_and_their_redirects_are_the_users() {
        assert!(user_started(true, false, false, false, false), "a typed URL");
        assert!(user_started(false, false, true, false, false), "a history step");
        assert!(user_started(false, false, false, true, true), "a redirect of the typed URL");
        assert!(!user_started(true, true, false, false, false), "a peeked link");
        assert!(!user_started(false, false, false, true, false), "a redirect of the page's own navigation");
        assert!(!user_started(false, false, false, false, true), "a script navigation while the user's load is pending");
    }

    #[test]
    fn level_changes_rebuild_only_shell_loads() {
        use ShieldLevel::*;
        let https = Url::parse("https://a.test/").unwrap();
        let http = Url::parse("http://a.test/").unwrap();
        let s = Site::new;
        assert_eq!(level_action(s(Strict), Standard, true, true, &http), LevelAction::Rebuild);
        assert_eq!(level_action(s(Strict), Standard, true, false, &https), LevelAction::Continue { javascript: true });
        assert_eq!(level_action(s(Strict), Standard, true, false, &http), LevelAction::BlockInsecure);
        assert_eq!(level_action(s(Strict), Lockdown, true, false, &http), LevelAction::Continue { javascript: false });
        assert_eq!(level_action(s(Standard), Lockdown, true, false, &https), LevelAction::Continue { javascript: false });
        assert_eq!(level_action(s(Lockdown), Standard, true, false, &https), LevelAction::Continue { javascript: false });
        assert_eq!(level_action(s(Standard), Standard, true, true, &http), LevelAction::Continue { javascript: true });
    }

    #[test]
    fn pausing_rebuilds_shell_loads_and_page_loads_keep_the_view() {
        let https = Url::parse("https://a.test/").unwrap();
        let paused = Site { filtered: false, ..Site::new(ShieldLevel::Standard) };
        assert_eq!(level_action(paused, ShieldLevel::Standard, true, true, &https), LevelAction::Rebuild);
        assert_eq!(level_action(Site::new(ShieldLevel::Standard), ShieldLevel::Standard, false, true, &https), LevelAction::Rebuild);
        assert_eq!(level_action(paused, ShieldLevel::Standard, true, false, &https), LevelAction::Continue { javascript: true });
    }

    #[test]
    fn gate_table() {
        use Category::*;
        use GateAction::*;
        // (blocked, main frame, from shell, download) -> action
        let cases = [
            (None, true, false, false, Allow),
            (None, false, false, true, Allow),
            (Some(Malware), true, false, false, Warn(Malware)),
            (Some(Malware), true, true, false, Warn(Malware)),
            (Some(Adult), true, true, false, Warn(Adult)),
            (Some(Malware), false, false, false, Cancel),
            (Some(Adult), false, false, false, Cancel),
            (Some(Malware), true, false, true, Cancel),
            (Some(Popups), true, false, false, Notice(Popups)),
            (Some(Popups), true, true, false, Allow),
            (Some(Popups), false, false, false, Cancel),
            (Some(Popups), true, false, true, Cancel),
        ];
        for (blocked, main, shell, download, expected) in cases {
            assert_eq!(gate(blocked, main, shell, download), expected, "{blocked:?} main={main} shell={shell} download={download}");
        }
    }

    #[test]
    fn popup_verdicts() {
        let site = |popups, blocked| Site { popups, blocked, ..Site::new(ShieldLevel::Standard) };
        use PopupRule::*;
        assert!(popup_allowed(site(Default, None), Some(site(Default, None))));
        assert!(popup_allowed(site(Default, None), None));
        assert!(!popup_allowed(site(Default, None), Some(site(Default, Some(Category::Popups)))));
        assert!(popup_allowed(site(Allow, None), Some(site(Default, Some(Category::Popups)))));
        assert!(!popup_allowed(site(Allow, None), Some(site(Default, Some(Category::Malware)))));
        assert!(!popup_allowed(site(Block, None), Some(site(Default, None))));
    }

    #[test]
    fn table() {
        let all = [Typed, Paste, Drop, ExternalApp, Link, Popup, FileOpen, Sync];
        let cases: &[(&str, &[Source], Decision)] = &[
            ("https://example.com/", &all, Allow),
            ("http://localhost:3000/", &all, Allow),
            ("about:blank", &all, Allow),
            ("about:config", &all, Block),
            ("blob:https://example.com/1234", &[Link, Popup], Allow),
            ("blob:https://example.com/1234", &[Typed, Paste, Drop, ExternalApp, Sync], Block),
            ("data:text/html,<script>alert(1)</script>", &all, Block),
            ("javascript:alert(1)", &all, Block),
            ("JaVaScRiPt:alert(1)", &all, Block),
            ("vbscript:msgbox", &all, Block),
            ("file:///etc/passwd", &[FileOpen], Allow),
            ("file:///etc/passwd", &[Typed, Paste, Drop, ExternalApp, Link, Popup, Sync], Block),
            ("tarn://settings", &all, Block),
            ("view-source:https://example.com", &all, Block),
            ("mailto:a@example.com", &[Typed, Link, Popup], Prompt),
            ("mailto:a@example.com", &[Paste, Drop, ExternalApp, Sync], Block),
            ("zoommtg://join", &[Link], Prompt),
            ("not a url", &all, Block),
        ];
        for (url, sources, expected) in cases {
            for source in *sources {
                assert_eq!(decide_str(url, *source), *expected, "{url} from {source:?}");
            }
        }
    }

    #[test]
    fn only_the_users_open_file_reaches_file_urls() {
        let file = Url::parse("file:///Users/u/page.html").unwrap();
        assert!(file_open(&file, true));
        assert!(!file_open(&file, false), "a page's own navigation");
        assert!(!file_open(&Url::parse("https://a.test/").unwrap(), true));
    }

    #[test]
    fn an_opened_file_loads_only_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (site, other) = (dir.path().join("site"), dir.path().join("other"));
        std::fs::create_dir_all(site.join("img")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        for f in [site.join("index.html"), site.join("img/a.png"), other.join("secret.txt")] {
            std::fs::write(f, b"x").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(other.join("secret.txt"), site.join("link.txt")).unwrap();
        let f = |p: std::path::PathBuf| Url::from_file_path(p).unwrap();
        let u = |s: &str| Url::parse(s).unwrap();
        let top = f(site.join("index.html"));
        assert!(file_subresource_allowed(&f(site.join("img/a.png")), Some(&top)));
        assert!(file_subresource_allowed(&u("https://a.test/x.js"), Some(&top)));
        let escapes = [
            f(other.join("secret.txt")),
            u(&format!("{}/../other/secret.txt", f(site.clone()))),
            u(&format!("{}%2F..%2Fother%2Fsecret.txt", f(site.clone()))),
            f(site.join("link.txt")),
        ];
        for refused in escapes {
            assert!(!file_subresource_allowed(&refused, Some(&top)), "{refused}");
        }
        assert!(!file_subresource_allowed(&f(site.join("img/a.png")), Some(&u("https://a.test/"))));
        assert!(!file_subresource_allowed(&f(site.join("img/a.png")), None));
        assert!(!file_subresource_allowed(&u("file:///etc/passwd"), Some(&u("http://localhost/x"))), "a web page is no opened file");
        assert!(!file_subresource_allowed(&u("file://evil.test/s/a.png"), Some(&top)), "a host is never asked");
    }

    #[cfg(windows)]
    #[test]
    fn an_alternate_data_stream_is_never_read() {
        let stream = Url::parse("file:///C:/d/a.html%3As").unwrap();
        assert_eq!(file_read_access(&stream), None);
        assert!(!file_subresource_allowed(&stream, Some(&Url::parse("file:///C:/d/index.html").unwrap())));
    }

    #[test]
    fn a_file_in_the_home_folder_or_a_root_reads_only_itself() {
        let home = std::env::home_dir().unwrap();
        let root = if cfg!(windows) { "C:\\page.html" } else { "/page.html" };
        for file in [home.join("page.html"), std::path::PathBuf::from(root)] {
            assert_eq!(file_read_access(&Url::from_file_path(&file).unwrap()), Some(file));
        }
        #[cfg(target_os = "macos")]
        if home.starts_with("/Users") {
            let firmlink = std::path::Path::new("/System/Volumes/Data").join(home.strip_prefix("/").unwrap()).join("page.html");
            assert_eq!(file_read_access(&Url::from_file_path(&firmlink).unwrap()), Some(firmlink), "the home folder by another path");
        }
        let deep = home.join("Sites/blog/index.html");
        assert_eq!(file_read_access(&Url::from_file_path(&deep).unwrap()), Some(home.join("Sites/blog")));
    }

    #[test]
    fn chromium_views_allow_only_web_pages() {
        let cases = [
            ("https://a.test/", Typed, Allow),
            ("http://a.test/", Link, Allow),
            ("about:blank", Link, Allow),
            ("blob:https://a.test/x", Link, Allow),
            ("blob:https://a.test/x", Typed, Block),
            ("mailto:a@b.test", Link, Block),
            ("chrome://settings", Typed, Block),
            ("chrome-extension://abc/x.html", Link, Block),
            ("file:///etc/passwd", FileOpen, Block),
            ("data:text/html,x", Link, Block),
            ("javascript:alert(1)", Typed, Block),
            ("view-source:https://a.test/", Typed, Block),
        ];
        for (input, source, expected) in cases {
            assert_eq!(Url::parse(input).map_or(Block, |url| decide_chromium(&url, source)), expected, "{input} {source:?}");
        }
    }
}
