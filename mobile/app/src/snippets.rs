use browser_core::snippets::{NO_FIELD, NO_INSERT, expand};
use browser_core::tabs::TabKey;
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::Snippet;
use url::Url;

use crate::data::Platform;
use crate::shell::{Effect, Shell};

impl<D: Platform> Shell<D> {
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
        let url = Url::parse(&self.tab_url(key)).ok();
        if self.model.profile().is_some_and(|(private, id)| self.core.level(private, id, url.as_ref()) == Some(ShieldLevel::Lockdown)) {
            self.notice = Some(NO_INSERT.into());
            return None;
        }
        Some(key)
    }

    pub fn insert_snippet(&mut self, body: &str) -> Vec<Effect> {
        let Some(key) = self.snippet_target() else { return Vec::new() };
        let seq = self.next_seq();
        self.inserting = Some((key, seq));
        vec![Effect::Command(key, "insertText", json!({ "seq": seq, "text": body }))]
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
