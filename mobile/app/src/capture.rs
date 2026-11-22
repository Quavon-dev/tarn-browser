use browser_core::capture::{NO_PICKER, area_args, element_area};
use browser_core::tabs::TabKey;
use browser_core::toast::Tone;
use engine::view_policy::SnapshotArea;
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::Id;
use url::Url;

use crate::data::Platform;
use crate::shell::{Effect, Shell};

/// A `screenshot` asked of a tab, recorded in `profile` once saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub key: TabKey,
    pub profile: (bool, Id),
    pub url: String,
    pub started: i64,
}

fn text(data: &Value, field: &str) -> String {
    data.get(field).and_then(Value::as_str).unwrap_or_default().to_owned()
}

impl<D: Platform> Shell<D> {
    pub fn take_screenshot(&mut self) -> Vec<Effect> {
        self.model.active.map(|key| self.screenshot(key, json!({}))).unwrap_or_default()
    }

    pub fn capture_full_page(&mut self) -> Vec<Effect> {
        self.model.active.map(|key| self.screenshot(key, area_args(SnapshotArea::FullPage))).unwrap_or_default()
    }

    /// Saves, and outside a private space copies, `area` of `key`'s page (`page_ops::snapshot_area`'s shape; empty for the visible page).
    pub fn screenshot(&mut self, key: TabKey, mut area: Value) -> Vec<Effect> {
        let Some(profile) = self.model.profile() else { return Vec::new() };
        let url = self.tab_url(key);
        area["name"] = json!(browser_core::text::capture_name(&self.page_title(key), Url::parse(&url).ok().as_ref(), "png"));
        area["copy"] = json!(browser_core::capture::copies(key.private));
        let seq = self.next_seq();
        area["seq"] = json!(seq);
        self.shots.insert(seq, Shot { key, profile, url, started: browser_core::now() });
        vec![Effect::Command(key, "screenshot", area)]
    }

    pub fn offers_capture(&self) -> bool {
        self.model.active.is_some() && !self.active_url().is_empty()
    }

    /// Capture Element on the active tab, whose visible page is `page` CSS pixels.
    pub fn capture_element(&mut self, page: (f64, f64)) -> Vec<Effect> {
        let Some(key) = self.model.active else { return Vec::new() };
        let url = Url::parse(&self.tab_url(key)).ok();
        if self.model.profile().is_some_and(|(private, id)| self.core.level(private, id, url.as_ref()) == Some(ShieldLevel::Lockdown)) {
            self.notice = Some(NO_PICKER.into());
            return Vec::new();
        }
        let mut effects = self.cancel_pick();
        let seq = self.next_seq();
        self.capturing = Some((key, seq, page));
        effects.push(Effect::Command(key, "pick", json!({ "seq": seq, "rect": true })));
        effects
    }

    pub(crate) fn capture_picked(&mut self, key: TabKey, data: &Value) -> Vec<Effect> {
        let Some((_, _, page)) = self.capturing.filter(|(k, seq, _)| *k == key && data.get("seq").and_then(Value::as_u64) == Some(*seq)) else {
            return Vec::new();
        };
        self.capturing = None;
        let rect = text(data, "rect");
        let shown = self.pages.get(&key).and_then(|p| p.committed.clone());
        if rect.is_empty() || shown.as_deref() != Some(text(data, "url").as_str()) {
            return Vec::new();
        }
        match element_area(&rect, page) {
            Some(area) => self.screenshot(key, area_args(area)),
            None => {
                self.notice = Some(browser_core::capture::OFF_SCREEN.into());
                Vec::new()
            }
        }
    }

    pub(crate) fn screenshot_event(&mut self, key: TabKey, data: &Value) {
        let Some(seq) = data.get("seq").and_then(Value::as_u64).filter(|s| self.shots.get(s).is_some_and(|shot| shot.key == key)) else { return };
        let Some(shot) = self.shots.remove(&seq) else { return };
        let path = text(data, "path");
        if path.is_empty() {
            self.notice = Some(format!("Download failed: {}", browser_core::text::bounded_title(&text(data, "message"))));
            return;
        }
        let (private, profile) = shot.profile;
        if self.core.keeps_record_since(private, profile, shot.started) {
            self.core.store(private).add_download(profile, &shot.url, &path, "finished", browser_core::now()).ok();
        }
        self.toast_as(Tone::Success, "Screenshot saved to Downloads".into());
    }
}
