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

pub struct Bang {
    pub key: &'static str,
    pub engine: Engine,
}

const fn bang(key: &'static str, name: &'static str, url: &'static str, param: &'static str) -> Bang {
    Bang { key, engine: Engine { name, url, param } }
}

pub const BANGS: &[Bang] = &[
    bang("g", "Google", "https://www.google.com/search", "q"),
    bang("b", "Bing", "https://www.bing.com/search", "q"),
    bang("ddg", "DuckDuckGo", "https://duckduckgo.com/", "q"),
    bang("sp", "Startpage", "https://www.startpage.com/sp/search", "query"),
    bang("brave", "Brave Search", "https://search.brave.com/search", "q"),
    bang("eco", "Ecosia", "https://www.ecosia.org/search", "q"),
    bang("qw", "Qwant", "https://www.qwant.com/", "q"),
    bang("mojeek", "Mojeek", "https://www.mojeek.com/search", "q"),
    bang("kagi", "Kagi", "https://kagi.com/search", "q"),
    bang("y", "Yahoo", "https://search.yahoo.com/search", "p"),
    bang("yandex", "Yandex", "https://yandex.com/search/", "text"),
    bang("baidu", "Baidu", "https://www.baidu.com/s", "wd"),
    bang("yt", "YouTube", "https://www.youtube.com/results", "search_query"),
    bang("ytm", "YouTube Music", "https://music.youtube.com/search", "q"),
    bang("vimeo", "Vimeo", "https://vimeo.com/search", "q"),
    bang("sc", "SoundCloud", "https://soundcloud.com/search", "q"),
    bang("bc", "Bandcamp", "https://bandcamp.com/search", "q"),
    bang("genius", "Genius", "https://genius.com/search", "q"),
    bang("discogs", "Discogs", "https://www.discogs.com/search/", "q"),
    bang("w", "Wikipedia", "https://en.wikipedia.org/w/index.php", "search"),
    bang("dew", "Wikipedia (Deutsch)", "https://de.wikipedia.org/w/index.php", "search"),
    bang("wikt", "Wiktionary", "https://en.wiktionary.org/w/index.php", "search"),
    bang("wa", "Wolfram|Alpha", "https://www.wolframalpha.com/input", "i"),
    bang("wh", "wikiHow", "https://www.wikihow.com/wikiHowTo", "search"),
    bang("quora", "Quora", "https://www.quora.com/search", "q"),
    bang("archive", "Internet Archive", "https://archive.org/search", "query"),
    bang("scholar", "Google Scholar", "https://scholar.google.com/scholar", "q"),
    bang("arxiv", "arXiv", "https://arxiv.org/search/?searchtype=all", "query"),
    bang("pubmed", "PubMed", "https://pubmed.ncbi.nlm.nih.gov/", "term"),
    bang("imdb", "IMDb", "https://www.imdb.com/find/", "q"),
    bang("tmdb", "TMDB", "https://www.themoviedb.org/search", "query"),
    bang("rt", "Rotten Tomatoes", "https://www.rottentomatoes.com/search", "search"),
    bang("mal", "MyAnimeList", "https://myanimelist.net/search/all", "q"),
    bang("gr", "Goodreads", "https://www.goodreads.com/search", "q"),
    bang("gh", "GitHub", "https://github.com/search", "q"),
    bang("so", "Stack Overflow", "https://stackoverflow.com/search", "q"),
    bang("su", "Super User", "https://superuser.com/search", "q"),
    bang("au", "Ask Ubuntu", "https://askubuntu.com/search", "q"),
    bang("mdn", "MDN", "https://developer.mozilla.org/en-US/search", "q"),
    bang("caniuse", "Can I use", "https://caniuse.com/", "search"),
    bang("rust", "Rust std", "https://doc.rust-lang.org/std/", "search"),
    bang("crates", "crates.io", "https://crates.io/search", "q"),
    bang("docs", "Docs.rs", "https://docs.rs/releases/search", "query"),
    bang("npm", "npm", "https://www.npmjs.com/search", "q"),
    bang("pypi", "PyPI", "https://pypi.org/search/", "q"),
    bang("py", "Python docs", "https://docs.python.org/3/search.html", "q"),
    bang("go", "Go Packages", "https://pkg.go.dev/search", "q"),
    bang("gem", "RubyGems", "https://rubygems.org/search", "query"),
    bang("hex", "Hex", "https://hex.pm/packages", "search"),
    bang("nuget", "NuGet", "https://www.nuget.org/packages", "q"),
    bang("mvn", "Maven Central", "https://central.sonatype.com/search", "q"),
    bang("dh", "Docker Hub", "https://hub.docker.com/search", "q"),
    bang("aur", "AUR", "https://aur.archlinux.org/packages", "K"),
    bang("arch", "ArchWiki", "https://wiki.archlinux.org/index.php", "search"),
    bang("man", "man pages", "https://man.archlinux.org/search", "q"),
    bang("snap", "Snapcraft", "https://snapcraft.io/search", "q"),
    bang("hn", "Hacker News", "https://hn.algolia.com/", "q"),
    bang("r", "Reddit", "https://www.reddit.com/search/", "q"),
    bang("x", "X", "https://x.com/search", "q"),
    bang("bsky", "Bluesky", "https://bsky.app/search", "q"),
    bang("pin", "Pinterest", "https://www.pinterest.com/search/pins/", "q"),
    bang("a", "Amazon", "https://www.amazon.com/s", "k"),
    bang("ade", "Amazon.de", "https://www.amazon.de/s", "k"),
    bang("ebay", "eBay", "https://www.ebay.com/sch/i.html", "_nkw"),
    bang("ebde", "eBay.de", "https://www.ebay.de/sch/i.html", "_nkw"),
    bang("etsy", "Etsy", "https://www.etsy.com/search", "q"),
    bang("ali", "AliExpress", "https://www.aliexpress.com/wholesale", "SearchText"),
    bang("idealo", "idealo", "https://www.idealo.de/preisvergleich/MainSearchProductCategory.html", "q"),
    bang("gz", "Geizhals", "https://geizhals.de/", "fs"),
    bang("steam", "Steam", "https://store.steampowered.com/search/", "term"),
    bang("play", "Google Play", "https://play.google.com/store/search", "q"),
    bang("as", "App Store", "https://apps.apple.com/us/iphone/search", "term"),
    bang("gm", "Google Maps", "https://www.google.com/maps", "q"),
    bang("osm", "OpenStreetMap", "https://www.openstreetmap.org/search", "query"),
    bang("bk", "Booking.com", "https://www.booking.com/searchresults.html", "ss"),
    bang("ta", "Tripadvisor", "https://www.tripadvisor.com/Search", "q"),
    bang("yelp", "Yelp", "https://www.yelp.com/search", "find_desc"),
    bang("gi", "Google Images", "https://www.google.com/search?udm=2", "q"),
    bang("bi", "Bing Images", "https://www.bing.com/images/search", "q"),
    bang("gn", "Google News", "https://news.google.com/search", "q"),
    bang("gt", "Google Translate", "https://translate.google.com/?sl=auto", "text"),
    bang("ud", "Urban Dictionary", "https://www.urbandictionary.com/define.php", "term"),
    bang("linguee", "Linguee", "https://www.linguee.com/english-german/search", "query"),
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

pub fn find_bang(input: &str) -> Option<(&'static Engine, &str)> {
    let input = input.trim();
    let lookup = |word: &str| {
        let key = word.strip_prefix('!')?;
        BANGS.iter().find(|b| b.key.eq_ignore_ascii_case(key)).map(|b| &b.engine)
    };
    let first = input.split(char::is_whitespace).next()?;
    if let Some(engine) = lookup(first) {
        return Some((engine, input[first.len()..].trim()));
    }
    let last = input.rsplit(char::is_whitespace).next()?;
    lookup(last).map(|engine| (engine, input[..input.len() - last.len()].trim()))
}

pub fn search_engine<'a>(raw: &str, engine: &'a Engine, shortcuts: bool) -> &'a Engine {
    find_bang(raw).filter(|_| shortcuts).map_or(engine, |(e, _)| e)
}

/// Turns command-bar text into a URL to load or a search with `engine`.
pub fn classify_input(raw: &str, engine: &Engine, shortcuts: bool) -> Option<Input> {
    let input = raw.trim();
    if input.is_empty() {
        return None;
    }
    if let Some((site, query)) = find_bang(input).filter(|_| shortcuts) {
        return Some(if query.is_empty() { Input::Navigate(Url::parse(site.url).expect("static URL")) } else { Input::Search(search_url(site, query)) });
    }
    let search_url = |query: &str| search_url(engine, query);
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
    let (_, tld) = host.rsplit_once('.')?;
    let has_tld = (tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())) || tld.starts_with("xn--");
    has_tld.then(|| Url::parse(&format!("https://{input}")).ok())?
}

pub const GLYPHS: &[(&str, &str)] = &[
    ("Settings", "gear"),
    ("Site Settings", "shield"),
    ("New Tab Group", "group"),
    ("New Tab", "plus"),
    ("New Window", "app-window"),
    ("New Space", "layout-grid"),
    ("Edit Space", "pencil"),
    ("Hide Space", "eye-off"),
    ("Show Space", "eye"),
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
    ("Glass Design", "sparkles"),
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
    ("Open File", "folder-open"),
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
    ("Enter or Exit Full Screen", "app-window"),
    ("Ask", "sparkles"),
    ("Import Browser Data", "folder-open"),
    ("Archived Tabs", "archive"),
    ("Picture in Picture", "picture-in-picture-2"),
    ("Take Screenshot", "camera"),
    ("Capture", "camera"),
    ("Measure", "code"),
    ("Insert Snippet", "file-text"),
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
    Now,
    Favorites,
    RecentTabs,
    Frequent,
    QuickActions,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Commands => "COMMANDS",
            Section::Tabs => "TABS",
            Section::Suggestions => "SUGGESTIONS",
            Section::History => "HISTORY",
            Section::Now => "NOW",
            Section::Favorites => "FAVORITES",
            Section::RecentTabs => "RECENT TABS",
            Section::Frequent => "FREQUENT",
            Section::QuickActions => "QUICK ACTIONS",
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

pub fn history_detail(url: &str) -> (String, String) {
    let Ok(parsed) = Url::parse(url) else { return (String::new(), crate::text::bounded_url(url)) };
    let host = parsed.host_str().map(|h| host_tail(h.trim_start_matches("www."), MAX_HOST)).unwrap_or_default();
    let rest = &parsed[url::Position::AfterHost..];
    (host, crate::text::bounded_url(rest.trim_end_matches('/')))
}

pub fn display_title(custom: &str, title: &str, url: &str) -> String {
    [custom, title]
        .into_iter()
        .map(crate::text::bounded_title)
        .find(|t| !t.trim().is_empty())
        .or_else(|| {
            Url::parse(url)
                .ok()
                .map(|u| file_name(&u).map_or_else(|| host_tail(&page_label(&u), crate::text::MAX_TITLE_CHARS), |n| crate::text::bounded_title(&n)))
        })
        .unwrap_or_else(|| "New Tab".into())
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
        classify_input(s, &ENGINES[0], false)
    }

    fn bang_search(s: &str) -> Option<String> {
        match classify_input(s, &ENGINES[0], true) {
            Some(Input::Search(u)) => Some(u.to_string()),
            _ => None,
        }
    }

    #[test]
    fn shortcuts_search_the_named_site_before_or_after_the_words() {
        assert_eq!(bang_search("!yt cats").unwrap(), "https://www.youtube.com/results?search_query=cats");
        assert_eq!(bang_search("  big cats !YT ").unwrap(), "https://www.youtube.com/results?search_query=big+cats");
        assert_eq!(bang_search("!g cats !yt").unwrap(), "https://www.google.com/search?q=cats+%21yt", "the first shortcut wins");
        assert_eq!(bang_search("!yt !g").unwrap(), "https://www.youtube.com/results?search_query=%21g");
        assert_eq!(bang_search("!w a&b=c").unwrap(), "https://en.wikipedia.org/w/index.php?search=a%26b%3Dc");
        assert_eq!(bang_search("!gi cat").unwrap(), "https://www.google.com/search?udm=2&q=cat");
        assert_eq!(bang_search("!gm Berlin").unwrap(), "https://www.google.com/maps?q=Berlin");
    }

    #[test]
    fn a_shortcut_wins_over_addresses_and_schemes() {
        assert_eq!(bang_search("!g example.com").unwrap(), "https://www.google.com/search?q=example.com");
        assert_eq!(bang_search("!g javascript:alert(1)").unwrap(), "https://www.google.com/search?q=javascript%3Aalert%281%29");
        assert_eq!(bang_search("https://a.test !g").unwrap(), "https://www.google.com/search?q=https%3A%2F%2Fa.test");
    }

    #[test]
    fn a_shortcut_alone_opens_the_site() {
        let open = |s| match classify_input(s, &ENGINES[0], true) {
            Some(Input::Navigate(u)) => u.to_string(),
            other => panic!("{s}: {other:?}"),
        };
        assert_eq!(open("!gm"), "https://www.google.com/maps");
        assert_eq!(open(" !RUST "), "https://doc.rust-lang.org/std/");
    }

    #[test]
    fn unknown_or_partial_shortcuts_search_the_whole_text() {
        for s in ["!zzz cats", "!", "!!", "! yt cats", "hello!", "a!b", "cats!yt", "!yt!"] {
            let Some(Input::Search(u)) = classify_input(s, &ENGINES[0], true) else { panic!("{s}") };
            assert_eq!(u.query_pairs().next().unwrap().1, s, "{s}");
        }
    }

    #[test]
    fn shortcuts_off_keep_the_default_engine() {
        let Some(Input::Search(u)) = kind("!yt cats") else { panic!() };
        assert_eq!(u.as_str(), "https://duckduckgo.com/?q=%21yt+cats");
        assert_eq!(search_engine("!yt cats", &ENGINES[0], false).name, "DuckDuckGo");
        assert_eq!(search_engine("!yt cats", &ENGINES[0], true).name, "YouTube");
        assert_eq!(search_engine("cats", &ENGINES[0], true).name, "DuckDuckGo");
    }

    #[test]
    fn every_shortcut_is_unique_lowercase_https_and_untagged() {
        let mut keys = std::collections::HashSet::new();
        for b in BANGS {
            assert!(keys.insert(b.key), "duplicate {}", b.key);
            assert!(!b.key.is_empty() && b.key.chars().all(|c| c.is_ascii_lowercase()), "{}", b.key);
            let url = search_url(&b.engine, "x");
            assert_eq!(url.scheme(), "https", "{}", b.key);
            assert!(url.query_pairs().all(|(k, _)| !["tag", "ref", "aff", "affiliate", "utm_source"].contains(&k.as_ref())), "{}", b.key);
        }
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
        assert_eq!(nav("пример.рф"), "https://xn--e1afmkfd.xn--p1ai/");
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
        assert!(matches!(kind("übersetzer"), Some(Input::Search(_))));
        assert!(matches!(kind("xn--bersetzer-p9a"), Some(Input::Search(_))));
        for s in ["intranet:3000/x", "übersetzer:3000/x", "xn--bersetzer-p9a:3000/x"] {
            assert!(matches!(kind(s), Some(Input::Search(_))), "{s}");
        }
        assert!(matches!(kind("what is a.b"), Some(Input::Search(_))));
        assert!(matches!(kind("mailto:a@example.com"), Some(Input::External(_))));
        assert_eq!(kind("   "), None);
        let Some(Input::Search(u)) = kind("a&b=c d") else { panic!() };
        assert_eq!(u.as_str(), "https://duckduckgo.com/?q=a%26b%3Dc+d");
        let Some(Input::Search(u)) = classify_input("a b", &ENGINES[1], false) else { panic!() };
        assert_eq!(u.as_str(), "https://www.startpage.com/sp/search?query=a+b");
    }

    #[test]
    fn a_history_detail_keeps_the_registrable_end_of_its_host() {
        let host = format!("bank.com.{}evil.test", "a.".repeat(30));
        let (shown, rest) = history_detail(&format!("https://{host}/log\u{202e}in?x=1"));
        assert!(shown.starts_with('…') && shown.ends_with("evil.test"), "{shown}");
        assert!(!rest.contains('\u{202e}') && rest.starts_with("/log") && rest.ends_with("in?x=1"), "{rest}");
        assert_eq!(history_detail("https://example.com/"), ("example.com".to_owned(), String::new()));
        assert_eq!(history_detail("http://example.com:8080/a/"), ("example.com".to_owned(), ":8080/a".to_owned()));
        let (_, long) = history_detail(&format!("https://example.com/{}", "p".repeat(900)));
        assert!(long.chars().count() <= 200);
        assert_eq!(history_detail("not a url").0, "");
    }

    #[test]
    fn a_tab_shows_a_bounded_title_or_its_page_label() {
        assert_eq!(display_title("", "", ""), "New Tab");
        assert_eq!(display_title("Mail", "Inbox", "https://mail.test/"), "Mail");
        assert_eq!(display_title(" ", "In\u{202e}box\n\tnow", "https://mail.test/"), "Inbox now");
        assert_eq!(display_title("", "", "https://www.example.com/a?b"), "www.example.com");
        assert_eq!(display_title("", "", "data:text/html,<h1>bank.test</h1>"), "data:");
        let file = if cfg!(windows) { "file:///C:/d/doc.pdf" } else { "file:///tmp/doc.pdf" };
        assert_eq!(display_title("", "", file), "doc.pdf", "an opened file without a title shows its name");
        assert!(display_title("", &"x".repeat(5000), "").chars().count() < 500, "a page cannot make a row unbounded");
        let host = format!("{}bank.test", "a.".repeat(120));
        let shown = display_title("", "", &format!("https://{host}/"));
        assert!(shown.chars().count() <= crate::text::MAX_TITLE_CHARS && shown.ends_with("bank.test"), "a long host keeps its registrable end: {shown}");
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
            "Hide Space",
            "Show Space",
            "Open in Split View",
            "Rotate Layout",
            "Close Split View",
            "Paste and Go",
            "Restore in New Space",
            "Customize in Settings…",
            "Show Tabs in Sidebar",
            "Classic Toolbar",
            "Compact Toolbar",
            "Glass Design",
            "Site Settings…",
        ];
        for label in labels {
            assert_ne!(glyph(label), "command", "{label}");
        }
        assert_eq!(glyph("Open in New Tab"), "app-window");
        assert_eq!(glyph("Delete…"), "trash");
    }
}
