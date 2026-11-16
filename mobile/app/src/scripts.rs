use browser_core::find::{insert_text_rows, named};
use browser_core::tabs::TabKey;
use engine::page_scripts::find_snippets;
use engine::reader::{Article, Block};
use serde_json::{Value, json};
use store::Counter;
use url::Url;

use crate::data::Platform;
use crate::shell::{Effect, Shell};

pub const HISTORY_TEXT: &str = "history_text";
pub const SORT_DOWNLOADS: &str = "sort_downloads";
const MAX_SNIPPET: usize = 320;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Probe,
    Article,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reader {
    pub key: TabKey,
    pub url: String,
    pub article: Article,
}

impl Reader {
    pub fn spoken(&self) -> String {
        let blocks = self.article.blocks.iter().map(|b| match b {
            Block::Heading(t) | Block::Paragraph(t) => t.as_str(),
        });
        std::iter::once(self.article.title.as_str()).chain(blocks).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n\n")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FindRow {
    pub key: TabKey,
    pub title: String,
    pub snippet: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FindTabs {
    pub seq: u64,
    pub query: String,
    pub rows: Vec<FindRow>,
    pub selected: usize,
    pub text_rows: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TransferState {
    Saving,
    Paused,
    Flagged(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub id: String,
    pub tab: TabKey,
    pub name: String,
    pub done: u64,
    pub total: Option<u64>,
    pub pausable: bool,
    pub state: TransferState,
    pub records: Option<(bool, store::Id)>,
}

fn text(data: &Value, field: &str) -> String {
    data.get(field).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn web(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

impl<D: Platform> Shell<D> {
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn ask(&mut self, key: TabKey, script: Script, name: &'static str, mut args: Value) -> Vec<Effect> {
        let seq = self.next_seq();
        args["seq"] = json!(seq);
        self.scripts.retain(|_, (k, s)| !(*k == key && *s == script));
        self.scripts.insert(seq, (key, script));
        vec![Effect::Command(key, name, args)]
    }

    fn answer_for(&mut self, key: TabKey, data: &Value, script: Script) -> Option<String> {
        let seq = data.get("seq").and_then(Value::as_u64)?;
        if self.scripts.get(&seq) != Some(&(key, script)) {
            return None;
        }
        self.scripts.remove(&seq);
        self.current_page(key, data)
    }

    fn current_page(&self, key: TabKey, data: &Value) -> Option<String> {
        let url = text(data, "url");
        let page = self.pages.get(&key)?;
        (page.committed.as_deref() == Some(url.as_str()) && !page.awaiting_commit).then_some(url)
    }

    pub(crate) fn after_finished(&mut self, key: TabKey) -> Vec<Effect> {
        if !self.pages.get(&key).and_then(|p| p.committed.as_deref()).is_some_and(web) {
            return Vec::new();
        }
        let mut effects = if D::page_world_scripts() { Vec::new() } else { self.ask(key, Script::Probe, "readArticle", json!({ "probe": true })) };
        if !self.model.private() && self.records() && self.setting_on(HISTORY_TEXT) && self.visits.contains_key(&key) {
            effects.extend(self.ask(key, Script::Text, "readText", json!({ "maxChars": store::MAX_HISTORY_TEXT })));
        }
        effects
    }

    pub fn offers_reader(&self) -> bool {
        self.active_page().is_some_and(|p| p.article || (D::page_world_scripts() && p.committed.as_deref().is_some_and(web)))
    }

    pub fn open_reader(&mut self) -> Vec<Effect> {
        match self.model.active {
            Some(key) => self.ask(key, Script::Article, "readArticle", json!({ "probe": false })),
            None => Vec::new(),
        }
    }

    pub fn close_reader(&mut self) {
        self.reader = None;
    }

    pub(crate) fn script_event(&mut self, key: TabKey, name: &str, data: &Value) -> Vec<Effect> {
        match name {
            "article" => {
                let seq = data.get("seq").and_then(Value::as_u64);
                let probe = seq.and_then(|s| self.scripts.get(&s)).is_some_and(|(_, s)| *s == Script::Probe);
                if probe {
                    if self.answer_for(key, data, Script::Probe).is_some()
                        && let Some(page) = self.pages.get_mut(&key)
                    {
                        page.article = text(data, "text") == "1";
                    }
                } else if let Some(url) = self.answer_for(key, data, Script::Article) {
                    match Article::parse(&text(data, "text")) {
                        Some(article) => self.reader = Some(Reader { key, url, article }),
                        None => self.notice = Some("This page has no article to read.".into()),
                    }
                }
            }
            "pageText" => {
                if let Some(url) = self.answer_for(key, data, Script::Text)
                    && let (Some(id), Some(visit)) = (self.profile_id(), self.visits.get(&key).copied())
                    && self.setting_on(HISTORY_TEXT)
                    && !self.model.private()
                {
                    let page = text(data, "text");
                    self.store().set_history_text(id, visit, &url, store::clip_history_text(&page)).ok();
                }
            }
            "textFound" => self.text_found(key, data),
            "picked" => self.picked(key, data),
            "shielded" => self.shielded_event(key, data),
            _ => {}
        }
        Vec::new()
    }

    pub fn tab_title(&self, key: TabKey) -> String {
        let page = self.pages.get(&key).map(|p| p.title.clone()).filter(|t| !t.is_empty());
        let title = page.or_else(|| self.model.tab(key).map(|t| t.title.clone()).filter(|t| !t.is_empty()));
        title.unwrap_or_else(|| self.tab_url(key))
    }

    pub fn find_in_tabs(&mut self, query: &str) -> Vec<Effect> {
        let seq = self.next_seq();
        let private = self.model.private();
        let query = query.trim().to_owned();
        let rows = if query.is_empty() {
            Vec::new()
        } else {
            let titles: std::collections::HashMap<_, _> =
                self.model.tabs.iter().map(|t| (t.id, (self.tab_title(TabKey { private, id: t.id }), self.tab_url(TabKey { private, id: t.id })))).collect();
            named(&self.model.tabs, &query, |t| titles.get(&t.id).map(|(title, url)| format!("{title} {url}")).unwrap_or_default())
                .into_iter()
                .take(browser_core::find::MAX_ROWS)
                .map(|t| FindRow {
                    key: TabKey { private, id: t.id },
                    title: titles.get(&t.id).map(|(title, _)| title.clone()).unwrap_or_default(),
                    snippet: None,
                })
                .collect()
        };
        self.find_tabs = crate::scripts::FindTabs { seq, query: query.clone(), rows, selected: 0, text_rows: 0 };
        if query.is_empty() {
            return Vec::new();
        }
        let keys: Vec<TabKey> = self
            .model
            .lifecycle
            .live()
            .filter(|k| self.model.tab(*k).is_some())
            .filter(|k| self.pages.get(k).and_then(|p| p.committed.as_deref()).is_some_and(web))
            .collect();
        keys.into_iter().map(|key| Effect::Command(key, "findText", json!({ "seq": seq, "query": query }))).collect()
    }

    fn text_found(&mut self, key: TabKey, data: &Value) {
        if data.get("seq").and_then(Value::as_u64) != Some(self.find_tabs.seq) || self.current_page(key, data).is_none() || self.model.tab(key).is_none() {
            return;
        }
        let title = self.tab_title(key);
        let joined = data.get("snippets").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("\n");
        let mut snippets: Vec<String> = find_snippets(&joined).into_iter().map(|s| s.chars().take(MAX_SNIPPET).collect()).collect();
        if snippets.is_empty()
            && let Some(n) = data.get("matched").and_then(Value::as_u64).filter(|n| *n > 0)
        {
            snippets.push(if n == 1 { "1 match on this page".into() } else { format!("{n} matches on this page") });
        }
        let rows = snippets.into_iter().map(|s| FindRow { key, title: title.clone(), snippet: Some(s) }).collect();
        let find = &mut self.find_tabs;
        insert_text_rows(&mut find.rows, &mut find.selected, &mut find.text_rows, rows);
    }

    pub fn pick(&mut self) -> Vec<Effect> {
        let Some(key) = self.model.active else { return Vec::new() };
        if self.model.private() {
            self.notice = Some("Hidden elements are not kept in a private space.".into());
            return Vec::new();
        }
        let seq = self.next_seq();
        self.picking = Some((key, seq));
        vec![Effect::Command(key, "pick", json!({ "seq": seq }))]
    }

    pub fn cancel_pick(&mut self) -> Vec<Effect> {
        match self.picking.take() {
            Some((key, _)) => vec![Effect::Command(key, "pickCancel", Value::Null)],
            None => Vec::new(),
        }
    }

    fn picked(&mut self, key: TabKey, data: &Value) {
        if self.picking.is_none_or(|(k, seq)| k != key || data.get("seq").and_then(Value::as_u64) != Some(seq)) {
            return;
        }
        self.picking = None;
        let selector = text(data, "selector");
        let host = Url::parse(&text(data, "url")).ok().and_then(|u| browser_core::site::site_host(&u).map(str::to_owned));
        let shown = self
            .pages
            .get(&key)
            .and_then(|p| p.committed.as_deref())
            .and_then(|u| Url::parse(u).ok())
            .and_then(|u| browser_core::site::site_host(&u).map(str::to_owned));
        let (Some(host), Some(id)) = (host.filter(|h| Some(h) == shown.as_ref()), self.profile_id()) else { return };
        if selector.is_empty() || !shield::lists::valid_zap(&selector) || self.model.private() {
            return;
        }
        let count = self.store().zapped_elements(id).unwrap_or_default().into_iter().filter(|(h, _)| *h == host).count();
        if count >= shield::lists::MAX_ZAPS_PER_SITE {
            self.notice = Some(format!("{host} already hides as many elements as Tarn keeps."));
            return;
        }
        if self.store().zap_element(id, &host, &selector).is_ok() {
            self.notice = Some(format!("Element hidden on {host}"));
            self.reload_host(&host);
        }
    }

    fn shielded_event(&mut self, key: TabKey, data: &Value) {
        let kind = text(data, "kind");
        let counter = match kind.as_str() {
            "https_upgrade" => Counter::HttpsUpgrades,
            "local_network" => Counter::LocalBlocked,
            "listed" => Counter::ListedStopped,
            "popup" => Counter::PopupsBlocked,
            _ => return,
        };
        let url = Url::parse(&text(data, "url")).ok();
        if counter == Counter::PopupsBlocked && url.as_ref().is_none_or(shield::rules::local_target) {
            return;
        }
        let seen = format!("{kind} {}", url.as_ref().map(Url::as_str).unwrap_or_default());
        if !self.shielded.entry(key).or_default().insert(seen) {
            return;
        }
        if let Some((private, id)) = self.model.profile() {
            let host = url.as_ref().and_then(|u| u.host_str());
            self.core.store(private).count(id, counter, 1, host).ok();
        }
    }

    pub(crate) fn download(&mut self, key: TabKey, data: &Value, now: i64) {
        let id = text(data, "id");
        let name = text(data, "name");
        let state = text(data, "state");
        let at = self.transfers.iter().position(|t| !id.is_empty() && t.id == id);
        if let Some(t) = at.and_then(|at| self.transfers.get_mut(at)) {
            t.tab = key;
        }
        let number = |field: &str| data.get(field).and_then(Value::as_u64);
        match state.as_str() {
            "started" => {
                self.notice = Some(format!("Downloading {name}…"));
                if !id.is_empty() && at.is_none() {
                    let pausable = data.get("pausable").and_then(Value::as_bool).unwrap_or(false);
                    let records = self.model.profile().filter(|_| self.records());
                    self.transfers.push(Transfer { id, tab: key, name, done: 0, total: None, pausable, state: TransferState::Saving, records });
                }
            }
            "progress" | "paused" | "resumed" | "flagged" => {
                let Some(t) = at.and_then(|at| self.transfers.get_mut(at)) else { return };
                t.done = number("done").unwrap_or(t.done);
                t.total = number("total").or(t.total);
                t.state = match state.as_str() {
                    "paused" => TransferState::Paused,
                    "flagged" => {
                        let host = text(data, "host");
                        self.notice = Some(format!("{} comes from {host}, a site on the malware list.", t.name));
                        TransferState::Flagged(host)
                    }
                    "resumed" | "progress" if t.state == TransferState::Paused && state == "progress" => TransferState::Paused,
                    _ => TransferState::Saving,
                };
            }
            "finished" | "failed" => {
                let records = match at {
                    Some(at) => self.transfers.remove(at).records,
                    None => self.model.profile().filter(|_| self.records()),
                };
                let path = text(data, "path");
                self.notice = Some(if state == "finished" {
                    format!("Downloaded {name} to Files › Tarn › Downloads.")
                } else {
                    format!("{name} could not be downloaded: {}", text(data, "message"))
                });
                if let Some((private, profile)) = records {
                    self.core.store(private).add_download(profile, &name, &path, &state, now).ok();
                }
            }
            _ => {}
        }
    }

    pub fn transfer_command(&mut self, id: &str, command: &'static str, keep: bool) -> Vec<Effect> {
        let Some(t) = self.transfers.iter().find(|t| t.id == id) else { return Vec::new() };
        let live = |k: &TabKey| self.model.lifecycle.live().any(|l| l == *k);
        let Some(tab) = Some(t.tab).filter(live).or(self.model.active).or_else(|| self.model.lifecycle.live().next()) else { return Vec::new() };
        let args = if command == "downloadKeep" { json!({ "id": id, "keep": keep }) } else { json!({ "id": id }) };
        if command == "downloadCancel" || (command == "downloadKeep" && !keep) {
            self.transfers.retain(|t| t.id != id);
        }
        vec![Effect::Command(tab, command, args)]
    }

    pub fn filter_history(&self, query: &str) -> Vec<store::HistoryEntry> {
        let history = self.history();
        let lower = query.trim().to_lowercase();
        if lower.is_empty() {
            return history;
        }
        history.into_iter().filter(|h| h.title.to_lowercase().contains(&lower) || h.url.to_lowercase().contains(&lower)).collect()
    }

    pub fn search_history(&self, query: &str) -> Vec<store::HistoryEntry> {
        let query = query.trim();
        let mut found = self.filter_history(query);
        if let Some(id) = self.profile_id().filter(|_| browser_core::suggest::searches_text(query) && self.setting_on(HISTORY_TEXT)) {
            for h in self.store().search_history_text(id, query, 50).unwrap_or_default() {
                if !found.iter().any(|f| f.url == h.url) {
                    found.push(h);
                }
            }
        }
        found
    }
}
