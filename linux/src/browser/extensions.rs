use browser_core::prompts::Prompt;
use browser_core::tabs::TabKey;
use serde_json::Value;
use store::Id;

use super::Browser;
use crate::extensions::{self, Button, Note};

pub const PROMPT_KIND: &str = "extension";
const FIRST_PROMPT: u64 = 1 << 50;

#[derive(Clone, Debug, PartialEq)]
pub struct ExtPrompt {
    pub profile: Id,
    pub install: String,
    pub host: String,
}

impl Browser {
    fn ext_profile(&self, key: TabKey) -> Option<Id> {
        (!key.private).then(|| self.model.profile_of(&self.core, key)).flatten()
    }

    pub(super) fn on_extension_message(&mut self, key: TabKey, data: &Value) {
        let (Some(profile), Some(world), Some(json)) = (self.ext_profile(key), data["world"].as_str(), data["json"].as_str()) else { return };
        extensions::message(profile, key.id as u64, world, json);
    }

    pub(super) fn ext_navigated(&self, key: TabKey) {
        if let Some(profile) = self.ext_profile(key) {
            extensions::navigated(profile, key.id as u64);
        }
    }

    pub(super) fn ext_tab_closed(&self, key: TabKey) {
        if !key.private {
            extensions::tab_closed(key.id as u64);
        }
    }

    pub(super) fn ext_activated(&self, key: TabKey) {
        extensions::activated((!key.private).then_some(key.id as u64));
    }

    pub fn extension_buttons(&self) -> Vec<(Id, Button)> {
        let Some(key) = self.model.active.filter(|k| !k.private) else { return Vec::new() };
        let Some(profile) = self.ext_profile(key) else { return Vec::new() };
        extensions::buttons(profile, Some(key.id as u64)).into_iter().map(|b| (profile, b)).collect()
    }

    pub fn click_extension(&mut self, profile: Id, install: &str) -> bool {
        let Some(key) = self.model.active.filter(|k| !k.private && self.ext_profile(*k) == Some(profile)) else { return false };
        extensions::clicked(profile, install, key.id as u64)
    }

    pub fn on_extension_note(&mut self, note: &Note) -> bool {
        match note {
            Note::Ask { profile, tab, install, host } => {
                let key = TabKey { private: false, id: *tab as Id };
                if !self.pages.contains_key(&key) {
                    return false;
                }
                let Some(asking) = extensions::asking(*profile, install) else { return true };
                self.ext_prompt_serial += 1;
                let id = FIRST_PROMPT + self.ext_prompt_serial;
                let prompt = Prompt {
                    id,
                    kind: PROMPT_KIND.to_owned(),
                    origin: host.clone(),
                    message: Some(asking.name),
                    default: None,
                    realm: Some(asking.badge),
                    insecure: false,
                    top_origin: None,
                };
                self.ext_prompts.insert(id, ExtPrompt { profile: *profile, install: install.clone(), host: host.clone() });
                self.pages.entry(key).or_default().prompts.push_back(prompt);
                true
            }
            Note::OpenTab { url, active } => {
                if *active {
                    self.new_tab(url.as_str());
                } else if self.model.new_tab(&self.core, url.as_str(), None).is_some() {
                    self.model.reload_space(&self.core);
                }
                true
            }
            Note::Changed | Note::PageClosed(_) => false,
        }
    }

    pub fn answer_site_prompt(&mut self, id: u64, accept: bool, remember: bool) {
        let asked = self.model.active.and_then(|k| self.pages.get(&k)).is_some_and(|p| p.prompts.iter().any(|p| p.id == id && p.kind == PROMPT_KIND));
        if let Some((key, prompt)) = asked.then(|| self.take_prompt(id)).flatten() {
            self.answer_extension(key, &prompt, accept, remember);
        }
    }

    pub(super) fn answer_extension(&mut self, key: TabKey, prompt: &Prompt, accept: bool, remember: bool) {
        let Some(asked) = self.ext_prompts.remove(&prompt.id) else { return };
        if extensions::answer(asked.profile, &asked.install, &asked.host, accept, remember, key.id as u64) {
            let url = self.pages.get(&key).map(|p| p.url.clone()).unwrap_or_default();
            self.load(key, &url);
        }
    }
}
