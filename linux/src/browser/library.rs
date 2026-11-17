use browser_core::library::{self, HISTORY_LOAD, SECTION_ROWS};
use browser_core::tabs::TabKey;
use browser_core::text::bounded_title;
use store::{Download, HistoryEntry, Id};

use super::Browser;

#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub name: String,
    pub url: String,
}

impl Link {
    fn new(title: &str, url: &str) -> Self {
        let name = bounded_title(title);
        Link { name: if name.is_empty() { bounded_title(url) } else { name }, url: url.to_owned() }
    }
}

#[derive(Default)]
pub struct Found {
    pub tabs: Vec<(TabKey, String, String)>,
    pub tab_count: usize,
    pub bookmarks: Vec<Link>,
    pub history: Vec<Link>,
    pub downloads: Vec<Download>,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty() && self.bookmarks.is_empty() && self.history.is_empty() && self.downloads.is_empty()
    }
}

impl Browser {
    pub fn library_history(&self) -> Vec<HistoryEntry> {
        let Some((private, id)) = self.model.profile() else { return Vec::new() };
        library::unique_history(self.core.store(private).history(id, HISTORY_LOAD).unwrap_or_default())
    }

    pub fn history_links(history: &[HistoryEntry], query: &str, limit: usize) -> Vec<Link> {
        library::history_hits(history, query).into_iter().take(limit).map(|ix| Link::new(&history[ix].title, &history[ix].url)).collect()
    }

    pub fn saved_downloads(&self) -> Vec<Download> {
        self.model.profile().and_then(|(private, id)| self.core.store(private).downloads(id).ok()).unwrap_or_default()
    }

    pub fn library_search(&self, query: &str, history: &[HistoryEntry]) -> Found {
        let open = library::open_tabs(&self.model.tabs, query);
        let tab_count = open.len();
        let tabs = open
            .into_iter()
            .take(SECTION_ROWS)
            .map(|t| {
                let key = self.model.key(t);
                let url = self.page_url(key).map_or_else(|| t.url.clone(), |u| u.to_string());
                (key, bounded_title(&crate::snapshot::tab_label(t, self, key)), url)
            })
            .collect();
        if query.is_empty() {
            return Found { tabs, tab_count, ..Found::default() };
        }
        let bookmarks = self.model.profile().map(|(private, id)| self.core.store(private).bookmarks(id).unwrap_or_default()).unwrap_or_default();
        let downloads = self.saved_downloads();
        Found {
            tabs,
            tab_count,
            bookmarks: library::bookmarks(&bookmarks, query).take(SECTION_ROWS).map(|b| Link::new(&b.title, b.url.as_deref().unwrap_or_default())).collect(),
            history: Self::history_links(history, query, SECTION_ROWS),
            downloads: library::downloads(&downloads, query).take(SECTION_ROWS).cloned().collect(),
        }
    }

    pub fn has_reading_list(&self) -> bool {
        let Some((private, id)) = self.model.profile() else { return false };
        self.reading_list_folder(&self.core.store(private).bookmarks(id).unwrap_or_default()).is_some()
    }

    pub fn open_url(&mut self, url: &str) {
        match self.model.tabs.iter().find(|t| t.url == url).map(|t| self.model.key(t)) {
            Some(key) => self.activate(key),
            None if url::Url::parse(url).is_ok() => {
                self.new_tab(url);
            }
            None => {}
        }
    }

    pub fn remove_download(&mut self, id: Id) {
        if let Some((private, profile)) = self.model.profile() {
            self.core.store(private).delete_download(profile, id).ok();
        }
    }

    pub fn rename_snapshot(&mut self, id: Id, name: &str) {
        let name = name.trim();
        if let Some(saved) = self.snapshots().into_iter().find(|s| s.id == id).filter(|_| !name.is_empty()) {
            self.core.disk.rename_snapshot(saved.profile_id, saved.id, name).ok();
        }
    }
}
