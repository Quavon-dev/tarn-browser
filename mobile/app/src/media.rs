use browser_core::media::Control;
use browser_core::page_ops::MediaState;
use browser_core::tabs::TabKey;
use serde_json::{Value, json};

use crate::data::Platform;
use crate::scripts::Script;
use crate::shell::{Effect, Shell};

impl<D: Platform> Shell<D> {
    fn ask_media(&mut self, key: TabKey) -> Vec<Effect> {
        self.ask(key, Script::Media, "readMedia", json!({}))
    }

    pub(crate) fn media_answered(&mut self, key: TabKey, data: &Value) {
        if self.answer_for(key, data, Script::Media).is_some() {
            self.player.answer(key, MediaState::parse(data.get("text").and_then(Value::as_str).unwrap_or_default()));
        }
    }

    /// The tabs on screen, whose pages show their own media controls.
    pub fn visible_tabs(&self) -> Vec<TabKey> {
        match self.shown_split().filter(|_| self.wide) {
            Some(split) => split.panes().to_vec(),
            None => self.model.active.into_iter().collect(),
        }
    }

    /// The mini-player's tab, its title and whether it plays.
    pub fn mini_player(&self) -> Option<(TabKey, String, bool)> {
        let (key, state) = self.player.shown(&self.visible_tabs())?;
        let title = if state.title.is_empty() { self.tab_title(key) } else { state.title.clone() };
        Some((key, title, state.playing))
    }

    /// After every action: asks the tab the user left, and lets go of a tab that closed or slept.
    pub fn settle_media(&mut self, before: Option<TabKey>) -> Vec<Effect> {
        if let Some(key) = self.player.tab().filter(|k| !self.model.lifecycle.live().any(|l| l == *k)) {
            self.player.forget(key);
        }
        match before.filter(|k| self.model.active != Some(*k) && self.model.lifecycle.live().any(|l| l == *k)) {
            Some(left) => self.ask_media(left),
            None => Vec::new(),
        }
    }

    pub fn poll_media(&mut self) -> Vec<Effect> {
        match self.player.tab().filter(|_| self.player.polls()) {
            Some(key) => self.ask_media(key),
            None => Vec::new(),
        }
    }

    pub fn press_media(&mut self, control: Control) -> Vec<Effect> {
        let Some((key, playing)) = self.player.shown(&[]).map(|(k, s)| (k, s.playing)) else { return Vec::new() };
        self.player.pressed(control);
        let mut effects = vec![Effect::Command(key, "media", control.args(playing))];
        effects.extend(self.ask_media(key));
        effects
    }
}
