use browser_core::command::{ENGINES, engine_key};
use browser_core::suggest::rank_history;
use store::{HistoryEntry, Id, Store};

pub use browser_core::command::display_host;

pub fn engine_index(store: &Store, private: bool, profile: Id) -> usize {
    let name = store.setting(&engine_key(private, profile)).ok().flatten();
    name.and_then(|n| ENGINES.iter().position(|e| e.name == n)).unwrap_or(0)
}

/// The ranked history rows, then the pages whose text matches (`text`), each address once.
pub fn suggestions(history: &[HistoryEntry], text: &[HistoryEntry], query: &str, max: usize) -> Vec<(String, String)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let rows = rank_history(query, history, max, browser_core::now()).into_iter().map(|h| (h.title.clone(), h.url.clone())).collect();
    with_text(rows, text, max)
}

pub fn with_text(rows: Vec<(String, String)>, text: &[HistoryEntry], max: usize) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    rows.into_iter().chain(text.iter().map(|h| (h.title.clone(), h.url.clone()))).filter(|(_, url)| seen.insert(url.clone())).take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_match_url_or_title_once() {
        let h = |url: &str, title: &str| HistoryEntry { url: url.into(), title: title.into(), visited_at: 0, visits: 1, typed: 0 };
        let history = [h("https://a.test/", "Alpha"), h("https://a.test/", "Alpha"), h("https://b.test/", "Beta"), h("https://c.test/alpha", "")];
        let found = suggestions(&history, &[], "ALPHA", 5);
        assert_eq!(found, [("Alpha".to_owned(), "https://a.test/".to_owned()), (String::new(), "https://c.test/alpha".to_owned())]);
        assert!(suggestions(&history, &[], "", 5).is_empty());
        let text = [h("https://d.test/", "Delta"), h("https://a.test/", "Alpha")];
        let found = suggestions(&history, &text, "alpha", 5);
        assert_eq!(found.len(), 3, "a page matched by its text follows the title matches once");
        assert_eq!(found[2].1, "https://d.test/");
    }
}
