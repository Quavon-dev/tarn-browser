use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::Path;

use store::{Bookmark, Download, HistoryEntry, Id, Store, Tab};

use crate::command::fuzzy_score;
use crate::text::bounded_title;

pub const HISTORY_LOAD: usize = 2000;
pub const SECTION_ROWS: usize = 8;
pub const LIST_ROWS: usize = 200;

pub fn matches(query: &str, title: &str, url: &str) -> bool {
    query.is_empty() || fuzzy_score(query, &format!("{title} {url}")).is_some()
}

pub fn unique_history(history: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    let mut seen = HashSet::new();
    history.into_iter().filter(|h| seen.insert(h.url.clone())).collect()
}

pub fn open_tabs<'a>(tabs: &'a [Tab], query: &str) -> Vec<&'a Tab> {
    let mut found: Vec<&Tab> = tabs.iter().filter(|t| matches(query, &t.title, &t.url)).collect();
    found.sort_by_key(|t| Reverse(t.last_active));
    found
}

pub fn bookmarks<'a>(bookmarks: &'a [Bookmark], query: &'a str) -> impl Iterator<Item = &'a Bookmark> {
    bookmarks.iter().filter(move |b| b.url.as_deref().is_some_and(|u| matches(query, &b.title, u)))
}

pub fn history_hits(history: &[HistoryEntry], query: &str) -> Vec<usize> {
    history.iter().enumerate().filter(|(_, h)| matches(query, &h.title, &h.url)).map(|(ix, _)| ix).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HistoryRange {
    LastHour,
    Today,
    All,
}

impl HistoryRange {
    pub const ALL: [HistoryRange; 3] = [HistoryRange::LastHour, HistoryRange::Today, HistoryRange::All];

    pub fn label(self) -> &'static str {
        match self {
            HistoryRange::LastHour => "Last Hour",
            HistoryRange::Today => "Today",
            HistoryRange::All => "Everything",
        }
    }
}

pub fn clear_history(store: &Store, profile_id: Id, range: HistoryRange, now: i64) -> Result<(), store::Error> {
    match range {
        HistoryRange::LastHour => store.delete_history_since(profile_id, now - 3_600_000)?,
        HistoryRange::Today => store.delete_history_since(profile_id, store.local_day_start(now)?)?,
        HistoryRange::All => return store.clear_history(profile_id),
    }
    forget_unvisited_sites(store, profile_id)
}

/// Drops the site icons and Shield site counts of hosts the profile's history no longer has.
fn forget_unvisited_sites(store: &Store, profile_id: Id) -> Result<(), store::Error> {
    let visited: HashSet<String> = store
        .history(profile_id, i64::MAX as usize)?
        .iter()
        .filter_map(|h| crate::command::page_host(&url::Url::parse(&h.url).ok()?))
        .map(|h| h.trim_end_matches('.').to_owned())
        .collect();
    let icons = store.favicons(profile_id)?.into_iter().map(|(host, _)| host);
    let sites = store.shield_sites(profile_id, usize::MAX >> 1)?.into_iter().map(|(host, _)| host);
    for host in icons.chain(sites).filter(|h| !visited.contains(h.trim_end_matches('.'))).collect::<HashSet<_>>() {
        store.forget_site_records(profile_id, &host)?;
    }
    Ok(())
}

/// "5 min ago", "3 h ago", "2 d ago", then the local date.
pub fn ago(store: &Store, ms: i64, now: i64) -> String {
    match (now - ms).max(0) / 60_000 {
        0 => "Just now".into(),
        min @ 1..60 => format!("{min} min ago"),
        min @ 60..1440 => format!("{} h ago", min / 60),
        min @ 1440..10_080 => format!("{} d ago", min / 1440),
        _ => store.local_date(ms).unwrap_or_else(|_| crate::shield_card::date(ms)),
    }
}

/// Where the local today and yesterday start, for `history_day`.
pub fn day_starts(store: &Store, now: i64) -> (i64, i64) {
    let today = store.local_day_start(now).unwrap_or(now);
    (today, store.local_day_start(today - 1).unwrap_or(today - 86_400_000))
}

pub fn history_day(visited_at: i64, (today, yesterday): (i64, i64)) -> &'static str {
    if visited_at >= today {
        "Today"
    } else if visited_at >= yesterday {
        "Yesterday"
    } else {
        "Earlier"
    }
}

pub fn download_name(download: &Download) -> String {
    bounded_title(&Path::new(&download.path).file_name().map(|n| n.to_string_lossy()).unwrap_or_default())
}

/// The note of a finished download whose file was moved or deleted since.
pub const MISSING: &str = "Missing · moved or deleted";

/// Whether `download` finished but its file is gone, so it has nothing to reveal or open.
pub fn download_missing(download: &Download) -> bool {
    download.state.starts_with("finished") && !Path::new(&download.path).exists()
}

pub fn downloads<'a>(downloads: &'a [Download], query: &'a str) -> impl Iterator<Item = &'a Download> {
    downloads.iter().filter(move |d| query.is_empty() || fuzzy_score(query, &download_name(d)).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: i64, url: &str, title: &str, last_active: i64) -> Tab {
        Tab {
            id,
            space_id: 1,
            folder_id: None,
            kind: store::TabKind::Ephemeral,
            url: url.into(),
            title: title.into(),
            position: id,
            last_active,
            group_id: None,
            pinned_url: None,
            custom_title: String::new(),
            web_engine: None,
        }
    }

    fn visit(url: &str, title: &str) -> HistoryEntry {
        HistoryEntry { url: url.into(), title: title.into(), visited_at: 0, visits: 1, typed: 0 }
    }

    fn download(id: i64, path: &str) -> Download {
        Download { id, url: format!("https://files.test/{id}"), path: path.into(), state: "finished".into(), started_at: 0 }
    }

    #[test]
    fn open_tabs_match_title_or_address_most_recent_first() {
        let tabs = [tab(1, "https://rust-lang.org/", "Rust", 5), tab(2, "https://docs.rs/", "Docs", 9), tab(3, "https://example.test/", "Rusty", 7)];
        let ids = |q| open_tabs(&tabs, q).iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids(""), [2, 3, 1]);
        assert_eq!(ids("rust"), [3, 1]);
        assert_eq!(ids("docs.rs"), [2]);
        assert!(ids("zzz").is_empty());
    }

    #[test]
    fn bookmark_search_skips_folders() {
        let bm = |id, url: Option<&str>, title: &str| Bookmark {
            id,
            profile_id: 1,
            parent_id: None,
            url: url.map(Into::into),
            title: title.into(),
            position: id,
            created_at: 0,
        };
        let all = [bm(1, None, "News"), bm(2, Some("https://news.test/"), "Daily"), bm(3, Some("https://cook.test/"), "Recipes")];
        assert_eq!(bookmarks(&all, "news").map(|b| b.id).collect::<Vec<_>>(), [2]);
        assert_eq!(bookmarks(&all, "").count(), 2);
    }

    #[test]
    fn history_keeps_the_newest_visit_of_each_address_and_searches_it() {
        let history = unique_history(vec![visit("https://a.test/", "Alpha"), visit("https://b.test/", "Beta"), visit("https://a.test/", "Old Alpha")]);
        assert_eq!(history.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(), ["Alpha", "Beta"]);
        assert_eq!(history_hits(&history, "beta"), [1]);
        assert_eq!(history_hits(&history, ""), [0, 1]);
    }

    #[test]
    fn clear_history_removes_the_chosen_range_only() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "u", false).unwrap().id;
        let now = store.local_day_start(10 * 86_400_000).unwrap() + 12 * 3_600_000;
        for (url, ago) in [("https://old.test/", 3 * 86_400_000), ("https://hour.test/", 1_800_000), ("https://earlier.test/", 7_200_000)] {
            store.add_history(p, url, "", now - ago, false).unwrap();
        }
        let urls = || store.history(p, 10).unwrap().into_iter().map(|h| h.url).collect::<Vec<_>>();
        for host in ["hour.test", "old.test"] {
            store.set_favicon(p, host, b"png").unwrap();
            store.count(p, store::Counter::HttpsUpgrades, 1, Some(host)).unwrap();
        }
        clear_history(&store, p, HistoryRange::LastHour, now).unwrap();
        assert_eq!(urls(), ["https://earlier.test/", "https://old.test/"]);
        assert_eq!(store.favicons(p).unwrap().into_iter().map(|f| f.0).collect::<Vec<_>>(), ["old.test"], "an icon goes with the host's last visit");
        assert_eq!(store.shield_sites(p, 10).unwrap().into_iter().map(|s| s.0).collect::<Vec<_>>(), ["old.test"]);
        clear_history(&store, p, HistoryRange::Today, now).unwrap();
        assert_eq!(urls(), ["https://old.test/"]);
        clear_history(&store, p, HistoryRange::All, now).unwrap();
        assert!(urls().is_empty());
    }

    #[test]
    fn ages_turn_into_local_dates_after_a_week() {
        let (store, min) = (Store::open_in_memory().unwrap(), 60_000);
        assert_eq!(ago(&store, 0, 59_999), "Just now");
        assert_eq!(ago(&store, 0, 5 * min), "5 min ago");
        assert_eq!(ago(&store, 0, 2 * 1440 * min), "2 d ago");
        assert_eq!(ago(&store, 10 * min, 0), "Just now");
        let midnight = store.local_day_start(30 * 1440 * min).unwrap();
        assert_eq!(ago(&store, midnight, midnight + 8 * 1440 * min), store.local_date(midnight + 12 * 60 * min).unwrap());
        assert_ne!(ago(&store, midnight, midnight + 8 * 1440 * min), ago(&store, midnight - 1, midnight + 8 * 1440 * min));
    }

    #[test]
    fn day_starts_are_local_midnights_a_day_apart() {
        let store = Store::open_in_memory().unwrap();
        let today = store.local_day_start(20 * 86_400_000).unwrap();
        let (start, yesterday) = day_starts(&store, today + 3_600_000);
        assert_eq!(start, today);
        assert_eq!(yesterday, store.local_day_start(today - 1).unwrap());
        assert!(today - yesterday >= 23 * 3_600_000 && today - yesterday <= 25 * 3_600_000);
    }

    #[test]
    fn visits_group_into_today_yesterday_and_earlier() {
        let starts = (200, 100);
        assert_eq!(
            [history_day(250, starts), history_day(200, starts), history_day(150, starts), history_day(99, starts)],
            ["Today", "Today", "Yesterday", "Earlier"]
        );
    }

    #[test]
    fn downloads_match_their_file_name_only() {
        let all = [download(1, "/home/me/Downloads/report.pdf"), download(2, "/home/me/report/photo.jpg")];
        assert_eq!(download_name(&all[0]), "report.pdf");
        assert_eq!(downloads(&all, "report").map(|d| d.id).collect::<Vec<_>>(), [1]);
        assert_eq!(downloads(&all, "").count(), 2);
    }

    #[test]
    fn a_finished_download_without_its_file_is_missing() {
        let dir = std::env::temp_dir().join(format!("tarn-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let kept = dir.join("kept.zip");
        std::fs::write(&kept, b"x").unwrap();
        let gone = dir.join("gone.zip").to_string_lossy().into_owned();
        assert!(!download_missing(&download(1, &kept.to_string_lossy())));
        assert!(download_missing(&download(2, &gone)));
        assert!(!download_missing(&Download { state: "failed".into(), ..download(3, &gone) }));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
