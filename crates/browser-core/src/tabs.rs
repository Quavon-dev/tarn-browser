//! One window's spaces and tabs: which space it shows, that space's tabs, folders and groups,
//! the active tab, the live tabs' lifecycle and the tabs it closed. The shell keeps the views.

use std::collections::HashSet;

use engine::lifecycle::Lifecycle;
use shield::ShieldLevel;
use store::{Folder, Group, Id, SavedWindow, Space, Tab, TabKind, WebEngine};
use url::Url;

use crate::closed::ClosedTabs;
use crate::{Core, WebsiteData};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TabKey {
    pub private: bool,
    pub id: Id,
}

#[derive(Clone, Debug)]
pub struct SpaceRef {
    pub private: bool,
    pub space: Space,
}

/// A space's tabs, folders and groups as stored.
pub type Lists = (Vec<Tab>, Vec<Folder>, Vec<Group>);

pub struct Model {
    pub spaces: Vec<SpaceRef>,
    pub space_ix: usize,
    pub tabs: Vec<Tab>,
    pub folders: Vec<Folder>,
    pub groups: Vec<Group>,
    /// The open folders of the current space's store.
    pub expanded: HashSet<Id>,
    /// The other store's open folders; private and on-disk folders number their ids apart.
    other_expanded: HashSet<Id>,
    /// Whether `expanded` holds the private store's folders.
    expanded_private: bool,
    pub active: Option<TabKey>,
    pub lifecycle: Lifecycle<TabKey>,
    pub closed_tabs: ClosedTabs,
    /// The saved active tab, picked once the window's space is shown.
    pub(crate) restore_tab: Option<Id>,
    /// The private-store space of this window's burner tab; other windows' burners stay hidden.
    pub burner: Option<Id>,
    burner_profile: Option<Id>,
}

impl Model {
    /// `live_tabs` is how many tabs keep a live view before the least recent one sleeps.
    pub fn new(live_tabs: usize) -> Self {
        Model {
            spaces: Vec::new(),
            space_ix: 0,
            tabs: Vec::new(),
            folders: Vec::new(),
            groups: Vec::new(),
            expanded: HashSet::new(),
            other_expanded: HashSet::new(),
            expanded_private: false,
            active: None,
            lifecycle: Lifecycle::new(live_tabs),
            closed_tabs: ClosedTabs::default(),
            restore_tab: None,
            burner: None,
            burner_profile: None,
        }
    }

    pub fn burner_shown(&self) -> bool {
        self.burner.is_some() && self.current_space().is_some_and(|s| s.private && Some(s.space.id) == self.burner)
    }

    /// The index of a new burner space: its own private-store profile at Strict, so its website
    /// data lives in a store of its own that goes with it.
    pub fn open_burner<D: WebsiteData>(&mut self, core: &Core<D>) -> Option<usize> {
        let store = core.store(true);
        let profile = store.create_profile("Burner", &uuid::Uuid::new_v4().to_string(), true).ok()?;
        let space = store.set_profile_shield_level(profile.id, ShieldLevel::Strict).and_then(|_| store.create_space(profile.id, "Burner", ""));
        let Ok(space) = space else {
            store.delete_profile(profile.id).ok();
            return None;
        };
        core.burners.borrow_mut().insert(profile.id);
        self.burner = Some(space.id);
        self.burner_profile = Some(profile.id);
        self.reload_spaces(core);
        self.spaces.iter().position(|s| s.private && s.space.id == space.id)
    }

    pub fn burner_tabs<D: WebsiteData>(&self, core: &Core<D>) -> Vec<TabKey> {
        let tabs = self.burner.map(|space| core.store(true).tabs(space).unwrap_or_default()).unwrap_or_default();
        tabs.into_iter().map(|t| TabKey { private: true, id: t.id }).collect()
    }

    /// Deletes every row of the burner's profile but the profile itself, drops its runtime with
    /// the data store, and returns the profile. The emptied profile row and its `Core::burners` entry stay until the
    /// last window closes, so its id is never reused by a profile that keeps records while a late
    /// download of the burner still reports it. Nothing of it was ever on disk.
    pub fn burn<D: WebsiteData>(&mut self, core: &Core<D>) -> Option<Id> {
        self.burner = None;
        let profile = self.burner_profile.take()?;
        core.store(true).empty_profile(profile).ok();
        core.profiles.borrow_mut().remove(&(true, profile));
        self.reload_spaces(core);
        Some(profile)
    }

    pub fn current_space(&self) -> Option<&SpaceRef> {
        self.spaces.get(self.space_ix)
    }

    pub fn tabs_showing<D: WebsiteData>(&self, core: &Core<D>, url: &Url) -> Vec<(TabKey, Tab, Option<String>)> {
        let Some(current) = self.current_space() else { return Vec::new() };
        let (private, profile_id) = (current.private, current.space.profile_id);
        let key = |t: &Tab| TabKey { private, id: t.id };
        let mut out: Vec<_> = self.tabs.iter().filter(|t| self.active != Some(key(t)) && same_page(url, &t.url)).map(|t| (key(t), t.clone(), None)).collect();
        for (ix, space) in self.spaces.iter().enumerate() {
            if ix == self.space_ix || space.private != private || space.space.profile_id != profile_id {
                continue;
            }
            let tabs = core.store(private).tabs(space.space.id).unwrap_or_default();
            out.extend(tabs.into_iter().filter(|t| same_page(url, &t.url)).map(|t| (key(&t), t, Some(space.space.name.clone()))));
        }
        out
    }

    pub fn private(&self) -> bool {
        self.current_space().is_some_and(|s| s.private)
    }

    pub fn profile(&self) -> Option<(bool, Id)> {
        self.current_space().map(|s| (s.private, s.space.profile_id))
    }

    pub fn key(&self, tab: &Tab) -> TabKey {
        TabKey { private: self.private(), id: tab.id }
    }

    pub fn tab(&self, key: TabKey) -> Option<&Tab> {
        (key.private == self.private()).then(|| self.tabs.iter().find(|t| t.id == key.id))?
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|k| self.tab(k))
    }

    /// Reads every space of both stores; the current space stays current, or the index points past
    /// the end when it is gone, so nothing reads or slides from a stale entry.
    pub fn reload_spaces<D: WebsiteData>(&mut self, core: &Core<D>) {
        let mut spaces = Vec::new();
        for private in [false, true] {
            let store = core.store(private);
            for profile in store.profiles().unwrap_or_default() {
                let burner = private && core.burners.borrow().contains(&profile.id);
                let spaces_of = store.spaces(profile.id).unwrap_or_default().into_iter().filter(|s| !burner || Some(s.id) == self.burner);
                spaces.extend(spaces_of.map(|space| SpaceRef { private, space }));
            }
        }
        let current = self.current_space().map(|s| (s.private, s.space.id));
        self.spaces = spaces;
        self.space_ix = current.and_then(|c| self.spaces.iter().position(|s| (s.private, s.space.id) == c)).unwrap_or(self.spaces.len());
    }

    pub fn space_lists<D: WebsiteData>(&self, core: &Core<D>, ix: usize) -> Lists {
        let Some(space) = self.spaces.get(ix) else { return Default::default() };
        let store = core.store(space.private);
        let id = space.space.id;
        (store.tabs(id).unwrap_or_default(), store.folders(id).unwrap_or_default(), store.groups(id).unwrap_or_default())
    }

    /// Replaces the current space's lists and returns the old tabs.
    pub fn reload_space<D: WebsiteData>(&mut self, core: &Core<D>) -> Vec<Tab> {
        let (tabs, folders, groups) = self.space_lists(core, self.space_ix);
        self.folders = folders;
        self.groups = groups;
        std::mem::replace(&mut self.tabs, tabs)
    }

    /// Shows the space at `ix`, with no active tab yet, and returns the lists it showed before.
    pub fn switch_space<D: WebsiteData>(&mut self, core: &Core<D>, ix: usize) -> Option<(usize, Lists)> {
        if ix >= self.spaces.len() {
            return None;
        }
        self.active = None;
        let from = std::mem::replace(&mut self.space_ix, ix);
        if self.private() != self.expanded_private {
            std::mem::swap(&mut self.expanded, &mut self.other_expanded);
            self.expanded_private = self.private();
        }
        let old = (std::mem::take(&mut self.tabs), std::mem::take(&mut self.folders), std::mem::take(&mut self.groups));
        self.reload_space(core);
        Some((from, old))
    }

    /// The space a window opens on: its saved one, else the space whose tab was used last.
    pub fn launch_space<D: WebsiteData>(&mut self, core: &Core<D>, saved: &SavedWindow) -> usize {
        let (space, tab) = crate::windows::restore(saved, &core.disk);
        self.restore_tab = tab;
        space.and_then(|id| self.spaces.iter().position(|s| !s.private && s.space.id == id)).unwrap_or_else(|| {
            self.spaces
                .iter()
                .enumerate()
                .filter_map(|(ix, s)| core.disk.tabs(s.space.id).ok()?.into_iter().max_by_key(|t| t.last_active).map(|t| (ix, t.last_active)))
                .max_by_key(|(_, at)| *at)
                .map_or(0, |(ix, _)| ix)
        })
    }

    /// The index of the private space, creating it and its profile when there is none.
    pub fn private_space<D: WebsiteData>(&mut self, core: &Core<D>) -> Option<usize> {
        if let Some(ix) = self.spaces.iter().position(|s| s.private && Some(s.space.id) != self.burner) {
            return Some(ix);
        }
        let uuid = uuid::Uuid::new_v4().to_string();
        let profile = core.store(true).create_profile("Private", &uuid, true).ok()?;
        core.store(true).create_space(profile.id, "Private", "").ok()?;
        self.reload_spaces(core);
        Some(self.spaces.iter().position(|s| s.private && Some(s.space.id) != self.burner).unwrap_or(0))
    }

    /// The current space's tabs in the order the sidebar lists them.
    pub fn visible_tab_order(&self) -> Vec<TabKey> {
        let private = self.private();
        let active = self.active.filter(|k| k.private == private).map(|k| k.id);
        visible_order(&self.tabs, &self.folders, &self.groups, &self.expanded, active).into_iter().map(|id| TabKey { private, id }).collect()
    }

    /// The `n`th visible tab, counted from 1; 9 is the last, as in other browsers.
    pub fn slot(&self, n: usize) -> Option<TabKey> {
        let order = self.visible_tab_order();
        tab_slot(order.len(), n).map(|ix| order[ix])
    }

    pub fn cycled(&self, forward: bool) -> Option<TabKey> {
        let order = self.visible_tab_order();
        let current = order.iter().position(|k| Some(*k) == self.active);
        cycle_index(order.len(), current, forward).map(|ix| order[ix])
    }

    pub fn zone_len(&self, kind: TabKind, folder: Option<Id>) -> usize {
        self.tabs.iter().filter(|t| t.kind == kind && t.folder_id == folder).count()
    }

    /// `tab`'s index in its zone, the index `Store::move_tab` takes; stored positions keep gaps
    /// after a close, so they are not indexes.
    pub fn zone_index(&self, tab: &Tab) -> usize {
        self.tabs.iter().filter(|t| t.kind == tab.kind && t.folder_id == tab.folder_id && t.position < tab.position).count()
    }

    /// The index `Store::move_tab` takes to put `dragged` just before `before`: one less when it
    /// comes from above in the same zone, since the store takes it out of the list first.
    pub fn drop_index(&self, dragged: TabKey, before: &Tab) -> usize {
        let from_above = self.tab(dragged).is_some_and(|t| t.kind == before.kind && t.folder_id == before.folder_id && t.position < before.position);
        self.zone_index(before) - usize::from(from_above)
    }

    /// Where adding `key` to the tiles or taking it out moves it: to the end of the tiles, or back
    /// to the end of the unpinned tabs.
    pub fn tile_toggle(&self, key: TabKey) -> Option<(TabKind, usize)> {
        let kind = if self.tab(key)?.kind == TabKind::Tile { TabKind::Ephemeral } else { TabKind::Tile };
        Some((kind, self.zone_len(kind, None)))
    }

    /// Where pinning or unpinning `key` moves it: the other kind, at the end of its loose tabs.
    pub fn pin_toggle(&self, key: TabKey) -> Option<(TabKind, usize)> {
        let kind = if self.tab(key)?.kind == TabKind::Ephemeral { TabKind::Pinned } else { TabKind::Ephemeral };
        Some((kind, self.zone_len(kind, None)))
    }

    pub fn ephemeral_keys(&self) -> Vec<TabKey> {
        self.tabs.iter().filter(|t| t.kind == TabKind::Ephemeral).map(|t| self.key(t)).collect()
    }

    /// The current space's other tabs.
    pub fn other_tabs(&self, key: TabKey) -> Vec<TabKey> {
        self.tabs.iter().map(|t| self.key(t)).filter(|k| *k != key).collect()
    }

    /// The unpinned tab nearest `key` in the sidebar that is not in `closing`, the previous one first
    /// when `previous_first`.
    pub fn close_neighbour(&self, key: TabKey, closing: &[TabKey], previous_first: bool) -> Option<TabKey> {
        let order: Vec<Id> = self
            .visible_tab_order()
            .into_iter()
            .filter(|k| *k == key || !closing.contains(k))
            .filter(|k| self.tab(*k).is_some_and(|t| t.kind == TabKind::Ephemeral))
            .map(|k| k.id)
            .collect();
        neighbour(&order, key.id, previous_first).map(|id| TabKey { private: key.private, id })
    }

    /// The tab to show when none is active: a live one of this space, else the saved one, else the
    /// most recently used; a tab live in another window stays there until the user picks it here.
    pub fn pick(&mut self, mut live_elsewhere: impl FnMut(TabKey) -> bool) -> Option<TabKey> {
        let private = self.private();
        let live = self.lifecycle.live().find(|k| k.private == private && self.tab(*k).is_some());
        let mut recent: Vec<&Tab> = self.tabs.iter().filter(|t| t.last_active > 0).collect();
        recent.sort_by_key(|t| std::cmp::Reverse(t.last_active));
        let saved = self.restore_tab.take().map(|id| TabKey { private, id }).filter(|k| self.tab(*k).is_some());
        let candidates: Vec<TabKey> = saved.into_iter().chain(recent.into_iter().map(|t| self.key(t))).collect();
        live.or_else(|| candidates.into_iter().find(|k| !live_elsewhere(*k)))
    }

    pub fn new_tab<D: WebsiteData>(&self, core: &Core<D>, url: &str, web_engine: Option<WebEngine>) -> Option<TabKey> {
        let space = self.current_space()?;
        let store = core.store(space.private);
        let tab = store.create_tab(space.space.id, TabKind::Ephemeral, None, url, "").ok()?;
        if let Some(engine) = web_engine {
            store.set_tab_web_engine(tab.id, Some(engine)).ok();
        }
        Some(TabKey { private: space.private, id: tab.id })
    }

    pub fn touch<D: WebsiteData>(&mut self, core: &Core<D>, key: TabKey, now: i64) {
        core.store(key.private).touch_tab(key.id, now).ok();
        if let Some(t) = self.tabs.iter_mut().find(|t| t.id == key.id) {
            t.last_active = now;
        }
    }

    /// Forgets that `ids` of the `private` store are open, as when its folders are deleted.
    pub fn forget_folders(&mut self, private: bool, ids: &[Id]) {
        let set = if private == self.expanded_private { &mut self.expanded } else { &mut self.other_expanded };
        set.retain(|id| !ids.contains(id));
    }

    pub fn toggle_folder(&mut self, id: Id) {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
    }

    /// Creates a folder named after the count, opened along with its parent.
    pub fn new_folder<D: WebsiteData>(&mut self, core: &Core<D>, parent: Option<Id>) -> Option<Folder> {
        let space = self.current_space()?;
        let folder = core.store(space.private).create_folder(space.space.id, parent, &format!("Folder {}", self.folders.len() + 1)).ok()?;
        self.expanded.extend(parent.into_iter().chain([folder.id]));
        Some(folder)
    }

    pub fn move_folder<D: WebsiteData>(&mut self, core: &Core<D>, id: Id, parent: Option<Id>) -> bool {
        let moved = core.store(self.private()).move_folder(id, parent).is_ok();
        if moved {
            self.expanded.extend(parent);
        }
        moved
    }

    pub fn pinned_drift(&self, key: TabKey, current: &str) -> Option<Url> {
        let pinned = self.tab(key).filter(|t| t.kind != TabKind::Ephemeral)?.pinned_url.as_deref()?;
        drifted(pinned, current).then(|| Url::parse(pinned).ok()).flatten()
    }

    /// The tabs inside folder `id` and its subfolders, and the folders themselves.
    pub fn folder_contents(&self, id: Id) -> (Vec<TabKey>, HashSet<Id>) {
        let tree = folder_tree(&self.folders, id);
        let keys = self.tabs.iter().filter(|t| t.folder_id.is_some_and(|f| tree.contains(&f))).map(|t| self.key(t)).collect();
        (keys, tree)
    }

    /// Creates a group named after the count, holding `tab` when it is unpinned.
    pub fn new_group<D: WebsiteData>(&self, core: &Core<D>, tab: Option<TabKey>) -> Option<Group> {
        let space = self.current_space()?;
        let store = core.store(space.private);
        let group = store.create_group(space.space.id, &format!("Group {}", self.groups.len() + 1)).ok()?;
        if let Some(tab) = tab.and_then(|k| self.tab(k)).filter(|t| t.kind == TabKind::Ephemeral) {
            store.set_tab_group(tab.id, Some(group.id)).ok();
        }
        Some(group)
    }

    /// Collapses or expands a group; true when it changed.
    pub fn toggle_group<D: WebsiteData>(&mut self, core: &Core<D>, id: Id) -> bool {
        let store = core.store(self.private());
        let Some(group) = self.groups.iter_mut().find(|g| g.id == id) else { return false };
        let changed = store.update_group(id, &group.name, !group.collapsed).is_ok();
        if changed {
            group.collapsed = !group.collapsed;
        }
        changed
    }

    /// The profile of the space holding `key`, which need not be the current one.
    pub fn profile_of<D: WebsiteData>(&self, core: &Core<D>, key: TabKey) -> Option<Id> {
        let space_id = match self.tab(key) {
            Some(t) => t.space_id,
            None => core.store(key.private).tab(key.id).ok()?.space_id,
        };
        self.spaces.iter().find(|s| s.private == key.private && s.space.id == space_id).map(|s| s.space.profile_id)
    }

    pub fn active_profile<D: WebsiteData>(&self, core: &Core<D>) -> Option<(bool, Id)> {
        let key = self.active?;
        Some((key.private, self.profile_of(core, key)?))
    }

    /// Whether the space at `ix` belongs to the profile, and the side, of the tab.
    pub fn same_profile<D: WebsiteData>(&self, core: &Core<D>, key: TabKey, ix: usize) -> bool {
        self.spaces.get(ix).is_some_and(|s| holds(s, key, self.profile_of(core, key)))
    }

    /// The other spaces the tab can move to.
    pub fn move_targets<D: WebsiteData>(&self, core: &Core<D>, key: TabKey) -> Vec<usize> {
        let profile = self.profile_of(core, key);
        self.spaces.iter().enumerate().filter(|(ix, s)| *ix != self.space_ix && holds(s, key, profile)).map(|(ix, _)| ix).collect()
    }

    /// The last space of a saved profile stays, so the profile remains reachable.
    pub fn can_delete_space(&self, ix: usize) -> bool {
        self.spaces
            .get(ix)
            .filter(|s| !(s.private && Some(s.space.id) == self.burner))
            .is_some_and(|s| s.private || self.spaces.iter().filter(|o| !o.private && o.space.profile_id == s.space.profile_id).count() > 1)
    }

    /// Moves the tab's row to the space at `ix` with the page it shows; false when that space
    /// belongs to another profile.
    pub fn move_to_space<D: WebsiteData>(&self, core: &Core<D>, key: TabKey, ix: usize, url: &str) -> bool {
        let Some(tab) = self.tab(key).filter(|_| self.same_profile(core, key, ix)) else { return false };
        let store = core.store(key.private);
        store.update_tab_page(key.id, url, &tab.title).ok();
        store.move_tab_to_space(key.id, self.spaces[ix].space.id).is_ok()
    }

    /// The index of a space that may be deleted and the tabs deleting it removes.
    pub fn space_to_delete<D: WebsiteData>(&self, core: &Core<D>, private: bool, id: Id) -> Option<(usize, Vec<TabKey>)> {
        let ix = self.spaces.iter().position(|s| s.private == private && s.space.id == id).filter(|ix| self.can_delete_space(*ix))?;
        Some((ix, core.store(private).tabs(id).unwrap_or_default().into_iter().map(|t| TabKey { private, id: t.id }).collect()))
    }
}

fn holds(space: &SpaceRef, key: TabKey, profile: Option<Id>) -> bool {
    space.private == key.private && Some(space.space.profile_id) == profile
}

/// Moves a tab to `engine`; the saved page state belongs to the old engine, so it goes.
pub fn set_web_engine<D: WebsiteData>(core: &Core<D>, key: TabKey, engine: WebEngine) -> Result<(), store::Error> {
    let store = core.store(key.private);
    store.set_tab_web_engine(key.id, Some(engine))?;
    store.set_interaction_state(key.id, None)
}

/// The engine a tab without a live view opens in: its own or the default, but WebKit for a
/// private tab or a page at `level` Lockdown. `level` is asked only for a tab bound for another
/// engine, since finding it parses the URL and every sleeping tab row asks each frame.
pub fn sleeping_engine(tab: &Tab, private: bool, level: impl FnOnce() -> ShieldLevel, default: WebEngine) -> WebEngine {
    match tab.web_engine.unwrap_or(default) {
        WebEngine::WebKit => WebEngine::WebKit,
        _ if private || level() == ShieldLevel::Lockdown => WebEngine::WebKit,
        engine => engine,
    }
}

/// Typed `typed` names the page an open tab shows: same host (ignoring `www.`), port and path.
pub fn same_page(typed: &Url, open: &str) -> bool {
    let Ok(open) = Url::parse(open) else { return false };
    let key = |u: &Url| u.host_str().map(|h| (h.trim_start_matches("www.").to_ascii_lowercase(), u.port(), u.path().trim_end_matches('/').to_owned()));
    key(typed).is_some() && key(typed) == key(&open)
}

pub fn switch_detail(space: Option<&str>) -> String {
    space.map_or_else(|| "Switch to Tab".to_owned(), |name| format!("Switch to Tab · {}", crate::text::bounded_title(name)))
}

/// A pinned page has drifted once the tab shows another host or path; query and fragment do not count.
pub fn drifted(pinned: &str, current: &str) -> bool {
    Url::parse(pinned).is_ok_and(|pinned| !same_page(&pinned, current))
}

/// A group's colour from the client's `palette` of named colours: its pick, or one fixed by its id.
pub fn group_color(group: &Group, palette: &[(&str, u32)]) -> u32 {
    palette.iter().find(|(n, _)| *n == group.color).unwrap_or(&palette[group.id.unsigned_abs() as usize % palette.len()]).1
}

pub fn custom_title(typed: &str, page_title: &str) -> String {
    if typed == crate::text::bounded_title(page_title) { String::new() } else { typed.to_owned() }
}

/// Unpinned tabs showing a page another tab of `tabs` shows too, the fragment ignored. Per page the
/// most recently used tab stays, a pinned tab or tile and then the active tab before any other;
/// pinned tabs and tiles are never picked. `url` is a tab's current page.
pub fn duplicates(tabs: &[Tab], active: Option<Id>, url: impl Fn(&Tab) -> String) -> Vec<Id> {
    let mut order: Vec<&Tab> = tabs.iter().collect();
    order.sort_by_key(|t| std::cmp::Reverse((t.kind != TabKind::Ephemeral, Some(t.id) == active, t.last_active)));
    let mut seen = HashSet::new();
    order
        .into_iter()
        .filter(|t| {
            let url = url(t);
            let page = Url::parse(&url).map_or(url, |mut u| {
                u.set_fragment(None);
                u.into()
            });
            !page.is_empty() && !seen.insert(page)
        })
        .filter(|t| t.kind == TabKind::Ephemeral)
        .map(|t| t.id)
        .collect()
}

/// The narrowest a tile gets before the tile grid starts another row.
pub const TILE_MIN_WIDTH: f32 = 46.;

pub fn tile_grid(width: f32, gap: f32) -> (usize, f32) {
    tile_row(width, gap, usize::MAX)
}

/// Columns and tile width for `count` tiles: fewer tiles than fit a row share its whole width.
pub fn tile_row(width: f32, gap: f32, count: usize) -> (usize, f32) {
    let columns = (((width + gap) / (TILE_MIN_WIDTH + gap)).floor() as usize).min(count).max(1);
    (columns, ((width - (columns - 1) as f32 * gap) / columns as f32).floor().max(0.))
}

/// The tab next to `closed` in `order`, the previous one first when `previous_first` is set.
fn neighbour(order: &[Id], closed: Id, previous_first: bool) -> Option<Id> {
    let ix = order.iter().position(|id| *id == closed)?;
    let (next, previous) = (order.get(ix + 1), ix.checked_sub(1).and_then(|i| order.get(i)));
    if previous_first { previous.or(next) } else { next.or(previous) }.copied()
}

fn tab_slot(len: usize, n: usize) -> Option<usize> {
    if n == 9 { len.checked_sub(1) } else { n.checked_sub(1).filter(|ix| *ix < len) }
}

fn cycle_index(len: usize, current: Option<usize>, forward: bool) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => last,
        (Some(ix), true) => {
            if ix >= last {
                0
            } else {
                ix + 1
            }
        }
        (Some(ix), false) => ix.checked_sub(1).unwrap_or(last),
    })
}

/// A folder and every folder inside it.
pub fn folder_tree(folders: &[Folder], root: Id) -> HashSet<Id> {
    let mut tree = HashSet::from([root]);
    loop {
        let inside: Vec<Id> = folders.iter().filter(|f| !tree.contains(&f.id) && f.parent_id.is_some_and(|p| tree.contains(&p))).map(|f| f.id).collect();
        if inside.is_empty() {
            return tree;
        }
        tree.extend(inside);
    }
}

/// Tiles, loose pinned tabs, folders (each folder's tabs before its subfolders), groups, then
/// ungrouped tabs; a closed folder or a collapsed group contributes only its active tab, as the
/// sidebar shows it.
fn visible_order(tabs: &[Tab], folders: &[Folder], groups: &[Group], expanded: &HashSet<Id>, active: Option<Id>) -> Vec<Id> {
    let pick = |keep: &dyn Fn(&Tab) -> bool| tabs.iter().filter(|t| keep(t)).map(|t| t.id).collect::<Vec<_>>();
    let mut order = pick(&|t| t.kind == TabKind::Tile);
    order.extend(pick(&|t| t.kind == TabKind::Pinned && t.folder_id.is_none()));
    let mut stack: Vec<&Folder> = folders.iter().filter(|f| f.parent_id.is_none()).rev().collect();
    while let Some(f) = stack.pop() {
        if expanded.contains(&f.id) {
            order.extend(pick(&|t| t.folder_id == Some(f.id)));
            stack.extend(folders.iter().filter(|c| c.parent_id == Some(f.id)).rev());
        } else {
            let tree = folder_tree(folders, f.id);
            order.extend(pick(&|t| Some(t.id) == active && t.folder_id.is_some_and(|id| tree.contains(&id))));
        }
    }
    for g in groups {
        order.extend(pick(&|t| t.kind == TabKind::Ephemeral && t.group_id == Some(g.id) && (!g.collapsed || Some(t.id) == active)));
    }
    order.extend(pick(&|t| t.kind == TabKind::Ephemeral && t.group_id.is_none()));
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestCore;
    use store::Store;

    #[test]
    fn visible_order_matches_the_sidebar() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        let s = store.create_space(p.id, "S", "").unwrap().id;
        let tab = |kind, folder| store.create_tab(s, kind, folder, "https://a.test/", "").unwrap().id;
        let loose = tab(TabKind::Ephemeral, None);
        let open = store.create_folder(s, None, "Open").unwrap().id;
        let shut = store.create_folder(s, None, "Shut").unwrap().id;
        let in_open = tab(TabKind::Pinned, Some(open));
        let sub = store.create_folder(s, Some(open), "Sub").unwrap().id;
        let in_sub = tab(TabKind::Pinned, Some(sub));
        let deep = store.create_folder(s, Some(shut), "Deep").unwrap().id;
        let active_in_deep = tab(TabKind::Pinned, Some(deep));
        let (in_shut, active_in_shut) = (tab(TabKind::Pinned, Some(shut)), tab(TabKind::Pinned, Some(shut)));
        let pinned = tab(TabKind::Pinned, None);
        let tile = tab(TabKind::Tile, None);
        let collapsed = store.create_group(s, "Collapsed").unwrap().id;
        let open_group = store.create_group(s, "Open").unwrap().id;
        store.update_group(collapsed, "Collapsed", true).unwrap();
        let (hidden, grouped) = (tab(TabKind::Ephemeral, None), tab(TabKind::Ephemeral, None));
        store.set_tab_group(hidden, Some(collapsed)).unwrap();
        store.set_tab_group(grouped, Some(open_group)).unwrap();
        let expanded = HashSet::from([open, sub, deep]);
        let order = |active| visible_order(&store.tabs(s).unwrap(), &store.folders(s).unwrap(), &store.groups(s).unwrap(), &expanded, active);
        assert_eq!(order(Some(active_in_shut)), [tile, pinned, in_open, in_sub, active_in_shut, grouped, loose]);
        assert_eq!(order(Some(hidden)), [tile, pinned, in_open, in_sub, hidden, grouped, loose]);
        assert_eq!(order(Some(active_in_deep)), [tile, pinned, in_open, in_sub, active_in_deep, grouped, loose]);
        assert!(!order(None).contains(&in_shut));
    }

    #[test]
    fn number_keys_pick_a_slot_and_nine_the_last() {
        assert_eq!(tab_slot(3, 1), Some(0));
        assert_eq!(tab_slot(3, 3), Some(2));
        assert_eq!(tab_slot(3, 4), None);
        assert_eq!(tab_slot(12, 9), Some(11));
        assert_eq!(tab_slot(2, 9), Some(1));
        assert_eq!(tab_slot(0, 9), None);
        assert_eq!(tab_slot(0, 1), None);
    }

    #[test]
    fn cycling_wraps_around() {
        assert_eq!(cycle_index(3, Some(2), true), Some(0));
        assert_eq!(cycle_index(3, Some(0), false), Some(2));
        assert_eq!(cycle_index(3, Some(1), true), Some(2));
        assert_eq!(cycle_index(3, None, true), Some(0));
        assert_eq!(cycle_index(3, None, false), Some(2));
        assert_eq!(cycle_index(1, Some(0), true), Some(0));
        assert_eq!(cycle_index(0, None, true), None);
    }

    #[test]
    fn closing_selects_the_configured_neighbour() {
        assert_eq!(neighbour(&[1, 2, 3], 2, false), Some(3));
        assert_eq!(neighbour(&[1, 2, 3], 2, true), Some(1));
        assert_eq!(neighbour(&[1, 2, 3], 3, false), Some(2));
        assert_eq!(neighbour(&[1, 2, 3], 1, true), Some(2));
        assert_eq!(neighbour(&[1], 1, true), None);
        assert_eq!(neighbour(&[1, 2], 9, false), None);
    }

    fn opened() -> (tempfile::TempDir, TestCore, Model) {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let mut model = Model::new(4);
        model.reload_spaces(&core);
        model.switch_space(&core, 0).unwrap();
        (dir, core, model)
    }

    #[test]
    fn spaces_list_both_sides_and_keep_the_current_one() {
        let (_dir, core, mut model) = opened();
        let p = core.disk.profiles().unwrap()[0].id;
        assert_eq!(model.spaces.len(), 1);
        let work = core.disk.create_space(p, "Work", "").unwrap();
        model.reload_spaces(&core);
        model.switch_space(&core, 1).unwrap();
        assert_eq!(model.current_space().unwrap().space.id, work.id);
        let ix = model.private_space(&core).unwrap();
        assert!(model.spaces[ix].private && model.spaces.len() == 3);
        assert_eq!(model.private_space(&core), Some(ix), "the private space is created once");
        assert_eq!(model.current_space().unwrap().space.id, work.id, "the current space stays current");
        core.disk.delete_space(work.id).unwrap();
        model.reload_spaces(&core);
        assert!(model.current_space().is_none(), "a deleted space is not read from a stale index");
        assert!(model.switch_space(&core, 9).is_none());
    }

    #[test]
    fn switching_spaces_returns_the_old_lists_and_clears_the_active_tab() {
        let (_dir, core, mut model) = opened();
        let p = core.disk.profiles().unwrap()[0].id;
        let home = model.spaces[0].space.id;
        let other = core.disk.create_space(p, "Other", "").unwrap().id;
        let tab = core.disk.create_tab(home, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
        model.reload_spaces(&core);
        model.reload_space(&core);
        model.active = Some(model.key(&tab));
        let (from, (tabs, _, _)) = model.switch_space(&core, 1).unwrap();
        assert_eq!((from, tabs.len()), (0, 1));
        assert!(model.active.is_none() && model.tabs.is_empty());
        assert_eq!(model.current_space().unwrap().space.id, other);
        assert!(model.tab(TabKey { private: false, id: tab.id }).is_none(), "a tab of another space");
    }

    #[test]
    fn a_window_reopens_on_its_saved_place_or_the_latest_used_space() {
        let (_dir, core, mut model) = opened();
        let p = core.disk.profiles().unwrap()[0].id;
        let (a, b) = (model.spaces[0].space.id, core.disk.create_space(p, "B", "").unwrap().id);
        let in_a = core.disk.create_tab(a, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
        let in_b = core.disk.create_tab(b, TabKind::Ephemeral, None, "https://b.test/", "").unwrap();
        core.disk.touch_tab(in_a.id, 10).unwrap();
        core.disk.touch_tab(in_b.id, 20).unwrap();
        model.reload_spaces(&core);
        assert_eq!(model.launch_space(&core, &SavedWindow::default()), 1, "the space used last");
        let saved = SavedWindow { space_id: Some(a), active_tab_id: Some(in_a.id), ..Default::default() };
        assert_eq!(model.launch_space(&core, &saved), 0);
        model.switch_space(&core, 0);
        assert_eq!(model.pick(|_| false), Some(TabKey { private: false, id: in_a.id }), "the saved tab first");
        assert!(model.restore_tab.is_none(), "and only once");
    }

    #[test]
    fn open_folders_belong_to_their_store() {
        let (_dir, core, mut model) = opened();
        let disk = core.disk.create_folder(model.spaces[0].space.id, None, "Disk").unwrap().id;
        let ix = model.private_space(&core).unwrap();
        let private = core.store(true).create_folder(model.spaces[ix].space.id, None, "Private").unwrap().id;
        assert_eq!(disk, private, "both stores number from the start");
        model.toggle_folder(disk);
        model.switch_space(&core, ix).unwrap();
        assert!(!model.expanded.contains(&private), "the private folder with that id stays shut");
        model.switch_space(&core, 0).unwrap();
        assert!(model.expanded.contains(&disk), "the open folder is open again");
    }

    #[test]
    fn dropping_on_a_row_uses_its_index_not_a_position_with_gaps() {
        let (_dir, core, mut model) = opened();
        let s = model.spaces[0].space.id;
        let tab = || core.disk.create_tab(s, TabKind::Ephemeral, None, "https://a.test/", "").unwrap().id;
        let (a, b, c, d) = (tab(), tab(), tab(), tab());
        core.disk.delete_tab(b).unwrap();
        model.reload_space(&core);
        let at = |id| model.tabs.iter().find(|t| t.id == id).map(|t| model.zone_index(t)).unwrap();
        assert_eq!((at(a), at(c), at(d)), (0, 1, 2));
        core.disk.move_tab(c, TabKind::Ephemeral, None, at(c)).unwrap();
        model.reload_space(&core);
        let order: Vec<_> = model.tabs.iter().map(|t| t.id).collect();
        assert_eq!(order, [a, c, d], "dropping a tab on its own row keeps it in place");
    }

    #[test]
    fn a_tab_dropped_before_a_row_lands_just_above_it_from_either_side() {
        let (_dir, core, mut model) = opened();
        let s = model.spaces[0].space.id;
        let tab = || core.disk.create_tab(s, TabKind::Ephemeral, None, "https://a.test/", "").unwrap().id;
        let (a, b, c) = (tab(), tab(), tab());
        model.reload_space(&core);
        let drop = |model: &mut Model, dragged: Id, before: Id| {
            let target = model.tabs.iter().find(|t| t.id == before).unwrap().clone();
            let index = model.drop_index(TabKey { private: false, id: dragged }, &target);
            core.disk.move_tab(dragged, TabKind::Ephemeral, None, index).unwrap();
            model.reload_space(&core);
            model.tabs.iter().map(|t| t.id).collect::<Vec<_>>()
        };
        assert_eq!(drop(&mut model, a, c), [b, a, c], "dragged down");
        assert_eq!(drop(&mut model, c, b), [c, b, a], "dragged up");
    }

    #[test]
    fn picking_prefers_live_then_recent_tabs_not_live_elsewhere() {
        let (_dir, core, mut model) = opened();
        let s = model.spaces[0].space.id;
        let tab = |at| {
            let t = core.disk.create_tab(s, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
            core.disk.touch_tab(t.id, at).unwrap();
            TabKey { private: false, id: t.id }
        };
        let (old, new, never) = (tab(5), tab(9), tab(0));
        model.reload_space(&core);
        assert_eq!(model.pick(|_| false), Some(new));
        assert_eq!(model.pick(|k| k == new), Some(old), "a tab live in another window stays there");
        assert_eq!(model.pick(|_| true), None);
        model.lifecycle.activate(never);
        assert_eq!(model.pick(|_| true), Some(never), "a tab live here wins");
        model.touch(&core, old, 99);
        model.lifecycle.remove(never);
        assert_eq!(model.pick(|_| false), Some(old));
        assert_eq!(core.disk.tab(old.id).unwrap().last_active, 99);
    }

    #[test]
    fn tiles_are_squares_of_one_size_in_as_many_columns_as_fit() {
        use super::tile_grid;
        assert_eq!(tile_grid(244., 8.), (4, 55.));
        assert_eq!(tile_grid(400., 8.), (7, 50.), "a wide sidebar fits more tiles a row");
        assert_eq!(tile_grid(250., 8.), (4, 56.), "the side is whole points so a full row fits");
        assert_eq!(tile_grid(10., 8.), (1, 10.), "a sidebar narrower than one tile stacks them");
    }

    #[test]
    fn fewer_tiles_than_fit_share_the_row() {
        use super::tile_row;
        assert_eq!(tile_row(244., 8., 2), (2, 118.));
        assert_eq!(tile_row(244., 8., 3), (3, 76.));
        assert_eq!(tile_row(244., 8., 4), (4, 55.));
        assert_eq!(tile_row(244., 8., 9), (4, 55.), "more tiles than fit keep the grid's columns");
        assert_eq!(tile_row(244., 8., 0), (1, 244.));
    }

    #[test]
    fn slots_cycles_and_neighbours_follow_the_visible_order() {
        let (_dir, core, mut model) = opened();
        let s = model.spaces[0].space.id;
        let ids: Vec<TabKey> =
            (0..3).map(|_| TabKey { private: false, id: core.disk.create_tab(s, TabKind::Ephemeral, None, "https://a.test/", "").unwrap().id }).collect();
        let pinned = TabKey { private: false, id: core.disk.create_tab(s, TabKind::Pinned, None, "https://p.test/", "").unwrap().id };
        model.reload_space(&core);
        assert_eq!(model.visible_tab_order(), [pinned, ids[0], ids[1], ids[2]]);
        assert_eq!((model.slot(1), model.slot(9)), (Some(pinned), Some(ids[2])));
        model.active = Some(ids[2]);
        assert_eq!(model.cycled(true), Some(pinned));
        assert_eq!(model.close_neighbour(ids[1], &[], false), Some(ids[2]));
        assert_eq!(model.close_neighbour(ids[0], &[], true), Some(ids[1]), "a pinned tab is no neighbour");
        assert_eq!(model.close_neighbour(ids[0], &[ids[0], ids[1]], false), Some(ids[2]), "tabs closing with it are skipped");
        assert_eq!(model.close_neighbour(ids[2], &[ids[1], ids[2]], true), Some(ids[0]));
        assert_eq!(model.close_neighbour(ids[1], &ids, false), None);
        assert_eq!(model.ephemeral_keys(), ids);
        assert_eq!(model.other_tabs(pinned), ids);
        assert_eq!(model.pin_toggle(pinned), Some((TabKind::Ephemeral, 3)));
        assert_eq!(model.pin_toggle(ids[0]), Some((TabKind::Pinned, 1)));
        assert_eq!(model.tile_toggle(pinned), Some((TabKind::Tile, 0)));
        let tile = TabKey { private: false, id: core.disk.create_tab(s, TabKind::Tile, None, "https://t.test/", "").unwrap().id };
        model.reload_space(&core);
        assert_eq!(model.tile_toggle(ids[0]), Some((TabKind::Tile, 1)));
        assert_eq!(model.tile_toggle(tile), Some((TabKind::Ephemeral, 3)));
    }

    #[test]
    fn folders_open_with_their_parent_and_hold_their_subfolders_tabs() {
        let (_dir, core, mut model) = opened();
        let s = model.spaces[0].space.id;
        let outer = model.new_folder(&core, None).unwrap();
        model.reload_space(&core);
        let inner = model.new_folder(&core, Some(outer.id)).unwrap();
        assert_eq!(inner.name, "Folder 2");
        assert!(model.expanded.contains(&outer.id) && model.expanded.contains(&inner.id));
        let tab = core.disk.create_tab(s, TabKind::Pinned, Some(inner.id), "https://a.test/", "").unwrap();
        model.reload_space(&core);
        let (tabs, tree) = model.folder_contents(outer.id);
        assert_eq!(tabs, [TabKey { private: false, id: tab.id }]);
        assert_eq!(tree, HashSet::from([outer.id, inner.id]));
        model.toggle_folder(outer.id);
        assert!(!model.expanded.contains(&outer.id));
        assert!(model.move_folder(&core, inner.id, None));
    }

    #[test]
    fn groups_take_only_unpinned_tabs_and_collapse() {
        let (_dir, core, mut model) = opened();
        let s = model.spaces[0].space.id;
        let loose = core.disk.create_tab(s, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
        let pinned = core.disk.create_tab(s, TabKind::Pinned, None, "https://b.test/", "").unwrap();
        model.reload_space(&core);
        let group = model.new_group(&core, Some(model.key(&loose))).unwrap();
        assert_eq!(group.name, "Group 1");
        model.new_group(&core, Some(model.key(&pinned))).unwrap();
        model.reload_space(&core);
        assert_eq!(model.tabs.iter().filter(|t| t.group_id.is_some()).map(|t| t.id).collect::<Vec<_>>(), [loose.id]);
        assert!(model.toggle_group(&core, group.id));
        assert!(model.groups[0].collapsed && core.disk.groups(s).unwrap()[0].collapsed);
        assert!(!model.toggle_group(&core, 999));
    }

    #[test]
    fn tabs_move_only_between_spaces_of_their_profile_and_last_spaces_stay() {
        let (_dir, core, mut model) = opened();
        let p = core.disk.profiles().unwrap()[0].id;
        let other = core.disk.create_profile("O", "uuid-o", false).unwrap();
        let theirs = core.disk.create_space(other.id, "Theirs", "").unwrap();
        let mine = core.disk.create_space(p, "Mine", "").unwrap();
        model.reload_spaces(&core);
        model.reload_space(&core);
        let home = model.spaces[0].space.id;
        let tab = core.disk.create_tab(home, TabKind::Ephemeral, None, "https://a.test/", "T").unwrap();
        model.reload_space(&core);
        let key = model.key(&tab);
        let ix = |id| model.spaces.iter().position(|s| s.space.id == id).unwrap();
        assert_eq!(model.move_targets(&core, key), [ix(mine.id)]);
        let elsewhere = core.disk.create_tab(theirs.id, TabKind::Ephemeral, None, "https://c.test/", "C").unwrap();
        assert_eq!(model.profile_of(&core, key), Some(p));
        assert_eq!(model.profile_of(&core, TabKey { private: false, id: elsewhere.id }), Some(other.id));
        assert_eq!(model.profile_of(&core, TabKey { private: true, id: tab.id }), None);
        assert!(!model.move_to_space(&core, key, ix(theirs.id), "https://b.test/"));
        assert!(model.move_to_space(&core, key, ix(mine.id), "https://b.test/"));
        let moved = core.disk.tab(tab.id).unwrap();
        assert_eq!((moved.space_id, moved.url.as_str()), (mine.id, "https://b.test/"));
        assert!(model.can_delete_space(ix(mine.id)));
        assert!(!model.can_delete_space(ix(theirs.id)), "the profile's last space");
        assert_eq!(model.space_to_delete(&core, false, mine.id), Some((ix(mine.id), vec![key])));
        assert_eq!(model.space_to_delete(&core, false, theirs.id), None);
    }

    #[test]
    fn typed_urls_match_open_tabs_by_host_port_and_path() {
        let typed = |s: &str| Url::parse(s).unwrap();
        assert!(same_page(&typed("https://example.com/"), "https://example.com/"));
        assert!(same_page(&typed("https://example.com/a"), "http://www.Example.com/a/?q=1#top"));
        assert!(!same_page(&typed("https://example.com/a"), "https://example.com/b"));
        assert!(!same_page(&typed("http://localhost:3000/"), "http://localhost:5173/"));
        assert!(!same_page(&typed("https://example.com/"), "https://other.test/"));
        assert!(!same_page(&typed("about:blank"), "about:blank"));
        assert!(!same_page(&typed("https://example.com/"), "not a url"));
    }

    #[test]
    fn duplicates_keep_the_pinned_then_active_then_latest_tab_per_page() {
        let tab = |id, kind, url: &str, last_active| Tab {
            id,
            space_id: 1,
            folder_id: None,
            kind,
            url: url.into(),
            title: String::new(),
            position: id,
            last_active,
            group_id: None,
            pinned_url: None,
            custom_title: String::new(),
            web_engine: None,
        };
        let tabs = [
            tab(1, TabKind::Ephemeral, "https://a.test/x#top", 5),
            tab(2, TabKind::Ephemeral, "https://a.test/x", 9),
            tab(3, TabKind::Ephemeral, "https://a.test/x#end", 1),
            tab(4, TabKind::Pinned, "https://b.test/", 0),
            tab(5, TabKind::Ephemeral, "https://b.test/", 7),
            tab(6, TabKind::Pinned, "https://b.test/", 0),
            tab(7, TabKind::Ephemeral, "https://a.test/y", 3),
            tab(8, TabKind::Ephemeral, "", 1),
            tab(9, TabKind::Ephemeral, "", 2),
        ];
        let sorted = |active| {
            let mut ids = duplicates(&tabs, active, |t| t.url.clone());
            ids.sort();
            ids
        };
        assert_eq!(sorted(None), [1, 3, 5]);
        assert_eq!(sorted(Some(3)), [1, 2, 5]);
        assert_eq!(duplicates(&tabs, None, |t| if t.id == 2 { "https://c.test/".into() } else { t.url.clone() }).len(), 2);
    }

    #[test]
    fn a_pinned_tab_drifts_from_its_pinned_url_and_a_loose_tab_never() {
        let (_dir, core, mut model) = opened();
        let s = model.spaces[0].space.id;
        let pinned = core.disk.create_tab(s, TabKind::Pinned, None, "https://mail.test/inbox", "").unwrap();
        let loose = core.disk.create_tab(s, TabKind::Ephemeral, None, "https://mail.test/inbox", "").unwrap();
        model.reload_space(&core);
        let (pinned, loose) = (model.key(model.tab(TabKey { private: false, id: pinned.id }).unwrap()), TabKey { private: false, id: loose.id });
        assert_eq!(model.pinned_drift(pinned, "https://mail.test/inbox?page=2").map(String::from), None);
        assert_eq!(model.pinned_drift(pinned, "https://other.test/").map(String::from).as_deref(), Some("https://mail.test/inbox"));
        assert_eq!(model.pinned_drift(loose, "https://other.test/"), None);
    }

    #[test]
    fn a_custom_title_equal_to_the_page_title_is_not_kept() {
        assert_eq!(custom_title("Inbox", "  Inbox "), "");
        assert_eq!(custom_title("Mail", "Inbox"), "Mail");
    }

    #[test]
    fn drift_counts_host_and_path_only() {
        assert!(!drifted("https://github.com/", "https://github.com/?tab=repos#top"));
        assert!(!drifted("https://github.com/a", "https://www.github.com/a/"));
        assert!(drifted("https://github.com/", "https://github.com/settings"));
        assert!(drifted("https://github.com/", "https://gitlab.com/"));
        assert!(!drifted("not a url", "https://a.test/"));
    }

    #[test]
    fn a_switch_sets_the_engine_clears_saved_state_and_survives_relaunch() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let space = core.disk.spaces(core.disk.profiles().unwrap()[0].id).unwrap()[0].id;
        let id = core.disk.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "A").unwrap().id;
        core.disk.set_interaction_state(id, Some(&[7u8][..])).unwrap();
        set_web_engine(&core, TabKey { private: false, id }, WebEngine::Chromium).unwrap();
        drop(core);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.tab(id).unwrap().web_engine, Some(WebEngine::Chromium));
        assert_eq!(store.interaction_state(id).unwrap(), None);
    }

    #[test]
    fn tabs_showing_a_page_skip_the_active_one_and_name_other_spaces() {
        let (_dir, core, mut model) = opened();
        let p = core.disk.profiles().unwrap()[0].id;
        let here = model.new_tab(&core, "https://www.a.test/x/", None).unwrap();
        let other = model.new_tab(&core, "https://a.test/x", None).unwrap();
        model.new_tab(&core, "https://a.test/y", None).unwrap();
        let work = core.disk.create_space(p, "Work", "").unwrap();
        let there = core.disk.create_tab(work.id, TabKind::Ephemeral, None, "https://a.test/x", "").unwrap().id;
        model.reload_spaces(&core);
        model.reload_space(&core);
        model.active = Some(here);
        let url = Url::parse("https://a.test/x").unwrap();
        let found: Vec<(Id, Option<String>)> = model.tabs_showing(&core, &url).into_iter().map(|(k, _, space)| (k.id, space)).collect();
        assert_eq!(found, [(other.id, None), (there, Some("Work".to_owned()))]);
    }

    #[test]
    fn new_tabs_join_the_current_space_with_the_engine_asked_for() {
        let (_dir, core, model) = opened();
        let key = model.new_tab(&core, "https://a.test/", Some(WebEngine::Chromium)).unwrap();
        let tab = core.disk.tab(key.id).unwrap();
        assert_eq!((tab.space_id, tab.kind, tab.web_engine), (model.spaces[0].space.id, TabKind::Ephemeral, Some(WebEngine::Chromium)));
        assert_eq!(core.disk.tab(model.new_tab(&core, "https://b.test/", None).unwrap().id).unwrap().web_engine, None);
        let default = WebEngine::Chromium;
        assert_eq!(sleeping_engine(&tab, false, || ShieldLevel::Standard, WebEngine::WebKit), WebEngine::Chromium);
        assert_eq!(sleeping_engine(&Tab { web_engine: None, ..tab.clone() }, false, || ShieldLevel::Strict, default), default);
        assert_eq!(sleeping_engine(&tab, false, || ShieldLevel::Lockdown, default), WebEngine::WebKit);
        assert_eq!(sleeping_engine(&tab, true, || ShieldLevel::Standard, default), WebEngine::WebKit);
        let webkit = Tab { web_engine: Some(WebEngine::WebKit), ..tab.clone() };
        assert_eq!(sleeping_engine(&webkit, false, || unreachable!("a WebKit tab needs no Shield level"), default), WebEngine::WebKit);
    }

    #[test]
    fn a_burner_is_hidden_from_other_windows_and_burns_without_a_trace() {
        let (_dir, core, mut model) = opened();
        let mut other = Model::new(6);
        other.reload_spaces(&core);
        let spaces = other.spaces.len();
        let ix = model.open_burner(&core).unwrap();
        model.switch_space(&core, ix).unwrap();
        assert!(model.burner_shown());
        let profile = model.current_space().unwrap().space.profile_id;
        assert_eq!(core.store(true).profile(profile).unwrap().shield_level, ShieldLevel::Strict);
        assert!(!core.keeps_records(true, profile) && core.keeps_records(false, profile));
        let key = model.new_tab(&core, "https://a.test/", None).unwrap();
        assert!(key.private);
        core.store(true).set_site_zoom(profile, "a.test", Some(1.5)).unwrap();
        other.reload_spaces(&core);
        assert_eq!(other.spaces.len(), spaces, "another window never lists this burner");
        let private = other.private_space(&core).unwrap();
        assert_ne!(Some(other.spaces[private].space.id), model.burner, "nor takes it for the private space");
        assert_eq!(model.private_space(&core).map(|ix| model.spaces[ix].space.profile_id == profile), Some(false));
        assert!(!model.can_delete_space(ix), "it burns instead");
        core.open_profile(true, profile, || std::rc::Rc::new(|_| {})).unwrap();
        assert_eq!(model.burner_tabs(&core), vec![key]);
        assert_eq!(model.burn(&core), Some(profile));
        assert!(model.burner.is_none() && !model.burner_shown() && model.current_space().is_none());
        assert!(core.store(true).spaces(profile).unwrap().is_empty() && core.store(true).tab(key.id).is_err());
        assert!(!core.profiles.borrow().contains_key(&(true, profile)) && core.store(true).site_zooms(profile).unwrap().is_empty());
        assert!(!core.keeps_records(true, profile), "a late download of the burned profile is still not kept");
        assert!(core.store(true).create_profile("Next", "u", true).unwrap().id != profile, "nor is its id reused");
        assert!(core.disk.history(core.disk.profiles().unwrap()[0].id, 10).unwrap().is_empty());
        assert_eq!(model.burn(&core), None);
    }

    #[test]
    fn a_private_space_writes_only_to_the_private_store() {
        let (_dir, core, mut model) = opened();
        let ix = model.private_space(&core).unwrap();
        model.switch_space(&core, ix).unwrap();
        let disk_tabs =
            |core: &TestCore| core.disk.spaces(core.disk.profiles().unwrap()[0].id).unwrap().iter().map(|s| core.disk.tabs(s.id).unwrap().len()).sum::<usize>();
        let key = model.new_tab(&core, "https://a.test/", None).unwrap();
        assert!(key.private);
        let folder = model.new_folder(&core, None).unwrap();
        let group = model.new_group(&core, Some(key)).unwrap();
        model.touch(&core, key, 7);
        let space = model.current_space().unwrap().space.id;
        let private = core.store(true);
        assert_eq!(private.tab(key.id).unwrap().last_active, 7);
        assert_eq!((private.folders(space).unwrap()[0].id, private.groups(space).unwrap()[0].id), (folder.id, group.id));
        assert_eq!(disk_tabs(&core), 0);
        assert!(core.disk.groups(space).unwrap().is_empty() && core.disk.folders(space).unwrap().is_empty());
    }
}
