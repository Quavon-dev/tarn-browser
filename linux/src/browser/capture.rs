use browser_core::capture::{ELEMENT_HINT, NO_PICKER, area_args, element_area};
use browser_core::now;
use browser_core::tabs::TabKey;
use browser_core::toast::Tone;
use engine::view_policy::SnapshotArea;
use serde_json::{Value, json};
use shield::ShieldLevel;
use tarn_nsh_shared::page_overlay::{Overlay, Purpose};
use url::Url;

use super::{Browser, Notice, Request, ToastAction};

impl Browser {
    pub fn take_screenshot(&mut self) {
        if let Some(key) = self.model.active {
            self.screenshot(key, json!({}));
        }
    }

    pub fn capture_full_page(&mut self) {
        if let Some(key) = self.model.active {
            self.screenshot(key, area_args(SnapshotArea::FullPage));
        }
    }

    /// Saves and copies `area` of the page (`page_ops::snapshot_area`'s shape; empty for the visible page).
    pub fn screenshot(&mut self, key: TabKey, mut area: Value) {
        let Some(profile) = self.model.profile_of(&self.core, key) else { return };
        let (url, title) = self.pages.get(&key).map(|p| (p.url.clone(), p.title.clone())).unwrap_or_default();
        let title = if title.is_empty() { self.model.tab(key).map(|t| t.title.clone()).unwrap_or_default() } else { title };
        area["name"] = json!(browser_core::text::capture_name(&title, Url::parse(&url).ok().as_ref(), "png"));
        self.ask(key, Request::Capture { key, profile, url, started: now() }, "screenshot", area);
    }

    /// Asks the active tab for a still of its visible page, `page` CSS pixels, to draw an overlay
    /// over; Measure only while the developer tools are on.
    pub fn page_still(&mut self, purpose: Purpose, page: (f64, f64)) -> Option<Overlay> {
        let key = self.model.active.filter(|_| purpose != Purpose::Measure || self.devtools_on())?;
        let seq = self.next_seq();
        self.outbox.push((key, "snapshotRect", json!({ "seq": seq, "x": 0, "y": 0, "width": page.0, "height": page.1 })));
        Some(match purpose {
            Purpose::Area => Overlay::area(key, seq, page),
            Purpose::Measure => Overlay::measure(key, seq, page),
        })
    }

    /// Capture Element on the active tab, whose visible page is `page` CSS pixels.
    pub fn capture_element(&mut self, page: (f64, f64)) {
        let Some(key) = self.model.active else { return };
        if self.page_url(key).is_some_and(|u| self.level_for(key, u.as_str()) == ShieldLevel::Lockdown) {
            return self.show_toast(NO_PICKER, None);
        }
        self.show_toast(ELEMENT_HINT, None);
        self.ask(key, Request::PickRect(key, page), "pick", json!({ "rect": true }));
    }

    pub(super) fn element_picked(&mut self, key: TabKey, rect: &str, page: (f64, f64)) {
        if rect.is_empty() {
            return self.dismiss_toast();
        }
        match element_area(rect, page) {
            Some(area) => self.screenshot(key, area_args(area)),
            None => self.show_toast("Nothing of that element is on screen", None),
        }
    }

    pub(super) fn on_snapshot(&mut self, from: TabKey, data: &Value) {
        let capture = |r: &Request| matches!(r, Request::Capture { key, .. } if *key == from);
        let Some(Request::Capture { key, profile, url, started }) =
            data["seq"].as_u64().filter(|s| self.requests.get(s).is_some_and(capture)).and_then(|s| self.requests.remove(&s))
        else {
            return;
        };
        match data["path"].as_str().filter(|p| !p.is_empty()) {
            Some(path) => {
                if self.core.keeps_record_since(key.private, profile, started) {
                    self.core.store(key.private).add_download(profile, &url, path, "finished", now()).ok();
                }
                self.show_toast_as(Tone::Success, "Screenshot saved", Some(ToastAction::Reveal(path.to_owned())));
            }
            None => {
                let message = browser_core::text::bounded_title(data["message"].as_str().unwrap_or_default());
                self.notice = Some(Notice::Message(format!("Download failed: {message}")));
            }
        }
    }
}
