use std::collections::HashSet;

use store::{Bookmark, Id, Store};

use crate::command::fuzzy_score;

pub const READING_LIST: &str = "Reading List";
pub const NEW_FOLDER: &str = "New Folder";
pub const RECENT: usize = 12;

pub use crate::text::title_or_host;

pub fn find<'a>(bookmarks: &'a [Bookmark], url: &str) -> Option<&'a Bookmark> {
    bookmarks.iter().find(|b| b.url.as_deref() == Some(url))
}

pub fn subtree(bookmarks: &[Bookmark], id: Id) -> HashSet<Id> {
    let mut inside = HashSet::from([id]);
    loop {
        let more: Vec<Id> = bookmarks.iter().filter(|b| !inside.contains(&b.id) && b.parent_id.is_some_and(|p| inside.contains(&p))).map(|b| b.id).collect();
        if more.is_empty() {
            return inside;
        }
        inside.extend(more);
    }
}

pub fn move_targets(bookmarks: &[Bookmark], id: Id) -> Vec<&Bookmark> {
    let inside = subtree(bookmarks, id);
    bookmarks.iter().filter(|f| f.url.is_none() && !inside.contains(&f.id)).collect()
}

/// The top-level bookmark folder named `name`, such as Archived Tabs or the Reading List.
pub fn top_folder(bookmarks: &[Bookmark], name: &str) -> Option<Id> {
    bookmarks.iter().find(|b| b.url.is_none() && b.parent_id.is_none() && b.title == name).map(|b| b.id)
}

pub fn reading_list(bookmarks: &[Bookmark]) -> Option<Id> {
    top_folder(bookmarks, READING_LIST)
}

pub fn recent(bookmarks: &[Bookmark], limit: usize) -> Vec<&Bookmark> {
    let mut pages: Vec<&Bookmark> = bookmarks.iter().filter(|b| b.url.is_some()).collect();
    pages.sort_by_key(|b| std::cmp::Reverse(b.created_at));
    pages.truncate(limit);
    pages
}

pub fn search<'a>(bookmarks: &'a [Bookmark], query: &str, limit: usize) -> Vec<&'a Bookmark> {
    let hit = |b: &Bookmark| b.url.as_deref().is_some_and(|u| query.is_empty() || fuzzy_score(query, &format!("{} {u}", b.title)).is_some());
    bookmarks.iter().filter(|b| hit(b)).take(limit).collect()
}

pub fn bookmark(store: &Store, profile_id: Id, bookmarks: &[Bookmark], url: &str, title: &str, at: i64) -> Option<Id> {
    if let Some(existing) = find(bookmarks, url) {
        return Some(existing.id);
    }
    store.add_bookmark(profile_id, None, Some(url), &title_or_host(title, url), at).ok().map(|b| b.id)
}

pub fn unbookmark(store: &Store, bookmarks: &[Bookmark], url: &str) {
    for b in bookmarks.iter().filter(|b| b.url.as_deref() == Some(url)) {
        store.delete_bookmark(b.id).ok();
    }
}

pub fn bookmark_all<'a>(store: &Store, profile_id: Id, pages: impl IntoIterator<Item = (&'a str, &'a str)>, at: i64) -> Option<Id> {
    let folder = store.add_bookmark(profile_id, None, None, &format!("Tabs {}", crate::shield_card::date(at)), at).ok()?.id;
    for (title, url) in pages.into_iter().filter(|(_, url)| !url.is_empty()) {
        store.add_bookmark(profile_id, Some(folder), Some(url), &title_or_host(title, url), at).ok();
    }
    Some(folder)
}

pub fn new_folder(store: &Store, profile_id: Id, parent: Option<Id>, at: i64) -> Option<Id> {
    store.add_bookmark(profile_id, parent, None, NEW_FOLDER, at).ok().map(|b| b.id)
}

pub fn save_for_later(store: &Store, profile_id: Id, bookmarks: &[Bookmark], url: &str, title: &str, at: i64) -> bool {
    let folder = match reading_list(bookmarks) {
        Some(id) => id,
        None => match store.add_bookmark(profile_id, None, None, READING_LIST, at) {
            Ok(folder) => folder.id,
            Err(_) => return false,
        },
    };
    bookmarks.iter().any(|b| b.parent_id == Some(folder) && b.url.as_deref() == Some(url))
        || store.add_bookmark(profile_id, Some(folder), Some(url), if title.is_empty() { url } else { title }, at).is_ok()
}

pub fn reading_list_pages(bookmarks: Vec<Bookmark>) -> Vec<Bookmark> {
    let Some(folder) = reading_list(&bookmarks) else { return Vec::new() };
    bookmarks.into_iter().filter(|b| b.parent_id == Some(folder) && b.url.is_some()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mark(id: Id, parent_id: Option<Id>, url: Option<&str>) -> Bookmark {
        Bookmark { id, profile_id: 1, parent_id, url: url.map(String::from), title: String::new(), position: 0, created_at: id }
    }

    fn store() -> (Store, Id) {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap().id;
        (store, p)
    }

    #[test]
    fn an_already_bookmarked_url_is_found_instead_of_added_again() {
        let marks = [mark(1, None, None), mark(2, Some(1), Some("https://a.test/")), mark(3, None, Some("https://b.test/"))];
        assert_eq!(find(&marks, "https://a.test/").map(|b| b.id), Some(2));
        assert!(find(&marks, "https://c.test/").is_none());
    }

    #[test]
    fn a_folder_cannot_move_into_itself_or_its_children() {
        let marks = [mark(1, None, None), mark(2, Some(1), None), mark(3, Some(2), Some("https://a.test/")), mark(4, None, None)];
        assert_eq!(subtree(&marks, 1), HashSet::from([1, 2, 3]));
        assert_eq!(subtree(&marks, 4), HashSet::from([4]));
        assert_eq!(move_targets(&marks, 1).iter().map(|b| b.id).collect::<Vec<_>>(), [4]);
        assert_eq!(move_targets(&marks, 3).iter().map(|b| b.id).collect::<Vec<_>>(), [1, 2, 4]);
    }

    #[test]
    fn recent_lists_the_newest_pages_and_search_matches_title_or_url() {
        let mut marks = vec![mark(1, None, None), mark(2, Some(1), Some("https://a.test/")), mark(3, None, Some("https://news.test/"))];
        marks[1].title = "Rust Book".into();
        assert_eq!(recent(&marks, 5).iter().map(|b| b.id).collect::<Vec<_>>(), [3, 2]);
        assert_eq!(recent(&marks, 1).len(), 1);
        assert_eq!(search(&marks, "rust", 10).iter().map(|b| b.id).collect::<Vec<_>>(), [2]);
        assert_eq!(search(&marks, "news", 10).iter().map(|b| b.id).collect::<Vec<_>>(), [3]);
        assert!(search(&marks, "zzz", 10).is_empty());
        assert_eq!(search(&marks, "", 1).len(), 1, "an empty query lists pages, never folders");
    }

    #[test]
    fn bookmarking_a_page_twice_keeps_one_and_unbookmark_removes_it() {
        let (store, p) = store();
        let id = bookmark(&store, p, &[], "https://a.test/", "", 1).unwrap();
        let marks = store.bookmarks(p).unwrap();
        assert_eq!(marks[0].title, "a.test", "an untitled page is named by its host");
        assert_eq!(bookmark(&store, p, &marks, "https://a.test/", "A", 2), Some(id));
        assert_eq!(store.bookmarks(p).unwrap().len(), 1);
        unbookmark(&store, &marks, "https://a.test/");
        assert!(store.bookmarks(p).unwrap().is_empty());
    }

    #[test]
    fn bookmark_all_files_every_page_into_a_dated_folder() {
        let (store, p) = store();
        let folder = bookmark_all(&store, p, [("A", "https://a.test/"), ("", ""), ("", "https://b.test/")], 0).unwrap();
        let marks = store.bookmarks(p).unwrap();
        let named = |id| marks.iter().find(|b| b.id == id).map(|b| b.title.clone());
        assert_eq!(named(folder).as_deref(), Some("Tabs 1970-01-01"));
        let inside: Vec<_> = marks.iter().filter(|b| b.parent_id == Some(folder)).map(|b| b.title.as_str()).collect();
        assert_eq!(inside, ["A", "b.test"]);
        let sub = new_folder(&store, p, Some(folder), 1).unwrap();
        assert_eq!(store.bookmarks(p).unwrap().iter().find(|b| b.id == sub).map(|b| (b.parent_id, b.title.clone())), Some((Some(folder), NEW_FOLDER.into())));
    }

    #[test]
    fn save_for_later_makes_the_folder_once_and_skips_a_saved_page() {
        let (store, p) = store();
        assert!(save_for_later(&store, p, &[], "https://a.test/", "", 1));
        let marks = store.bookmarks(p).unwrap();
        assert!(save_for_later(&store, p, &marks, "https://a.test/", "A", 2));
        assert!(save_for_later(&store, p, &marks, "https://b.test/", "B", 3));
        let pages = reading_list_pages(store.bookmarks(p).unwrap());
        assert_eq!(pages.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), ["https://a.test/", "B"]);
        assert_eq!(store.bookmarks(p).unwrap().iter().filter(|b| b.url.is_none()).count(), 1);
    }
}
