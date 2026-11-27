use std::collections::HashSet;
use std::time::{Duration, Instant};

use shield::ShieldLevel;
use store::{Folder, Id, Store, Tab, TabKind};

use crate::closed::ClosedGroup;
use crate::split::Splits;
use crate::tabs::{Model, SpaceRef, TabKey, folder_tree};
use crate::toast::Tone;
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

fn folder_rows(model: &Model, splits: &Splits<TabKey>, folder: &Folder, depth: usize, open_all: bool, rows: &mut Vec<Row>) {
    rows.push(Row { kind: RowKind::Folder(folder.id), depth });
    if !open_all && !model.expanded.contains(&folder.id) {
        let tree = folder_tree(&model.folders, folder.id);
        let active = sorted(model, |t| t.folder_id.is_some_and(|f| tree.contains(&f)) && model.active == Some(model.key(t)));
        rows.extend(tab_rows(model, splits, &active, depth + 1));
        return;
    }
    let tabs = sorted(model, |t| t.kind == TabKind::Pinned && t.folder_id == Some(folder.id));
    rows.extend(tab_rows(model, splits, &tabs, depth + 1));
    for child in children(model, Some(folder.id)) {
        folder_rows(model, splits, child, depth + 1, open_all, rows);
    }
}

fn children(model: &Model, parent: Option<Id>) -> Vec<&Folder> {
    let mut folders: Vec<&Folder> = model.folders.iter().filter(|f| f.parent_id == parent).collect();
    folders.sort_by_key(|f| f.position);
    folders
}

pub fn sidebar(model: &Model, splits: &Splits<TabKey>) -> Sidebar {
    rows(model, splits, false)
}

/// `open_all` lists every folder's and group's tabs as if none were collapsed.
fn rows(model: &Model, splits: &Splits<TabKey>, open_all: bool) -> Sidebar {
    let tiles = sorted(model, |t| t.kind == TabKind::Tile).into_iter().map(|t| model.key(t)).collect();
    let mut pinned: Vec<Vec<Row>> =
        tab_rows(model, splits, &sorted(model, |t| t.kind == TabKind::Pinned && t.folder_id.is_none()), 0).into_iter().map(|r| vec![r]).collect();
    for folder in children(model, None) {
        let mut block = Vec::new();
        folder_rows(model, splits, folder, 0, open_all, &mut block);
        pinned.push(block);
    }
    let groups = model
        .groups
        .iter()
        .map(|g| {
            let tabs =
                sorted(model, |t| t.kind == TabKind::Ephemeral && t.group_id == Some(g.id) && (open_all || !g.collapsed || model.active == Some(model.key(t))));
            (g.id, tab_rows(model, splits, &tabs, 0))
        })
        .collect();
    let loose = tab_rows(model, splits, &sorted(model, |t| t.kind == TabKind::Ephemeral && t.group_id.is_none()), 0);
    Sidebar { tiles, pinned, groups, loose }
}

/// The tab strip of a window with tabs on top (`layout::TabPlacement::Top`): the pinned tabs as
/// icons, then the other tabs, in the sidebar's order, also those of collapsed folders and groups.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Strip {
    pub pinned: Vec<TabKey>,
    pub tabs: Vec<TabKey>,
}

pub fn strip(model: &Model) -> Strip {
    let bar = rows(model, &Splits::default(), true);
    let keys = |rows: &[Row]| -> Vec<TabKey> {
        rows.iter()
            .flat_map(|r| match &r.kind {
                RowKind::Tab(key) => vec![*key],
                RowKind::Split(keys) => keys.clone(),
                RowKind::Folder(_) => Vec::new(),
            })
            .collect()
    };
    let pinned = bar.tiles.iter().copied().chain(bar.pinned.iter().flat_map(|block| keys(block))).collect();
    let tabs = bar.groups.iter().flat_map(|(_, rows)| keys(rows)).chain(keys(&bar.loose)).collect();
    Strip { pinned, tabs }
}

pub fn cleanable(model: &Model) -> bool {
    !model.burner_shown() && crate::cleanup::cleanable(&model.tabs)
}

pub const ORGANIZE: &str = "Organize Tabs";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Organize {
    NewGroup,
    NewFolder,
    CleanUp,
    GroupWithAssistant,
}

pub fn organize(cleanable: bool, assistant: bool) -> Vec<(&'static str, Organize)> {
    let mut items = vec![("New Tab Group", Organize::NewGroup), ("New Folder", Organize::NewFolder)];
    if cleanable {
        items.push(("Clean Up", Organize::CleanUp));
        if assistant {
            items.push(("Group with Assistant", Organize::GroupWithAssistant));
        }
    }
    items
}

fn takes(model: &Model, kind: TabKind) -> bool {
    !model.burner_shown() || kind == TabKind::Ephemeral
}

pub fn move_tab<D: WebsiteData>(model: &mut Model, core: &Core<D>, key: TabKey, kind: TabKind, folder: Option<Id>, index: usize) -> bool {
    if !takes(model, kind) {
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

pub fn closed_group(model: &Model, id: Id) -> Option<ClosedGroup> {
    model.groups.iter().find(|g| g.id == id).map(|g| ClosedGroup { name: g.name.clone(), color: g.color.clone(), icon: g.icon.clone() })
}

pub fn rename_tab<D: WebsiteData>(model: &mut Model, core: &Core<D>, key: TabKey, typed: &str, page_title: &str) -> bool {
    let typed = crate::text::bounded_title(typed);
    if typed.is_empty() || model.tab(key).is_none_or(|t| t.kind == TabKind::Tile) {
        return false;
    }
    let custom = if typed == crate::text::bounded_title(page_title) { "" } else { typed.as_str() };
    let saved = core.store(key.private).set_tab_custom_title(key.id, custom).is_ok();
    if saved {
        model.reload_space(core);
    }
    saved
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DropAt {
    pub kind: TabKind,
    pub folder: Option<Id>,
    pub group: Option<Id>,
    pub before: Option<TabKey>,
    /// Lands after `before` instead of in its place (a drop on the lower half of its row).
    pub below: bool,
}

impl DropAt {
    pub fn zone(kind: TabKind, folder: Option<Id>, group: Option<Id>) -> Self {
        DropAt { kind, folder, group, before: None, below: false }
    }

    pub fn before(model: &Model, key: TabKey) -> Option<Self> {
        model.tab(key).map(|t| DropAt { kind: t.kind, folder: t.folder_id, group: t.group_id, before: Some(key), below: false })
    }
}

pub fn drop_tab<D: WebsiteData>(model: &mut Model, core: &Core<D>, dragged: TabKey, at: DropAt) -> Option<Vec<Tab>> {
    let before = at.before.and_then(|k| model.tab(k).filter(|_| k.private == dragged.private));
    if at.before.is_some() && before.is_none() {
        return None;
    }
    let index = before.map_or_else(|| model.zone_len(at.kind, at.folder), |t| model.drop_index(dragged, t, at.below));
    place(model, core, dragged, at, index)
}

pub fn land_on<D: WebsiteData>(model: &mut Model, core: &Core<D>, dragged: TabKey, target: TabKey) -> Option<Vec<Tab>> {
    let at = DropAt::before(model, target).filter(|_| target.private == dragged.private)?;
    let index = model.tab(target).map(|t| model.zone_index(t))?;
    place(model, core, dragged, at, index)
}

fn place<D: WebsiteData>(model: &mut Model, core: &Core<D>, dragged: TabKey, at: DropAt, index: usize) -> Option<Vec<Tab>> {
    if at.before == Some(dragged) || model.tab(dragged).is_none() || !takes(model, at.kind) {
        return None;
    }
    let group = (at.kind == TabKind::Ephemeral).then_some(at.group);
    let from = crate::undo::Move::of(model, dragged);
    core.store(dragged.private).move_tab_to(dragged.id, at.kind, at.folder, index, group).ok()?;
    if let Some(from) = from {
        model.undo.push(crate::undo::Step::Moved(from), dragged.private);
    }
    let old = model.reload_space(core);
    model.expanded.extend(at.folder);
    Some(old)
}

pub fn slides_under(sidebar: &Sidebar, closed: TabKey) -> Option<TabKey> {
    let lists = sidebar.groups.iter().map(|(_, rows)| rows).chain([&sidebar.loose]);
    lists.into_iter().find_map(|rows| {
        let at = rows.iter().position(|r| r.kind == RowKind::Tab(closed))?;
        match rows.get(at + 1).map(|r| &r.kind) {
            Some(RowKind::Tab(next)) => Some(Some(*next)),
            _ => Some(None),
        }
    })?
}

pub fn saved_share(transfers: impl IntoIterator<Item = (u64, Option<u64>)>) -> Option<f32> {
    let (done, total) = transfers.into_iter().filter_map(|(done, total)| Some((done, total?))).fold((0, 0), |(d, s), (done, total)| (d + done, s + total));
    (total > 0).then(|| (done as f64 / total as f64) as f32)
}

pub struct CleanUp {
    pub text: String,
    pub tone: Tone,
    pub undo: Option<(bool, crate::cleanup::Undo)>,
}

pub fn clean_up<D: WebsiteData>(model: &mut Model, core: &Core<D>, url: impl Fn(&Tab) -> String) -> Option<CleanUp> {
    let (private, space_id) = model.current_space().filter(|_| !model.burner_shown()).map(|s| (s.private, s.space.id))?;
    let store = core.store(private);
    let (Ok(tabs), Ok(groups)) = (store.tabs(space_id), store.groups(space_id)) else {
        return Some(CleanUp { text: "Could not sort the tabs".to_owned(), tone: Tone::Warning, undo: None });
    };
    let sort = crate::cleanup::plan(&tabs, &groups, url);
    Some(apply_sort(model, core, (private, space_id), &sort))
}

fn apply_sort<D: WebsiteData>(model: &mut Model, core: &Core<D>, (private, space_id): (bool, Id), sort: &crate::cleanup::Sort) -> CleanUp {
    let store = core.store(private);
    let applied = if sort.changed { crate::cleanup::apply(store, space_id, sort).map(Some) } else { Ok(None) };
    let text = sort.summary(applied.is_ok(), 0, false);
    let tone = if applied.is_ok() { Tone::Info } else { Tone::Warning };
    if sort.changed {
        model.reload_space(core);
    }
    let undo = applied.ok().flatten();
    if let Some(undo) = &undo {
        model.undo.push(crate::undo::Step::CleanUp(undo.clone()), private);
    }
    CleanUp { text, tone, undo: undo.map(|undo| (private, undo)) }
}

/// Clean Up's assistant request for the current space's loose tabs, only where the palette would
/// offer Ask; `page` gives each tab's address and effective Shield level. The space comes back for
/// `group_as_proposed`.
pub fn assistant_grouping<D: WebsiteData>(
    model: &Model,
    core: &Core<D>,
    page: impl Fn(&Tab) -> (String, ShieldLevel),
) -> Option<((bool, Id), crate::cleanup::ai::Request)> {
    let (private, space_id) = model.current_space().map(|s| (s.private, s.space.id))?;
    let origin = model.assistant_origin(core, None).filter(|_| model.assistant_offered(core, None))?;
    let tabs = core.store(private).tabs(space_id).ok()?;
    Some(((private, space_id), crate::cleanup::ai::request(origin, &tabs, page)?))
}

/// Whether a grouping asked for in `space` may still be shown and applied: that space is still the
/// current one and the assistant is still allowed there with its profile's AI on.
pub fn grouping_here<D: WebsiteData>(model: &Model, core: &Core<D>, space: (bool, Id)) -> bool {
    model.current_space().map(|s| (s.private, s.space.id)) == Some(space) && model.assistant_offered(core, None)
}

/// Files the tabs of `space` the assistant grouped, of those it was `sent`, as an undoable Clean Up.
pub fn group_as_proposed<D: WebsiteData>(model: &mut Model, core: &Core<D>, space: (bool, Id), sent: &HashSet<Id>, proposal: &[(String, Vec<Id>)]) -> CleanUp {
    let store = core.store(space.0);
    let (Ok(tabs), Ok(groups)) = (store.tabs(space.1), store.groups(space.1)) else {
        return CleanUp { text: "Could not sort the tabs".to_owned(), tone: Tone::Warning, undo: None };
    };
    apply_sort(model, core, space, &crate::cleanup::ai::sort_from(&tabs, &groups, sent, proposal))
}

pub fn undo_sort<D: WebsiteData>(model: &mut Model, core: &Core<D>, (private, undo): &(bool, crate::cleanup::Undo)) -> bool {
    let done = match crate::cleanup::undo(core.store(*private), undo) {
        Ok(dropped) => {
            dropped.iter().for_each(|id| model.closed_tabs.forget_group(*private, *id));
            true
        }
        Err(_) => false,
    };
    model.reload_space(core);
    done
}

pub fn tabs_below(model: &Model, key: TabKey) -> Vec<TabKey> {
    model.tabs_below(key)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Started,
    Moved,
    Ended,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Swipe {
    dx: f32,
    dy: f32,
    armed: bool,
    last: Option<Instant>,
    natural: bool,
}

impl Swipe {
    pub fn natural(natural: bool) -> Self {
        Swipe { natural, ..Swipe::default() }
    }

    fn restart(&mut self) {
        *self = Swipe { armed: true, ..Swipe::natural(self.natural) };
    }

    pub fn feed(&mut self, dx: f32, dy: f32, at: Instant) -> Option<Step> {
        if self.last.is_none_or(|last| at.saturating_duration_since(last) > SWIPE_GAP) {
            self.restart();
        }
        self.last = Some(at);
        self.moved(dx, dy)
    }

    pub fn touch(&mut self, phase: Phase, dx: f32, dy: f32) -> Option<Step> {
        match phase {
            Phase::Started => self.restart(),
            Phase::Ended => self.armed = false,
            Phase::Moved => return self.moved(dx, dy),
        }
        None
    }

    fn moved(&mut self, dx: f32, dy: f32) -> Option<Step> {
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
        Some(if (self.dx > 0.) != self.natural { Step::Next } else { Step::Previous })
    }
}

/// Where the list of the space switched to starts its spring, `width` away on the side it comes
/// from: a switch made mid-slide or mid-drag picks up where the old list stood (`shown`), so a
/// second press retargets the spring instead of jumping. `None` with Reduce Motion: no slide.
pub fn slide_start(shown: Option<f32>, from: usize, to: usize, width: f32, reduce: bool) -> Option<f32> {
    let dir = if to > from { 1. } else { -1. };
    (from != to && !reduce).then(|| shown.unwrap_or(0.) + dir * width)
}

pub fn neighbour_space(model: &Model, step: Step, allowed: impl Fn(&SpaceRef) -> bool) -> Option<usize> {
    let mut ix = model.space_ix;
    loop {
        ix = match step {
            Step::Next => ix.checked_add(1)?,
            Step::Previous => ix.checked_sub(1)?,
        };
        let space = model.spaces.get(ix)?;
        if model.space_shown(ix) && allowed(space) {
            return Some(ix);
        }
    }
}

pub const SPACE_DOT: f32 = 14.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceBar {
    pub width: f32,
    pub icon: f32,
    pub gap: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpaceStrip {
    pub compact: bool,
    pub scroll: Option<f32>,
}

impl SpaceBar {
    pub fn strip(&self, count: usize, current: usize) -> SpaceStrip {
        let gaps = count.saturating_sub(1) as f32 * self.gap;
        let compact = count as f32 * self.icon + gaps > self.width;
        if !compact {
            return SpaceStrip { compact, scroll: None };
        }
        let content = self.icon + count.saturating_sub(1) as f32 * SPACE_DOT + gaps;
        let scroll = (content > self.width).then(|| {
            let middle = current.min(count.saturating_sub(1)) as f32 * (SPACE_DOT + self.gap) + self.icon / 2.;
            (middle - self.width / 2.).clamp(0., content - self.width)
        });
        SpaceStrip { compact, scroll }
    }
}

/// How many of a profile's latest chats the sidebar lists.
pub const CHATS_SHOWN: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatRow {
    pub id: Id,
    pub title: String,
    /// The time it last changed today, else its date.
    pub when: String,
}

/// The profile's latest chats, newest first.
pub fn chat_rows(store: &Store, profile: Id, now: i64) -> Vec<ChatRow> {
    let today = store.local_date(now).unwrap_or_default();
    let mut chats = store.chat_titles(profile).unwrap_or_default();
    chats.truncate(CHATS_SHOWN);
    chats
        .into_iter()
        .map(|c| {
            let date = store.local_date(c.updated).unwrap_or_default();
            let when = if date == today { store.local_clock(c.updated).unwrap_or_default() } else { short_date(&date) };
            ChatRow { id: c.id, title: c.title, when }
        })
        .collect()
}

/// `YYYY-MM-DD` as `Mon D`, or as given when it is not such a date.
pub fn short_date(date: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut parts = date.split('-').skip(1).map(|p| p.parse::<usize>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(m @ 1..=12), Some(d)) => format!("{} {d}", MONTHS[m - 1]),
        _ => date.to_owned(),
    }
}

/// A collapsed group's first few page addresses and how many tabs it holds.
pub fn group_summary(model: &Model, group: Id) -> (Vec<String>, usize) {
    let tabs: Vec<&Tab> = model.tabs.iter().filter(|t| t.kind == TabKind::Ephemeral && t.group_id == Some(group)).collect();
    (tabs.iter().take(3).map(|t| t.url.clone()).collect(), tabs.len())
}

#[cfg(test)]
mod tests {
    #[test]
    fn chats_show_their_time_today_and_their_date_before() {
        use super::{CHATS_SHOWN, chat_rows, short_date};
        let store = store::Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "u", false).unwrap().id;
        assert!(chat_rows(&store, profile, crate::now()).is_empty());
        let turns = [store::ChatTurn { you: true, text: "hi".into(), by: None, interrupted: false, command: false }];
        for n in 0..CHATS_SHOWN + 1 {
            store.save_chat(profile, None, Some(&format!("Chat {n}")), &turns, &[]).unwrap();
        }
        let rows = chat_rows(&store, profile, crate::now());
        assert_eq!(rows.len(), CHATS_SHOWN);
        assert!(rows.iter().all(|r| r.when.len() == 5 && r.when.as_bytes()[2] == b':'), "{rows:?}");
        assert!(chat_rows(&store, profile, crate::now() + 3 * 86_400_000).iter().all(|r| r.when.contains(' ')), "older chats show a date");
        assert_eq!(short_date("2026-11-07"), "Nov 7");
        assert_eq!(short_date("garbage"), "garbage");
    }

    #[test]
    fn organize_offers_clean_up_only_with_tabs_to_sort_and_the_assistant_only_when_allowed() {
        use super::{Organize, organize};
        let names = |c, a| organize(c, a).into_iter().map(|(_, o)| o).collect::<Vec<_>>();
        assert_eq!(names(false, true), [Organize::NewGroup, Organize::NewFolder]);
        assert_eq!(names(true, false), [Organize::NewGroup, Organize::NewFolder, Organize::CleanUp]);
        assert_eq!(names(true, true), [Organize::NewGroup, Organize::NewFolder, Organize::CleanUp, Organize::GroupWithAssistant]);
    }

    #[test]
    fn a_second_switch_retargets_the_slide_from_where_it_stands() {
        assert_eq!(slide_start(None, 0, 1, 250., false), Some(250.));
        assert_eq!(slide_start(None, 2, 1, 250., false), Some(-250.));
        assert_eq!(slide_start(Some(100.), 1, 2, 250., false), Some(350.), "mid-slide, it starts where the list stands");
        assert_eq!(slide_start(None, 1, 1, 250., false), None);
        assert_eq!(slide_start(None, 0, 1, 250., true), None, "Reduce Motion switches at once");
    }

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
    fn the_strip_lists_pinned_tabs_then_the_others_in_sidebar_order() {
        let (_dir, core, mut model) = opened();
        let tile = tab(&core, &mut model, TabKind::Tile, None);
        let pinned = tab(&core, &mut model, TabKind::Pinned, None);
        let (a, b) = (tab(&core, &mut model, TabKind::Ephemeral, None), tab(&core, &mut model, TabKind::Ephemeral, None));
        let shown = strip(&model);
        assert_eq!(shown.pinned, [tile, pinned]);
        assert_eq!(shown.tabs, model.ephemeral_keys());
        assert!(shown.tabs.contains(&a) && shown.tabs.contains(&b));
    }

    #[test]
    fn the_strip_reaches_tabs_inside_collapsed_folders_and_groups() {
        let (_dir, core, mut model) = opened();
        let outer = new_folder(&mut model, &core, None).unwrap();
        let inner = new_folder(&mut model, &core, Some(outer)).unwrap();
        let (top, deep) = (tab(&core, &mut model, TabKind::Pinned, Some(outer)), tab(&core, &mut model, TabKind::Pinned, Some(inner)));
        let grouped = tab(&core, &mut model, TabKind::Ephemeral, None);
        let group = new_group(&mut model, &core, Some(grouped)).unwrap();
        let open = strip(&model);
        model.toggle_folder(outer);
        model.toggle_folder(inner);
        assert!(model.toggle_group(&core, group));
        model.active = None;
        let shown = strip(&model);
        assert_eq!(shown, open, "collapsing hides nothing from the strip");
        assert_eq!(shown.pinned, [top, deep]);
        assert_eq!(shown.tabs, [grouped]);
    }

    #[test]
    fn a_drop_into_a_group_that_is_gone_changes_nothing() {
        let (_dir, core, mut model) = opened();
        let pinned = tab(&core, &mut model, TabKind::Pinned, None);
        let gone = DropAt::zone(TabKind::Ephemeral, None, Some(9_999));
        assert!(drop_tab(&mut model, &core, pinned, gone).is_none(), "the drop reports its failure");
        assert_eq!(model.tab(pinned).unwrap().kind, TabKind::Pinned, "the tab keeps its place");
    }

    #[test]
    fn a_tab_landing_on_a_row_takes_its_place_and_its_group_in_one_write() {
        let (_dir, core, mut model) = opened();
        let (a, b, c) = (
            tab(&core, &mut model, TabKind::Ephemeral, None),
            tab(&core, &mut model, TabKind::Ephemeral, None),
            tab(&core, &mut model, TabKind::Ephemeral, None),
        );
        let group = new_group(&mut model, &core, Some(a)).unwrap();
        assert!(land_on(&mut model, &core, c, a).is_some());
        assert_eq!(model.tab(c).unwrap().group_id, Some(group), "the row's group comes with its place");
        assert_eq!(model.ephemeral_keys(), [c, a, b]);
        assert!(land_on(&mut model, &core, c, b).is_some());
        assert_eq!((model.ephemeral_keys(), model.tab(c).unwrap().group_id), (vec![a, b, c], None), "from above it lands after the row");
        assert!(land_on(&mut model, &core, c, c).is_none());
        core.store(false).delete_group(group).unwrap();
        assert!(land_on(&mut model, &core, b, a).is_none(), "a row whose group is gone moves nothing");
    }

    #[test]
    fn a_burner_is_never_cleaned_up() {
        let (_dir, core, mut model) = opened();
        tab(&core, &mut model, TabKind::Ephemeral, None);
        tab(&core, &mut model, TabKind::Ephemeral, None);
        assert!(cleanable(&model));
        let ix = model.open_burner(&core).unwrap();
        model.switch_space(&core, ix).unwrap();
        let burner = model.burner.unwrap();
        for url in ["https://github.com/a", "https://gitlab.com/b"] {
            core.store(true).create_tab(burner, TabKind::Ephemeral, None, url, "").unwrap();
        }
        model.reload_space(&core);
        assert!(crate::cleanup::cleanable(&model.tabs) && model.burner_shown());
        assert!(!cleanable(&model), "a burner's tabs are never sorted");
    }

    #[test]
    fn tabs_below_follow_the_sidebar_where_groups_come_before_loose_tabs() {
        let (_dir, core, mut model) = opened();
        let (a, b, c) = (
            tab(&core, &mut model, TabKind::Ephemeral, None),
            tab(&core, &mut model, TabKind::Ephemeral, None),
            tab(&core, &mut model, TabKind::Ephemeral, None),
        );
        let group = new_group(&mut model, &core, Some(c)).unwrap();
        assert_eq!(group_tabs(&model, group), [c]);
        assert_eq!(tabs_below(&model, a), [b], "the grouped tab shows above the loose ones");
        assert_eq!(tabs_below(&model, c), [a, b]);
        assert_eq!(tabs_below(&model, b), []);
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
        assert!(crate::cleanup::cleanable(&model.tabs));
        let sorted = clean_up(&mut model, &core, |t| t.url.clone()).unwrap();
        assert_eq!((sorted.text.as_str(), sorted.tone), ("Sorted 2 tabs into groups", Tone::Info));
        assert_eq!(model.groups.len(), 1);
        assert_eq!(model.groups[0].icon, "code", "a new group shows its category's icon");
        let again = clean_up(&mut model, &core, |t| t.url.clone()).unwrap();
        assert!(again.undo.is_none(), "a second run changes nothing");
        assert!(undo_sort(&mut model, &core, &sorted.undo.unwrap()));
        assert!(model.groups.is_empty(), "Undo Sort removes the group it made");
    }

    #[test]
    fn the_assistant_groups_only_what_it_was_sent_and_undo_takes_it_back() {
        let (_dir, core, mut model) = opened();
        let s = model.spaces[0].space.id;
        let tab = |url: &str| core.disk.create_tab(s, TabKind::Ephemeral, None, url, "T").unwrap().id;
        let (a, b, later) = (tab("https://a.test/"), tab("https://b.test/"), tab("file:///x"));
        model.reload_space(&core);
        let profile = model.profile().unwrap().1;
        core.open_profile(false, profile, || std::rc::Rc::new(|_| {}));
        let page = |t: &Tab| (t.url.clone(), ShieldLevel::Standard);
        assert!(assistant_grouping(&model, &core, page).is_none(), "nothing is sent with AI off");
        core.disk.set_profile_ai(profile, true).unwrap();
        let (space, request) = assistant_grouping(&model, &core, page).unwrap();
        assert_eq!(request.sent, HashSet::from([a, b]));
        assert!(grouping_here(&model, &core, space));
        assert!(!grouping_here(&model, &core, (true, space.1)), "another space is not where it was asked");
        core.disk.set_profile_ai(profile, false).unwrap();
        assert!(!grouping_here(&model, &core, space), "AI turned off since the request");
        core.disk.set_profile_ai(profile, true).unwrap();
        let done = group_as_proposed(&mut model, &core, space, &request.sent, &[("Reading".into(), vec![a, b, later])]);
        assert_eq!(done.tone, Tone::Info);
        let grouped: Vec<Id> = model.tabs.iter().filter(|t| t.group_id.is_some()).map(|t| t.id).collect();
        assert_eq!(grouped.len(), 2, "a tab that was not sent stays loose");
        assert!(!grouped.contains(&later));
        assert!(undo_sort(&mut model, &core, &done.undo.unwrap()));
        assert!(model.groups.is_empty());
    }

    #[test]
    fn only_the_next_row_of_the_same_list_slides_under_a_closed_tab() {
        let key = |id| TabKey { private: false, id };
        let row = |id| Row { kind: RowKind::Tab(key(id)), depth: 0 };
        let bar = Sidebar { groups: vec![(9, vec![row(1), row(2)])], loose: vec![row(3), row(4)], ..Sidebar::default() };
        assert_eq!(slides_under(&bar, key(1)), Some(key(2)));
        assert_eq!(slides_under(&bar, key(2)), None, "the last tab of a group leaves the group header under the pointer");
        assert_eq!(slides_under(&bar, key(3)), Some(key(4)));
        assert_eq!(slides_under(&bar, key(4)), None);
        assert_eq!(slides_under(&bar, key(7)), None);
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
    fn a_closed_group_keeps_its_name_and_colour_to_reopen() {
        let (_dir, core, mut model) = opened();
        let key = tab(&core, &mut model, TabKind::Ephemeral, None);
        let group = new_group(&mut model, &core, Some(key)).unwrap();
        core.disk.set_group_color(group, "green").unwrap();
        model.reload_space(&core);
        let closed = closed_group(&model, group).unwrap();
        assert_eq!((closed.name.as_str(), closed.color.as_str()), (model.groups[0].name.as_str(), "green"));
        assert!(closed_group(&model, group + 1).is_none());
    }

    #[test]
    fn a_rename_keeps_the_typed_name_until_it_is_the_page_title() {
        let (_dir, core, mut model) = opened();
        let key = tab(&core, &mut model, TabKind::Ephemeral, None);
        assert!(rename_tab(&mut model, &core, key, "  Mail  ", "Inbox"));
        let shown = |model: &Model, title: &str, url: &str| crate::command::display_title(&model.tab(key).unwrap().custom_title, title, url);
        assert_eq!(shown(&model, "Inbox", ""), "Mail");
        assert!(!rename_tab(&mut model, &core, key, "   ", "Inbox"), "a blank name is ignored");
        assert!(rename_tab(&mut model, &core, key, "Inbox", "Inbox"));
        assert_eq!(model.tab(key).unwrap().custom_title, "", "the page title again follows the page");
        assert_eq!(shown(&model, "", ""), "New Tab");
        assert_eq!(shown(&model, "", "data:text/html,x"), "data:", "a page without a title shows its label, never its whole address");
        let tile = tab(&core, &mut model, TabKind::Tile, None);
        assert!(!rename_tab(&mut model, &core, tile, "Tile", ""), "tiles show no title");
    }

    #[test]
    fn undo_takes_back_moves_and_closes_newest_first() {
        let (_dir, core, mut model) = opened();
        let (a, b) = (tab(&core, &mut model, TabKind::Ephemeral, None), tab(&core, &mut model, TabKind::Ephemeral, None));
        let group = model.new_group(&core, Some(a)).unwrap().id;
        model.reload_space(&core);
        assert!(drop_tab(&mut model, &core, a, DropAt::zone(TabKind::Pinned, None, None)).is_some());
        let space = model.current_space().map(|s| (s.space.profile_id, s.space.id)).unwrap();
        let serial = model.push_closed(crate::closed::Closed { private: false, profile_id: space.0, space_id: space.1, tabs: Vec::new(), group: None });
        assert_eq!(model.pop_undo(&core), Some(crate::undo::Step::Closed(serial)));
        let Some(crate::undo::Step::Moved(from)) = model.pop_undo(&core) else { panic!("the move is next") };
        assert!(from.revert(&mut model, &core));
        let back = model.tab(a).unwrap();
        assert_eq!((back.kind, back.group_id, model.ephemeral_keys()), (TabKind::Ephemeral, Some(group), vec![a, b]));
        let serial = model.push_closed(crate::closed::Closed { private: false, profile_id: space.0, space_id: space.1, tabs: Vec::new(), group: None });
        model.closed_tabs.take(serial, &[]);
        assert_eq!(model.pop_undo(&core), None, "a close reopened since is skipped");
        model.push_closed(crate::closed::Closed { private: true, profile_id: space.0, space_id: space.1, tabs: Vec::new(), group: None });
        assert!(model.undo.is_empty() && !model.closed_tabs.is_empty(), "a private close is reopenable but never kept for Undo");
    }

    #[test]
    fn undo_skips_steps_whose_tab_or_space_is_gone() {
        let (_dir, core, mut model) = opened();
        let (a, b) = (tab(&core, &mut model, TabKind::Ephemeral, None), tab(&core, &mut model, TabKind::Ephemeral, None));
        assert!(drop_tab(&mut model, &core, b, DropAt::zone(TabKind::Pinned, None, None)).is_some());
        assert!(drop_tab(&mut model, &core, a, DropAt::zone(TabKind::Pinned, None, None)).is_some());
        core.disk.delete_tab(a.id).unwrap();
        model.reload_space(&core);
        let profile = model.spaces[0].space.profile_id;
        model.push_closed(crate::closed::Closed { private: false, profile_id: profile, space_id: 999, tabs: Vec::new(), group: None });
        let Some(crate::undo::Step::Moved(from)) = model.pop_undo(&core) else { panic!("the gone space's close and the gone tab's move are skipped") };
        assert_eq!(from.key, b);
        assert_eq!(model.pop_undo(&core), None);
    }

    #[test]
    fn a_dropped_tab_moves_into_the_zone_folder_or_group_it_lands_on() {
        let (_dir, core, mut model) = opened();
        let (a, b, c) = (
            tab(&core, &mut model, TabKind::Ephemeral, None),
            tab(&core, &mut model, TabKind::Ephemeral, None),
            tab(&core, &mut model, TabKind::Ephemeral, None),
        );
        let at = DropAt::before(&model, a).unwrap();
        assert!(drop_tab(&mut model, &core, c, at).is_some());
        assert_eq!(model.ephemeral_keys(), [c, a, b], "from below it goes before the row");
        let at = DropAt::before(&model, b).unwrap();
        assert!(drop_tab(&mut model, &core, c, at).is_some());
        assert_eq!(model.ephemeral_keys(), [a, c, b], "from above it goes after the row");
        let at = DropAt::before(&model, a).unwrap();
        assert!(drop_tab(&mut model, &core, a, at).is_none());

        assert!(drop_tab(&mut model, &core, a, DropAt::zone(TabKind::Pinned, None, None)).is_some());
        assert_eq!(model.tab(a).unwrap().kind, TabKind::Pinned);
        let at = DropAt::before(&model, a).unwrap();
        assert!(drop_tab(&mut model, &core, b, at).is_some());
        assert_eq!(model.tab(b).unwrap().kind, TabKind::Pinned, "a row of another zone takes the tab into it");
        assert!(drop_tab(&mut model, &core, b, DropAt::zone(TabKind::Tile, None, None)).is_some());
        assert_eq!(model.tab(b).unwrap().kind, TabKind::Tile);

        let folder = new_folder(&mut model, &core, None).unwrap();
        model.toggle_folder(folder);
        assert!(drop_tab(&mut model, &core, a, DropAt::zone(TabKind::Pinned, Some(folder), None)).is_some());
        assert_eq!(model.tab(a).unwrap().folder_id, Some(folder));
        assert!(model.expanded.contains(&folder), "the folder opens to show it");

        let group = new_group(&mut model, &core, None).unwrap();
        assert!(drop_tab(&mut model, &core, a, DropAt::zone(TabKind::Ephemeral, None, Some(group))).is_some());
        assert_eq!((model.tab(a).unwrap().kind, model.tab(a).unwrap().group_id), (TabKind::Ephemeral, Some(group)));
        assert!(drop_tab(&mut model, &core, a, DropAt::zone(TabKind::Ephemeral, None, None)).is_some());
        assert_eq!(model.tab(a).unwrap().group_id, None, "the loose tabs take it out of its group");
    }

    #[test]
    fn the_ring_shows_the_saved_share_of_downloads_with_a_known_size() {
        assert_eq!(saved_share([(50, Some(100)), (50, None), (150, Some(300))]), Some(0.5));
        assert_eq!(saved_share([(10, None)]), None);
        assert_eq!(saved_share([]), None);
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
    fn touch_phases_bound_one_gesture_and_natural_scrolling_flips_it() {
        let mut s = Swipe::natural(true);
        assert_eq!(s.touch(Phase::Moved, -100., 0.), None, "momentum without a started gesture");
        s.touch(Phase::Started, 0., 0.);
        assert_eq!(s.touch(Phase::Moved, -40., 0.), None);
        assert_eq!(s.touch(Phase::Moved, -40., 5.), Some(Step::Next));
        assert_eq!(s.touch(Phase::Moved, -200., 0.), None, "the rest of the gesture");
        s.touch(Phase::Ended, 0., 0.);
        assert_eq!(s.touch(Phase::Moved, 70., 0.), None, "after the fingers lift");
        s.touch(Phase::Started, 0., 0.);
        assert_eq!(s.touch(Phase::Moved, 70., 0.), Some(Step::Previous));
        s.touch(Phase::Started, 0., 0.);
        assert_eq!(s.touch(Phase::Moved, 0., 80.), None, "a vertical scroll");
        assert_eq!(s.touch(Phase::Moved, -200., 0.), None, "stays vertical until the next gesture");
        s.touch(Phase::Started, 0., 0.);
        assert_eq!(s.touch(Phase::Moved, -70., 40.), None, "too steep");

        let mut s = Swipe::natural(false);
        s.touch(Phase::Started, 0., 0.);
        assert_eq!(s.touch(Phase::Moved, -70., 0.), Some(Step::Previous));
        let mut s = Swipe::natural(true);
        assert_eq!(s.feed(-70., 0., Instant::now()), Some(Step::Next), "timed wheel gestures follow the setting too");
    }

    #[test]
    fn a_swipe_reaches_only_spaces_that_exist() {
        let (_dir, core, mut model) = opened();
        let any = |_: &SpaceRef| true;
        assert_eq!(neighbour_space(&model, Step::Previous, any), None);
        assert_eq!(neighbour_space(&model, Step::Next, any), None);
        let profile = model.spaces[0].space.profile_id;
        core.disk.create_space(profile, "Work", "").unwrap();
        core.disk.create_space(profile, "Play", "").unwrap();
        model.reload_spaces(&core);
        assert_eq!(neighbour_space(&model, Step::Next, any), Some(1));
        let not_work = |s: &SpaceRef| s.space.name != "Work";
        assert_eq!(neighbour_space(&model, Step::Next, not_work), Some(2), "a space the process may not show is stepped over");
        assert_eq!(neighbour_space(&model, Step::Next, |_| false), None);
    }

    #[test]
    fn the_space_bar_shrinks_to_dots_and_then_scrolls() {
        let bar = SpaceBar { width: 150., icon: 28., gap: 2. };
        let plain = SpaceStrip { compact: false, scroll: None };
        assert_eq!(bar.strip(0, 0), plain);
        assert_eq!(bar.strip(5, 0), plain, "five icons with their gaps fill it exactly");
        assert_eq!(bar.strip(6, 0), SpaceStrip { compact: true, scroll: None });
        assert_eq!(bar.strip(8, 0), SpaceStrip { compact: true, scroll: None }, "one icon and seven dots with gaps fit");
        assert_eq!(bar.strip(9, 0), SpaceStrip { compact: true, scroll: Some(0.) }, "the dots' gaps count");
        assert_eq!(bar.strip(20, 10), SpaceStrip { compact: true, scroll: Some(99.) }, "a middle space is centred");
        assert_eq!(bar.strip(20, 19), SpaceStrip { compact: true, scroll: Some(182.) }, "the last space ends the strip");
        assert_eq!(bar.strip(20, 25), bar.strip(20, 19), "an index past the end is the last space");
    }
}
