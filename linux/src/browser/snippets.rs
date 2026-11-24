use browser_core::snippets::{NO_FIELD, NO_INSERT, PAGE_CHANGED};
use browser_core::tabs::TabKey;
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::Snippet;

use super::{Browser, Request};

impl Browser {
    /// The current profile's snippets (`Core::snippets`).
    pub fn snippets(&self) -> Vec<Snippet> {
        self.core.snippets(self.model.profile()).to_vec()
    }

    pub fn snippets_changed(&self) {
        self.core.snippets_changed();
    }

    /// A shell field's text with a trailing `;name` expanded.
    pub fn expand_snippet(&self, text: &str) -> Option<String> {
        self.core.expand_snippet(self.model.profile(), text)
    }

    /// Opens Insert Snippet on the active page, which it remembers with its committed address;
    /// says why not under Lockdown.
    pub fn snippet_target(&mut self) -> Option<TabKey> {
        let key = self.model.active?;
        if self.page_url(key).is_some_and(|u| self.level_for(key, u.as_str()) == ShieldLevel::Lockdown) {
            self.show_toast(NO_INSERT, None);
            return None;
        }
        self.snippet_page = self.page_url(key).map(|url| (key, url.into()));
        Some(key)
    }

    /// Types `body` into the page Insert Snippet opened on, refused once the tab shows another.
    pub fn insert_snippet(&mut self, body: &str) {
        let Some((key, url)) = self.snippet_page.take() else { return };
        if self.model.active != Some(key) || self.page_url(key).is_none_or(|u| u.as_str() != url) {
            return self.show_toast(PAGE_CHANGED, None);
        }
        self.requests.retain(|_, r| *r != Request::Insert(key));
        self.ask(key, Request::Insert(key), "insertText", json!({ "text": body }));
    }

    pub(super) fn text_inserted(&mut self, key: TabKey, data: &Value) {
        let asked = data["seq"].as_u64().filter(|s| self.requests.get(s) == Some(&Request::Insert(key)));
        if let Some(seq) = asked {
            self.requests.remove(&seq);
            if data["inserted"].as_bool() != Some(true) {
                self.show_toast(NO_FIELD, None);
            }
        }
    }
}
