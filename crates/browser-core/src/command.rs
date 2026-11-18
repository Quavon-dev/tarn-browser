use std::net::IpAddr;

use engine::nav_policy::{self, Decision, Source};
use url::Url;

use crate::text::bounded_origin;

pub struct Engine {
    pub name: &'static str,
    url: &'static str,
    param: &'static str,
}

/// The first entry is the default.
pub const ENGINES: [Engine; 6] = [
    Engine { name: "DuckDuckGo", url: "https://duckduckgo.com/", param: "q" },
    Engine { name: "Startpage", url: "https://www.startpage.com/sp/search", param: "query" },
    Engine { name: "Brave Search", url: "https://search.brave.com/search", param: "q" },
    Engine { name: "Ecosia", url: "https://www.ecosia.org/search", param: "q" },
    Engine { name: "Google", url: "https://www.google.com/search", param: "q" },
    Engine { name: "Bing", url: "https://www.bing.com/search", param: "q" },
];
const KNOWN_SCHEMES: &[&str] =
    &["http", "https", "about", "file", "data", "javascript", "vbscript", "blob", "view-source", "tarn", "mailto", "tel", "sms", "facetime"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Navigate(Url),
    Search(Url),
    /// Another app's scheme; ask before handing it off.
    External(Url),
    Rejected,
    /// An entry of the app's command table, by index.
    Command(usize),
}

impl Input {
    pub fn url(&self) -> Option<&Url> {
        match self {
            Input::Navigate(u) | Input::Search(u) | Input::External(u) => Some(u),
            Input::Rejected | Input::Command(_) => None,
        }
    }
}

pub fn search_url(engine: &Engine, query: &str) -> Url {
    let mut url = Url::parse(engine.url).expect("static URL");
    url.query_pairs_mut().append_pair(engine.param, query);
    url
}

/// Turns command-bar text into a URL to load or a search with `engine`.
pub fn classify_input(raw: &str, engine: &Engine) -> Option<Input> {
    let search_url = |query: &str| search_url(engine, query);
    let input = raw.trim();
    if input.is_empty() {
        return None;
    }
    if let Some((scheme, _)) = input.split_once(':') {
        let scheme = scheme.to_ascii_lowercase();
        if input.contains("://") || KNOWN_SCHEMES.contains(&scheme.as_str()) {
            return Some(match Url::parse(input) {
                Ok(url) => by_policy(url),
                Err(_) if KNOWN_SCHEMES.contains(&scheme.as_str()) => Input::Rejected,
                Err(_) => Input::Search(search_url(input)),
            });
        }
    }
    if input.contains(char::is_whitespace) {
        return Some(Input::Search(search_url(input)));
    }
    Some(match host_like(input) {
        Some(url) => by_policy(url),
        None => Input::Search(search_url(input)),
    })
}

fn by_policy(url: Url) -> Input {
    match nav_policy::decide(&url, Source::Typed) {
        Decision::Allow => Input::Navigate(url),
        Decision::Prompt => Input::External(url),
        Decision::Block => Input::Rejected,
    }
}

/// `example.com/x`, `localhost:3000`, `192.168.0.1` and the like, without a scheme.
fn host_like(input: &str) -> Option<Url> {
    let local = Url::parse(&format!("http://{input}")).ok()?;
    let host = local.host_str()?;
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if host == "localhost" || host.ends_with(".localhost") || bare.parse::<IpAddr>().is_ok() {
        return Some(local);
    }
    let tld = host.rsplit('.').next()?;
    let has_tld = host.contains('.') && tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic());
    let punycode_tld = tld.starts_with("xn--");
    (has_tld || punycode_tld).then(|| Url::parse(&format!("https://{input}")).ok())?
}

pub const GLYPHS: &[(&str, &str)] = &[
    ("Settings", "gear"),
    ("Site Settings", "shield"),
    ("New Tab Group", "group"),
    ("New Tab", "plus"),
    ("New Window", "app-window"),
    ("New Space", "layout-grid"),
    ("Edit Space", "pencil"),
    ("Private Space", "hand"),
    ("Burner Tab", "flame"),
    ("New Burner Tab", "flame"),
    ("New Profile", "user-plus"),
    ("Edit Profile", "person"),
    ("Bookmark", "bookmark"),
    ("Remove Bookmark", "bookmark"),
    ("Manage Bookmarks", "bookmark"),
    ("Save for Later", "bookmark"),
    ("Reading List", "bookmark"),
    ("Reopen Closed Tab", "undo-2"),
    ("Toggle Sidebar", "sidebar"),
    ("Auto-Hide Sidebar", "sidebar"),
    ("Compact Toolbar", "app-window"),
    ("Close All Tabs", "close"),
    ("Close Duplicate Tabs", "copy"),
    ("Close Split View", "close"),
    ("Show Library", "library"),
    ("Save Space as Snapshot", "archive"),
    ("Restore Snapshot", "history"),
    ("Show Downloads", "circle-arrow-down"),
    ("Reload", "reload"),
    ("Copy Link", "link"),
    ("Copy Clean Link", "link"),
    ("Copy Title", "copy"),
    ("Pin", "pin"),
    ("Unpin", "pin"),
    ("Add to Tiles", "layout-grid"),
    ("Remove from Tiles", "layout-grid"),
    ("Close Tab", "close"),
    ("New Subfolder", "folder-plus"),
    ("Rotate Layout", "reload"),
    ("Remove from Split View", "arrow-up-right"),
    ("Mute or Unmute Site", "volume-x"),
    ("Show History", "history"),
    ("Reset Tab Title", "undo-2"),
    ("Return to Pinned URL", "undo-2"),
    ("Set Pinned URL", "pin"),
    ("Sleep Other Tabs", "moon"),
    ("Show Memory Use", "gauge"),
    ("Memory", "gauge"),
    ("Zap Element", "zap"),
    ("Open or Close Split View", "columns-2"),
    ("Focus Next Pane", "chevron-right"),
    ("Focus Previous Pane", "chevron-left"),
    ("Swap Split Panes", "arrow-left-right"),
    ("Rotate Split Layout", "reload"),
    ("Remove Tab from Split View", "arrow-up-right"),
    ("Reader", "book-open"),
    ("Show Reader", "book-open"),
    ("Hide Reader", "book-open"),
    ("Stop Loading", "close"),
    ("Close Window", "close"),
    ("Picture in Picture", "picture-in-picture-2"),
    ("Take Screenshot", "camera"),
    ("Save Page as PDF", "file-down"),
    ("Print", "printer"),
    ("Share", "share"),
    ("Show QR Code", "qr-code"),
    ("Find in Tabs", "search"),
    ("Find", "search"),
    ("Keyboard Shortcuts", "keyboard"),
    ("Clean Up", "broom-sparkles"),
    ("Clear Site Data", "trash"),
    ("Open Link in New Tab", "app-window"),
    ("Open in Split View", "split-add"),
    ("Open in Private", "hand"),
    ("Inspect", "code"),
    ("Open", "arrow-up-right"),
    ("Open in New Tab", "app-window"),
    ("Duplicate", "copy"),
    ("Copy Download Link", "link"),
    ("Paste and Go", "arrow-up-right"),
    ("Mute Site", "volume-x"),
    ("Unmute Site", "volume-2"),
    ("Rename", "pencil"),
    ("Change Icon", "sparkles"),
    ("Reset Title", "undo-2"),
    ("Return to ", "undo-2"),
    ("Close Other Tabs", "close"),
    ("Close Tabs Below", "close"),
    ("Close Group", "close"),
    ("Unload Tab", "moon"),
    ("Sleep Tab", "moon"),
    ("New Group", "group"),
    ("Ungroup", "group"),
    ("Remove from Group", "group"),
    ("New Folder", "folder-plus"),
    ("Delete", "trash"),
    ("Remove from List", "close"),
    ("Restore in", "history"),
    ("Customize in Settings", "gear"),
    ("Show Tabs in Sidebar", "sidebar"),
    ("Auto-Hide Tabs", "sidebar"),
    ("Classic Toolbar", "app-window"),
];

pub fn glyph(label: &str) -> &'static str {
    GLYPHS.iter().filter(|(start, _)| label.starts_with(start)).max_by_key(|(start, _)| start.len()).map_or("command", |(_, g)| g)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Commands,
    Tabs,
    Suggestions,
    History,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Commands => "COMMANDS",
            Section::Tabs => "TABS",
            Section::Suggestions => "SUGGESTIONS",
            Section::History => "HISTORY",
        }
    }
}

pub fn section_headers(rows: &[Section]) -> Vec<Option<Section>> {
    rows.iter().enumerate().map(|(ix, s)| (ix == 0 || rows[ix - 1] != *s).then_some(*s)).collect()
}

/// The longest host the address shows in full.
pub const MAX_HOST: usize = 40;

/// The setting that names a profile's search engine; the private space has its own.
pub fn engine_key(private: bool, profile: store::Id) -> String {
    if private { "search_engine.private".into() } else { format!("search_engine.{profile}") }
}

/// Keeps the end of a long host, where the registrable domain is, and drops its start.
pub fn host_tail(host: &str, max: usize) -> String {
    let count = host.chars().count();
    if count <= max {
        return host.to_owned();
    }
    let tail: String = host.chars().skip(count - (max - 1)).collect();
    format!("…{tail}")
}

/// The host a page belongs to; a `blob:` page belongs to the host of the URL it wraps.
pub fn page_host(url: &Url) -> Option<String> {
    let inner = (url.scheme() == "blob").then(|| Url::parse(url.path()).ok()).flatten();
    inner.as_ref().unwrap_or(url).host_str().map(|h| h.to_ascii_lowercase())
}

/// What names a page to the user: its host, or for a page without one, such as an `about:blank`
/// its opener filled, its scheme alone, since the rest of such a URL is the page's own choice.
pub fn page_label(url: &Url) -> String {
    page_host(url).unwrap_or_else(|| if url.scheme() == "about" { bounded_origin(&format!("about:{}", url.path())) } else { format!("{}:", url.scheme()) })
}

/// The address at rest: the page's label without `www.`, and whether it was loaded without TLS.
/// An opened file shows `File:` and its name, marked since its author chose the name.
pub fn address(url: &Url) -> (String, bool) {
    let host = match file_name(url) {
        Some(name) => format!("File: {}", host_tail(&name, MAX_HOST)),
        None => host_tail(page_label(url).trim_start_matches("www."), MAX_HOST),
    };
    (host, url.scheme() == "http" || url.as_str().starts_with("blob:http:"))
}

/// The name of a local file a `file:` URL shows, safe to display; a tab without a page title
/// shows it.
pub fn file_name(url: &Url) -> Option<String> {
    let path = url.to_file_path().ok().filter(|_| url.scheme() == "file" && url.host().is_none())?;
    path.file_name().map(|n| engine::downloads::sanitize_filename(&n.to_string_lossy()))
}

/// The address at rest as text: `Not Secure — host` for plain HTTP, empty without a page.
pub fn display_host(url: &str) -> String {
    match Url::parse(url).map(|u| address(&u)) {
        Ok((host, true)) => format!("Not Secure — {host}"),
        Ok((host, false)) => host,
        Err(_) => String::new(),
    }
}

/// Subsequence match score of `query` in `text`, higher is better; `None` if it does not match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    fuzzy_chars(&fuzzy_query(query), text)
}

/// `query` as `fuzzy_chars` takes it: folded by `fold`, without whitespace.
pub fn fuzzy_query(query: &str) -> Vec<char> {
    query.chars().flat_map(fold).filter(|c| !c.is_whitespace()).collect()
}

/// Lowercase one character at a time, with the final sigma as plain sigma: only whole-string
/// lowercasing knows where a word ends, and matching must not depend on it.
pub fn fold(c: char) -> impl Iterator<Item = char> {
    c.to_lowercase().map(|c| if c == 'ς' { 'σ' } else { c })
}

/// `fuzzy_score` for a query already passed through `fuzzy_query`, without allocating.
pub fn fuzzy_chars(query: &[char], text: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let lower = |c: char| fold(c).next().unwrap_or(c);
    let (mut qi, mut score, mut last, mut prev, mut len) = (0, 0i64, None::<usize>, None::<char>, 0usize);
    for (ti, c) in text.chars().enumerate() {
        if qi < query.len() && lower(c) == query[qi] {
            let word_start = prev.is_none_or(|p| !p.is_alphanumeric() || (c.is_uppercase() && p.is_lowercase()));
            score += match last {
                Some(l) if l + 1 == ti => 8,
                _ if word_start => 6,
                _ => 1,
            };
            last = Some(ti);
            qi += 1;
        }
        prev = Some(c);
        len = ti + 1;
    }
    (qi == query.len()).then(|| score * 100 - len as i64)
}

/// Indices of the `(label, keywords)` commands that `query` names, best first. Plain text must
/// start words of the label or keywords; after a leading `>` any fuzzy match counts.
pub fn rank_commands<'a>(query: &str, commands: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<usize> {
    let query = query.trim();
    let (forced, query) = match query.strip_prefix('>') {
        Some(rest) => (true, rest.trim().to_lowercase()),
        None => (false, query.to_lowercase()),
    };
    if !forced && query.chars().count() < 3 {
        return Vec::new();
    }
    let mut ranked: Vec<_> = commands
        .into_iter()
        .enumerate()
        .filter_map(|(ix, (label, keywords))| {
            let text = format!("{label} {keywords}").to_lowercase();
            let by_words = query.split_whitespace().all(|q| text.split_whitespace().any(|w| w.starts_with(q)));
            let fuzzy = fuzzy_score(&query, label).or_else(|| fuzzy_score(&query, &text).map(|s| s - 1_000_000));
            (by_words || forced && fuzzy.is_some()).then(|| ((label.to_lowercase().starts_with(&query), by_words, fuzzy.unwrap_or(i64::MIN)), ix))
        })
        .collect();
    ranked.sort_by_key(|(rank, _)| std::cmp::Reverse(*rank));
    ranked.into_iter().map(|(_, ix)| ix).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_header_starts_each_run_of_one_kind() {
        use super::{Section::*, section_headers};
        assert_eq!(
            section_headers(&[Suggestions, Commands, Commands, Tabs, History, History]),
            [Some(Suggestions), Some(Commands), None, Some(Tabs), Some(History), None]
        );
        assert_eq!(section_headers(&[Tabs, History, Tabs]), [Some(Tabs), Some(History), Some(Tabs)]);
        assert!(section_headers(&[]).is_empty());
    }

    use super::*;

    fn kind(s: &str) -> Option<Input> {
        classify_input(s, &ENGINES[0])
    }

    fn nav(s: &str) -> String {
        match kind(s) {
            Some(Input::Navigate(u)) => u.to_string(),
            other => panic!("{s}: {other:?}"),
        }
    }

    #[test]
    fn urls() {
        assert_eq!(nav("example.com"), "https://example.com/");
        assert_eq!(nav("  https://example.com/a?b=c "), "https://example.com/a?b=c");
        assert_eq!(nav("localhost:3000"), "http://localhost:3000/");
        assert_eq!(nav("localhost"), "http://localhost/");
        assert_eq!(nav("127.0.0.1:8080/x"), "http://127.0.0.1:8080/x");
        assert_eq!(nav("[::1]:3000"), "http://[::1]:3000/");
        assert_eq!(nav("app.localhost:5173"), "http://app.localhost:5173/");
        assert_eq!(nav("example.com:8443/x"), "https://example.com:8443/x");
        assert_eq!(nav("münchen.de"), "https://xn--mnchen-3ya.de/");
        assert_eq!(nav("about:blank"), "about:blank");
    }

    #[test]
    fn dangerous_input_is_rejected() {
        for s in [
            "javascript:alert(1)",
            "JavaScript:alert(document.cookie)",
            " javascript:void(0) x",
            "data:text/html,hi",
            "file:///etc/passwd",
            "vbscript:x",
            "tarn://settings",
            "view-source:https://a.test",
        ] {
            assert_eq!(kind(s), Some(Input::Rejected), "{s}");
        }
    }

    #[test]
    fn searches_and_externals() {
        assert!(matches!(kind("rust lifetimes"), Some(Input::Search(_))));
        assert!(matches!(kind("re:invent"), Some(Input::Search(_))));
        assert!(matches!(kind("hello"), Some(Input::Search(_))));
        assert!(matches!(kind("what is a.b"), Some(Input::Search(_))));
        assert!(matches!(kind("mailto:a@example.com"), Some(Input::External(_))));
        assert_eq!(kind("   "), None);
        let Some(Input::Search(u)) = kind("a&b=c d") else { panic!() };
        assert_eq!(u.as_str(), "https://duckduckgo.com/?q=a%26b%3Dc+d");
        let Some(Input::Search(u)) = classify_input("a b", &ENGINES[1]) else { panic!() };
        assert_eq!(u.as_str(), "https://www.startpage.com/sp/search?query=a+b");
    }

    #[test]
    fn the_address_shows_the_host_with_its_registrable_end() {
        assert_eq!(display_host("https://www.example.com/a?b"), "example.com");
        assert_eq!(display_host("http://localhost:8765/x"), "Not Secure — localhost");
        assert_eq!(display_host("https://example.com./"), "example.com.");
        assert_eq!(display_host("blob:https://www.evil.test/0b1c"), "evil.test");
        assert_eq!(display_host("blob:http://evil.test/0b1c"), "Not Secure — evil.test");
        assert_eq!(display_host("about:blank#bank.test"), "about:blank");
        assert_eq!(display_host("data:text/html,bank.test"), "data:");
        let home = if cfg!(windows) { "file:///C:/Users/u" } else { "file:///Users/u" };
        assert_eq!(display_host(&format!("{home}/My%20Notes.html")), "File: My Notes.html");
        assert_eq!(display_host(&format!("{home}/txt.\u{202E}exe")), "File: txt.exe");
        assert_eq!(display_host("file://server.test/share/a.html"), "server.test", "a network file names its server");
        assert_eq!(file_name(&Url::parse(&format!("{home}/doc.pdf")).unwrap()).as_deref(), Some("doc.pdf"));
        assert_eq!(file_name(&Url::parse("https://a.test/doc.pdf").unwrap()), None);
        assert_eq!(display_host(""), "");
        let long = format!("https://{}.login.attacker.example/", "a".repeat(40));
        let shown = display_host(&long);
        assert!(shown.starts_with('…') && shown.ends_with("login.attacker.example"));
        assert_eq!(shown.chars().count(), MAX_HOST);
        assert_eq!(engine_key(true, 3), "search_engine.private");
        assert_eq!(engine_key(false, 3), "search_engine.3");
    }

    #[test]
    fn every_engine_url_parses() {
        for engine in &ENGINES {
            assert!(search_url(engine, "x").as_str().starts_with("https://"), "{}", engine.name);
        }
    }

    #[test]
    fn fuzzy_prefers_contiguous_and_word_starts() {
        assert!(fuzzy_score("gh", "GitHub").unwrap() > fuzzy_score("gh", "Laughing").unwrap());
        assert!(fuzzy_score("rust", "rust-lang.org").unwrap() > fuzzy_score("rust", "r u s t").unwrap());
        assert_eq!(fuzzy_score("xyz", "github"), None);
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn commands_rank_by_label_prefix_then_words_then_fuzzy() {
        let table = [("Settings", "preferences"), ("New Tab Group", "folder"), ("New Space", ""), ("Show Library", "bookmarks history")];
        let rank = |q| rank_commands(q, table);
        assert_eq!(rank("settings"), [0]);
        assert_eq!(rank("pref"), [0]);
        assert_eq!(rank("new group"), [1]);
        assert_eq!(rank("new"), [2, 1], "shorter label first");
        assert_eq!(rank("new s"), [2]);
        assert_eq!(rank("library"), [3]);
        assert_eq!(rank("se"), Vec::<usize>::new(), "short plain text stays a search");
        assert_eq!(rank("github"), Vec::<usize>::new());
        assert_eq!(rank("ntg"), Vec::<usize>::new());
        assert_eq!(rank(">ntg"), [1]);
        assert_eq!(rank("> new space"), [2]);
        assert_eq!(rank(">"), [0, 1, 2, 3]);
        assert_eq!(rank(">hist"), [3]);
    }

    #[test]
    fn commands_take_the_glyph_of_their_longest_matching_label() {
        assert_eq!(glyph("New Tab Group"), "group");
        assert_eq!(glyph("New Tab"), "plus");
        assert_eq!(glyph("Copy Clean Link"), "link");
        assert_eq!(glyph("Reload in Chromium"), "reload");
        assert_eq!(glyph("Bookmark All Tabs"), "bookmark");
        assert_eq!((glyph("Unpin"), glyph("Remove from Tiles"), glyph("New Burner Tab")), ("pin", "layout-grid", "flame"));
        let labels: std::collections::HashSet<&str> = GLYPHS.iter().map(|(label, _)| *label).collect();
        assert_eq!(labels.len(), GLYPHS.len(), "each label has one glyph");
        assert_eq!((glyph("Close Tabs Below"), glyph("Close All Tabs")), ("close", "close"));
        assert_eq!(glyph("Something New"), "command");
    }

    #[test]
    fn every_fixed_context_menu_label_has_a_glyph() {
        let labels = [
            "Open",
            "Duplicate",
            "Reload in WebKit",
            "Copy Link",
            "Copy Clean Link",
            "Bookmark Tab…",
            "Remove Bookmark",
            "Pin Tab",
            "Unpin Tab",
            "Add to Tiles",
            "Remove from Tiles",
            "Rename",
            "Change Icon…",
            "Reset Title",
            "Return to tarn.example",
            "Set Pinned URL to Current Page",
            "Close Tab",
            "Unload Tab",
            "Sleep Tab",
            "Close Other Tabs",
            "Close Tabs Below",
            "New Group",
            "Ungroup",
            "Remove from Group",
            "New Folder",
            "Delete Folder…",
            "Delete Space…",
            "Edit Space…",
            "Open in Split View",
            "Rotate Layout",
            "Close Split View",
            "Paste and Go",
            "Restore in New Space",
            "Customize in Settings…",
            "Show Tabs in Sidebar",
            "Classic Toolbar",
            "Compact Toolbar",
            "Site Settings…",
        ];
        for label in labels {
            assert_ne!(glyph(label), "command", "{label}");
        }
        assert_eq!(glyph("Open in New Tab"), "app-window");
        assert_eq!(glyph("Delete…"), "trash");
    }
}
