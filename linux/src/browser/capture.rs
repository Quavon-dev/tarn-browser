use browser_core::now;
use browser_core::tabs::TabKey;
use browser_core::toast::Tone;
use serde_json::{Value, json};
use url::Url;

use super::{Browser, Notice, Request, ToastAction};

impl Browser {
    pub fn take_screenshot(&mut self) {
        let Some(key) = self.model.active else { return };
        let Some(profile) = self.model.profile_of(&self.core, key) else { return };
        let (url, title) = self.pages.get(&key).map(|p| (p.url.clone(), p.title.clone())).unwrap_or_default();
        let title = if title.is_empty() { self.model.tab(key).map(|t| t.title.clone()).unwrap_or_default() } else { title };
        let name = browser_core::capture::file_name(&title, Url::parse(&url).ok().as_ref(), "png");
        self.ask(key, Request::Capture { key, profile, url, started: now() }, "snapshot", json!({ "name": name }));
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
