use browser_core::prompts::Prompt;
use browser_core::suggest;
use browser_core::tabs::TabKey;
use serde_json::{Value, json};
use url::Url;

use crate::data::Platform;
use crate::shell::{Effect, PromptRequest, Shell};

pub const MAX_QUEUED_PROMPTS: usize = 8;

fn text(data: &Value, field: &str) -> Option<String> {
    data.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn blocked_message(reason: &str) -> &'static str {
    match reason {
        "https_only" => "Shield stopped a page that is not secure (HTTPS-only).",
        "local_network" => engine::nav_policy::LOCAL_NETWORK_BLOCKED,
        "shield" => "Shield blocked this address.",
        "nav_policy" => "Tarn does not open this kind of address.",
        _ => "Shield needs to open this site in a new view; open it from the address bar.",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub accept: bool,
    pub text: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub remember: bool,
}

impl<D: Platform> Shell<D> {
    pub fn event(&mut self, key: TabKey, name: &str, data: &Value, now: i64) -> Vec<Effect> {
        if name == "download" {
            self.download(key, data, now);
            return Vec::new();
        }
        if self.model.tab(key).is_none() {
            return Vec::new();
        }
        match name {
            "committed" | "finished" => return self.page_event(key, name == "committed", data, now),
            "title" => {
                let title = text(data, "title").unwrap_or_default();
                let committed = self.pages.get_mut(&key).and_then(|page| {
                    page.title = title.clone();
                    page.committed.clone().filter(|_| !page.awaiting_commit)
                });
                if let Some(url) = committed {
                    self.retitle(key, &url, &title);
                }
            }
            "failed" => {
                if let Some(page) = self.pages.get_mut(&key) {
                    page.loading = false;
                    suggest::typed_visit(&mut page.typed, &text(data, "url").unwrap_or_default());
                    page.error = text(data, "message");
                }
            }
            "blocked" => {
                let url = text(data, "url").unwrap_or_default();
                if let Some(page) = self.pages.get_mut(&key) {
                    suggest::typed_visit(&mut page.typed, &url);
                }
                let reason = text(data, "reason").unwrap_or_default();
                let offer = (reason == "local_network" && data.get("popup") != Some(&Value::Bool(true)))
                    .then(|| Url::parse(&url).ok())
                    .flatten()
                    .and_then(|u| engine::nav_policy::open_anyway_notice(&u).map(|m| (u, m)));
                self.notice = Some(match offer {
                    Some((u, message)) => {
                        self.open_offer = Some((key, u.to_string(), message.clone()));
                        message
                    }
                    None => blocked_message(&reason).into(),
                });
            }
            "external" if self.model.active == Some(key) && self.external.is_none() => {
                self.external = text(data, "url").map(|u| (key, u));
                self.external_origin = text(data, "origin").unwrap_or_default();
            }
            "popup" => {
                if let Some(url) = text(data, "url") {
                    self.new_popup_tab(key, &url, data.get("link").and_then(Value::as_bool).unwrap_or(false));
                }
            }
            "prompt" => return self.prompt(key, data),
            "article" | "pageText" | "iconLinks" | "iconData" | "pageIcon" | "textFound" | "picked" | "shielded" => return self.script_event(key, name, data),
            "found" => self.found = data.get("matched").and_then(Value::as_u64),
            "history" => {
                if let Some(page) = self.pages.get_mut(&key) {
                    page.back = data.get("canGoBack").and_then(Value::as_bool).unwrap_or(page.back);
                    page.forward = data.get("canGoForward").and_then(Value::as_bool).unwrap_or(page.forward);
                }
            }
            "pressed" if self.wide && self.model.active != Some(key) && self.shown_split().is_some_and(|s| s.panes().contains(&key)) => self.activate(key),
            "crashed" => return self.crashed(key),
            _ => {}
        }
        Vec::new()
    }

    fn page_event(&mut self, key: TabKey, committed: bool, data: &Value, now: i64) -> Vec<Effect> {
        let url = text(data, "url").unwrap_or_default();
        let title = text(data, "title").unwrap_or_default();
        let Some(page) = self.pages.get_mut(&key) else { return Vec::new() };
        if !committed && page.awaiting_commit {
            page.loading = false;
            return Vec::new();
        }
        if committed && url != "about:blank" && self.open_offer.as_ref().is_some_and(|(k, ..)| *k == key) {
            if self.can_open_anyway() {
                self.notice = None;
            }
            self.open_offer = None;
        }
        let Some(page) = self.pages.get_mut(&key) else { return Vec::new() };
        if committed {
            page.article = false;
            self.shielded.remove(&key);
            if self.reader.as_ref().is_some_and(|r| r.key == key) {
                self.reader = None;
            }
            if self.picking.is_some_and(|(k, _)| k == key) {
                self.picking = None;
            }
        }
        if !committed {
            self.crashed.remove(&key);
        }
        if committed {
            self.record_visit(key, &url, &title, now);
        }
        if let Some(page) = self.pages.get_mut(&key) {
            let error_page = committed && page.awaiting_commit && page.error.is_some() && page.url == url;
            page.url = url.clone();
            page.title = title.clone();
            if error_page {
                page.committed = Some(url.clone());
                page.awaiting_commit = false;
                page.loading = false;
                return Vec::new();
            }
            if committed {
                page.committed = Some(url.clone());
                page.awaiting_commit = false;
            }
            page.back = data.get("canGoBack").and_then(Value::as_bool).unwrap_or(false);
            page.forward = data.get("canGoForward").and_then(Value::as_bool).unwrap_or(false);
            page.loading = committed;
            page.error = None;
        }
        let saved = if committed { Some(url) } else { self.pages.get(&key).and_then(|p| p.committed.clone()) };
        if let Some(saved) = saved {
            self.retitle(key, &saved, &title);
        }
        if committed { Vec::new() } else { self.after_finished(key) }
    }

    fn prompt(&mut self, key: TabKey, data: &Value) -> Vec<Effect> {
        let Some(prompt) = Prompt::from_event(data) else { return Vec::new() };
        let id = prompt.id;
        let request = PromptRequest { tab: key, prompt };
        if self.prompts.iter().filter(|p| p.tab == key).count() >= MAX_QUEUED_PROMPTS {
            return vec![Effect::Command(key, "answer", json!({ "id": id, "accept": false }))];
        }
        if request.rememberable()
            && let Some(profile) = self.profile_id()
            && let Ok(Some(decision)) = self.store().permission(profile, &request.origin, &request.kind)
        {
            return vec![Effect::Command(key, "answer", json!({ "id": id, "accept": decision == "allow" }))];
        }
        self.prompts.push(request);
        Vec::new()
    }

    pub fn answer(&mut self, answer: Answer) -> Vec<Effect> {
        let Some(at) = self.prompts.iter().position(|p| Some(p.tab) == self.model.active) else { return Vec::new() };
        let prompt = self.prompts.remove(at);
        if answer.remember
            && prompt.rememberable()
            && let Some(profile) = self.profile_id()
        {
            self.store().set_permission(profile, &prompt.origin, &prompt.kind, if answer.accept { "allow" } else { "deny" }).ok();
        }
        let args = prompt.answer(answer.accept, answer.text, answer.user, answer.password);
        vec![Effect::Command(prompt.tab, "answer", args)]
    }

    fn crashed(&mut self, key: TabKey) -> Vec<Effect> {
        if !self.crashed.insert(key) {
            if let Some(page) = self.pages.get_mut(&key) {
                page.loading = false;
                page.error = Some("This page stopped working. Reload it to try again.".into());
            }
            return Vec::new();
        }
        vec![Effect::Command(key, "reload", Value::Null)]
    }

    pub fn can_open_anyway(&self) -> bool {
        self.open_offer.as_ref().is_some_and(|(.., message)| self.notice.as_ref() == Some(message))
    }

    pub fn open_anyway(&mut self) -> bool {
        let usable = self.can_open_anyway();
        let Some((key, url, _)) = self.open_offer.take().filter(|_| usable) else { return false };
        self.notice = None;
        if self.model.tab(key).is_none() {
            return false;
        }
        if self.model.active != Some(key) {
            self.activate(key);
        }
        self.load_in(key, &url, None);
        true
    }

    pub fn confirm_external(&mut self, open: bool) -> Vec<Effect> {
        match self.external.take() {
            Some((_, url)) if open => vec![Effect::Open(url)],
            _ => Vec::new(),
        }
    }
}
