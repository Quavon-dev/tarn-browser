use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use store::{HistoryEntry, Id, Tab};
use url::Url;

use crate::command::{fold, fuzzy_chars, fuzzy_query};

/// A page the address field can suggest: an open tab or a history entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page<'a> {
    pub url: &'a str,
    pub title: &'a str,
    pub visits: i64,
    /// Visits the user opened from the address field or a bookmark.
    pub typed: i64,
    pub last_visit: i64,
    pub open: bool,
}

impl<'a> From<&'a HistoryEntry> for Page<'a> {
    fn from(h: &'a HistoryEntry) -> Self {
        Page { url: &h.url, title: &h.title, visits: h.visits, typed: h.typed, last_visit: h.visited_at, open: false }
    }
}

impl<'a> From<&'a Tab> for Page<'a> {
    fn from(t: &'a Tab) -> Self {
        Page { url: &t.url, title: &t.title, visits: 1, typed: 0, last_visit: t.last_active, open: true }
    }
}

/// Text the field prefills after what was typed, selected, and the URL Enter then opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub suffix: String,
    pub url: Url,
}

const DAY: i64 = 86_400_000;

/// Visits weighted by how recent the last one was; times in milliseconds.
pub fn frecency(visits: i64, last_visit: i64, now: i64) -> i64 {
    let weight = match (now - last_visit) / DAY {
        ..=3 => 100,
        4..=13 => 70,
        14..=30 => 50,
        31..=89 => 30,
        _ => 10,
    };
    visits.max(1) * weight
}

fn bare(s: &str) -> &str {
    let s = s.split_once("://").map_or(s, |(_, rest)| rest);
    s.strip_prefix("www.").unwrap_or(s)
}

/// A lowercased query, split once for every page `tier` scores.
struct Query<'a> {
    text: &'a str,
    words: Vec<&'a str>,
    chars: Vec<char>,
}

/// How well `query` names a page: 3 a prefix of its address, 2 word starts, 1 a fuzzy match.
/// `text` is a buffer reused across pages, so ranking a long history allocates nothing per page.
fn tier(query: &Query, page: &Page, text: &mut String) -> Option<(u8, i64)> {
    if query.text.is_empty() {
        return Some((0, 0));
    }
    let typed = bare(query.text);
    let url = bare(page.url);
    // Serialised URLs are ASCII, where an ASCII-insensitive prefix equals folding the address.
    let prefix = if url.is_ascii() {
        url.get(..typed.len()).is_some_and(|u| u.eq_ignore_ascii_case(typed))
    } else {
        url.chars().flat_map(fold).collect::<String>().starts_with(typed)
    };
    if !typed.is_empty() && query.words.len() == 1 && prefix {
        return Some((3, 0));
    }
    text.clear();
    text.extend(page.title.chars().chain([' ']).chain(page.url.chars()).flat_map(fold));
    let words = text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty());
    if query.words.iter().all(|q| words.clone().any(|w| w.starts_with(q))) {
        return Some((2, 0));
    }
    fuzzy_chars(&query.chars, text).map(|s| (1, s))
}

/// Indices of the pages `query` matches, best first: every open tab, and history once per URL no
/// tab shows. An address prefix beats word starts beats a fuzzy match, the better fuzzy match first;
/// then frecency (a tab's taken from its URL's history), then recency.
pub fn rank_pages(query: &str, pages: &[Page], now: i64) -> Vec<usize> {
    let lowered: String = query.trim().chars().flat_map(fold).collect();
    let query = Query { text: &lowered, words: lowered.split_whitespace().collect(), chars: fuzzy_query(&lowered) };
    let mut text = String::new();
    let mut best: HashMap<&str, i64> = HashMap::new();
    for page in pages {
        let score = best.entry(page.url).or_default();
        *score = (*score).max(frecency(page.visits, page.last_visit, now));
    }
    let open: HashSet<&str> = pages.iter().filter(|p| p.open).map(|p| p.url).collect();
    let mut seen = HashSet::new();
    let mut ranked: Vec<_> = pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.open || !open.contains(p.url) && seen.insert(p.url))
        .filter_map(|(ix, p)| {
            tier(&query, p, &mut text).map(|(tier, fuzzy)| ((Reverse(tier), Reverse(fuzzy), Reverse(best[p.url]), Reverse(p.last_visit), ix), ix))
        })
        .collect();
    ranked.sort_unstable();
    ranked.into_iter().map(|(_, ix)| ix).collect()
}

/// The history entries `query` names, best first as `rank_pages` orders them, at most `max`.
pub fn rank_history<'a>(query: &str, history: &'a [HistoryEntry], max: usize, now: i64) -> Vec<&'a HistoryEntry> {
    let pages: Vec<Page> = history.iter().map(Page::from).collect();
    top_history(query, history, &pages, max, now)
}

/// Whether the field may prefill after an edit: text was added at the end, and it reads as an
/// address rather than a search or a command.
pub fn may_complete(text: &str, inserted: bool, caret_at_end: bool) -> bool {
    inserted && caret_at_end && !text.is_empty() && !text.contains(char::is_whitespace) && !text.starts_with(['>', '?'])
}

/// Whether a full page load of `visited` is the one the user opened, `pending`: the same address,
/// or the server's redirect of it within its host, with or without `www.` and from http to https.
/// The first load result takes `pending` whether it matches or not, so a later page-driven visit
/// can never inherit the credit.
pub fn typed_visit(pending: &mut Option<String>, visited: &str) -> bool {
    pending.take().is_some_and(|p| p == visited || same_site(&p, visited))
}

fn same_site(typed: &str, visited: &str) -> bool {
    let (Ok(typed), Ok(visited)) = (Url::parse(typed), Url::parse(visited)) else { return false };
    let host = |u: &Url| u.host_str().map(|h| h.strip_prefix("www.").unwrap_or(h).to_owned());
    matches!((typed.scheme(), visited.scheme()), ("http", "http" | "https") | ("https", "https"))
        && host(&typed).is_some()
        && host(&typed) == host(&visited)
        && typed.port() == visited.port()
}

/// The visited host (or, once `typed` has a `/`, address) that starts with `typed`, ignoring `www.`
/// and, unless `typed` names one, the scheme, with the highest frecency summed over its pages. Only
/// pages the user opened themselves at least once complete, so a page cannot plant a completion by
/// reloading itself, opening popups or navigating its own tab, and nothing completes when `typed`
/// already is a visited address. The target is the shown text's most frecent https variant, else its most
/// frecent one, and never carries userinfo or a fragment the field would not show.
pub fn inline_completion(typed: &str, pages: &[Page], now: i64) -> Option<Completion> {
    let lower = typed.to_lowercase();
    let (scheme, rest) = lower.split_once("://").map_or((None, lower.as_str()), |(scheme, rest)| (Some(scheme), rest));
    if rest.is_empty() {
        return None;
    }
    let mut found: HashMap<(String, Url), i64> = HashMap::new();
    for page in pages.iter().filter(|p| p.typed >= 1 && p.url.to_lowercase().contains(rest)) {
        let Ok(mut url) = Url::parse(page.url) else { continue };
        if scheme.is_some_and(|s| s != url.scheme()) {
            continue;
        }
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_fragment(None);
        let Some(host) = url.host_str().filter(|_| matches!(url.scheme(), "http" | "https")) else { continue };
        let host = url.port().map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"));
        let shown = if rest.starts_with("www.") { &host } else { host.strip_prefix("www.").unwrap_or(&host) };
        let (text, target) = if rest.contains('/') {
            let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
            (format!("{shown}{}{query}", url.path()).trim_end_matches('/').to_owned(), url.clone())
        } else {
            let mut root = url.clone();
            root.set_path("/");
            root.set_query(None);
            (shown.to_owned(), root)
        };
        let lower = text.to_ascii_lowercase();
        if lower == rest {
            return None;
        }
        if lower.starts_with(rest) {
            *found.entry((text, target)).or_default() += frecency(page.visits, page.last_visit, now);
        }
    }
    let mut variants: Vec<_> = found.into_iter().collect();
    variants.sort_by_cached_key(|((_, url), score)| Reverse((*score, url.scheme() == "https", Reverse(url.to_string()))));
    let mut texts: HashMap<String, (i64, Url)> = HashMap::new();
    for ((text, url), score) in variants {
        let entry = texts.entry(text).or_insert((0, url.clone()));
        entry.0 += score;
        if entry.1.scheme() != "https" && url.scheme() == "https" {
            entry.1 = url;
        }
    }
    let (text, (_, url)) = texts.into_iter().max_by_key(|(text, (score, _))| (*score, Reverse(text.len()), Reverse(text.clone())))?;
    Some(Completion { suffix: text[rest.len()..].to_owned(), url })
}

/// For a field that reports only its new text: the completion to prefill after `value` when the
/// edit typed one character at the end of `typed`, the user's own part of the field before it.
/// The focused field starts fully selected, so `typed` is "" until the first edit; a paste or an
/// edit elsewhere in the text never prefills.
pub fn prefill_edit(typed: &str, value: &str, pages: &[Page], now: i64) -> Option<Completion> {
    let grew = value.starts_with(typed) && value.chars().count() == typed.chars().count() + 1;
    may_complete(value, grew, true).then(|| inline_completion(value, pages, now)).flatten()
}

/// An address field edit from `typed` to `value`: the history rows `value` names, best first as
/// `rank_pages` orders them, at most `max`, and the completion to prefill from the open `tabs` and
/// the history.
pub fn edit<'a>(typed: &str, value: &str, tabs: &[Tab], history: &'a [HistoryEntry], max: usize, now: i64) -> (Vec<&'a HistoryEntry>, Option<Completion>) {
    let pages: Vec<Page> = history.iter().map(Page::from).collect();
    let rows = top_history(value, history, &pages, max, now);
    let pages: Vec<Page> = tabs.iter().map(Page::from).chain(pages).collect();
    (rows, prefill_edit(typed, value, &pages, now))
}

pub const TEXT_SEARCH_DELAY: std::time::Duration = std::time::Duration::from_millis(120);
pub const TEXT_SEARCH_MIN_CHARS: usize = 3;

pub fn searches_text(query: &str) -> bool {
    query.chars().count() >= TEXT_SEARCH_MIN_CHARS
}

#[derive(Debug, Default)]
pub struct TextMatches(Option<(Id, String, Vec<HistoryEntry>)>);

impl TextMatches {
    pub fn get(&self, profile: Id, query: &str) -> Option<&[HistoryEntry]> {
        self.0.as_ref().filter(|(p, q, _)| *p == profile && q == query).map(|(_, _, found)| found.as_slice())
    }

    pub fn put(&mut self, profile: Id, query: &str, found: Vec<HistoryEntry>) {
        self.0 = Some((profile, query.to_owned(), found));
    }

    pub fn clear(&mut self) {
        self.0 = None;
    }
}

fn top_history<'a>(query: &str, history: &'a [HistoryEntry], pages: &[Page], max: usize, now: i64) -> Vec<&'a HistoryEntry> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    rank_pages(query, pages, now).into_iter().take(max).map(|ix| &history[ix]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_000 * DAY;

    fn page(url: &'static str, title: &'static str, visits: i64, days_ago: i64) -> Page<'static> {
        Page { url, title, visits, typed: visits, last_visit: NOW - days_ago * DAY, open: false }
    }

    fn complete(typed: &str, pages: &[Page]) -> Option<(String, String)> {
        inline_completion(typed, pages, NOW).map(|c| (c.suffix, c.url.to_string()))
    }

    #[test]
    fn text_search_waits_for_three_characters_and_keeps_one_query() {
        assert!(!searches_text("ab") && searches_text("abc") && searches_text("äöü"));
        let mut memo = TextMatches::default();
        let h = HistoryEntry { url: "https://a.test/".into(), title: "A".into(), visited_at: 0, visits: 1, typed: 0 };
        memo.put(1, "abc", vec![h]);
        assert_eq!(memo.get(1, "abc").map(<[_]>::len), Some(1));
        assert!(memo.get(2, "abc").is_none() && memo.get(1, "abcd").is_none());
        memo.clear();
        assert!(memo.get(1, "abc").is_none());
    }

    #[test]
    fn frecency_weighs_visits_by_recency() {
        assert!(frecency(1, NOW, NOW) > frecency(1, NOW - 20 * DAY, NOW));
        assert!(frecency(10, NOW - 60 * DAY, NOW) > frecency(1, NOW, NOW));
        assert_eq!(frecency(0, NOW, NOW), frecency(1, NOW, NOW));
    }

    #[test]
    fn completes_the_most_frecent_host_ignoring_scheme_and_www() {
        let pages = [
            page("https://www.netflix.com/browse", "Netflix", 30, 1),
            page("https://www.netflix.com/title/1", "Show", 5, 1),
            page("https://netlify.com/", "Netlify", 2, 1),
            page("https://duckduckgo.com/?q=net", "net at DuckDuckGo", 2, 0),
        ];
        assert_eq!(complete("net", &pages), Some(("flix.com".into(), "https://www.netflix.com/".into())));
        assert_eq!(complete("NET", &pages).unwrap().0, "flix.com");
        assert_eq!(complete("https://net", &pages).unwrap().0, "flix.com");
        assert_eq!(complete("www.net", &pages).unwrap().0, "flix.com");
        assert_eq!(complete("netl", &pages).unwrap().0, "ify.com");
        assert_eq!(complete("netflix.com", &pages), None, "nothing left to add");
        assert_eq!(complete("flix", &pages), None, "only prefixes of the host");
        assert_eq!(complete("DuckDuckGo", &pages).unwrap().0, ".com", "never from the title alone");
    }

    #[test]
    fn completes_a_path_once_a_slash_is_typed() {
        let pages = [page("https://www.netflix.com/browse", "Netflix", 30, 1), page("https://github.com/rust-lang/rust", "Rust", 3, 1)];
        assert_eq!(complete("netflix.com/b", &pages), Some(("rowse".into(), "https://www.netflix.com/browse".into())));
        assert_eq!(complete("github.com/rust-lang/", &pages).unwrap().0, "rust");
    }

    #[test]
    fn ties_go_to_the_shorter_host_and_other_schemes_never_complete() {
        let pages = [page("https://abc.example/", "", 2, 1), page("https://ab.example/", "", 2, 1), page("file:///abc", "", 99, 0)];
        assert_eq!(complete("a", &pages).unwrap().0, "b.example");
        assert_eq!(complete("", &pages), None);
        assert_eq!(complete("https://", &pages), None);
    }

    #[test]
    fn a_typed_scheme_must_match_and_targets_drop_userinfo_and_fragment() {
        let pages = [page("http://bank.example/", "", 9, 0), page("https://user:pw@shop.example/cart#top", "", 2, 0)];
        assert_eq!(complete("https://ba", &pages), None, "never downgrade a typed https");
        assert_eq!(complete("http://ba", &pages).unwrap().1, "http://bank.example/");
        assert_eq!(complete("shop.example/c", &pages), Some(("art".into(), "https://shop.example/cart".into())));
        assert_eq!(complete("sh", &pages).unwrap().1, "https://shop.example/");
    }

    #[test]
    fn only_typing_at_the_end_of_an_address_completes() {
        assert!(may_complete("net", true, true));
        assert!(!may_complete("net", false, true), "backspace or delete");
        assert!(!may_complete("net", true, false), "caret moved");
        assert!(!may_complete("net flix", true, true), "a search");
        assert!(!may_complete(">new", true, true), "a command");
        assert!(!may_complete("?net", true, true));
        assert!(!may_complete("", true, true));
    }

    #[test]
    fn prefill_follows_one_character_typed_at_the_end() {
        let pages = [page("https://www.netflix.com/", "Netflix", 3, 1)];
        let prefill = |typed: &str, value: &str| prefill_edit(typed, value, &pages, NOW).map(|c| c.suffix);
        assert_eq!(prefill("ne", "net").as_deref(), Some("flix.com"));
        assert_eq!(prefill("", "n").as_deref(), Some("etflix.com"), "the first key replaces the selected address");
        assert_eq!(prefill("", "https://www.netflix.com/xn"), None, "an edit inside the address shown on focus");
        assert_eq!(prefill("", "netf"), None, "a paste");
        assert_eq!(prefill("ne", "netf"), None, "a paste after typing");
        assert_eq!(prefill("net", "net"), None, "backspace over the prefill");
        assert_eq!(prefill("netf", "net"), None);
        assert_eq!(prefill("net", "nxet"), None, "typed in the middle");
        assert_eq!(prefill("net", "net "), None);
        let history = [HistoryEntry { url: "https://www.netflix.com/".into(), title: "Netflix".into(), visited_at: NOW, visits: 3, typed: 1 }];
        let (rows, completion) = edit("ne", "net", &[], &history, 5, NOW);
        assert_eq!((rows.len(), completion.map(|c| c.url.to_string())), (1, Some("https://www.netflix.com/".into())));
        let tab = Tab {
            id: 1,
            space_id: 1,
            folder_id: None,
            kind: store::TabKind::Ephemeral,
            url: "https://netlify.com/app".into(),
            title: String::new(),
            position: 0,
            last_active: NOW,
            group_id: None,
            pinned_url: None,
            custom_title: String::new(),
            web_engine: None,
        };
        let completion = edit("netl", "netli", std::slice::from_ref(&tab), &history, 5, NOW).1;
        assert_eq!(completion, None, "an open tab alone never completes");
    }

    #[test]
    fn only_pages_the_user_opened_complete() {
        let mut popup = page("https://paypal.com.evil.example/", "", 1, 0);
        (popup.open, popup.typed) = (true, 0);
        let mut planted = page("https://paypal.com.evil.example/", "", 9, 0);
        planted.typed = 0;
        assert_eq!(complete("paypal.com", &[planted, popup]), None, "a page reloading itself, opening popups or navigating its tab");
        let pages = [planted, popup, page("https://paypal.com/", "", 1, 3)];
        assert_eq!(complete("paypal.c", &pages), Some(("om".into(), "https://paypal.com/".into())));
    }

    #[test]
    fn a_visit_is_typed_only_when_it_loads_what_the_user_opened() {
        let typed = |opened: &str, visited: &str| typed_visit(&mut Some(opened.to_owned()), visited);
        assert!(!typed("http://example.com/", "https://evil.test/"), "a redirect to another host");
        assert!(!typed("http://example.com/", "https://example.com.evil.test/"));
        assert!(!typed("http://example.com/", "http://example.com:8080/"));
        assert!(typed("http://example.com/", "https://www.example.com/home"), "the server's own upgrade and redirect");
        assert!(!typed("https://bank.test/", "http://bank.test/"), "never a downgrade");
        assert!(typed("https://www.shop.test/a", "https://shop.test/b"));
        assert!(typed("file:///etc/hosts", "file:///etc/hosts"));
        let mut pending = Some("https://bank.test/".to_owned());
        assert!(!typed_visit(&mut pending, "https://other.test/"), "the old page's own load result");
        assert_eq!(pending, None, "the first result takes the mark");
        assert!(!typed_visit(&mut pending, "https://bank.test/"), "a later page-driven visit inherits nothing");
        assert!(!typed_visit(&mut None, "https://example.com/"));
    }

    #[test]
    fn the_target_is_the_most_frecent_https_variant() {
        let pages = [page("http://example.org/", "", 2, 0), page("https://www.example.org/", "", 9, 0), page("https://example.org/", "", 3, 0)];
        assert_eq!(complete("exa", &pages).unwrap().1, "https://www.example.org/");
        let pages = [page("http://site.test/", "", 2, 0), page("https://site.test/", "", 2, 0)];
        assert_eq!(complete("si", &pages).unwrap().1, "https://site.test/");
        let pages = [page("http://intranet.corp/", "", 2, 0)];
        assert_eq!(complete("intra", &pages).unwrap().1, "http://intranet.corp/");
        let pages = [page("http://www.shop.test/", "", 9, 0), page("https://shop.test/", "", 1, 9)];
        assert_eq!(complete("sh", &pages).unwrap().1, "https://shop.test/", "https wins over a more frecent http");
    }

    #[test]
    fn an_exact_visited_address_is_not_extended() {
        let pages = [page("https://example.com/a", "", 2, 0), page("https://example.com/abc", "", 9, 0)];
        assert_eq!(complete("example.com/a", &pages), None);
        assert_eq!(complete("example.com/ab", &pages).unwrap().0, "c");
    }

    #[test]
    fn ranks_host_prefix_over_title_words_over_fuzzy() {
        let mut tab = page("https://duckduckgo.com/?q=net", "net at DuckDuckGo", 1, 0);
        tab.open = true;
        let pages = [
            tab,
            page("https://example.com/n-e-t", "N e t", 50, 0),
            page("https://www.netflix.com/browse", "Netflix", 30, 1),
            page("https://news.test/", "Internet news", 1, 1),
            page("https://news.test/", "Internet news", 1, 1),
        ];
        assert_eq!(rank_pages("net", &pages, NOW), [2, 0, 1, 3]);
        assert_eq!(rank_pages("  NET ", &pages, NOW), [2, 0, 1, 3]);
        assert_eq!(rank_pages("zzz", &pages, NOW), Vec::<usize>::new());
    }

    #[test]
    fn non_ascii_titles_match_word_starts_and_fuzzily_in_any_case() {
        let pages = [page("https://a.test/x", "Zürich Ödland", 1, 0), page("https://b.test/", "École", 1, 0), page("https://c.test/", "other", 1, 0)];
        assert_eq!(rank_pages("ÖD", &pages, NOW), [0]);
        assert_eq!(rank_pages("éco", &pages, NOW), [1]);
        assert_eq!(rank_pages("zrh", &pages, NOW), [0]);
        assert_eq!(rank_pages("A.TEST/X", &pages, NOW), [0]);
        let greek = [page("https://gr.test/", "ΕΛΛΑΣ", 1, 0), page("https://de.test/wiki/Ärzte", "", 1, 0)];
        assert_eq!(rank_pages("ελλας", &greek, NOW), [0]);
        assert_eq!(rank_pages("ΕΛΛΑΣ", &greek, NOW), [0]);
        assert_eq!(rank_pages("de.test/wiki/ä", &greek, NOW), [1]);
    }

    #[test]
    fn frecency_breaks_ties_and_an_open_tab_stands_for_its_url() {
        let mut tab = page("https://a.test/", "A", 1, 30);
        tab.open = true;
        let pages = [page("https://a.test/", "A", 9, 0), page("https://ab.test/", "AB", 2, 0), tab];
        assert_eq!(rank_pages("a", &pages, NOW), [2, 1], "the tab keeps the history's frecency");
        assert_eq!(rank_pages("", &pages, NOW), [2, 1]);
    }

    #[test]
    fn every_open_tab_is_listed_and_history_once_per_url() {
        let mut tab = page("https://a.test/", "A", 1, 0);
        tab.open = true;
        let mut blank = page("", "", 1, 0);
        blank.open = true;
        let pages = [tab, tab, blank, blank, page("https://a.test/", "A", 9, 0), page("https://ab.test/", "AB", 1, 1), page("https://ab.test/", "AB", 1, 1)];
        assert_eq!(rank_pages("a", &pages, NOW), [0, 1, 5]);
        assert_eq!(rank_pages("", &pages, NOW), [0, 1, 2, 3, 5]);
        let history = [page("https://ab.test/", "AB", 1, 1), page("https://ab.test/", "AB", 1, 1), page("https://b.test/", "B", 1, 1)].map(|p| HistoryEntry {
            url: p.url.into(),
            title: p.title.into(),
            visited_at: p.last_visit,
            visits: p.visits,
            typed: p.typed,
        });
        assert_eq!(rank_history("a", &history, 5, NOW).len(), 1);
        assert!(rank_history(" ", &history, 5, NOW).is_empty());
    }

    #[test]
    fn a_stronger_fuzzy_match_beats_frecency() {
        let pages = [page("https://b.test/", "afxlxixx", 50, 0), page("https://www.netflix.com/", "Netflix", 1, 1)];
        assert_eq!(rank_pages("flix", &pages, NOW), [1, 0]);
    }
}
