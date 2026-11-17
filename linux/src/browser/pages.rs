use browser_core::now;
use browser_core::tabs::TabKey;
use engine::reader::{Article, Block};
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::{Counter, Id};
use url::Url;

use super::{Browser, DownloadItem};

pub const HISTORY_TEXT: &str = "history_text";
pub const SORT_DOWNLOADS: &str = "sort_downloads";

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Probe(TabKey),
    Read(TabKey),
    Text(TabKey, Id),
    Find(TabKey, String),
    Pick(TabKey),
    IconLinks(TabKey, super::icons::Profile, String, engine::favicon::Ticket),
    IconData(TabKey, super::icons::Profile, String, engine::favicon::Ticket),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FindRow {
    pub key: TabKey,
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReaderView {
    pub key: TabKey,
    pub title: String,
    pub byline: String,
    pub blocks: Vec<(String, bool)>,
}

impl ReaderView {
    pub fn text(&self) -> String {
        std::iter::once(self.title.as_str()).chain(self.blocks.iter().map(|(t, _)| t.as_str())).collect::<Vec<_>>().join("\n\n")
    }
}

impl Browser {
    pub fn setting_off(&self, key: &str) -> bool {
        self.core.disk.setting(key).ok().flatten().is_none_or(|v| v != "on")
    }

    pub fn toggle_off_setting(&mut self, key: &str) {
        let on = !self.setting_off(key);
        self.core.disk.set_setting(key, if on { "off" } else { "on" }).ok();
        if key == HISTORY_TEXT && on {
            self.core.disk.clear_history_text().ok();
            self.core.store(true).clear_history_text().ok();
            self.text_found.clear();
        }
    }

    pub fn new_shield_design(&mut self) {
        if let Some((private, id)) = self.model.profile() {
            self.core.store(private).new_shield_design(id).ok();
        }
    }

    fn text_profile(&self, query: &str) -> Option<Id> {
        let (private, id) = self.model.profile()?;
        (!private && browser_core::suggest::searches_text(query) && self.suggestions() && !self.setting_off(HISTORY_TEXT)).then_some(id)
    }

    pub fn suggestions(&self) -> bool {
        self.setting_on(browser_core::settings::SUGGESTIONS)
    }

    /// The history rows and inline completion for the address field's new `value`; none while
    /// Suggestions is off.
    pub fn address_edit(
        &self,
        typed: &str,
        value: &str,
        history: &[store::HistoryEntry],
        max: usize,
    ) -> (Vec<(String, String)>, Option<browser_core::suggest::Completion>) {
        if !self.suggestions() {
            return (Vec::new(), None);
        }
        let (rows, completion) = browser_core::suggest::edit(typed, value, &self.model.tabs, history, max, now());
        (rows.into_iter().map(|h| (h.title.clone(), h.url.clone())).collect(), completion)
    }

    pub fn cached_text_matches(&self, query: &str) -> Option<Vec<store::HistoryEntry>> {
        match self.text_profile(query) {
            Some(id) => self.text_found.get(id, query).map(<[_]>::to_vec),
            None => Some(Vec::new()),
        }
    }

    pub fn search_text_matches(&mut self, query: &str, max: usize) -> Vec<store::HistoryEntry> {
        let Some(id) = self.text_profile(query) else { return Vec::new() };
        if let Some(found) = self.text_found.get(id, query) {
            return found.to_vec();
        }
        let found = self.core.disk.search_history_text(id, query, max).unwrap_or_default();
        self.text_found.put(id, query, found.clone());
        found
    }

    pub fn page_url(&self, key: TabKey) -> Option<Url> {
        self.pages.get(&key).and_then(|p| Url::parse(&p.url).ok())
    }

    pub fn unlocked(&self, key: TabKey) -> Option<Url> {
        let url = self.page_url(key).filter(|u| matches!(u.scheme(), "http" | "https"))?;
        (self.level_for(key, url.as_str()) != ShieldLevel::Lockdown).then_some(url)
    }

    pub(super) fn ask(&mut self, key: TabKey, request: Request, name: &'static str, args: Value) {
        let seq = self.next_seq();
        self.requests.insert(seq, request);
        let mut args = args;
        args["seq"] = json!(seq);
        self.outbox.push((key, name, args));
    }

    fn answered(&mut self, data: &Value, key: TabKey) -> Option<Request> {
        let request = self.requests.remove(&data["seq"].as_u64()?)?;
        let url = self.pages.get(&key).map(|p| p.url.as_str()).unwrap_or_default();
        (data["url"].as_str() == Some(url)).then_some(request)
    }

    pub(super) fn on_script_event(&mut self, key: TabKey, name: &str, data: &Value) {
        let text = data["text"].as_str().unwrap_or_default();
        match (name, self.answered(data, key)) {
            ("article", Some(Request::Probe(k))) if k == key && text == "1" => {
                self.articles.insert(key);
            }
            ("article", Some(Request::Read(k))) if k == key => self.open_article(key, text),
            ("pageText", Some(Request::Text(k, visit))) if k == key => self.keep_text(key, visit, text),
            ("textFound", Some(Request::Find(k, query))) if k == key && query == self.find_query => {
                let snippets: Vec<String> = data["snippets"].as_array().into_iter().flatten().filter_map(|s| s.as_str()).map(str::to_owned).collect();
                self.add_text_rows(key, engine::page_scripts::find_snippets(&snippets.join("\n")));
            }
            ("picked", Some(Request::Pick(k))) if k == key => {
                let selector = data["selector"].as_str().unwrap_or_default().to_owned();
                self.zap_picked(key, &selector);
            }
            _ => {}
        }
    }

    pub(super) fn after_load(&mut self, key: TabKey, finished: bool, url: &Url, visit: Option<Id>) {
        if !finished {
            self.articles.remove(&key);
            self.counted.remove(&key);
            if self.reader.as_ref().is_some_and(|r| r.key == key) {
                self.close_reader();
            }
            return;
        }
        if self.unlocked(key).is_none() {
            return;
        }
        self.ask(key, Request::Probe(key), "readArticle", json!({ "probe": true }));
        self.request_favicon(key, url);
        let keeps = self.model.profile_of(&self.core, key).is_some_and(|p| self.core.keeps_records(key.private, p));
        if let Some(visit) = visit.filter(|_| !key.private && keeps && !self.setting_off(HISTORY_TEXT) && url.scheme() != "file") {
            self.ask(key, Request::Text(key, visit), "readText", json!({ "maxChars": store::MAX_HISTORY_TEXT }));
        }
    }

    fn keep_text(&mut self, key: TabKey, visit: Id, text: &str) {
        let (Some(profile), Some(url)) = (self.model.profile_of(&self.core, key), self.pages.get(&key).map(|p| p.url.clone())) else { return };
        if !key.private && !self.setting_off(HISTORY_TEXT) && !text.trim().is_empty() {
            self.core.disk.set_history_text(profile, visit, &url, store::clip_history_text(text)).ok();
            self.text_found.clear();
        }
    }

    pub fn toggle_reader(&mut self) {
        let Some(key) = self.model.active else { return };
        if self.reader.as_ref().is_some_and(|r| r.key == key) {
            return self.close_reader();
        }
        if self.unlocked(key).is_none() {
            return self.show_toast("No article found on this page", None);
        }
        self.ask(key, Request::Read(key), "readArticle", json!({ "probe": false }));
    }

    fn open_article(&mut self, key: TabKey, text: &str) {
        if self.model.active != Some(key) {
            return;
        }
        let Some(Article { title, byline, blocks }) = Article::parse(text) else {
            return self.show_toast("No article found on this page", None);
        };
        let blocks = blocks
            .into_iter()
            .map(|b| match b {
                Block::Heading(t) => (t, true),
                Block::Paragraph(t) => (t, false),
            })
            .collect();
        self.reader = Some(ReaderView { key, title: engine::view_policy::title(&title), byline, blocks });
    }

    pub fn close_reader(&mut self) {
        if self.reader.take().is_some() {
            crate::speech::stop();
        }
    }

    pub fn find_in_tabs(&mut self, query: &str) {
        self.find_query = query.to_owned();
        self.requests.retain(|_, r| !matches!(r, Request::Find(..)));
        let label = |b: &Browser, key: TabKey, tab: &store::Tab| crate::snapshot::tab_label(tab, b, key);
        self.find_rows = browser_core::find::named(&self.model.tabs, query, |t| label(self, self.model.key(t), t))
            .into_iter()
            .map(|t| FindRow { key: self.model.key(t), label: label(self, self.model.key(t), t), detail: "Title or address".to_owned() })
            .collect();
        self.text_rows = 0;
        if query.is_empty() {
            return;
        }
        let awake: Vec<TabKey> = self.model.lifecycle.live().filter(|k| self.model.tab(*k).is_some() && self.unlocked(*k).is_some()).collect();
        for key in awake {
            self.ask(key, Request::Find(key, query.to_owned()), "findText", json!({ "query": query }));
        }
    }

    fn add_text_rows(&mut self, key: TabKey, snippets: Vec<String>) {
        let Some(title) = self.model.tab(key).map(|t| crate::snapshot::tab_label(t, self, key)) else { return };
        let rows = snippets.into_iter().map(|s| FindRow { key, label: engine::view_policy::title(&s), detail: title.clone() }).collect();
        let mut selected = 0;
        browser_core::find::insert_text_rows(&mut self.find_rows, &mut selected, &mut self.text_rows, rows);
    }

    pub fn open_find_row(&mut self, key: TabKey) -> String {
        let query = std::mem::take(&mut self.find_query);
        self.find_rows.clear();
        self.activate(key);
        query
    }

    pub(super) fn on_shielded(&mut self, key: TabKey, data: &Value) {
        let url = data["url"].as_str().unwrap_or_default();
        let counter = match data["kind"].as_str().unwrap_or_default() {
            "https_upgrade" => Counter::HttpsUpgrades,
            "local_network" => Counter::LocalBlocked,
            "listed" => Counter::ListedStopped,
            "popup" if Url::parse(url).is_ok_and(|u| !shield::rules::local_target(&u)) => Counter::PopupsBlocked,
            _ => return,
        };
        let kind = data["kind"].as_str().unwrap_or_default().to_owned();
        if !self.counted.entry(key).or_default().insert(kind) {
            return;
        }
        let host = Url::parse(url).ok().and_then(|u| browser_core::site::site_host(&u).map(str::to_owned));
        if let Some(profile) = self.model.profile_of(&self.core, key).filter(|p| self.core.keeps_records(key.private, *p)) {
            self.core.store(key.private).count(profile, counter, 1, host.as_deref()).ok();
        }
    }

    pub fn on_download(&mut self, key: TabKey, data: &Value, record: bool) -> bool {
        let text = |k: &str| data[k].as_str().unwrap_or_default().to_owned();
        let id = text("id");
        let state = text("state");
        let (owner, started) = *self.download_owners.entry(id.clone()).or_insert_with(|| (self.model.profile_of(&self.core, key), now()));
        if state == "finished" || state == "failed" {
            self.download_owners.remove(&id);
        }
        let known = owner.is_some();
        if state == "progress" {
            if let Some(item) = self.downloads.iter_mut().find(|d| d.id == id) {
                item.done = data["done"].as_u64().unwrap_or(item.done);
                if item.total.is_none() && data["total"].as_u64().is_some() {
                    self.downloads_moved = std::time::Instant::now();
                }
                item.total = data["total"].as_u64().or(item.total);
            }
            return known;
        }
        self.settle_downloads();
        let state_is_flagged = state == "flagged";
        if state_is_flagged && known {
            self.show_toast_as(browser_core::toast::Tone::Warning, engine::downloads::flagged_notice(&text("name"), &text("host")), None);
        }
        let previous = self.downloads.iter().position(|d| !id.is_empty() && d.id == id);
        let old = previous.map(|ix| self.downloads.remove(ix));
        let item = DownloadItem {
            id,
            name: Some(text("name")).filter(|n| !n.is_empty()).or_else(|| old.as_ref().map(|o| o.name.clone())).unwrap_or_default(),
            path: Some(text("path")).filter(|p| !p.is_empty()).or_else(|| old.as_ref().map(|o| o.path.clone())).unwrap_or_default(),
            state,
            message: if state_is_flagged { engine::downloads::flagged_note(&text("host")) } else { text("message") },
            done: old.as_ref().map_or(0, |o| o.done),
            total: old.as_ref().and_then(|o| o.total),
        };
        let wiped = |p: Id| self.core.wiped_at.borrow().get(&(key.private, p)).is_some_and(|at| started < *at);
        let keeps = owner.filter(|p| self.core.keeps_records(key.private, *p) && !wiped(*p));
        if record
            && item.state == "finished"
            && let Some(profile) = keeps
        {
            self.core.store(key.private).add_download(profile, &text("url"), &item.path, "finished", now()).ok();
        }
        self.downloads.push(item);
        known
    }

    pub fn cancel_download(&mut self, id: &str) {
        self.cancelled_downloads.push(id.to_owned());
    }

    pub fn keep_download(&mut self, id: &str, keep: bool) {
        if keep && !browser_core::prompts::stable(self.downloads_moved, std::time::Instant::now()) {
            return;
        }
        self.kept_downloads.push((id.to_owned(), keep));
        match self.downloads.iter_mut().find(|d| d.id == id) {
            Some(item) if keep => {
                item.state = "started".to_owned();
                item.message.clear();
                self.settle_downloads();
            }
            _ => self.forget_download(id),
        }
    }

    pub fn settle_downloads(&mut self) {
        self.downloads_moved = std::time::Instant::now();
    }

    pub fn forget_download(&mut self, id: &str) {
        self.settle_downloads();
        self.downloads.retain(|d| d.id != id);
        self.download_owners.remove(id);
    }
}
