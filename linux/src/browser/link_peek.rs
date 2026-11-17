use browser_core::tabs::TabKey;
use serde_json::{Value, json};
use shield::ShieldLevel;

use super::Browser;

pub const PEEK_EVENT: &str = "peek";

#[derive(Clone, Debug, PartialEq)]
pub struct LinkPeek {
    pub opener: TabKey,
    pub url: String,
    pub title: String,
    pub load: (String, u64),
    pub level: ShieldLevel,
}

impl Browser {
    pub fn open_link_peek(&mut self, opener: TabKey, url: &str) {
        if self.model.profile_of(&self.core, opener).is_none() {
            return;
        }
        let seq = self.next_seq();
        let level = self.level_for(opener, url);
        self.link_peek = Some(LinkPeek { opener, url: url.to_owned(), title: String::new(), load: (url.to_owned(), seq), level });
    }

    pub fn close_link_peek(&mut self) {
        self.link_peek = None;
    }

    pub fn on_peek_event(&mut self, name: &str, data: &Value) {
        let text = |k: &str| data[k].as_str().unwrap_or_default().to_owned();
        match name {
            PEEK_EVENT => {
                if let Some(opener) = self.link_peek.as_ref().map(|p| p.opener) {
                    self.open_link_peek(opener, &text("url"));
                }
            }
            "blocked" if text("reason") == "rebuild" => {
                if let Some(opener) = self.link_peek.as_ref().map(|p| p.opener) {
                    self.open_link_peek(opener, &text("url"));
                }
            }
            "prompt" => {
                if let Some(id) = data["id"].as_u64() {
                    self.peek_outbox.push(("answer", json!({ "id": id, "accept": false })));
                }
            }
            _ => {}
        }
        let Some(peek) = self.link_peek.as_mut() else { return };
        match name {
            "committed" | "finished" if !text("url").is_empty() => {
                peek.url = text("url");
                peek.title = text("title");
            }
            "title" => peek.title = text("title"),
            _ => {}
        }
    }

    pub fn link_peek_to_tab(&mut self) {
        let Some(peek) = self.link_peek.take() else { return };
        let opener = self.model.profile_of(&self.core, peek.opener).map(|p| (peek.opener.private, p));
        if opener.is_none() || opener != self.model.profile() {
            return;
        }
        if self.new_tab(&peek.url).is_some()
            && let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k))
        {
            page.link = true;
        }
    }
}
