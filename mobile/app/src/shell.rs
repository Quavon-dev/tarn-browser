use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use browser_core::closed::{Closed, ClosedTab};
use browser_core::command::{ENGINES, Input, classify_input};
use browser_core::prompts::Prompt;
use browser_core::space_card::{SpaceDraft, Unsaved};
use browser_core::split::Splits;
use browser_core::suggest;
use browser_core::tabs::{Model, TabKey};
use browser_core::{Core, site::site_host};
use engine::view_policy::{DEFAULT_LEVEL_KEY, zoom_factor};
use serde_json::{Value, json};
use shield::ShieldLevel;
use store::{Download, HistoryEntry, Id, Profile, SavedWindow, Store};
use url::Url;

use crate::data::Platform;

pub const LIVE_TABS: usize = 6;
pub const SUGGESTIONS: usize = 6;
pub const FILTER_SWITCHES: [(&str, &str, bool); 4] =
    [("block_ads", "Ads", true), ("block_trackers", "Trackers", true), ("block_annoyances", "Cookie banners", true), ("block_malware", "Malware", true)];
pub const BURNER_PREFIX: &str = "burner:";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub back: bool,
    pub forward: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub zoom: f64,
    pub committed: Option<String>,
    pub awaiting_commit: bool,
    pub article: bool,
    /// The address the user last opened in this tab from the address field, until a load of it
    /// commits: that visit counts toward inline completion (`suggest::typed_visit`).
    pub typed: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Load {
    pub url: String,
    pub seq: u64,
    pub opener: Option<String>,
    /// A file the user opened (S6): only this load may reach a `file:` URL.
    pub file: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewProps {
    pub key: TabKey,
    /// The key's wake number; a new view replaces the old one when a closed tab's id is reused.
    pub born: u64,
    pub tab: String,
    pub profile: String,
    pub level: ShieldLevel,
    pub levels: BTreeMap<String, String>,
    pub lists: Vec<String>,
    pub styles: BTreeMap<String, String>,
    pub theme_styles: Vec<(String, String)>,
    pub zaps: BTreeMap<String, Vec<String>>,
    pub sort_downloads: bool,
    pub load: Option<Load>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileProps {
    pub profile: String,
    pub level: ShieldLevel,
    pub levels: BTreeMap<String, String>,
    pub lists: Vec<String>,
    pub styles: BTreeMap<String, String>,
    pub theme_styles: Vec<(String, String)>,
    pub zaps: BTreeMap<String, Vec<String>>,
    pub sort_downloads: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PromptRequest {
    pub tab: TabKey,
    pub prompt: Prompt,
}

impl std::ops::Deref for PromptRequest {
    type Target = Prompt;

    fn deref(&self) -> &Prompt {
        &self.prompt
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Command(TabKey, &'static str, Value),
    Open(String),
    PagePost(String, engine::tarn_ext::PageKind, String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpaceItem {
    pub ix: usize,
    pub name: String,
    pub profile: String,
    pub private: bool,
    pub burner: bool,
}

pub(crate) type IconAsk = ((bool, Id), String);

pub struct Shell<D: Platform> {
    pub core: Core<D>,
    pub model: Model,
    pub splits: Splits<TabKey>,
    /// Whether the layout is wide enough to show a split's panes; a phone shows one tab.
    pub wide: bool,
    pub pages: HashMap<TabKey, Page>,
    pub prompts: Vec<PromptRequest>,
    pub external: Option<(TabKey, String)>,
    pub external_origin: String,
    pub notice: Option<String>,
    /// The tone of the notice with this text; any other notice reads as information.
    pub tone: Option<(String, browser_core::toast::Tone)>,
    pub open_offer: Option<(TabKey, String, String)>,
    pub found: Option<u64>,
    pub undo: Option<(String, u64)>,
    pub transfers: Vec<crate::scripts::Transfer>,
    pub reader: Option<crate::scripts::Reader>,
    pub find_tabs: crate::scripts::FindTabs,
    pub picking: Option<(TabKey, u64)>,
    pub switched_at: Option<i64>,
    pub(crate) scripts: HashMap<u64, (TabKey, crate::scripts::Script)>,
    pub(crate) icon_asks: HashMap<TabKey, HashMap<IconAsk, engine::favicon::Ticket>>,
    pub(crate) shielded: HashMap<TabKey, HashSet<String>>,
    pub(crate) burner_from: Option<(bool, Id)>,
    loads: HashMap<TabKey, Load>,
    pub(crate) visits: HashMap<TabKey, Id>,
    pub(crate) crashed: HashSet<TabKey>,
    pub(crate) seq: u64,
    pub themes: crate::themes::Themes,
    pub ext: crate::extensions::Runtime,
    pub icons: engine::favicon::Tracker<(bool, Id), String>,
    pub(crate) opened: PathBuf,
    /// The file each tab opened with Open File: a slept tab loads it again as Open File.
    opened_files: HashMap<TabKey, String>,
}

fn downloads_sink() -> Rc<dyn Fn(engine::Event)> {
    Rc::new(|_| {})
}

impl<D: Platform> Shell<D> {
    pub fn open(dir: &Path) -> Self {
        let core = Core::open(dir, false);
        let mut model = Model::new(LIVE_TABS);
        model.reload_spaces(&core);
        let ix = if D::private_process() {
            model.private_space(&core).filter(|&ix| model.spaces.get(ix).is_some_and(|s| s.private)).unwrap_or(usize::MAX)
        } else {
            model.launch_space(&core, &SavedWindow::default())
        };
        let themes = crate::themes::Themes::open(&core.disk, dir);
        let opened = dir.join("Opened");
        browser_core::open_file::clear(&opened);
        let mut shell = Shell {
            core,
            model,
            splits: Splits::default(),
            wide: false,
            pages: HashMap::new(),
            prompts: Vec::new(),
            external: None,
            external_origin: String::new(),
            notice: None,
            tone: None,
            open_offer: None,
            found: None,
            undo: None,
            transfers: Vec::new(),
            reader: None,
            find_tabs: crate::scripts::FindTabs::default(),
            picking: None,
            switched_at: None,
            scripts: HashMap::new(),
            icon_asks: HashMap::new(),
            shielded: HashMap::new(),
            burner_from: None,
            loads: HashMap::new(),
            visits: HashMap::new(),
            crashed: HashSet::new(),
            seq: 0,
            themes,
            ext: crate::extensions::Runtime::default(),
            icons: engine::favicon::Tracker::default(),
            opened,
            opened_files: HashMap::new(),
        };
        shell.ext.root = dir.join("Extensions");
        shell.switch_space(ix);
        shell.apply_theme();
        if shell.model.current_space().is_none() {
            shell.toast_as(browser_core::toast::Tone::Warning, "The private space could not be opened.".into());
        }
        shell
    }

    pub fn store(&self) -> &Store {
        self.core.store(self.model.private())
    }

    pub fn profile_id(&self) -> Option<Id> {
        self.model.profile().map(|(_, id)| id)
    }

    pub fn active_prompt(&self) -> Option<&PromptRequest> {
        self.prompts.iter().find(|p| Some(p.tab) == self.model.active)
    }

    pub fn active_page(&self) -> Option<&Page> {
        self.model.active.and_then(|k| self.pages.get(&k))
    }

    pub fn active_url(&self) -> String {
        self.active_page().map(|p| p.url.clone()).filter(|u| !u.is_empty()).or_else(|| self.model.active_tab().map(|t| t.url.clone())).unwrap_or_default()
    }

    pub fn tab_count(&self) -> usize {
        self.model.tabs.len()
    }

    pub fn store_id(&self, private: bool, profile_id: Id) -> Option<String> {
        let id = self.core.profiles.borrow().get(&(private, profile_id)).map(|rt| rt.data.store_id())?;
        let burner = private && self.core.burners.borrow().contains(&profile_id);
        Some(match id.strip_prefix("private:") {
            Some(session) if burner => format!("{BURNER_PREFIX}{session}"),
            _ => id,
        })
    }

    pub fn in_burner(&self) -> bool {
        self.model.burner_shown()
    }

    pub fn records(&self) -> bool {
        self.model.profile().is_some_and(|(private, id)| self.core.keeps_records(private, id))
    }

    /// What every view of the current profile shares, read from its stores.
    pub fn profile_props(&self) -> Option<ProfileProps> {
        let (private, profile_id) = self.model.profile()?;
        let profile = self.store_id(private, profile_id)?;
        let profiles = self.core.profiles.borrow();
        let rt = profiles.get(&(private, profile_id))?;
        let mut levels: BTreeMap<String, String> = rt.overrides.borrow().iter().map(|(h, l)| (h.clone(), l.as_str().to_owned())).collect();
        levels.insert(DEFAULT_LEVEL_KEY.to_owned(), rt.profile.shield_level.as_str().to_owned());
        let store = self.core.store(private);
        let styles = engine::page_scripts::site_styles(store.site_css(profile_id).unwrap_or_default(), store.dark_sites(profile_id).unwrap_or_default())
            .into_iter()
            .collect();
        let mut zaps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if !private {
            for (host, selector) in store.zapped_elements(profile_id).unwrap_or_default() {
                zaps.entry(host).or_default().push(selector);
            }
        }
        Some(ProfileProps {
            profile,
            level: rt.profile.shield_level,
            levels,
            lists: shield::groups::switched_on(|k| self.core.disk.setting(k).ok().flatten()),
            styles,
            theme_styles: self.theme_styles(private, profile_id),
            zaps,
            sort_downloads: self.setting_on(crate::scripts::SORT_DOWNLOADS),
        })
    }

    pub fn views(&self) -> Vec<ViewProps> {
        self.views_from(self.profile_props().as_ref())
    }

    /// The live tabs' views over `props`, which `profile_props` read and the caller may keep until
    /// the profile's data changes.
    pub fn views_from(&self, props: Option<&ProfileProps>) -> Vec<ViewProps> {
        let Some(props) = props else { return Vec::new() };
        let mut keys: Vec<TabKey> = self.model.lifecycle.live().filter(|k| self.model.tab(*k).is_some()).collect();
        keys.sort_by_key(|k| k.id);
        keys.into_iter()
            .map(|key| ViewProps {
                key,
                born: self.model.lifecycle.born(key).unwrap_or_default(),
                tab: format!("{}{}", if key.private { "p" } else { "n" }, key.id),
                profile: props.profile.clone(),
                level: props.level,
                levels: props.levels.clone(),
                lists: props.lists.clone(),
                styles: props.styles.clone(),
                theme_styles: props.theme_styles.clone(),
                zaps: props.zaps.clone(),
                sort_downloads: props.sort_downloads,
                load: self.loads.get(&key).cloned(),
            })
            .collect()
    }

    fn open_current_profile(&self) {
        if let Some((private, id)) = self.model.profile() {
            self.core.open_profile(private, id, downloads_sink);
        }
    }

    pub fn activate(&mut self, key: TabKey) {
        if self.model.tab(key).is_none() {
            return;
        }
        self.model.active = Some(key);
        if self.external.as_ref().is_some_and(|(k, _)| *k != key) {
            self.external = None;
        }
        let partners = self.splits.of(key).filter(|_| self.wide).map(|s| s.others(key)).unwrap_or_default();
        let gone = self.model.lifecycle.activate_with(&partners, key);
        for gone in gone {
            if gone == key || partners.contains(&gone) {
                self.model.lifecycle.keep(gone);
            } else {
                self.unmount(gone);
            }
        }
        self.model.touch(&self.core, key, browser_core::now());
        for pane in partners.into_iter().chain([key]) {
            self.wake(pane);
        }
    }

    /// Follows the layout's width: a wide one wakes the active tab's split, a phone lets it sleep.
    pub fn set_wide(&mut self, wide: bool) {
        if self.wide != wide {
            self.wide = wide;
            if let Some(active) = self.model.active {
                self.activate(active);
            }
        }
    }

    fn wake(&mut self, key: TabKey) {
        let Some(tab) = self.model.tab(key).cloned() else { return };
        if !self.loads.contains_key(&key) && !tab.url.is_empty() {
            self.load_in(key, &tab.url, None);
        }
        self.pages.entry(key).or_insert_with(|| Page { url: tab.url, title: tab.title, zoom: 1.0, ..Page::default() });
    }

    pub(crate) fn load_in(&mut self, key: TabKey, url: &str, opener: Option<String>) {
        self.shielded.remove(&key);
        self.seq += 1;
        let file = self.opened_files.get(&key).is_some_and(|f| f == url);
        self.loads.insert(key, Load { url: url.to_owned(), seq: self.seq, opener, file });
        let page = self.pages.entry(key).or_insert_with(|| Page { zoom: 1.0, ..Page::default() });
        page.url = url.to_owned();
        page.error = None;
        page.loading = true;
        page.awaiting_commit = true;
    }

    pub fn new_tab(&mut self, url: Option<&str>) -> Option<TabKey> {
        if self.in_burner()
            && let Some(key) = self.model.active
        {
            if let Some(url) = url.filter(|u| !u.is_empty()) {
                self.load_in(key, url, None);
            }
            return Some(key);
        }
        let key = self.model.new_tab(&self.core, url.unwrap_or_default(), None)?;
        self.model.reload_space(&self.core);
        self.activate(key);
        Some(key)
    }

    /// Shows a file the user picked, staged by `browser_core::open_file::stage`, in a new tab.
    pub fn open_file(&mut self, url: &Url) {
        let Some(key) = self.new_tab(None) else { return };
        // A burner's one view would keep the read grant of the file it showed before.
        if self.opened_files.insert(key, url.to_string()).is_some() {
            self.unmount(key);
            self.activate(key);
        }
        self.load_in(key, url.as_str(), None);
    }

    /// Where Open File copies a picked file: under this process's data folder, cleared at launch;
    /// a private space's and a burner's copies apart, so closing either deletes them.
    pub fn opened_dir(&self) -> PathBuf {
        if self.in_burner() {
            self.opened.join("burner")
        } else if self.model.private() {
            self.opened.join("private")
        } else {
            self.opened.clone()
        }
    }

    pub fn navigate(&mut self, text: &str) -> Vec<Effect> {
        let engine = &ENGINES[self.search_engine()];
        let input = match classify_input(text, engine) {
            Some(Input::Navigate(url)) => Some(Input::Navigate(self.core.clean_counted_url(self.model.profile(), url))),
            input => input,
        };
        let credit = matches!(input, Some(Input::Navigate(_)));
        match input {
            Some(Input::Navigate(url) | Input::Search(url)) => {
                match self.model.active {
                    Some(key) => self.load_in(key, url.as_str(), None),
                    None => {
                        self.new_tab(Some(url.as_str()));
                    }
                }
                if let Some(page) = self.model.active.filter(|_| credit).and_then(|key| self.pages.get_mut(&key)) {
                    page.typed = Some(url.to_string());
                }
                Vec::new()
            }
            Some(Input::External(url)) => {
                if let Some(key) = self.model.active {
                    self.external = Some((key, url.to_string()));
                    self.external_origin.clear();
                }
                Vec::new()
            }
            Some(Input::Rejected) => {
                self.notice = Some("Tarn does not open this kind of address.".into());
                Vec::new()
            }
            Some(Input::Command(_)) | None => Vec::new(),
        }
    }

    pub fn close_tab(&mut self, key: TabKey) {
        if self.in_burner() {
            self.burn();
            return;
        }
        if self.close_tabs(&[key]).is_some() {
            self.toast("Tab closed".into());
        }
    }

    pub fn discard_new_tab(&mut self, key: TabKey, back: TabKey) {
        if self.model.active != Some(key) || !self.active_url().is_empty() || self.loads.contains_key(&key) || self.model.tab(back).is_none() {
            return;
        }
        self.forget(key);
        self.store().delete_tab(key.id).ok();
        self.model.reload_space(&self.core);
        self.activate(back);
    }

    pub(crate) fn close_tabs(&mut self, keys: &[TabKey]) -> Option<u64> {
        let (private, profile_id) = self.model.profile()?;
        let tabs: Vec<_> = keys.iter().filter_map(|k| self.model.tab(*k).cloned().map(|t| (*k, t))).collect();
        let space_id = tabs.first()?.1.space_id;
        let active = self.model.active.filter(|a| keys.contains(a));
        let neighbour = active.and_then(|a| {
            if let Some(heir) = self.splits.heir(a, keys) {
                return Some(heir);
            }
            let order = self.model.visible_tab_order();
            let at = order.iter().position(|k| *k == a)?;
            order[at + 1..].iter().chain(order[..at].iter().rev()).find(|k| !keys.contains(k)).copied()
        });
        let closed = tabs
            .iter()
            .map(|(key, tab)| ClosedTab {
                url: self.pages.get(key).and_then(|p| p.committed.clone()).unwrap_or(tab.url.clone()),
                title: tab.title.clone(),
                state: None,
                position: tab.position,
                group: None,
                web_engine: None,
            })
            .collect();
        let serial = self.model.closed_tabs.push(Closed { private, profile_id, space_id, tabs: closed, group: None });
        for (key, _) in &tabs {
            self.forget(*key);
            self.store().delete_tab(key.id).ok();
        }
        self.model.reload_space(&self.core);
        if active.is_some() {
            self.model.active = None;
            match neighbour.or_else(|| self.model.pick(|_| false)) {
                Some(next) => self.activate(next),
                None => {
                    self.new_tab(None);
                }
            }
        }
        Some(serial)
    }

    fn unmount(&mut self, key: TabKey) {
        self.drop_scripts(key);
        self.extension_tab_gone(key, false);
        self.model.lifecycle.remove(key);
        self.pages.remove(&key);
        self.loads.remove(&key);
        self.prompts.retain(|p| p.tab != key);
        if self.external.as_ref().is_some_and(|(k, _)| *k == key) {
            self.external = None;
        }
    }

    pub(crate) fn forget(&mut self, key: TabKey) {
        self.opened_files.remove(&key);
        self.splits.leave(key);
        self.unmount(key);
        self.extension_tab_gone(key, true);
        self.shielded.remove(&key);
        if self.reader.as_ref().is_some_and(|r| r.key == key) {
            self.reader = None;
        }
        self.visits.remove(&key);
        self.crashed.remove(&key);
    }

    pub fn reopen_closed(&mut self) -> bool {
        let Some((ix, closed)) = self.model.closed_tabs.pop(&self.model.spaces) else { return false };
        self.restore_closed(ix, closed)
    }

    pub(crate) fn restore_closed(&mut self, ix: usize, closed: Closed) -> bool {
        if ix != self.model.space_ix {
            self.switch_space(ix);
        }
        let Some(id) = closed.restore(&self.core) else { return false };
        self.model.reload_space(&self.core);
        self.activate(TabKey { private: closed.private, id });
        true
    }

    pub fn spaces(&self) -> Vec<SpaceItem> {
        let names: HashMap<(bool, Id), String> = [false, true]
            .into_iter()
            .flat_map(|private| self.core.store(private).profiles().unwrap_or_default().into_iter().map(move |p| ((private, p.id), p.name)))
            .collect();
        self.model
            .spaces
            .iter()
            .enumerate()
            .map(|(ix, s)| SpaceItem {
                ix,
                name: s.space.name.clone(),
                profile: names.get(&(s.private, s.space.profile_id)).cloned().unwrap_or_default(),
                private: s.private,
                burner: s.private && Some(s.space.id) == self.model.burner,
            })
            .filter(|s| s.private || !D::private_process())
            .collect()
    }

    pub fn switch_space(&mut self, ix: usize) {
        let leaving_burner = self.in_burner() && ix != self.model.space_ix;
        if self.model.switch_space(&self.core, ix).is_none() {
            return;
        }
        if leaving_burner {
            self.drop_burner();
        }
        let hidden: Vec<TabKey> = self.model.lifecycle.live().filter(|k| self.model.tab(*k).is_none()).collect();
        for key in hidden {
            self.unmount(key);
        }
        self.open_current_profile();
        if let Some(profile) = self.model.profile() {
            self.load_icons(profile);
        }
        self.ensure_extensions();
        match self.model.pick(|_| false) {
            Some(key) => self.activate(key),
            None => {
                self.new_tab(None);
            }
        }
    }

    pub fn profiles(&self) -> Vec<Profile> {
        self.core.disk.profiles().unwrap_or_default()
    }

    pub fn space_draft(&self, editing: Option<usize>) -> SpaceDraft {
        match editing.and_then(|ix| self.model.spaces.get(ix)) {
            Some(space) => SpaceDraft::edit(space),
            None => SpaceDraft::new(&self.model.spaces, self.model.current_space(), &self.profiles()),
        }
    }

    pub fn save_space(&mut self, draft: &SpaceDraft) -> bool {
        if D::private_process() && !draft.private {
            return false;
        }
        match draft.save(&self.core.disk, self.core.store(draft.private), &self.model.spaces, &self.profiles()) {
            Ok(id) if draft.editing.is_none() => self.show_space(draft.private, id),
            Ok(_) => {
                self.model.reload_spaces(&self.core);
                true
            }
            Err(Unsaved::Invalid) => false,
            Err(Unsaved::Failed(_)) => {
                self.toast_as(browser_core::toast::Tone::Warning, "The space could not be saved.".into());
                false
            }
        }
    }

    fn show_space(&mut self, private: bool, id: Id) -> bool {
        self.model.reload_spaces(&self.core);
        let Some(ix) = self.model.spaces.iter().position(|s| s.private == private && s.space.id == id) else { return false };
        self.switch_space(ix);
        true
    }

    pub fn enter_private(&mut self) {
        if let Some(started) = D::open_private_space() {
            if !started {
                self.toast_as(browser_core::toast::Tone::Warning, "The private space could not be opened.".into());
            }
            return;
        }
        if let Some(ix) = self.model.private_space(&self.core) {
            self.switch_space(ix);
        }
    }

    pub fn close_private(&mut self) {
        if let Some(closed) = D::close_private_space() {
            if !closed {
                self.toast_as(browser_core::toast::Tone::Warning, "The private space could not be closed.".into());
            }
            return;
        }
        if self.in_burner() {
            self.burn();
        }
        browser_core::open_file::clear(&self.opened.join("private"));
        let burner = self.model.burner;
        let private: Vec<(Id, Id)> =
            self.model.spaces.iter().filter(|s| s.private && Some(s.space.id) != burner).map(|s| (s.space.id, s.space.profile_id)).collect();
        let profiles: HashSet<Id> = private.iter().map(|(_, p)| *p).collect();
        let keys: Vec<TabKey> =
            self.model.lifecycle.live().filter(|k| k.private && self.model.profile_of(&self.core, *k).is_none_or(|p| profiles.contains(&p))).collect();
        for key in keys {
            self.forget(key);
        }
        // Private tab ids start again in the next session, so no private pane may outlive this one.
        self.splits.retain(|k| !k.private);
        self.opened_files.retain(|k, _| !k.private);
        for (_, profile_id) in &private {
            self.core.store(true).delete_profile(*profile_id).ok();
            self.forget_icons((true, *profile_id));
            self.core.profiles.borrow_mut().remove(&(true, *profile_id));
            self.model.closed_tabs.clear_profile(true, *profile_id);
        }
        let was_private = self.model.private();
        self.model.reload_spaces(&self.core);
        if was_private || self.model.current_space().is_none() {
            let ix = self.model.spaces.iter().position(|s| !s.private).unwrap_or(0);
            self.switch_space(ix);
        }
    }

    pub fn site_level(&self) -> Option<ShieldLevel> {
        let (private, id) = self.model.profile()?;
        let url = Url::parse(&self.active_url()).ok();
        self.core.level(private, id, url.as_ref())
    }

    pub fn active_host(&self) -> Option<String> {
        Url::parse(&self.active_url()).ok().and_then(|u| site_host(&u).map(str::to_owned))
    }

    pub fn set_site_level(&mut self, level: Option<ShieldLevel>) -> bool {
        let (Some((private, id)), Some(host)) = (self.model.profile(), self.active_host()) else { return false };
        if self.core.store(private).set_site_override(id, &host, level).is_err() {
            return false;
        }
        if let Some(rt) = self.core.profiles.borrow().get(&(private, id)) {
            let mut overrides = rt.overrides.borrow_mut();
            match level {
                Some(level) => overrides.insert(host, level),
                None => overrides.remove(&host),
            };
        }
        if !private {
            self.push_rules(id, std::time::Instant::now());
        }
        true
    }

    pub fn default_level(&self) -> ShieldLevel {
        let Some((private, id)) = self.model.profile() else { return ShieldLevel::Standard };
        self.core.store(private).profile(id).map(|p| p.shield_level).unwrap_or_default()
    }

    pub fn set_default_level(&mut self, level: ShieldLevel) -> bool {
        let Some((private, id)) = self.model.profile() else { return false };
        if self.core.store(private).set_profile_shield_level(id, level).is_err() {
            return false;
        }
        self.core.profiles.borrow_mut().remove(&(private, id));
        self.open_current_profile();
        if !private {
            self.load_extensions(id);
            self.extension_level_changed(id);
        }
        true
    }

    fn engine_key(&self) -> Option<String> {
        let (private, id) = self.model.profile()?;
        Some(browser_core::command::engine_key(private, id))
    }

    pub fn search_engine(&self) -> usize {
        let stored = self.engine_key().and_then(|k| self.core.disk.setting(&k).ok().flatten());
        stored.and_then(|s| ENGINES.iter().position(|e| e.name == s)).unwrap_or(0)
    }

    pub fn set_search_engine(&self, ix: usize) {
        if let (Some(key), Some(engine)) = (self.engine_key(), ENGINES.get(ix)) {
            self.core.disk.set_setting(&key, engine.name).ok();
        }
    }

    pub fn filter_switch(&self, key: &str) -> bool {
        let default = FILTER_SWITCHES.iter().find(|(k, _, _)| *k == key).is_some_and(|(_, _, d)| *d);
        self.core.disk.setting(key).ok().flatten().map_or(default, |v| v == "on")
    }

    pub fn set_filter_switch(&self, key: &str, on: bool) {
        self.core.disk.set_setting(key, if on { "on" } else { "off" }).ok();
    }

    pub fn clear_data(&mut self) {
        let Some((private, id)) = self.model.profile() else { return };
        let store = self.core.store(private);
        store.clear_history(id).ok();
        store.clear_downloads(id).ok();
        self.model.closed_tabs.clear_profile(private, id);
        self.forget_icons((private, id));
        let before = self.store_id(private, id);
        if let Some(rt) = self.core.profiles.borrow_mut().remove(&(private, id)) {
            D::wipe_persistent(&rt.own_store);
        }
        self.open_current_profile();
        if !private {
            self.load_extensions(id);
        }
        if before.is_some() && before == self.store_id(private, id) && before.as_deref().is_some_and(|s| s.starts_with("private:")) {
            self.notice = Some("History cleared. Website data goes when the private space closes.".into());
            return;
        }
        self.notice = Some("Browsing data cleared.".into());
    }

    pub fn history(&self) -> Vec<HistoryEntry> {
        self.profile_id().and_then(|id| self.store().history(id, 200).ok()).unwrap_or_default()
    }

    pub fn downloads(&self) -> Vec<Download> {
        self.profile_id().and_then(|id| self.store().downloads(id).ok()).unwrap_or_default()
    }

    pub fn clear_downloads(&mut self) {
        if let Some(id) = self.profile_id() {
            self.store().clear_downloads(id).ok();
        }
    }

    /// For an address field edited from `typed` to `value` (`suggest::edit`): the history rows
    /// `value` names, and the completion to prefill from the open tabs and the history.
    pub fn suggestions(&self, typed: &str, value: &str) -> (Vec<HistoryEntry>, Option<suggest::Completion>) {
        self.suggest_from(&self.suggestion_history(), typed, value)
    }

    pub fn suggestion_history(&self) -> Vec<HistoryEntry> {
        self.profile_id().and_then(|id| self.store().history(id, 500).ok()).unwrap_or_default()
    }

    pub fn suggest_from(&self, history: &[HistoryEntry], typed: &str, value: &str) -> (Vec<HistoryEntry>, Option<suggest::Completion>) {
        let (rows, completion) = suggest::edit(typed, value, &self.model.tabs, history, SUGGESTIONS, browser_core::now());
        (rows.into_iter().cloned().collect(), completion)
    }

    pub(crate) fn record_visit(&mut self, key: TabKey, url: &str, title: &str, now: i64) {
        let typed = self.pages.get_mut(&key).is_some_and(|p| suggest::typed_visit(&mut p.typed, url));
        let Some(id) = self.profile_id().filter(|_| self.records()) else { return };
        if !matches!(Url::parse(url).map(|u| u.scheme().to_owned()).as_deref(), Ok("http" | "https")) {
            return;
        }
        if let Ok(visit) = self.store().add_history(id, url, title, now, typed) {
            self.visits.insert(key, visit);
        }
    }

    pub(crate) fn retitle(&mut self, key: TabKey, url: &str, title: &str) {
        if let (Some(id), Some(visit)) = (self.profile_id(), self.visits.get(&key).copied()) {
            self.store().retitle_visit(id, visit, url, title).ok();
        }
        self.store().update_tab_page(key.id, url, title).ok();
        if let Some(tab) = self.model.tabs.iter_mut().find(|t| t.id == key.id) {
            tab.url = url.to_owned();
            tab.title = title.to_owned();
        }
    }

    pub(crate) fn new_popup_tab(&mut self, opener: TabKey, url: &str, link: bool) {
        let Some(from) = self.pages.get(&opener).and_then(|p| p.committed.clone()) else {
            self.notice = Some("Tarn did not open a popup from a page that is still loading.".into());
            return;
        };
        if self.in_burner() {
            self.notice = Some("A burner tab does not open pop-ups.".into());
            return;
        }
        let fresh = Url::parse(url).ok().filter(|u| link && browser_core::extensions::store_install(u).is_none());
        if let Some(ix) = fresh.and_then(|u| self.routed_space(&u)) {
            self.switch_space(ix);
            self.new_tab(Some(url));
            return;
        }
        let Some(key) = self.model.new_tab(&self.core, "", None) else { return };
        self.model.reload_space(&self.core);
        self.load_in(key, url, Some(from));
        self.activate(key);
    }

    pub fn active_command(&self, name: &'static str, args: Value) -> Vec<Effect> {
        self.model.active.map(|k| vec![Effect::Command(k, name, args)]).unwrap_or_default()
    }

    pub fn dismiss_error(&mut self) -> Vec<Effect> {
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)) {
            page.error = None;
        }
        Vec::new()
    }

    pub fn zoom(&mut self, factor: f64) -> Vec<Effect> {
        let Some(key) = self.model.active else { return Vec::new() };
        let factor = zoom_factor(factor);
        if let Some(page) = self.pages.get_mut(&key) {
            page.zoom = factor;
        }
        vec![Effect::Command(key, "zoom", json!({ "factor": factor }))]
    }
}
