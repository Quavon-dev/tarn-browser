use browser_core::snippets::{NO_FIELD, NO_INSERT, expand};
use browser_core::tabs::TabKey;
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::Snippet;

use super::{Browser, Request};

impl Browser {
    /// The current profile's snippets, read once per profile until `snippets_changed`; none in a
    /// private space, whose profile is not on disk.
    pub fn snippets(&self) -> Vec<Snippet> {
        let Some((false, profile)) = self.model.profile() else { return Vec::new() };
        let mut cache = self.snippet_cache.borrow_mut();
        if cache.as_ref().is_none_or(|(p, _)| *p != profile) {
            *cache = Some((profile, self.core.disk.snippets(profile).unwrap_or_default()));
        }
        cache.as_ref().map(|(_, s)| s.clone()).unwrap_or_default()
    }

    pub fn snippets_changed(&self) {
        self.snippet_cache.borrow_mut().take();
    }

    /// A shell field's text with a trailing `;name` expanded.
    pub fn expand_snippet(&self, text: &str) -> Option<String> {
        expand(text, &self.snippets())
    }

    /// Whether Insert Snippet can type into the active page; says why not under Lockdown.
    pub fn snippet_target(&mut self) -> Option<TabKey> {
        let key = self.model.active?;
        if self.page_url(key).is_some_and(|u| self.level_for(key, u.as_str()) == ShieldLevel::Lockdown) {
            self.show_toast(NO_INSERT, None);
            return None;
        }
        Some(key)
    }

    pub fn insert_snippet(&mut self, key: TabKey, body: &str) {
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
