pub mod extensions;
pub mod pages;
mod site;
pub mod spaces;
mod split;
pub mod swipe;
pub mod themes;
pub mod tools;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::rc::Rc;

use browser_core::closed::{Closed, ClosedTab};
use browser_core::command::{ENGINES, Input, classify_input, engine_key};
use browser_core::prompts::Prompt;
use browser_core::suggest;
use browser_core::tabs::{Model, TabKey};
use browser_core::{Core, now};
use engine::nav_policy::{self, Decision, Source};
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::{Id, SavedWindow, TabKind};
use url::Url;

use crate::data::Data;
use crate::input;
use crate::webview::policy::DEFAULT_LEVEL_KEY;

pub const LIVE_TABS: usize = 6;
pub const START_URL: &str = "about:blank";
pub const ZOOM_STEPS: [f64; 9] = [0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 2.0];

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub can_back: bool,
    pub can_forward: bool,
    pub loading: bool,
    pub load: Option<(String, u64)>,
    pub level: ShieldLevel,
    pub zoom: f64,
    pub found: Option<u64>,
    pub error: Option<String>,
    pub crashed: bool,
    pub prompts: VecDeque<Prompt>,
    /// The address the user last opened in this tab from the address field, until a load of it
    /// commits: that visit counts toward inline completion (`suggest::typed_visit`).
    pub typed: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    Message(String),
    External { url: String, origin: String },
    Open { key: TabKey, url: String, message: String },
    NewTab { key: TabKey, url: String, message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DownloadItem {
    pub id: String,
    pub name: String,
    pub path: String,
    pub state: String,
    pub message: String,
    pub done: u64,
    pub total: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToastAction {
    Undo(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub text: String,
    pub action: Option<ToastAction>,
    pub serial: u64,
}

pub use pages::{FindRow, ReaderView, Request};

pub struct Browser {
    pub core: Rc<Core<Data>>,
    pub model: Model,
    pub pages: HashMap<TabKey, Page>,
    pub outbox: Vec<(TabKey, &'static str, Value)>,
    pub notice: Option<Notice>,
    pub downloads: Vec<DownloadItem>,
    pub toast: Option<Toast>,
    pub reader: Option<ReaderView>,
    pub articles: HashSet<TabKey>,
    pub requests: HashMap<u64, Request>,
    pub find_rows: Vec<FindRow>,
    pub find_query: String,
    pub counted: HashMap<TabKey, HashSet<String>>,
    pub switched_at: Option<i64>,
    pub processes: HashMap<TabKey, i32>,
    pub samples: crate::memory::Samples,
    pub used: HashMap<TabKey, std::time::Instant>,
    pub playing: HashSet<TabKey>,
    pub swipe: swipe::Swipe,
    pub released: Vec<crate::data::StoreKey>,
    pub download_owners: HashMap<String, Option<Id>>,
    pub cancelled_downloads: Vec<String>,
    pub text_found: suggest::TextMatches,
    pub elsewhere: HashSet<TabKey>,
    /// Tabs this window closed or moved between lists; other windows holding them reload theirs.
    pub forgotten: Vec<TabKey>,
    pub themes: themes::Themes,
    pub ext_prompts: HashMap<u64, extensions::ExtPrompt>,
    ext_prompt_serial: u64,
    /// Every split of this window, for the session only.
    pub splits: browser_core::split::Splits<TabKey>,
    text_rows: usize,
    burner_return: Option<(bool, Id)>,
    window: SavedWindow,
    seq: u64,
    toast_serial: u64,
}

impl Browser {
    pub fn open_core(dir: &Path) -> Rc<Core<Data>> {
        Rc::new(Core::<Data>::open(dir, false))
    }

    pub fn saved_windows(core: &Core<Data>) -> Vec<SavedWindow> {
        let saved = core.disk.windows().unwrap_or_default();
        if saved.is_empty() { vec![Browser::window_row(core, SavedWindow::default())] } else { saved }
    }

    pub fn window_row(core: &Core<Data>, row: SavedWindow) -> SavedWindow {
        core.disk.create_window(&row).unwrap_or(row)
    }

    #[cfg(test)]
    pub fn open(dir: &Path) -> Browser {
        let core = Browser::open_core(dir);
        let row = Browser::saved_windows(&core).remove(0);
        let themes = themes::Themes::open(&core.disk, &dir.join("filters"));
        Browser::new(core, row, HashSet::new(), themes)
    }

    pub fn new(core: Rc<Core<Data>>, window: SavedWindow, elsewhere: HashSet<TabKey>, themes: themes::Themes) -> Browser {
        let mut model = Model::new(LIVE_TABS);
        model.reload_spaces(&core);
        let mut browser = Browser {
            core,
            model,
            pages: HashMap::new(),
            outbox: Vec::new(),
            notice: None,
            downloads: Vec::new(),
            toast: None,
            reader: None,
            articles: HashSet::new(),
            requests: HashMap::new(),
            find_rows: Vec::new(),
            find_query: String::new(),
            counted: HashMap::new(),
            switched_at: None,
            processes: HashMap::new(),
            samples: Default::default(),
            used: HashMap::new(),
            playing: HashSet::new(),
            swipe: Default::default(),
            released: Vec::new(),
            download_owners: HashMap::new(),
            cancelled_downloads: Vec::new(),
            text_found: Default::default(),
            elsewhere,
            forgotten: Vec::new(),
            themes,
            ext_prompts: HashMap::new(),
            ext_prompt_serial: 0,
            splits: Default::default(),
            text_rows: 0,
            burner_return: None,
            window,
            seq: 0,
            toast_serial: 0,
        };
        let ix = browser.model.launch_space(&browser.core, &browser.window);
        browser.switch_space(ix);
        browser.apply_theme();
        browser
    }

    fn profile_of_space(&self) -> Option<(bool, Id)> {
        self.model.profile()
    }

    fn open_profile(&self) {
        if let Some((private, id)) = self.profile_of_space() {
            self.core.open_profile(private, id, || Rc::new(|_| {}));
        }
    }

    pub fn switch_space(&mut self, ix: usize) {
        let burning = self.model.burner_shown();
        self.close_reader();
        if self.model.switch_space(&self.core, ix).is_none() {
            return;
        }
        if burning && !self.model.burner_shown() {
            self.burn();
        }
        self.open_profile();
        let picked = self.model.pick(|k| self.elsewhere.contains(&k));
        match picked {
            Some(key) => self.activate(key),
            None => {
                self.new_tab(START_URL);
            }
        }
        self.save_window();
    }

    pub fn private_space(&mut self) {
        if let Some(ix) = self.model.private_space(&self.core) {
            self.switch_space(ix);
        }
    }

    pub fn new_space(&mut self) {
        let Some((private, profile)) = self.profile_of_space() else { return };
        let n = self.model.spaces.iter().filter(|s| s.private == private && s.space.profile_id == profile).count() + 1;
        let Ok(space) = self.core.store(private).create_space(profile, &format!("Space {n}"), "") else { return };
        self.model.reload_spaces(&self.core);
        if let Some(ix) = self.model.spaces.iter().position(|s| s.private == private && s.space.id == space.id) {
            self.switch_space(ix);
        }
    }

    pub fn new_profile(&mut self) {
        let n = self.core.disk.profiles().map(|p| p.len()).unwrap_or(0) + 1;
        let uuid = uuid::Uuid::new_v4().to_string();
        let Ok(profile) = self.core.disk.create_profile(&format!("Profile {n}"), &uuid, false) else { return };
        let Ok(space) = self.core.disk.create_space(profile.id, "Home", "") else { return };
        self.model.reload_spaces(&self.core);
        if let Some(ix) = self.model.spaces.iter().position(|s| !s.private && s.space.id == space.id) {
            self.switch_space(ix);
        }
    }

    pub fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    pub fn show_toast(&mut self, text: impl Into<String>, action: Option<ToastAction>) {
        self.toast_serial += 1;
        self.toast = Some(Toast { text: text.into(), action, serial: self.toast_serial });
    }

    pub fn expire_toast(&mut self, serial: u64) {
        if self.toast.as_ref().is_some_and(|t| t.serial == serial) {
            self.toast = None;
        }
    }

    pub fn take_toast_action(&mut self) {
        if let Some(ToastAction::Undo(serial)) = self.toast.take().and_then(|t| t.action) {
            self.undo_close(serial);
        }
    }

    pub fn activate(&mut self, key: TabKey) {
        let Some(tab) = self.model.tab(key).cloned() else { return };
        if self.model.active != Some(key) {
            self.close_reader();
        }
        let at = std::time::Instant::now();
        self.used.extend(self.model.active.into_iter().chain([key]).map(|k| (k, at)));
        self.model.active = Some(key);
        self.ext_activated(key);
        self.model.touch(&self.core, key, now());
        self.forget_silent_views();
        let partners = self.split_partners(key);
        let evicted = self.model.lifecycle.activate_with(&partners, key);
        for evicted in evicted {
            // The split's panes stay awake above the live-tab limit while it lasts.
            if evicted == key || partners.contains(&evicted) || self.playing.contains(&evicted) {
                self.model.lifecycle.keep(evicted);
            } else {
                self.unload(evicted);
            }
        }
        for shown in partners.into_iter().chain([key]) {
            if self.pages.get(&shown).is_none_or(|p| p.load.is_none()) {
                let stored = self.model.tab(shown).map(|t| t.url.clone()).unwrap_or_else(|| tab.url.clone());
                let url = self.pages.get(&shown).map(|p| p.url.clone()).filter(|u| !u.is_empty()).unwrap_or(stored);
                self.load(shown, &url);
            }
        }
        self.save_window();
    }

    pub fn new_tab(&mut self, url: &str) -> Option<TabKey> {
        let key = self.model.new_tab(&self.core, url, None)?;
        self.model.reload_space(&self.core);
        self.activate(key);
        Some(key)
    }

    pub fn level_for(&self, key: TabKey, url: &str) -> ShieldLevel {
        let profile = self.model.profile_of(&self.core, key);
        let url = Url::parse(url).ok();
        profile.and_then(|p| self.core.level(key.private, p, url.as_ref())).unwrap_or_default()
    }

    pub fn load(&mut self, key: TabKey, url: &str) {
        let seq = self.next_seq();
        let level = self.level_for(key, url);
        let page = self.pages.entry(key).or_default();
        page.load = Some((url.to_owned(), seq));
        page.loading = true;
        page.level = level;
        page.error = None;
        page.found = None;
        if page.url.is_empty() {
            page.url = url.to_owned();
        }
    }

    pub fn navigate(&mut self, text: &str) {
        let Some((private, profile)) = self.profile_of_space() else { return };
        let engine = &ENGINES[input::engine_index(self.core.store(private), private, profile)];
        let input = match classify_input(text, engine) {
            Some(Input::Navigate(url)) => Some(Input::Navigate(self.core.clean_counted_url(Some((private, profile)), url))),
            input => input,
        };
        let credit = matches!(input, Some(Input::Navigate(_)));
        match input {
            Some(Input::Navigate(url)) if self.active_page().is_none_or(|p| p.url.is_empty() || p.url == START_URL) && self.open_routed(url.as_str()) => {}
            Some(Input::Navigate(url) | Input::Search(url)) => {
                match self.model.active {
                    Some(key) => self.load(key, url.as_str()),
                    None => {
                        self.new_tab(url.as_str());
                    }
                }
                if let Some(key) = self.model.active.filter(|_| credit) {
                    self.pages.entry(key).or_default().typed = Some(url.to_string());
                }
            }
            Some(Input::External(url)) => self.notice = Some(Notice::External { url: url.to_string(), origin: String::new() }),
            Some(Input::Rejected | Input::Command(_)) => self.notice = Some(Notice::Message("Tarn does not open this kind of address.".into())),
            None => {}
        }
    }

    pub fn close_tab(&mut self, key: TabKey) {
        self.close_tabs(vec![key]);
    }

    pub fn close_tabs(&mut self, keys: Vec<TabKey>) {
        let Some((private, profile_id)) = self.profile_of_space() else { return };
        let keys: Vec<TabKey> = keys.into_iter().filter(|k| self.model.tab(*k).is_some()).collect();
        let Some(first) = keys.first().and_then(|k| self.model.tab(*k)).cloned() else { return };
        let burner = self.model.burner_shown();
        let mut next = self.model.active;
        let mut closed = Vec::new();
        for key in &keys {
            let Some(tab) = self.model.tab(*key).cloned() else { continue };
            if next == Some(*key) {
                // A split that keeps two panes keeps the focus too.
                next = self.splits.heir(*key, &keys).or_else(|| self.model.close_neighbour(*key, &keys, false));
            }
            let url = self.pages.get(key).map(|p| p.url.clone()).filter(|u| !u.is_empty()).unwrap_or(tab.url.clone());
            let title = self.pages.get(key).map(|p| p.title.clone()).unwrap_or(tab.title.clone());
            closed.push(ClosedTab { url, title, state: None, position: tab.position, group: None, web_engine: None });
            self.core.store(private).delete_tab(key.id).ok();
            self.ext_tab_closed(*key);
            self.forget_tab(*key);
        }
        if burner && self.model.tabs.len() == keys.len() {
            self.model.reload_space(&self.core);
            return self.leave_burner();
        }
        if !burner {
            let count = closed.len();
            let serial = self.model.closed_tabs.push(Closed { private, profile_id, space_id: first.space_id, tabs: closed, group: None });
            let text = if count == 1 { "Tab closed".to_owned() } else { format!("{count} tabs closed") };
            self.show_toast(text, Some(ToastAction::Undo(serial)));
        }
        self.model.reload_space(&self.core);
        // A tab still open and still the next one stays active, with its Reader.
        self.model.active = self.model.active.filter(|a| next == Some(*a) && self.model.tab(*a).is_some());
        match next.filter(|k| self.model.tab(*k).is_some() && !self.elsewhere.contains(k)).or_else(|| self.model.pick(|k| self.elsewhere.contains(&k))) {
            Some(next) => self.activate(next),
            None => {
                self.new_tab(START_URL);
            }
        }
    }

    fn forget_tab(&mut self, key: TabKey) {
        self.forgotten.push(key);
        self.splits.leave(key);
        self.model.lifecycle.remove(key);
        self.pages.remove(&key);
        self.articles.remove(&key);
        self.counted.remove(&key);
        self.processes.remove(&key);
        self.used.remove(&key);
        if self.reader.as_ref().is_some_and(|r| r.key == key) {
            self.close_reader();
        }
    }

    fn undo_close(&mut self, serial: u64) {
        if self.model.closed_tabs.newest() != Some(serial) {
            return;
        }
        self.reopen();
    }

    pub fn reopen(&mut self) {
        let Some((ix, closed)) = self.model.closed_tabs.pop(&self.model.spaces) else { return };
        let Some(id) = closed.restore(&self.core) else { return };
        if ix != self.model.space_ix {
            self.switch_space(ix);
        }
        self.model.reload_space(&self.core);
        self.activate(TabKey { private: closed.private, id });
    }

    pub fn cycle(&mut self, forward: bool) {
        if let Some(key) = self.model.cycled(forward) {
            self.activate(key);
        }
    }

    pub fn slot(&mut self, n: usize) {
        if let Some(key) = self.model.slot(n) {
            self.activate(key);
        }
    }

    pub fn pin_toggle(&mut self, key: TabKey) {
        let Some((kind, index)) = self.model.pin_toggle(key) else { return };
        self.core.store(key.private).move_tab(key.id, kind, None, index).ok();
        self.model.reload_space(&self.core);
        self.forgotten.push(key);
    }

    /// The current profile's remembered permissions, for Settings › Permissions.
    pub fn permissions(&self) -> Vec<browser_core::permissions::SitePermission> {
        self.profile_of_space().map(|(private, profile)| browser_core::permissions::list(self.core.store(private), profile)).unwrap_or_default()
    }

    pub fn set_permission(&mut self, origin: &str, kind: &str, allow: Option<bool>) {
        if let Some((private, profile)) = self.profile_of_space() {
            self.core.set_permission(private, profile, origin, kind, allow).ok();
        }
    }

    pub fn tile_toggle(&mut self, key: TabKey) {
        let Some((kind, index)) = self.model.tile_toggle(key) else { return };
        self.core.store(key.private).move_tab(key.id, kind, None, index).ok();
        self.model.reload_space(&self.core);
        self.forgotten.push(key);
    }

    pub fn new_folder(&mut self, parent: Option<Id>) {
        self.model.new_folder(&self.core, parent);
        self.model.reload_space(&self.core);
    }

    pub fn rename_folder(&mut self, id: Id, name: &str) {
        if !name.trim().is_empty() && self.core.store(self.model.private()).rename_folder(id, name.trim()).is_ok() {
            self.model.reload_space(&self.core);
        }
    }

    pub fn rename_space(&mut self, name: &str) {
        let Some((private, id)) = self.model.current_space().map(|s| (s.private, s.space.id)) else { return };
        if !name.trim().is_empty() && self.core.store(private).rename_space(id, name.trim()).is_ok() {
            self.model.reload_spaces(&self.core);
        }
    }

    /// Moves `key` to `target`'s place in its zone: before it from below, after it from above.
    pub fn move_tab_to(&mut self, key: TabKey, target: TabKey) {
        let Some(tab) = self.model.tab(target).filter(|_| key != target && key.private == target.private) else { return };
        let (kind, folder, index) = (tab.kind, tab.folder_id, self.model.zone_index(tab));
        if self.model.burner_shown() && kind != TabKind::Ephemeral {
            return;
        }
        if self.core.store(key.private).move_tab(key.id, kind, folder, index).is_ok() {
            self.model.reload_space(&self.core);
        }
    }

    pub fn toggle_folder(&mut self, id: Id) {
        self.model.toggle_folder(id);
    }

    pub fn move_to_folder(&mut self, key: TabKey, folder: Id) {
        let index = self.model.zone_len(TabKind::Pinned, Some(folder));
        if self.core.store(key.private).move_tab(key.id, TabKind::Pinned, Some(folder), index).is_ok() {
            self.model.expanded.insert(folder);
        }
        self.model.reload_space(&self.core);
    }

    pub fn move_to_space(&mut self, key: TabKey, ix: usize) {
        let url = self.pages.get(&key).map(|p| p.url.clone()).unwrap_or_default();
        if self.model.move_to_space(&self.core, key, ix, &url) {
            self.forget_tab(key);
            self.model.reload_space(&self.core);
            if self.model.active.is_some_and(|a| a != key && self.model.tab(a).is_some()) {
                return;
            }
            self.model.active = None;
            match self.model.pick(|k| self.elsewhere.contains(&k)) {
                Some(next) => self.activate(next),
                None => {
                    self.new_tab(START_URL);
                }
            }
        }
    }

    fn command(&mut self, name: &'static str, args: Value) {
        if let Some(key) = self.model.active {
            self.outbox.push((key, name, args));
        }
    }

    pub fn back(&mut self) {
        self.command("back", Value::Null);
    }

    pub fn forward(&mut self) {
        self.command("forward", Value::Null);
    }

    pub fn reload(&mut self) {
        self.command("reload", Value::Null);
    }

    pub fn stop(&mut self) {
        self.command("stop", Value::Null);
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)) {
            page.loading = false;
        }
    }

    /// Steps to the next or previous match of `text`, already searched.
    pub fn find(&mut self, text: &str, backwards: bool) {
        self.command("find", json!({ "text": text, "backwards": backwards }));
    }

    /// Searches a new text; the count of the one before no longer holds.
    pub fn find_typed(&mut self, text: &str) {
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)) {
            page.found = None;
        }
        self.find(text, false);
    }

    pub fn zoom(&mut self, step: i32) {
        let Some(key) = self.model.active else { return };
        let Some(page) = self.pages.get_mut(&key) else { return };
        let current = if page.zoom > 0.0 { page.zoom } else { 1.0 };
        let ix = ZOOM_STEPS.iter().position(|z| (*z - current).abs() < 0.001).unwrap_or(4) as i32;
        let factor = if step == 0 { 1.0 } else { ZOOM_STEPS[(ix + step).clamp(0, ZOOM_STEPS.len() as i32 - 1) as usize] };
        page.zoom = factor;
        let host = Url::parse(&page.url).ok().and_then(|u| browser_core::site::site_host(&u).map(str::to_owned));
        if let (Some(host), Some(profile)) = (host, self.model.profile_of(&self.core, key)) {
            let stored = (factor != 1.0).then_some(factor);
            self.core.store(key.private).set_site_zoom(profile, &host, stored).ok();
        }
        self.outbox.push((key, "zoom", json!({ "factor": factor })));
    }

    pub fn dismiss_error(&mut self) {
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)) {
            page.error = None;
        }
    }

    pub fn active_page(&self) -> Option<&Page> {
        self.model.active.and_then(|k| self.pages.get(&k))
    }

    pub fn site_host(&self) -> Option<String> {
        let page = self.active_page()?;
        let url = Url::parse(&page.url).ok()?;
        browser_core::site::site_host(&url).map(str::to_owned)
    }

    pub fn set_site_level(&mut self, level: Option<ShieldLevel>) {
        let (Some(key), Some(host)) = (self.model.active, self.site_host()) else { return };
        let Some(profile) = self.model.profile_of(&self.core, key) else { return };
        if self.core.store(key.private).set_site_override(profile, &host, level).is_err() {
            return;
        }
        if let Some(rt) = self.core.profiles.borrow().get(&(key.private, profile)) {
            let mut overrides = rt.overrides.borrow_mut();
            match level {
                Some(level) => overrides.insert(host, level),
                None => overrides.remove(&host),
            };
        }
        if !key.private {
            crate::extensions::levels_changed(profile);
        }
        let url = self.pages.get(&key).map(|p| p.url.clone()).unwrap_or_default();
        self.load(key, &url);
    }

    pub fn default_level(&self) -> ShieldLevel {
        let Some((private, id)) = self.profile_of_space() else { return ShieldLevel::Standard };
        self.core.profiles.borrow().get(&(private, id)).map(|rt| rt.profile.shield_level).unwrap_or_default()
    }

    pub fn set_default_level(&mut self, level: ShieldLevel) {
        let Some((private, id)) = self.profile_of_space() else { return };
        if self.core.store(private).set_profile_shield_level(id, level).is_ok()
            && let Some(rt) = self.core.profiles.borrow_mut().get_mut(&(private, id))
        {
            rt.profile.shield_level = level;
        }
        if !private {
            crate::extensions::levels_changed(id);
        }
        if level == ShieldLevel::Lockdown {
            self.notice =
                Some(Notice::Message("Lockdown: JavaScript is off for new loads; this profile's stored website data is deleted when Tarn next starts.".into()));
        }
        let reload: Vec<TabKey> = if level == ShieldLevel::Lockdown {
            self.pages.keys().copied().filter(|k| k.private == private && self.model.profile_of(&self.core, *k) == Some(id)).collect()
        } else {
            self.model.active.into_iter().collect()
        };
        for key in reload {
            let url = self.pages.get(&key).map(|p| p.url.clone()).unwrap_or_default();
            self.load(key, &url);
        }
    }

    pub fn search_engine(&self) -> usize {
        self.profile_of_space().map_or(0, |(p, id)| input::engine_index(self.core.store(p), p, id))
    }

    pub fn set_search_engine(&mut self, ix: usize) {
        let (Some((private, id)), Some(engine)) = (self.profile_of_space(), ENGINES.get(ix)) else { return };
        self.core.store(private).set_setting(&engine_key(private, id), engine.name).ok();
    }

    pub fn setting_on(&self, key: &str) -> bool {
        self.core.disk.setting(key).ok().flatten().is_none_or(|v| v != "off")
    }

    pub fn toggle_setting(&mut self, key: &str) {
        let on = self.setting_on(key);
        self.core.disk.set_setting(key, if on { "off" } else { "on" }).ok();
    }

    pub fn lists(&self) -> Vec<String> {
        shield::groups::switched_on(|k| self.core.disk.setting(k).ok().flatten())
    }

    pub fn clear_history(&mut self) {
        if let Some((private, id)) = self.profile_of_space() {
            self.core.store(private).clear_history(id).ok();
        }
        self.text_found.clear();
    }

    pub fn clear_downloads(&mut self) {
        if let Some((private, id)) = self.profile_of_space() {
            self.core.store(private).clear_downloads(id).ok();
        }
        self.downloads.retain(|d| d.state == "started");
    }

    pub fn store_key(&self) -> Option<crate::data::StoreKey> {
        let (private, id) = self.profile_of_space()?;
        self.core.profiles.borrow().get(&(private, id)).map(|rt| rt.data.key.clone())
    }

    fn take_prompt(&mut self, id: u64) -> Option<(TabKey, Prompt)> {
        let key = self.model.active?;
        let page = self.pages.get_mut(&key)?;
        let at = page.prompts.iter().position(|p| p.id == id)?;
        page.prompts.remove(at).map(|prompt| (key, prompt))
    }

    pub fn answer(&mut self, id: u64, accept: bool, remember: bool, text: &str, user: &str, password: &str) {
        let Some((key, prompt)) = self.take_prompt(id) else { return };
        if prompt.kind == extensions::PROMPT_KIND {
            return self.answer_extension(key, &prompt, accept, remember);
        }
        if remember
            && prompt.rememberable()
            && let Some(profile) = self.model.profile_of(&self.core, key)
        {
            let decision = if accept { "allow" } else { "deny" };
            self.core.store(key.private).set_permission(profile, &prompt.origin, &prompt.kind, decision).ok();
        }
        self.outbox.push((key, "answer", prompt.answer(accept, Some(text.to_owned()), Some(user.to_owned()), Some(password.to_owned()))));
    }

    pub fn on_event(&mut self, key: TabKey, name: &str, data: &Value) {
        let text = |k: &str| data[k].as_str().unwrap_or_default().to_owned();
        match name {
            "committed" | "finished" => self.on_page(key, name == "finished", data),
            "started" | "stopped" => self.pages.entry(key).or_default().loading = name == "started",
            "title" => {
                if let Some(page) = self.pages.get_mut(&key) {
                    page.title = text("title");
                    self.core.store(key.private).set_tab_title(key.id, &page.title).ok();
                }
            }
            "failed" => {
                let page = self.pages.entry(key).or_default();
                page.loading = false;
                suggest::typed_visit(&mut page.typed, &text("url"));
                page.error = Some(text("message"));
            }
            "blocked" if data["popup"].as_bool() == Some(true) => {
                if matches!(self.notice, None | Some(Notice::Message(_))) {
                    let url = Url::parse(&text("url")).ok();
                    let message = nav_policy::popup_blocked_notice(url.as_ref());
                    let offer = url.filter(|u| nav_policy::decide(u, Source::Typed) == Decision::Allow);
                    self.notice = Some(match offer {
                        Some(url) => Notice::NewTab { key, url: url.into(), message },
                        None => Notice::Message(message),
                    });
                }
            }
            "blocked" => {
                // A rebuild reloads the same address, which keeps its typed credit.
                if text("reason") != "rebuild" {
                    let page = self.pages.entry(key).or_default();
                    page.loading = false;
                    suggest::typed_visit(&mut page.typed, &text("url"));
                }
                self.on_blocked(key, &text("url"), &text("reason"));
            }
            "external" => self.notice = Some(Notice::External { url: text("url"), origin: text("origin") }),
            "popup" => {
                if !(data["link"].as_bool() == Some(true) && self.open_routed(&text("url"))) {
                    self.new_tab(&text("url"));
                }
            }
            "article" | "pageText" | "textFound" | "picked" => self.on_script_event(key, name, data),
            "shielded" => self.on_shielded(key, data),
            "process" => {
                if let Some(pid) = data["pid"].as_i64().and_then(|p| i32::try_from(p).ok()) {
                    self.processes.insert(key, pid);
                }
            }
            "prompt" => self.on_prompt(key, data),
            "extensionMessage" => self.on_extension_message(key, data),
            "found" => {
                if let Some(page) = self.pages.get_mut(&key) {
                    page.found = data["matched"].as_u64();
                }
            }
            "crashed" => self.on_crash(key),
            "pressed" if self.in_split(key) && self.model.active != Some(key) => self.activate(key),
            "playing" => {
                if data["playing"].as_bool() == Some(true) {
                    self.playing.insert(key);
                } else {
                    self.playing.remove(&key);
                }
            }
            _ => {}
        }
    }

    fn on_page(&mut self, key: TabKey, finished: bool, data: &Value) {
        let url = data["url"].as_str().unwrap_or_default().to_owned();
        if url.is_empty() {
            return;
        }
        let title = data["title"].as_str().unwrap_or_default().to_owned();
        let profile = self.model.profile_of(&self.core, key);
        let mut dropped = Vec::new();
        let page = self.pages.entry(key).or_default();
        let new_url = page.url != url;
        page.url = url.clone();
        if !title.is_empty() || new_url {
            page.title = title.clone();
        }
        page.can_back = data["canGoBack"].as_bool().unwrap_or(false);
        page.can_forward = data["canGoForward"].as_bool().unwrap_or(false);
        page.loading = !finished;
        if !finished && url != START_URL && matches!(&self.notice, Some(Notice::Open { key: k, .. }) if *k == key) {
            self.notice = None;
        }
        let typed = finished && suggest::typed_visit(&mut page.typed, &url);
        if finished {
            page.crashed = false;
        } else {
            page.error = None;
            let host = Url::parse(&url).ok().and_then(|u| u.host_str().map(|h| h.trim_end_matches('.').to_ascii_lowercase()));
            page.prompts.retain(|p| {
                let keep = p.kind == extensions::PROMPT_KIND && host.as_deref() == Some(p.origin.as_str());
                if !keep && p.kind == extensions::PROMPT_KIND {
                    dropped.push(p.id);
                }
                keep
            });
        }
        let page_title = page.title.clone();
        self.ext_prompts_dropped(dropped);
        if !finished {
            self.ext_navigated(key);
        }
        let store = self.core.store(key.private);
        store.update_tab_page(key.id, &url, &page_title).ok();
        let parsed = Url::parse(&url).ok().filter(|u| matches!(u.scheme(), "http" | "https"));
        let mut visit = None;
        if let (Some(profile), Some(parsed)) = (profile, parsed.as_ref()) {
            if finished {
                if self.core.keeps_records(key.private, profile) {
                    visit = store.add_history(profile, &url, &page_title, now(), typed).ok();
                }
            } else if let Some(zoom) = browser_core::site::site_host(parsed).and_then(|h| store.site_zoom(profile, h).ok().flatten()) {
                self.pages.entry(key).or_default().zoom = zoom;
                self.outbox.push((key, "zoom", json!({ "factor": zoom })));
            }
        }
        if let Some(parsed) = parsed {
            self.after_load(key, finished, &parsed, visit);
        }
    }

    fn on_blocked(&mut self, key: TabKey, url: &str, reason: &str) {
        let message = match reason {
            "rebuild" => return self.load(key, url),
            "https_only" => "This site is set to HTTPS only; the insecure address was blocked.",
            "shield" => "Shield blocked this address.",
            "local_network" => {
                let offer = Url::parse(url).ok().and_then(|u| nav_policy::open_anyway_notice(&u).map(|m| (u, m)));
                self.notice = Some(match offer {
                    Some((u, message)) => Notice::Open { key, url: u.to_string(), message },
                    None => Notice::Message(nav_policy::LOCAL_NETWORK_BLOCKED.to_owned()),
                });
                return;
            }
            _ => "Tarn does not open this kind of address.",
        };
        self.notice = Some(Notice::Message(message.to_owned()));
    }

    pub fn open_anyway(&mut self) {
        let Some(Notice::Open { key, url, .. }) = self.notice.take_if(|n| matches!(n, Notice::Open { .. })) else { return };
        if self.model.tab(key).is_none() {
            return;
        }
        if self.model.active != Some(key) {
            self.activate(key);
        }
        self.load(key, &url);
    }

    pub fn open_blocked_popup(&mut self) {
        if let Some(Notice::NewTab { key, url, .. }) = self.notice.take_if(|n| matches!(n, Notice::NewTab { .. }))
            && self.model.tab(key).is_some()
        {
            self.new_tab(&url);
        }
    }

    fn on_prompt(&mut self, key: TabKey, data: &Value) {
        let Some(prompt) = Prompt::from_event(data) else { return };
        if prompt.rememberable()
            && let Some(profile) = self.model.profile_of(&self.core, key)
            && let Some(decision) = self.core.store(key.private).permission(profile, &prompt.origin, &prompt.kind).ok().flatten()
        {
            let accept = decision == "allow";
            self.outbox.push((key, "answer", json!({ "id": prompt.id, "accept": accept })));
            return;
        }
        self.pages.entry(key).or_default().prompts.push_back(prompt);
    }

    fn on_crash(&mut self, key: TabKey) {
        let page = self.pages.entry(key).or_default();
        if page.crashed {
            page.loading = false;
            page.error = Some("The page crashed again and was not reloaded.".into());
            return;
        }
        page.crashed = true;
        let url = page.url.clone();
        self.load(key, &url);
    }

    pub fn save_window(&mut self) {
        let private = self.model.private();
        let space = self.model.current_space().map(|s| s.space.id);
        let tab = self.model.active.map(|k| k.id);
        let (space_id, active_tab_id) = browser_core::windows::saved_place(private, space, tab);
        if private {
            return;
        }
        let updated = SavedWindow { space_id, active_tab_id, ..self.window.clone() };
        if updated != self.window && self.core.disk.update_window(&updated).is_ok() {
            self.window = updated;
        }
    }

    pub fn window(&self) -> &SavedWindow {
        &self.window
    }

    pub fn set_window_size(&mut self, width: f64, height: f64) {
        let updated = SavedWindow { width, height, ..self.window.clone() };
        if updated != self.window && self.core.disk.update_window(&updated).is_ok() {
            self.window = updated;
        }
    }

    pub fn close_window(&mut self) {
        self.burn();
        self.core.disk.delete_window(self.window.id).ok();
    }

    pub fn give_up(&mut self, key: TabKey) {
        self.splits.leave(key);
        self.model.lifecycle.remove(key);
        self.pages.remove(&key);
        if self.model.active == Some(key) {
            self.model.active = None;
            match self.model.pick(|k| self.elsewhere.contains(&k)) {
                Some(next) => self.activate(next),
                None => {
                    self.new_tab(START_URL);
                }
            }
        }
    }

    pub fn sync_shared(&mut self) {
        let gone: Vec<TabKey> = self.pages.keys().copied().filter(|k| self.core.store(k.private).tab(k.id).is_err()).collect();
        gone.iter().for_each(|k| self.forget_tab(*k));
        let profile = self.model.profile();
        self.model.reload_spaces(&self.core);
        self.model.closed_tabs.retain(&self.model.spaces);
        if self.model.current_space().is_none() {
            let ix = self.model.spaces.iter().position(|s| Some((s.private, s.space.profile_id)) == profile).unwrap_or(0);
            return self.switch_space(ix);
        }
        self.model.reload_space(&self.core);
        if self.model.active.is_some_and(|k| gone.contains(&k)) {
            self.model.active = None;
            match self.model.pick(|k| self.elsewhere.contains(&k)) {
                Some(key) => self.activate(key),
                None => {
                    self.new_tab(START_URL);
                }
            }
        }
    }

    pub fn levels(&self, key: TabKey) -> Value {
        let Some(profile) = self.model.profile_of(&self.core, key) else { return json!({}) };
        let profiles = self.core.profiles.borrow();
        let Some(rt) = profiles.get(&(key.private, profile)) else { return json!({}) };
        let mut map: serde_json::Map<String, Value> = rt.overrides.borrow().iter().map(|(h, l)| (h.clone(), Value::from(l.as_str()))).collect();
        map.insert(DEFAULT_LEVEL_KEY.to_owned(), Value::from(rt.profile.shield_level.as_str()));
        Value::Object(map)
    }

    pub fn store_prop(&self, key: TabKey) -> String {
        let profile = self.model.profile_of(&self.core, key);
        let profiles = self.core.profiles.borrow();
        let burner = profile.is_some_and(|p| key.private && self.core.burners.borrow().contains(&p));
        profile
            .and_then(|p| profiles.get(&(key.private, p)))
            .map(|rt| if burner { rt.data.key.burner().prop() } else { rt.data.key.prop() })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod feature_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod window_tests;
