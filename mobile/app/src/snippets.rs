use browser_core::snippets::{NO_FIELD, NO_INSERT, PAGE_CHANGED};
use browser_core::tabs::TabKey;
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::Snippet;
use url::Url;

use crate::data::Platform;
use crate::shell::{Effect, Shell};

impl<D: Platform> Shell<D> {
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
        let url = Url::parse(&self.tab_url(key)).ok();
        if self.model.profile().is_some_and(|(private, id)| self.core.level(private, id, url.as_ref()) == Some(ShieldLevel::Lockdown)) {
            self.notice = Some(NO_INSERT.into());
            return None;
        }
        self.snippet_page = self.committed(key).map(|url| (key, url));
        Some(key)
    }

    /// Types `body` into the page Insert Snippet opened on, refused once the tab shows another.
    pub fn insert_snippet(&mut self, body: &str) -> Vec<Effect> {
        let Some((key, url)) = self.snippet_page.take() else { return Vec::new() };
        if self.model.active != Some(key) || self.committed(key).as_deref() != Some(url.as_str()) {
            self.notice = Some(PAGE_CHANGED.into());
            return Vec::new();
        }
        let seq = self.next_seq();
        self.inserting = Some((key, seq));
        vec![Effect::Command(key, "insertText", json!({ "seq": seq, "text": body }))]
    }

    fn committed(&self, key: TabKey) -> Option<String> {
        self.pages.get(&key).and_then(|p| p.committed.clone())
    }

    pub(crate) fn text_inserted(&mut self, key: TabKey, data: &Value) {
        if self.inserting != Some((key, data.get("seq").and_then(Value::as_u64).unwrap_or_default())) {
            return;
        }
        self.inserting = None;
        if data.get("inserted").and_then(Value::as_bool) != Some(true) {
            self.notice = Some(NO_FIELD.into());
        }
    }
}
