use std::time::{Duration, Instant};

use store::{Folder, Id, Tab, TabKind};

use crate::split::Splits;
use crate::tabs::{Model, TabKey, folder_tree};
use crate::{Core, WebsiteData};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowKind {
    Tab(TabKey),
    Split(Vec<TabKey>),
    Folder(Id),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Row {
    pub kind: RowKind,
    pub depth: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sidebar {
    pub tiles: Vec<TabKey>,
    pub pinned: Vec<Vec<Row>>,
    pub groups: Vec<(Id, Vec<Row>)>,
    pub loose: Vec<Row>,
}

fn sorted(model: &Model, keep: impl Fn(&Tab) -> bool) -> Vec<&Tab> {
    let mut tabs: Vec<&Tab> = model.tabs.iter().filter(|t| keep(t)).collect();
    tabs.sort_by_key(|t| t.position);
    tabs
}

fn tab_rows(model: &Model, splits: &Splits<TabKey>, tabs: &[&Tab], depth: usize) -> Vec<Row> {
    let keys: Vec<TabKey> = tabs.iter().map(|t| model.key(t)).collect();
    keys.iter()
        .filter_map(|key| {
            let kind = match splits.entry(*key, &keys)?.as_slice() {
                [one] => RowKind::Tab(*one),
                panes => RowKind::Split(panes.to_vec()),
            };
            Some(Row { kind, depth })
        })
        .collect()
}

fn folder_rows(model: &Model, splits: &Splits<TabKey>, folder: &Folder, depth: usize, rows: &mut Vec<Row>) {
    rows.push(Row { kind: RowKind::Folder(folder.id), depth });
    if !model.expanded.contains(&folder.id) {
        let tree = folder_tree(&model.folders, folder.id);
        let active = sorted(model, |t| t.folder_id.is_some_and(|f| tree.contains(&f)) && model.active == Some(model.key(t)));
        rows.extend(tab_rows(model, splits, &active, depth + 1));
        return;
    }
    let tabs = sorted(model, |t| t.kind == TabKind::Pinned && t.folder_id == Some(folder.id));
    rows.extend(tab_rows(model, splits, &tabs, depth + 1));
    for child in children(model, Some(folder.id)) {
        folder_rows(model, splits, child, depth + 1, rows);
    }
}

fn children(model: &Model, parent: Option<Id>) -> Vec<&Folder> {
    let mut folders: Vec<&Folder> = model.folders.iter().filter(|f| f.parent_id == parent).collect();
    folders.sort_by_key(|f| f.position);
    folders
}

pub fn sidebar(model: &Model, splits: &Splits<TabKey>) -> Sidebar {
    let tiles = sorted(model, |t| t.kind == TabKind::Tile).into_iter().map(|t| model.key(t)).collect();
    let mut pinned: Vec<Vec<Row>> =
        tab_rows(model, splits, &sorted(model, |t| t.kind == TabKind::Pinned && t.folder_id.is_none()), 0).into_iter().map(|r| vec![r]).collect();
    for folder in children(model, None) {
        let mut block = Vec::new();
        folder_rows(model, splits, folder, 0, &mut block);
        pinned.push(block);
    }
    let groups = model
        .groups
        .iter()
        .map(|g| {
            let tabs = sorted(model, |t| t.kind == TabKind::Ephemeral && t.group_id == Some(g.id) && (!g.collapsed || model.active == Some(model.key(t))));
            (g.id, tab_rows(model, splits, &tabs, 0))
        })
        .collect();
    let loose = tab_rows(model, splits, &sorted(model, |t| t.kind == TabKind::Ephemeral && t.group_id.is_none()), 0);
    Sidebar { tiles, pinned, groups, loose }
}

pub fn cleanable(model: &Model) -> bool {
    !model.burner_shown() && model.tabs.iter().filter(|t| t.kind == TabKind::Ephemeral && t.group_id.is_none()).count() >= 2
}

pub fn move_tab<D: WebsiteData>(model: &mut Model, core: &Core<D>, key: TabKey, kind: TabKind, folder: Option<Id>, index: usize) -> bool {
    if model.burner_shown() && kind != TabKind::Ephemeral {
        return false;
    }
    let moved = core.store(key.private).move_tab(key.id, kind, folder, index).is_ok();
    if moved {
        model.reload_space(core);
    }
    moved
}

pub fn toggle_pin<D: WebsiteData>(model: &mut Model, core: &Core<D>, key: TabKey) -> bool {
    model.pin_toggle(key).is_some_and(|(kind, end)| move_tab(model, core, key, kind, None, end))
}

pub fn toggle_tile<D: WebsiteData>(model: &mut Model, core: &Core<D>, key: TabKey) -> bool {
    model.tile_toggle(key).is_some_and(|(kind, end)| move_tab(model, core, key, kind, None, end))
}

pub fn new_folder<D: WebsiteData>(model: &mut Model, core: &Core<D>, parent: Option<Id>) -> Option<Id> {
    let folder = model.new_folder(core, parent)?;
    model.reload_space(core);
    Some(folder.id)
}

pub fn delete_folder<D: WebsiteData>(model: &mut Model, core: &Core<D>, id: Id) -> bool {
    let tree = folder_tree(&model.folders, id);
    let deleted = core.store(model.private()).delete_folder(id).is_ok();
    if deleted {
        model.expanded.retain(|f| !tree.contains(f));
        model.reload_space(core);
    }
    deleted
}

pub fn new_group<D: WebsiteData>(model: &mut Model, core: &Core<D>, tab: Option<TabKey>) -> Option<Id> {
    let group = model.new_group(core, tab)?;
    model.reload_space(core);
    Some(group.id)
}

pub fn set_group<D: WebsiteData>(model: &mut Model, core: &Core<D>, key: TabKey, group: Option<Id>) -> bool {
    let set = model.tab(key).is_some_and(|t| t.kind == TabKind::Ephemeral) && core.store(key.private).set_tab_group(key.id, group).is_ok();
    if set {
        model.reload_space(core);
    }
    set
}

pub fn group_tabs(model: &Model, id: Id) -> Vec<TabKey> {
    model.tabs.iter().filter(|t| t.group_id == Some(id)).map(|t| model.key(t)).collect()
}

pub fn remove_group<D: WebsiteData>(model: &mut Model, core: &Core<D>, id: Id, keep_tabs: bool) -> bool {
    let private = model.private();
    let store = core.store(private);
    let removed = if keep_tabs { store.ungroup(id) } else { store.delete_group(id) }.is_ok();
    if removed {
        model.closed_tabs.forget_group(private, id);
        model.reload_space(core);
    }
    removed
}

pub fn clean_up<D: WebsiteData>(model: &mut Model, core: &Core<D>, url: impl Fn(&Tab) -> String) -> usize {
    let Some(space_id) = model.current_space().filter(|_| !model.burner_shown()).map(|s| s.space.id) else { return 0 };
    let plan = crate::cleanup::plan(&model.tabs, &model.groups, url);
    let sorted = crate::cleanup::apply(core.store(model.private()), space_id, plan);
    if sorted > 0 {
        model.reload_space(core);
    }
    sorted
}

pub fn clean_up_text(sorted: usize) -> String {
    match sorted {
        0 => "Already tidy".to_owned(),
        1 => "Sorted 1 tab into groups".to_owned(),
        n => format!("Sorted {n} tabs into groups"),
    }
}

pub fn tabs_below(model: &Model, key: TabKey) -> Vec<TabKey> {
    model.ephemeral_keys().into_iter().skip_while(|k| *k != key).skip(1).collect()
}

pub fn other_unpinned(model: &Model, key: TabKey) -> Vec<TabKey> {
    model.ephemeral_keys().into_iter().filter(|k| *k != key).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
}

const SWIPE_THRESHOLD: f32 = 60.;
const SWIPE_GAP: Duration = Duration::from_millis(200);

#[derive(Clone, Copy, Debug, Default)]
pub struct Swipe {
    dx: f32,
    dy: f32,
    armed: bool,
    last: Option<Instant>,
}

impl Swipe {
    pub fn feed(&mut self, dx: f32, dy: f32, at: Instant) -> Option<Step> {
        if self.last.is_none_or(|last| at.saturating_duration_since(last) > SWIPE_GAP) {
            *self = Swipe { armed: true, ..Swipe::default() };
        }
        self.last = Some(at);
        if !self.armed {
            return None;
        }
        (self.dx, self.dy) = (self.dx + dx, self.dy + dy);
        if self.dy.abs() > SWIPE_THRESHOLD {
            self.armed = false;
        }
        if !self.armed || self.dx.abs() < SWIPE_THRESHOLD || self.dx.abs() < 2. * self.dy.abs() {
            return None;
        }
        self.armed = false;
        Some(if self.dx > 0. { Step::Next } else { Step::Previous })
    }
}

pub fn neighbour_space(model: &Model, step: Step) -> Option<usize> {
    match step {
        Step::Next => Some(model.space_ix + 1),
        Step::Previous => model.space_ix.checked_sub(1),
    }
    .filter(|ix| *ix < model.spaces.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestCore;

    fn opened() -> (tempfile::TempDir, TestCore, Model) {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let mut model = Model::new(4);
        model.reload_spaces(&core);
        model.switch_space(&core, 0).unwrap();
        (dir, core, model)
    }

    fn tab(core: &TestCore, model: &mut Model, kind: TabKind, folder: Option<Id>) -> TabKey {
        let s = model.spaces[0].space.id;
        let t = core.disk.create_tab(s, kind, folder, "https://a.test/", "").unwrap();
        model.reload_space(core);
        model.key(&t)
    }

    #[test]
    fn rows_follow_the_sidebar_tiles_pinned_folders_groups_then_loose_tabs() {
        let (_dir, core, mut model) = opened();
        let tile = tab(&core, &mut model, TabKind::Tile, None);
        let pinned = tab(&core, &mut model, TabKind::Pinned, None);
        let folder = new_folder(&mut model, &core, None).unwrap();
        let inside = tab(&core, &mut model, TabKind::Pinned, Some(folder));
        let grouped = tab(&core, &mut model, TabKind::Ephemeral, None);
        let group = new_group(&mut model, &core, Some(grouped)).unwrap();
        let (a, b) = (tab(&core, &mut model, TabKind::Ephemeral, None), tab(&core, &mut model, TabKind::Ephemeral, None));
        let mut splits = Splits::default();
        assert!(splits.join(a, b));
        let bar = sidebar(&model, &splits);
        assert_eq!(bar.tiles, [tile]);
        let row = |kind, depth| Row { kind, depth };
        assert_eq!(bar.pinned, [vec![row(RowKind::Tab(pinned), 0)], vec![row(RowKind::Folder(folder), 0), row(RowKind::Tab(inside), 1)]]);
        assert_eq!(bar.groups, [(group, vec![row(RowKind::Tab(grouped), 0)])]);
        assert_eq!(bar.loose, [row(RowKind::Split(vec![a, b]), 0)], "a split is one row");

        model.toggle_folder(folder);
        assert_eq!(sidebar(&model, &splits).pinned[1].len(), 1, "a closed folder hides its tabs");
        model.active = Some(inside);
        assert_eq!(sidebar(&model, &splits).pinned[1].len(), 2, "but keeps its active tab");
        assert!(model.toggle_group(&core, group));
        assert!(sidebar(&model, &splits).groups[0].1.is_empty(), "a collapsed group hides its tabs");
    }

    #[test]
    fn rows_pin_tile_group_and_clean_up_through_the_store() {
        let (_dir, core, mut model) = opened();
        let key = tab(&core, &mut model, TabKind::Ephemeral, None);
        assert!(toggle_pin(&mut model, &core, key));
        assert_eq!(model.tab(key).unwrap().kind, TabKind::Pinned);
        assert!(!set_group(&mut model, &core, key, None), "a pinned tab joins no group");
        assert!(toggle_pin(&mut model, &core, key) && toggle_tile(&mut model, &core, key));
        assert_eq!(model.tab(key).unwrap().kind, TabKind::Tile);
        assert!(toggle_tile(&mut model, &core, key));
        let group = new_group(&mut model, &core, None).unwrap();
        assert!(set_group(&mut model, &core, key, Some(group)));
        assert_eq!(group_tabs(&model, group), [key]);
        assert!(remove_group(&mut model, &core, group, true));
        assert!(model.groups.is_empty() && model.tab(key).unwrap().group_id.is_none(), "ungrouping keeps the tabs");

        let s = model.spaces[0].space.id;
        for url in ["https://github.com/a", "https://gitlab.com/b"] {
            core.disk.create_tab(s, TabKind::Ephemeral, None, url, "").unwrap();
        }
        model.reload_space(&core);
        assert!(cleanable(&model));
        assert_eq!(clean_up(&mut model, &core, |t| t.url.clone()), 2);
        assert_eq!(model.groups.len(), 1);
        assert_eq!(clean_up_text(2), "Sorted 2 tabs into groups");
        assert_eq!(clean_up_text(0), "Already tidy");
    }

    #[test]
    fn deleting_a_folder_closes_it_and_its_subfolders() {
        let (_dir, core, mut model) = opened();
        let outer = new_folder(&mut model, &core, None).unwrap();
        let inner = new_folder(&mut model, &core, Some(outer)).unwrap();
        let inside = tab(&core, &mut model, TabKind::Pinned, Some(inner));
        assert!(delete_folder(&mut model, &core, outer));
        assert!(model.folders.is_empty());
        assert!(model.tab(inside).is_none(), "its tabs go with it");
        assert!(!model.expanded.contains(&outer) && !model.expanded.contains(&inner));
    }

    #[test]
    fn closing_below_and_others_counts_only_unpinned_tabs() {
        let (_dir, core, mut model) = opened();
        let first = tab(&core, &mut model, TabKind::Ephemeral, None);
        let pinned = tab(&core, &mut model, TabKind::Pinned, None);
        let second = tab(&core, &mut model, TabKind::Ephemeral, None);
        assert_eq!(tabs_below(&model, first), [second]);
        assert!(tabs_below(&model, second).is_empty());
        assert_eq!(other_unpinned(&model, first), [second]);
        assert!(!other_unpinned(&model, first).contains(&pinned));
    }

    #[test]
    fn one_swipe_step_per_gesture_past_the_threshold() {
        let (mut s, t) = (Swipe::default(), Instant::now());
        let ms = |n| t + Duration::from_millis(n);
        assert_eq!(s.feed(40., 0., ms(0)), None);
        assert_eq!(s.feed(40., 5., ms(10)), Some(Step::Next));
        assert_eq!(s.feed(200., 0., ms(20)), None, "the rest of the gesture");
        assert_eq!(s.feed(-70., 0., ms(400)), Some(Step::Previous), "a pause starts the next gesture");
        assert_eq!(s.feed(10., -70., ms(800)), None, "a vertical scroll");
        assert_eq!(s.feed(100., 0., ms(810)), None, "stays vertical until it pauses");
    }

    #[test]
    fn a_swipe_reaches_only_spaces_that_exist() {
        let (_dir, core, mut model) = opened();
        assert_eq!(neighbour_space(&model, Step::Previous), None);
        assert_eq!(neighbour_space(&model, Step::Next), None);
        let profile = model.spaces[0].space.profile_id;
        core.disk.create_space(profile, "Work", "").unwrap();
        model.reload_spaces(&core);
        assert_eq!(neighbour_space(&model, Step::Next), Some(1));
    }
}
