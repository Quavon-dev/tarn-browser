use browser_core::find::{insert_text_rows, named};
use browser_core::tabs::TabKey;
use engine::favicon;
use engine::favicon::Ticket;
use engine::page_scripts::find_snippets;
use engine::reader::{Article, Block};
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::{Counter, Id};
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
    IconLinks,
    Icon,
    Media,
    Assistant,
    AssistantTab,
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
    pub started: i64,
    pub path: String,
}

fn text(data: &Value, field: &str) -> String {
    data.get(field).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn web(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

impl<D: Platform> Shell<D> {
    pub(crate) fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    pub(crate) fn ask(&mut self, key: TabKey, script: Script, name: &'static str, mut args: Value) -> Vec<Effect> {
        let seq = self.next_seq();
        args["seq"] = json!(seq);
        self.scripts.retain(|_, (k, s)| !(*k == key && *s == script));
        self.scripts.insert(seq, (key, script));
        vec![Effect::Command(key, name, args)]
    }

    pub(crate) fn drop_scripts(&mut self, key: TabKey) {
        for ((profile, host), ticket) in self.icon_asks.remove(&key).unwrap_or_default() {
            self.icons.withdraw(&profile, &host, ticket);
        }
        self.scripts.retain(|_, (k, _)| *k != key);
    }

    pub(crate) fn answer_for(&mut self, key: TabKey, data: &Value, script: Script) -> Option<String> {
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
        effects.extend(self.request_icon(key));
        effects
    }

    fn icon_key(&self, key: TabKey, url: &str) -> Option<((bool, Id), String)> {
        let (profile, host) = self.asked_icon(key, url)?;
        let url = Url::parse(url).ok()?;
        let lockdown = self.core.level(profile.0, profile.1, Some(&url)).is_none_or(|l| l == ShieldLevel::Lockdown);
        (self.core.keeps_records(profile.0, profile.1) && !lockdown).then_some((profile, host))
    }

    fn asked_icon(&self, key: TabKey, url: &str) -> Option<((bool, Id), String)> {
        let url = Url::parse(url).ok()?;
        let profile = self.model.profile_of(&self.core, key)?;
        Some(((key.private, profile), favicon::icon_host(&url, false)?))
    }

    pub(crate) fn load_icons(&mut self, (private, profile): (bool, Id)) {
        let store = self.core.store(private);
        self.icons.load(&(private, profile), || {
            let icons = store.favicons(profile).unwrap_or_default().into_iter().map(|(host, png)| (host, favicon::data_uri(&png))).collect();
            (icons, store.stale_favicons(profile, browser_core::now() - favicon::MAX_AGE).unwrap_or_default())
        });
    }

    pub fn site_icon(&self, url: &str) -> Option<String> {
        let host = Url::parse(url).ok()?.host_str()?.to_owned();
        self.icons.find(&self.model.profile()?, &host).cloned()
    }

    fn request_icon(&mut self, key: TabKey) -> Vec<Effect> {
        if D::page_world_scripts() {
            return Vec::new();
        }
        let Some(url) = self.pages.get(&key).and_then(|p| p.committed.clone()) else { return Vec::new() };
        let Some((profile, host)) = self.icon_key(key, &url) else { return Vec::new() };
        self.load_icons(profile);
        let Some(ticket) = self.icons.request(&profile, &host) else { return Vec::new() };
        self.icon_asks.entry(key).or_default().insert((profile, host), ticket);
        self.ask(key, Script::IconLinks, "readIconLinks", json!({}))
    }

    fn icon_answer(&mut self, key: TabKey, data: &Value, script: Script) -> Option<(String, (bool, Id), String, Ticket)> {
        let asked = text(data, "url");
        let (profile, host) = self.asked_icon(key, &asked)?;
        let ticket = *self.icon_asks.get(&key)?.get(&(profile, host.clone()))?;
        let accepted = self.icon_key(key, &asked).is_some();
        match self.answer_for(key, data, script).filter(|_| accepted) {
            Some(url) if self.icons.is_requested(&profile, &host, ticket) => Some((url, profile, host, ticket)),
            _ => {
                self.icon_done(key, &profile, &host);
                self.icons.withdraw(&profile, &host, ticket);
                None
            }
        }
    }

    fn icon_links(&mut self, key: TabKey, data: &Value) -> Vec<Effect> {
        let Some((url, profile, host, ticket)) = self.icon_answer(key, data, Script::IconLinks) else { return Vec::new() };
        let urls: Vec<String> =
            Url::parse(&url).map(|page| favicon::candidates(&page, &text(data, "text"))).unwrap_or_default().into_iter().map(String::from).collect();
        if urls.is_empty() {
            self.icon_done(key, &profile, &host);
            self.icons.withdraw(&profile, &host, ticket);
            return Vec::new();
        }
        self.ask(key, Script::Icon, "fetchIcon", json!({ "page": url, "urls": urls }))
    }

    pub fn icon_event(&mut self, key: TabKey, name: &str, data: &Value, png: Option<Vec<u8>>) {
        if name == "iconData" {
            self.icon_data(key, data, png);
        }
    }

    fn icon_data(&mut self, key: TabKey, data: &Value, png: Option<Vec<u8>>) {
        let Some((_, profile, host, ticket)) = self.icon_answer(key, data, Script::Icon) else { return };
        self.icon_done(key, &profile, &host);
        self.keep_icon(profile, &host, ticket, png);
    }

    pub(crate) fn forget_icons(&mut self, profile: (bool, Id)) {
        self.icons.forget(&profile);
        for asks in self.icon_asks.values_mut() {
            asks.retain(|(p, _), _| *p != profile);
        }
    }

    fn icon_done(&mut self, key: TabKey, profile: &(bool, Id), host: &str) {
        if let Some(asks) = self.icon_asks.get_mut(&key) {
            asks.remove(&(*profile, host.to_owned()));
        }
    }

    fn keep_icon(&mut self, profile: (bool, Id), host: &str, ticket: Ticket, png: Option<Vec<u8>>) {
        let kept = self.icons.accept(&profile, host, ticket, png.as_deref().map(favicon::data_uri));
        if let Some(png) = png.filter(|_| kept && self.core.keeps_records(profile.0, profile.1)) {
            self.core.store(profile.0).set_favicon(profile.1, host, &png).ok();
        }
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
            "pageText" if data.get("seq").and_then(Value::as_u64).and_then(|s| self.scripts.get(&s)).is_some_and(|(_, s)| *s == Script::Assistant) => {
                self.page_for_assistant(key, data);
            }
            "pageText" if data.get("seq").and_then(Value::as_u64).and_then(|s| self.scripts.get(&s)).is_some_and(|(_, s)| *s == Script::AssistantTab) => {
                self.tab_for_assistant(key, data);
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
            "iconLinks" => return self.icon_links(key, data),
            "iconData" => self.icon_event(key, name, data, favicon::normalize(&text(data, "text"))),
            "textFound" => self.text_found(key, data),
            "mediaState" => self.media_answered(key, data),
            "picked" if self.capturing.is_some_and(|(k, ..)| k == key) => return self.capture_picked(key, data),
            "picked" => self.picked(key, data),
            "shielded" => self.shielded_event(key, data),
            _ => {}
        }
        Vec::new()
    }

    /// The assistant's `Origin` for the active page, or the current space with no page.
    pub fn assistant_origin(&self) -> Option<browser_core::assistant::Origin> {
        let url = self.active_url();
        self.model.assistant_origin(&self.core, Some(&url).filter(|u| !u.is_empty()).map(String::as_str))
    }

    /// Reads the active web page's text for the assistant; it arrives in `assistant_pages`.
    pub fn read_page_for_assistant(&mut self) -> Vec<Effect> {
        let Some(key) = self.model.active.filter(|_| web(&self.active_url())) else {
            self.notice = Some("This page cannot be shared with the assistant.".into());
            return Vec::new();
        };
        self.ask(key, Script::Assistant, "readText", json!({ "maxChars": store::MAX_HISTORY_TEXT }))
    }

    fn page_for_assistant(&mut self, key: TabKey, data: &Value) {
        let Some(url) = self.answer_for(key, data, Script::Assistant).filter(|_| self.model.active == Some(key)) else { return };
        if let Some(origin) = self.model.assistant_origin(&self.core, Some(&url)) {
            let pill = browser_core::assistant::Pill::Page { title: self.page_title(key), text: text(data, "text"), url };
            self.assistant_pages.push((origin, pill, None));
        }
    }

    /// Reads another tab's loaded page for the assistant's tab picker; a sleeping tab is never
    /// woken for it. The text arrives in `assistant_pages`.
    pub fn read_tab_for_assistant(&mut self, key: TabKey) -> Vec<Effect> {
        if !self.model.lifecycle.live().any(|k| k == key) || !web(&self.tab_url(key)) {
            self.notice = Some("That tab is asleep or cannot be shared. Open it to add it.".into());
            return Vec::new();
        }
        self.ask(key, Script::AssistantTab, "readText", json!({ "maxChars": store::MAX_HISTORY_TEXT }))
    }

    fn tab_for_assistant(&mut self, key: TabKey, data: &Value) {
        let Some(url) = self.answer_for(key, data, Script::AssistantTab).filter(|_| self.model.tab(key).is_some()) else { return };
        if let Some(origin) = self.model.assistant_origin(&self.core, Some(&url)) {
            let pill = browser_core::assistant::Pill::Tab { title: self.page_title(key), text: text(data, "text"), url };
            self.assistant_pages.push((origin, pill, None));
        }
    }

    /// The selection the page's menu sent (`assistantSelection`), where the assistant may still
    /// see that page; a command sends it once the chat attached it.
    pub(crate) fn selection_for_assistant(&mut self, key: TabKey, data: &Value) {
        let Some(url) = self.current_page(key, data) else { return };
        let here = |o: &browser_core::assistant::Origin| {
            self.model.profile_of(&self.core, key) == Some(o.profile) && self.model.assistant_offered(&self.core, Some(&url))
        };
        let Some(origin) = self.model.assistant_origin(&self.core, Some(&url)).filter(here) else {
            self.notice = Some("This page cannot be shared with the assistant.".into());
            return;
        };
        let command = data.get("command").and_then(Value::as_str).map(str::to_owned);
        match browser_core::assistant::selection(&url, &text(data, "text")) {
            Some(pill) => self.assistant_pages.push((origin, pill, command)),
            None => self.notice = Some("No text that can be shared is selected.".into()),
        }
    }

    pub fn page_title(&self, key: TabKey) -> String {
        let page = self.pages.get(&key).map(|p| p.title.clone()).filter(|t| !t.is_empty());
        page.or_else(|| self.model.tab(key).map(|t| t.title.clone())).unwrap_or_default()
    }

    pub fn tab_title(&self, key: TabKey) -> String {
        let custom = self.model.tab(key).map(|t| t.custom_title.as_str()).unwrap_or_default();
        browser_core::command::display_title(custom, &self.page_title(key), &self.tab_url(key))
    }

    pub fn page_shown_title(&self, key: TabKey) -> String {
        browser_core::command::display_title("", &self.page_title(key), &self.tab_url(key))
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
        self.capturing = None;
        let seq = self.next_seq();
        self.picking = Some((key, seq));
        vec![Effect::Command(key, "pick", json!({ "seq": seq }))]
    }

    pub fn cancel_pick(&mut self) -> Vec<Effect> {
        match self.picking.take().map(|(k, _)| k).or(self.capturing.take().map(|(k, ..)| k)) {
            Some(key) => vec![Effect::Command(key, "pickCancel", Value::Null)],
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
        let bar_appears = at.and_then(|at| self.transfers.get(at)).is_some_and(|t| t.total.is_none() && number("total").is_some());
        if state != "progress" || bar_appears {
            browser_core::prompts::restart(&mut self.transfers_moved, std::time::Instant::now());
        }
        match state.as_str() {
            "started" => {
                self.download_notice(format!("Downloading {name}…"), false);
                if !id.is_empty() && at.is_none() {
                    let pausable = data.get("pausable").and_then(Value::as_bool).unwrap_or(false);
                    let records = self.model.profile().filter(|_| self.records());
                    let path = text(data, "path");
                    self.transfers.push(Transfer {
                        id,
                        tab: key,
                        name,
                        done: 0,
                        total: None,
                        pausable,
                        state: TransferState::Saving,
                        records,
                        started: now,
                        path,
                    });
                }
            }
            "progress" | "paused" | "resumed" | "flagged" => {
                let Some(t) = at.and_then(|at| self.transfers.get_mut(at)) else { return };
                t.done = number("done").unwrap_or(t.done);
                if let Some(path) = Some(text(data, "path")).filter(|p| !p.is_empty()) {
                    t.path = path;
                }
                t.total = number("total").or(t.total);
                let mut flagged = None;
                t.state = match state.as_str() {
                    "paused" => TransferState::Paused,
                    "flagged" => {
                        let host = text(data, "host");
                        flagged = Some(engine::downloads::flagged_notice(&t.name, &host));
                        TransferState::Flagged(host)
                    }
                    "resumed" | "progress" if t.state == TransferState::Paused && state == "progress" => TransferState::Paused,
                    _ => TransferState::Saving,
                };
                if let Some(notice) = flagged {
                    self.download_notice(notice, true);
                }
            }
            "finished" | "failed" => {
                let (records, started, kept_path) = match at {
                    Some(at) => {
                        let t = self.transfers.remove(at);
                        (t.records, t.started, t.path)
                    }
                    None => (self.model.profile().filter(|_| self.records()), now, String::new()),
                };
                let path = Some(text(data, "path")).filter(|p| !p.is_empty()).unwrap_or(kept_path);
                if state == "finished" {
                    let warns = engine::downloads::is_dangerous(&name);
                    self.download_notice(format!("Downloaded {name} to Files › Tarn › Downloads."), warns);
                } else {
                    self.download_notice(format!("{name} could not be downloaded: {}", text(data, "message")), true);
                }
                if let Some((private, profile)) = records {
                    let at = if state == "finished" { now } else { started };
                    self.core.store(private).add_download(profile, &name, &path, &state, at).ok();
                }
            }
            _ => {}
        }
    }

    pub fn transfer_command(&mut self, id: &str, command: &'static str, keep: bool) -> Vec<Effect> {
        let Some(t) = self.transfers.iter().find(|t| t.id == id) else { return Vec::new() };
        if command == "downloadKeep" && keep && !browser_core::prompts::stable(self.transfers_moved, std::time::Instant::now()) {
            return Vec::new();
        }
        let live = |k: &TabKey| self.model.lifecycle.live().any(|l| l == *k);
        let Some(tab) = Some(t.tab).filter(live).or(self.model.active).or_else(|| self.model.lifecycle.live().next()) else { return Vec::new() };
        let args = if command == "downloadKeep" { json!({ "id": id, "keep": keep }) } else { json!({ "id": id }) };
        if command == "downloadCancel" || (command == "downloadKeep" && !keep) {
            if let Some((private, profile)) = t.records {
                self.core.store(private).add_download(profile, &t.name, &t.path, "cancelled", t.started).ok();
            }
            self.transfers.retain(|t| t.id != id);
            browser_core::prompts::restart(&mut self.transfers_moved, std::time::Instant::now());
        } else if command == "downloadKeep"
            && let Some(t) = self.transfers.iter_mut().find(|t| t.id == id)
        {
            t.state = TransferState::Saving;
            browser_core::prompts::restart(&mut self.transfers_moved, std::time::Instant::now());
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
