use std::path::Path;

use browser_core::tabs::TabKey;
use browser_core::{bookmarks, now};
use serde_json::{Value, json};
use store::{Bookmark, Counter, Id};
use url::Url;

use super::{Browser, START_URL};

pub const QUIET: usize = 3;

pub fn copied(removed: i64) -> String {
    match removed {
        0 => "Link copied".to_owned(),
        1 => "Link copied, 1 tracking parameter removed".to_owned(),
        n => format!("Link copied, {n} tracking parameters removed"),
    }
}

impl Browser {
    pub fn active_url(&self) -> Option<String> {
        self.active_page().map(|p| p.url.clone()).filter(|u| !u.is_empty() && u != START_URL)
    }

    pub fn copy_link(&mut self, clean: bool) -> Option<String> {
        let url = self.active_url()?;
        if !clean {
            self.show_toast_as(browser_core::toast::Tone::Success, "Link copied", None);
            return Some(url);
        }
        let (link, removed) = shield::url_clean::clean_link(&url);
        if removed > 0
            && let Some(key) = self.model.active
            && let Some(profile) = self.model.profile_of(&self.core, key).filter(|p| self.core.keeps_records(key.private, *p))
        {
            let host = Url::parse(&url).ok().and_then(|u| browser_core::site::site_host(&u).map(str::to_owned));
            self.core.store(key.private).count(profile, Counter::TrackingParams, removed, host.as_deref()).ok();
        }
        self.show_toast_as(browser_core::toast::Tone::Success, copied(removed), None);
        Some(link)
    }

    pub fn qr_code(&mut self) -> Option<(Vec<Vec<bool>>, String)> {
        let url = self.active_url()?;
        let Some(code) = engine::qr::encode(&url) else {
            self.show_toast("This link is too long for a QR code", None);
            return None;
        };
        let label = Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(|h| browser_core::command::host_tail(h.trim_start_matches("www."), 40)))
            .unwrap_or_else(|| engine::view_policy::title(&url));
        Some((code, label))
    }

    pub fn pdf_name(&self) -> Option<(TabKey, String, String)> {
        let (key, url) = (self.model.active?, self.active_url()?);
        let title = self.active_page().map(|p| p.title.as_str()).unwrap_or_default();
        let name = browser_core::text::capture_name(title, Url::parse(&url).ok().as_ref(), "pdf");
        Some((key, url, name))
    }

    pub fn save_pdf(&mut self, key: TabKey, url: &str, path: &Path) {
        if self.model.tab(key).is_some() && self.pages.get(&key).is_some_and(|p| p.url == url) {
            self.outbox.push((key, "pdf", json!({ "path": path })));
        }
    }

    pub fn print(&mut self) {
        if self.active_url().is_some() {
            self.command("print", Value::Null);
        }
    }

    pub fn reading_list(&self) -> Vec<Bookmark> {
        bookmarks::reading_list_pages(self.bookmarks())
    }

    pub fn save_for_later(&mut self) {
        let (Some(url), Some((private, profile))) = (self.active_url().filter(|u| u.starts_with("http")), self.model.profile()) else { return };
        let title = self.active_page().map(|p| p.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| url.clone());
        let saved = bookmarks::save_for_later(self.core.store(private), profile, &self.bookmarks(), &url, &title, now());
        let (tone, text) = browser_core::toast::reading_list(saved);
        self.show_toast_as(tone, text, None);
    }

    pub fn remove_from_reading_list(&mut self, id: Id) {
        if let Some((private, _)) = self.model.profile() {
            self.core.store(private).delete_bookmark(id).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_messages_count_removed_parameters() {
        assert_eq!(copied(0), "Link copied");
        assert_eq!(copied(1), "Link copied, 1 tracking parameter removed");
        assert_eq!(copied(3), "Link copied, 3 tracking parameters removed");
    }
}
