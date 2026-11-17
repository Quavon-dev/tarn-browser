use browser_core::command::{ENGINES, Input, classify_input};
use browser_core::tabs::TabKey;
use browser_core::{bookmarks, now};
use store::{Bookmark, Id, Store};

use super::{Browser, Notice, START_URL};

impl Browser {
    pub fn bookmarks(&self) -> Vec<Bookmark> {
        self.model.profile().and_then(|(private, profile)| self.core.store(private).bookmarks(profile).ok()).unwrap_or_default()
    }

    fn bookmark_store(&self) -> Option<(&Store, Id)> {
        self.model.profile().map(|(private, profile)| (self.core.store(private), profile))
    }

    fn tab_page(&self, key: TabKey) -> Option<(String, String)> {
        let tab = self.model.tab(key)?;
        let page = self.pages.get(&key);
        let url = page.map(|p| p.url.clone()).filter(|u| !u.is_empty()).unwrap_or_else(|| tab.url.clone());
        let title = page.map(|p| p.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| tab.title.clone());
        (!url.is_empty() && url != START_URL).then_some((title, url))
    }

    pub fn bookmark_tab(&mut self, key: Option<TabKey>) -> Option<Id> {
        let (title, url) = self.tab_page(key.or(self.model.active)?)?;
        let marks = self.bookmarks();
        let (store, profile) = self.bookmark_store()?;
        bookmarks::bookmark(store, profile, &marks, &url, &title, now())
    }

    pub fn bookmark_all_tabs(&mut self) -> Option<Id> {
        let pages: Vec<(String, String)> = self.model.tabs.iter().filter_map(|t| self.tab_page(self.model.key(t))).collect();
        if pages.is_empty() {
            return None;
        }
        let (store, profile) = self.bookmark_store()?;
        bookmarks::bookmark_all(store, profile, pages.iter().map(|(t, u)| (t.as_str(), u.as_str())), now())
    }

    pub fn new_bookmark_folder(&mut self, parent: Option<Id>) -> Option<Id> {
        let (store, profile) = self.bookmark_store()?;
        bookmarks::new_folder(store, profile, parent, now())
    }

    pub fn rename_bookmark(&mut self, id: Id, title: &str) {
        let title = title.trim();
        if let Some((store, _)) = self.bookmark_store().filter(|_| !title.is_empty()) {
            store.rename_bookmark(id, title).ok();
        }
    }

    pub fn move_bookmark(&mut self, id: Id, parent: Option<Id>) {
        if let Some((store, _)) = self.bookmark_store() {
            store.move_bookmark(id, parent).ok();
        }
    }

    pub fn delete_bookmark(&mut self, id: Id) {
        if let Some((store, _)) = self.bookmark_store() {
            store.delete_bookmark(id).ok();
        }
    }

    pub fn open_bookmark(&mut self, url: &str) {
        let Some(Input::Navigate(url)) = classify_input(url, &ENGINES[0]) else {
            self.notice = Some(Notice::Message("Tarn does not open this kind of address.".into()));
            return;
        };
        let url = url.to_string();
        let open = self.model.tabs.iter().map(|t| self.model.key(t)).find(|k| self.tab_page(*k).is_some_and(|(_, u)| u == url));
        if let Some(key) = open {
            return self.activate(key);
        }
        if let Some(key) = self.new_tab(&url) {
            self.pages.entry(key).or_default().typed = Some(url);
        }
    }
}
