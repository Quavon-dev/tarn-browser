use browser_core::media::Control;
use browser_core::page_ops::MediaState;
use browser_core::tabs::TabKey;
use serde_json::json;

use super::{Browser, Request};

impl Browser {
    pub(super) fn ask_media(&mut self, key: TabKey) {
        self.requests.retain(|_, r| *r != Request::Media(key));
        self.ask(key, Request::Media(key), "readMedia", json!({}));
    }

    pub(super) fn media_answered(&mut self, key: TabKey, text: &str) {
        self.player.answer(key, MediaState::parse(text));
    }

    /// The tabs on screen, whose pages show their own media controls.
    pub fn visible_tabs(&self) -> Vec<TabKey> {
        self.shown_split().map_or_else(|| self.model.active.into_iter().collect(), |s| s.panes().to_vec())
    }

    /// The mini-player's tab, its title and whether it plays.
    pub fn mini_player(&self) -> Option<(TabKey, String, bool)> {
        let (key, state) = self.player.shown(&self.visible_tabs())?;
        let title = if state.title.is_empty() { self.model.tab(key).map(|t| t.title.clone()).unwrap_or_default() } else { state.title.clone() };
        Some((key, title, state.playing))
    }

    /// After every action: asks the tab the user left, and lets go of a tab that closed or slept.
    pub fn settle_media(&mut self, before: Option<TabKey>) {
        if let Some(left) = before.filter(|k| self.model.active != Some(*k) && self.model.lifecycle.live().any(|l| l == *k)) {
            self.ask_media(left);
        }
        if let Some(key) = self.player.tab().filter(|k| !self.model.lifecycle.live().any(|l| l == *k)) {
            self.player.forget(key);
        }
    }

    pub fn poll_media(&mut self) {
        if let Some(key) = self.player.tab().filter(|_| self.player.polls()) {
            self.ask_media(key);
        }
    }

    pub fn press_media(&mut self, control: Control) {
        let Some((key, playing)) = self.player.shown(&[]).map(|(k, s)| (k, s.playing)) else { return };
        self.outbox.push((key, "media", control.args(playing)));
        self.player.pressed(control);
        self.ask_media(key);
    }
}
