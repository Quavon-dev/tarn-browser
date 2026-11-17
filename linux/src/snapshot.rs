use std::collections::{HashMap, HashSet};

use browser_core::prompts::Prompt;
use browser_core::split::{Divider, Share};
use browser_core::tabs::TabKey;
use serde_json::Value;
use store::{Folder, Group, Id, Tab, TabKind};

use crate::browser::{Browser, Notice};

#[derive(Clone, Debug, PartialEq)]
pub struct SpaceItem {
    pub ix: usize,
    pub label: String,
    pub current: bool,
    pub icon: String,
    pub color: String,
    pub private: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    Tab(TabKey),
    Folder(Id),
    /// A split's panes side by side, at the place of the first of them in the list.
    Split(TabKey),
    /// A tab group's card: its header and, in `panes`, its tabs; only the active one while collapsed.
    Group(Id),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    pub label: String,
    pub host: String,
    pub depth: usize,
    pub active: bool,
    pub pinned: bool,
    pub open: bool,
    pub burner: bool,
    pub heavy: bool,
    pub asleep: bool,
    pub loading: bool,
    pub playing: bool,
    pub muted: bool,
    /// In a split: `Some(true)` for the split on screen.
    pub split: Option<bool>,
    /// A split row's panes, in layout order.
    pub panes: Vec<Row>,
    /// A group's colour as `0xRRGGBB`.
    pub color: u32,
}

impl Row {
    pub fn id(&self) -> String {
        match &self.kind {
            RowKind::Tab(k) => format!("t{}-{}", k.private, k.id),
            RowKind::Folder(id) => format!("f{id}"),
            RowKind::Split(k) => format!("s{}-{}", k.private, k.id),
            RowKind::Group(id) => format!("g{id}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewItem {
    pub key: TabKey,
    /// The key's wake number; a new view replaces the old one when a closed tab's id is reused.
    pub born: u64,
    pub profile: String,
    pub level: String,
    pub levels: Value,
    pub lists: Vec<String>,
    pub styles: Value,
    pub theme_styles: Value,
    pub zaps: Value,
    pub sort_downloads: bool,
    pub youtube: bool,
    pub load: Option<(String, u64)>,
    pub link: bool,
    pub muted: bool,
    pub active: bool,
    /// Its place in the split on screen.
    pub pane: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pane {
    pub key: TabKey,
    pub title: String,
    pub host: String,
    pub focused: bool,
}

/// The split on screen, while the page area shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitView {
    pub panes: Vec<Pane>,
    pub layout: Vec<Share>,
    pub dividers: Vec<(Divider, Share)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub spaces: Vec<SpaceItem>,
    pub profile: String,
    pub profile_color: String,
    pub profile_name: String,
    pub space_name: String,
    pub space_icon: String,
    /// The current space, private or not; the start card's text belongs to it.
    pub space: Option<(bool, Id)>,
    pub blank: bool,
    pub secure: bool,
    pub private: bool,
    pub rows: Vec<Row>,
    /// The tile grid above the pinned tabs.
    pub tiles: Vec<Row>,
    pub views: Vec<ViewItem>,
    pub split: Option<SplitView>,
    pub url: String,
    pub title: String,
    pub can_back: bool,
    pub can_forward: bool,
    pub loading: bool,
    pub level: String,
    pub overridden: bool,
    pub host: String,
    pub site_total: i64,
    pub zoom: f64,
    pub found: Option<u64>,
    pub error: Option<String>,
    pub prompt: Option<Prompt>,
    pub notice: Option<Notice>,
    pub folders: Vec<(Id, String)>,
    pub toast: Option<crate::browser::Toast>,
    pub reader: Option<crate::browser::ReaderView>,
    pub article: bool,
    pub burner: bool,
    pub find_rows: Vec<crate::browser::FindRow>,
    pub downloading: Option<f32>,
    /// Whether Clean Up has loose tabs to sort.
    pub cleanable: bool,
}

pub fn tab_label(tab: &Tab, b: &Browser, key: TabKey) -> String {
    let page = b.pages.get(&key);
    let title = page.map(|p| p.title.as_str()).filter(|t| !t.is_empty()).unwrap_or(&tab.title);
    let custom = (!tab.custom_title.is_empty()).then_some(tab.custom_title.as_str());
    let url = page.map(|p| p.url.as_str()).filter(|u| !u.is_empty()).unwrap_or(&tab.url);
    let url = if url.is_empty() || url == crate::browser::START_URL { "New Tab" } else { url };
    custom.or((!title.is_empty()).then_some(title)).unwrap_or(url).to_owned()
}

fn space_label(b: &Browser, ix: usize) -> String {
    let s = &b.model.spaces[ix];
    if s.private {
        return if Some(s.space.id) == b.model.burner { "Burner".into() } else { "Private".into() };
    }
    let profile = b.core.disk.profile(s.space.profile_id).map(|p| p.name).unwrap_or_default();
    format!("{} · {}", profile, s.space.name)
}

/// Which tabs a list shows: those in `folder` and `group`, of `kind` when given.
#[derive(Clone, Copy)]
struct Zone {
    folder: Option<Id>,
    group: Option<Id>,
    kind: Option<TabKind>,
}

impl Zone {
    fn of(kind: TabKind) -> Zone {
        Zone { folder: None, group: None, kind: Some(kind) }
    }
}

fn tab_rows(b: &Browser, heavy: &HashSet<TabKey>, zone: Zone, depth: usize, rows: &mut Vec<Row>) {
    let mut list = Vec::new();
    tab_list(b, heavy, zone, depth, &mut list);
    let keys: Vec<TabKey> = list.iter().filter_map(|r| if let RowKind::Tab(k) = r.kind { Some(k) } else { None }).collect();
    let pane = |key| list.iter().find(|r| r.kind == RowKind::Tab(key)).cloned();
    for (row, &key) in list.iter().zip(&keys) {
        match b.splits.entry(key, &keys).as_deref() {
            None => {}
            Some([_]) => rows.push(row.clone()),
            Some(panes) => {
                let panes: Vec<Row> = panes.iter().filter_map(|k| pane(*k)).collect();
                let (active, loading) = (panes.iter().any(|p| p.active), panes.iter().any(|p| p.loading));
                rows.push(Row { kind: RowKind::Split(key), active, loading, panes, ..row.clone() });
            }
        }
    }
}

fn tab_list(b: &Browser, heavy: &HashSet<TabKey>, zone: Zone, depth: usize, rows: &mut Vec<Row>) {
    let mut tabs: Vec<&Tab> =
        b.model.tabs.iter().filter(|t| t.folder_id == zone.folder && t.group_id == zone.group && zone.kind.is_none_or(|k| t.kind == k)).collect();
    tabs.sort_by_key(|t| t.position);
    let muted = if tabs.is_empty() { Vec::new() } else { b.muted_sites() };
    for tab in tabs {
        let key = b.model.key(tab);
        let url = b.pages.get(&key).map(|p| p.url.as_str()).filter(|u| !u.is_empty()).unwrap_or(&tab.url);
        let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
        rows.push(Row {
            kind: RowKind::Tab(key),
            label: tab_label(tab, b, key),
            burner: b.model.burner_shown(),
            heavy: heavy.contains(&key),
            asleep: b.model.active != Some(key) && !b.elsewhere.contains(&key) && !b.model.lifecycle.live().any(|k| k == key),
            loading: b.pages.get(&key).is_some_and(|p| p.loading),
            playing: b.playing.contains(&key),
            muted: b.muted_with(key, &muted),
            split: b.splits.contains(key).then(|| b.in_split(key)),
            host,
            depth,
            active: b.model.active == Some(key),
            pinned: tab.kind == TabKind::Pinned,
            open: false,
            panes: Vec::new(),
            color: 0,
        });
    }
}

fn folder_rows(b: &Browser, heavy: &HashSet<TabKey>, parent: Option<Id>, depth: usize, rows: &mut Vec<Row>) {
    let mut folders: Vec<&Folder> = b.model.folders.iter().filter(|f| f.parent_id == parent).collect();
    folders.sort_by_key(|f| f.position);
    for folder in folders {
        let open = b.model.expanded.contains(&folder.id);
        rows.push(Row {
            kind: RowKind::Folder(folder.id),
            label: folder.name.clone(),
            host: String::new(),
            depth,
            active: false,
            pinned: false,
            open,
            burner: false,
            heavy: false,
            asleep: false,
            loading: false,
            playing: false,
            muted: false,
            split: None,
            panes: Vec::new(),
            color: 0,
        });
        if open {
            folder_rows(b, heavy, Some(folder.id), depth + 1, rows);
            tab_rows(b, heavy, Zone { folder: Some(folder.id), group: None, kind: None }, depth + 1, rows);
        }
    }
}

fn group_rows(b: &Browser, heavy: &HashSet<TabKey>, rows: &mut Vec<Row>) {
    let mut groups: Vec<&Group> = b.model.groups.iter().collect();
    groups.sort_by_key(|g| g.position);
    for group in groups {
        let mut panes = Vec::new();
        tab_rows(b, heavy, Zone { group: Some(group.id), ..Zone::of(TabKind::Ephemeral) }, 0, &mut panes);
        if group.collapsed {
            panes.retain(|r| r.active);
        }
        rows.push(Row {
            kind: RowKind::Group(group.id),
            label: group.name.clone(),
            host: String::new(),
            depth: 0,
            active: false,
            pinned: false,
            open: !group.collapsed,
            burner: false,
            heavy: false,
            asleep: false,
            loading: false,
            playing: false,
            muted: false,
            split: None,
            panes,
            color: browser_core::tabs::group_color(group, &tarn_nsh_shared::theme::PROFILE_COLORS),
        });
    }
}

#[cfg(test)]
pub fn rows(b: &Browser) -> Vec<Row> {
    rows_with(b, &b.heavy_tabs())
}

fn rows_with(b: &Browser, heavy: &HashSet<TabKey>) -> Vec<Row> {
    let mut rows = Vec::new();
    tab_rows(b, heavy, Zone::of(TabKind::Pinned), 0, &mut rows);
    folder_rows(b, heavy, None, 0, &mut rows);
    group_rows(b, heavy, &mut rows);
    tab_rows(b, heavy, Zone::of(TabKind::Ephemeral), 0, &mut rows);
    rows
}

#[cfg(test)]
pub fn tiles(b: &Browser) -> Vec<Row> {
    tiles_with(b, &b.heavy_tabs())
}

fn tiles_with(b: &Browser, heavy: &HashSet<TabKey>) -> Vec<Row> {
    let mut tiles = Vec::new();
    tab_list(b, heavy, Zone::of(TabKind::Tile), 0, &mut tiles);
    tiles
}

pub fn views(b: &Browser, shown: bool) -> Vec<ViewItem> {
    let panes = b.shown_split().map(|s| s.panes().to_vec()).unwrap_or_default();
    let lists = b.lists();
    let sort_downloads = !b.setting_off(crate::browser::pages::SORT_DOWNLOADS);
    let youtube = b.setting_on(shield::groups::YOUTUBE_SETTING);
    let mut keys: Vec<TabKey> = b.model.lifecycle.live().filter(|k| b.model.tab(*k).is_some()).collect();
    keys.sort_by_key(|k| k.id);
    let mut site = HashMap::new();
    let muted = b.muted_sites();
    keys.into_iter()
        .map(|key| {
            let page = b.pages.get(&key);
            let (styles, theme_styles, zaps) =
                site.entry((key.private, b.model.profile_of(&b.core, key))).or_insert_with(|| (b.styles(key), b.theme_styles(key), b.zaps(key))).clone();
            ViewItem {
                key,
                born: b.model.lifecycle.born(key).unwrap_or_default(),
                profile: b.store_prop(key),
                level: page.map(|p| p.level).unwrap_or_default().as_str().to_owned(),
                levels: b.levels(key),
                lists: lists.clone(),
                styles,
                theme_styles,
                zaps,
                sort_downloads,
                youtube,
                load: page.and_then(|p| p.load.clone()),
                link: page.is_some_and(|p| p.link),
                muted: b.muted_with(key, &muted),
                active: shown && (b.model.active == Some(key) || panes.contains(&key)),
                pane: panes.iter().position(|k| *k == key).filter(|_| shown),
            }
        })
        .collect()
}

pub fn snapshot(b: &Browser, web_shown: bool) -> Snapshot {
    let key = b.model.active;
    let heavy = b.heavy_tabs();
    let page = b.active_page().cloned().unwrap_or_default();
    let host = b.site_host().unwrap_or_default();
    let profile_id = b.model.profile().map(|p| p.1);
    let private = b.model.private();
    let profile_row = b.model.current_space().filter(|s| !s.private).and_then(|s| b.core.disk.profile(s.space.profile_id).ok());
    let overridden = profile_id.is_some_and(|p| b.core.has_override(private, p, &host));
    let split = b.shown_split();
    let blank = key.is_some() && split.is_none() && (page.url.is_empty() || page.url == crate::browser::START_URL) && page.error.is_none();
    Snapshot {
        spaces: (0..b.model.spaces.len())
            .map(|ix| {
                let s = &b.model.spaces[ix];
                SpaceItem {
                    ix,
                    label: space_label(b, ix),
                    current: ix == b.model.space_ix,
                    icon: s.space.icon.clone(),
                    color: s.space.color.clone(),
                    private: s.private,
                }
            })
            .collect(),
        profile: b.model.current_space().map(|_| space_label(b, b.model.space_ix)).unwrap_or_default(),
        profile_color: profile_row.as_ref().map(|p| p.color.clone()).unwrap_or_default(),
        profile_name: profile_row.map(|p| p.name).unwrap_or_default(),
        space_name: b.model.current_space().map(|s| if s.private { space_label(b, b.model.space_ix) } else { s.space.name.clone() }).unwrap_or_default(),
        space_icon: b.model.current_space().map(|s| s.space.icon.clone()).unwrap_or_default(),
        space: b.model.current_space().map(|s| (s.private, s.space.id)),
        blank,
        secure: !page.url.starts_with("http:"),
        private,
        rows: rows_with(b, &heavy),
        tiles: tiles_with(b, &heavy),
        views: views(b, web_shown && !blank),
        split: split.filter(|_| web_shown).map(|s| split_view(b, s)),
        url: page.url.clone(),
        title: page.title.clone(),
        can_back: page.can_back,
        can_forward: page.can_forward,
        loading: page.loading,
        level: page.level.as_str().to_owned(),
        overridden,
        site_total: b.model.active_profile(&b.core).zip(url::Url::parse(&page.url).ok()).map_or(0, |((private, p), url)| b.core.site_stopped(private, p, &url)),
        host,
        zoom: if page.zoom > 0.0 { page.zoom } else { 1.0 },
        found: page.found,
        error: page.error.clone(),
        prompt: page.prompts.front().cloned(),
        notice: b.notice.clone(),
        folders: b.model.folders.iter().map(|f| (f.id, f.name.clone())).collect(),
        toast: b.toast.clone(),
        reader: b.reader.clone().filter(|r| Some(r.key) == key),
        article: key.is_some_and(|k| b.articles.contains(&k)),
        burner: b.model.burner_shown(),
        find_rows: b.find_rows.clone(),
        downloading: downloading(b),
        cleanable: browser_core::cleanup::cleanable(&b.model.tabs) && !b.model.burner_shown(),
    }
}

fn split_view(b: &Browser, split: &browser_core::split::Split<TabKey>) -> SplitView {
    let panes = split
        .panes()
        .iter()
        .map(|key| {
            let url = b.pages.get(key).map(|p| p.url.as_str()).filter(|u| !u.is_empty()).or(b.model.tab(*key).map(|t| t.url.as_str())).unwrap_or_default();
            Pane {
                key: *key,
                title: b.model.tab(*key).map(|t| tab_label(t, b, *key)).unwrap_or_default(),
                host: url::Url::parse(url).ok().map(|u| browser_core::command::address(&u).0).unwrap_or_default(),
                focused: b.model.active == Some(*key),
            }
        })
        .collect();
    SplitView { panes, layout: split.layout(), dividers: split.dividers() }
}

fn downloading(b: &Browser) -> Option<f32> {
    let running: Vec<_> = b.downloads.iter().filter(|d| d.state == "started").collect();
    if running.is_empty() {
        return None;
    }
    let (done, total) = running.iter().filter_map(|d| Some((d.done, d.total?))).fold((0u64, 0u64), |(a, b), (d, t)| (a + d, b + t));
    Some(if total > 0 { (done as f32 / total as f32).min(1.0) } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_list_pinned_then_folders_then_tabs_and_views_follow_the_live_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::open(dir.path());
        let pinned = b.model.active.unwrap();
        b.pin_toggle(pinned);
        b.new_folder(None);
        let folder = b.model.folders[0].id;
        let inside = b.new_tab("https://in.test/").unwrap();
        b.move_to_folder(inside, folder);
        let loose = b.new_tab("https://loose.test/").unwrap();
        let rows = rows(&b);
        let kinds: Vec<_> = rows.iter().map(|r| (r.kind.clone(), r.depth)).collect();
        assert_eq!(kinds, [(RowKind::Tab(pinned), 0), (RowKind::Folder(folder), 0), (RowKind::Tab(inside), 1), (RowKind::Tab(loose), 0)]);
        assert_eq!(rows[3].label, "https://loose.test/");
        assert!(rows[3].active);
        b.toggle_folder(folder);
        assert_eq!(super::rows(&b).len(), 3);
        let snap = snapshot(&b, true);
        assert_eq!(snap.views.iter().filter(|v| v.active).count(), 1);
        assert!(snapshot(&b, false).views.iter().all(|v| !v.active));
        assert_eq!(snap.views.len(), 3);
        assert!(snap.views.iter().all(|v| v.levels["*"] == "standard" && v.lists.len() == shield::groups::SETTINGS.len()));
        assert!(snap.views.iter().all(|v| v.youtube), "Block YouTube ads defaults on");
        b.toggle_setting(shield::groups::YOUTUBE_SETTING);
        assert!(snapshot(&b, true).views.iter().all(|v| !v.youtube));
        b.toggle_setting(shield::groups::YOUTUBE_SETTING);
        assert_eq!(snap.space_name, "Home");
        assert!(!snap.blank && snap.secure, "the active tab shows https://loose.test/");
    }

    #[test]
    fn the_snapshot_counts_what_shield_stopped_on_the_site_and_marks_playing_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::open(dir.path());
        let key = b.model.active.unwrap();
        b.on_event(key, "committed", &serde_json::json!({ "url": "https://c.test/" }));
        assert_eq!(snapshot(&b, true).site_total, 0);
        b.on_event(key, "shielded", &serde_json::json!({ "kind": "https_upgrade", "url": "https://c.test/" }));
        b.on_event(key, "playing", &serde_json::json!({ "playing": true }));
        let snap = snapshot(&b, true);
        assert_eq!((snap.host.as_str(), snap.site_total), ("c.test", 1));
        assert!(snap.rows.iter().any(|r| r.kind == RowKind::Tab(key) && r.playing));
        assert!(!snap.profile_name.is_empty());
    }
}
