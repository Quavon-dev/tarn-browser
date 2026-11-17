//! Tabs a window closed, which "Reopen Closed Tab" brings back, and the archive of unused tabs.

use store::{Bookmark, Id, Store, Tab, TabKind, WebEngine};

use crate::tabs::{Model, SpaceRef, TabKey};
use crate::{Core, WebsiteData};

const MAX_CLOSED_TABS: usize = 20;
pub const ARCHIVE_FOLDER: &str = "Archived Tabs";
/// The setting that holds Settings › Tabs › "Close unused tabs after", by option name.
pub const ARCHIVE_AFTER_SETTING: &str = "archive_after";
const HOUR: i64 = 3_600_000;
/// The choices for how long an unused tab stays open, in milliseconds; `None` keeps tabs open.
pub const ARCHIVE_AFTER: [(&str, Option<i64>); 4] =
    [("Never", None), ("12 hours", Some(12 * HOUR)), ("1 day", Some(24 * HOUR)), ("7 days", Some(7 * 24 * HOUR))];
/// How often the shells archive unused tabs while running.
pub const ARCHIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);

/// The stored choice's place in `ARCHIVE_AFTER`; "Never" when unset or unknown.
pub fn archive_after_index(store: &Store) -> usize {
    let stored = store.setting(ARCHIVE_AFTER_SETTING).ok().flatten();
    ARCHIVE_AFTER.iter().position(|(name, _)| Some(*name) == stored.as_deref()).unwrap_or(0)
}

/// How long an unused tab stays open, in milliseconds; `None` keeps tabs open.
pub fn archive_after(store: &Store) -> Option<i64> {
    ARCHIVE_AFTER[archive_after_index(store)].1
}

#[derive(Debug)]
pub struct ClosedTab {
    pub url: String,
    pub title: String,
    pub state: Option<Vec<u8>>,
    pub position: i64,
    /// The group's id and name; it goes back into the group only while both match.
    pub group: Option<(Id, String)>,
    pub web_engine: Option<WebEngine>,
}

#[derive(Debug)]
pub struct ClosedGroup {
    pub name: String,
    pub color: String,
    pub icon: String,
}

/// Tabs closed together, such as a group or "Close All Tabs"; they reopen together.
#[derive(Debug)]
pub struct Closed {
    pub private: bool,
    pub profile_id: Id,
    pub space_id: Id,
    pub tabs: Vec<ClosedTab>,
    pub group: Option<ClosedGroup>,
}

impl Closed {
    pub fn space_ix(&self, spaces: &[SpaceRef]) -> Option<usize> {
        spaces.iter().position(|s| (s.private, s.space.profile_id, s.space.id) == (self.private, self.profile_id, self.space_id))
    }

    /// Recreates the tabs at the end of their space, a closed group as a new group holding them,
    /// and returns the last tab. A tab whose group still exists goes back into it; ids are
    /// reused, so the name must match too.
    pub fn restore<D: WebsiteData>(&self, core: &Core<D>) -> Option<Id> {
        let store = core.store(self.private);
        let group = self.group.as_ref().and_then(|g| {
            let group = store.create_group(self.space_id, &g.name).ok()?;
            store.set_group_color(group.id, &g.color).ok();
            crate::icon::set_group(store, group.id, &g.icon).ok();
            Some(group.id)
        });
        let groups = store.groups(self.space_id).unwrap_or_default();
        let mut last = None;
        for closed in &self.tabs {
            let Ok(tab) = store.create_tab(self.space_id, TabKind::Ephemeral, None, &closed.url, &closed.title) else { continue };
            store.set_interaction_state(tab.id, closed.state.as_deref()).ok();
            if closed.web_engine.is_some() {
                store.set_tab_web_engine(tab.id, closed.web_engine).ok();
            }
            let old = closed.group.as_ref().filter(|(id, name)| groups.iter().any(|g| g.id == *id && g.name == *name)).map(|(id, _)| *id);
            if let Some(group) = group.or(old) {
                store.set_tab_group(tab.id, Some(group)).ok();
            }
            last = Some(tab.id);
        }
        last
    }
}

/// Recently closed tabs, newest last; a tab only ever returns to the space it was closed in.
#[derive(Default)]
pub struct ClosedTabs {
    entries: Vec<(u64, Closed)>,
    serial: u64,
}

impl ClosedTabs {
    /// A deleted group's id goes to the next new group, often under the same name, so tabs closed
    /// from it no longer go back into a group.
    pub fn forget_group(&mut self, private: bool, id: Id) {
        for (_, closed) in self.entries.iter_mut().filter(|(_, c)| c.private == private) {
            for tab in closed.tabs.iter_mut().filter(|t| t.group.as_ref().is_some_and(|(g, _)| *g == id)) {
                tab.group = None;
            }
        }
    }

    pub fn push(&mut self, closed: Closed) -> u64 {
        if self.entries.len() == MAX_CLOSED_TABS {
            self.entries.remove(0);
        }
        self.serial += 1;
        self.entries.push((self.serial, closed));
        self.serial
    }

    /// The newest entry whose space still exists, with that space's index; entries of gone spaces are dropped.
    pub fn pop(&mut self, spaces: &[SpaceRef]) -> Option<(usize, Closed)> {
        while let Some((_, closed)) = self.entries.pop() {
            if let Some(ix) = closed.space_ix(spaces) {
                return Some((ix, closed));
            }
        }
        None
    }

    pub fn take(&mut self, serial: u64, spaces: &[SpaceRef]) -> Option<(usize, Closed)> {
        let at = self.entries.iter().position(|(s, _)| *s == serial)?;
        let (_, closed) = self.entries.remove(at);
        Some((closed.space_ix(spaces)?, closed))
    }

    /// The profile's entries, newest first: serial, a title and the first tab's URL.
    pub fn recent(&self, private: bool, profile_id: Id) -> Vec<(u64, String, String)> {
        let entries = self.entries.iter().rev().filter(|(_, c)| (c.private, c.profile_id) == (private, profile_id));
        entries
            .filter_map(|(serial, c)| {
                let first = c.tabs.first()?;
                let title = match (&c.group, c.tabs.len()) {
                    (Some(group), 1) => format!("{} (1 Tab)", group.name),
                    (Some(group), n) => format!("{} ({n} Tabs)", group.name),
                    (None, 1) if !first.title.is_empty() => first.title.clone(),
                    (None, 1) => first.url.clone(),
                    (None, n) => format!("{n} Tabs"),
                };
                Some((*serial, title, first.url.clone()))
            })
            .collect()
    }

    pub fn newest(&self) -> Option<u64> {
        self.entries.last().map(|(serial, _)| *serial)
    }

    /// Drops the tabs of deleted spaces, so a space that later reuses the id never receives them.
    pub fn retain(&mut self, spaces: &[SpaceRef]) {
        self.entries.retain(|(_, c)| c.space_ix(spaces).is_some());
    }

    pub fn clear_profile(&mut self, private: bool, profile_id: Id) {
        self.entries.retain(|(_, c)| (c.private, c.profile_id) != (private, profile_id));
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Changes whenever the profile's entries change: their newest serial and count.
    pub fn signature(&self, private: bool, profile_id: Id) -> (Option<u64>, usize) {
        let mut entries = self.entries.iter().filter(|(_, c)| (c.private, c.profile_id) == (private, profile_id));
        let count = entries.clone().count();
        (entries.next_back().map(|(serial, _)| *serial), count)
    }
}

/// Groups no tab is in.
pub fn empty_groups(groups: &[store::Group], tabs: &[Tab]) -> Vec<Id> {
    groups.iter().filter(|g| !tabs.iter().any(|t| t.group_id == Some(g.id))).map(|g| g.id).collect()
}

/// Unpinned tabs other than the active one, last used before `now - max_age`; tabs never used are kept.
pub fn unused_tabs(tabs: &[Tab], active: Option<Id>, now: i64, max_age: i64) -> impl Iterator<Item = &Tab> {
    tabs.iter().filter(move |t| t.kind == TabKind::Ephemeral && Some(t.id) != active && t.last_active > 0 && t.last_active < now - max_age)
}

/// One space's unused tabs on their way into its profile's "Archived Tabs" folder.
#[derive(Debug)]
pub struct Archive {
    private: bool,
    profile_id: Id,
    space_id: Id,
    folder: Id,
    stale: Vec<Tab>,
}

impl Archive {
    pub fn private(&self) -> bool {
        self.private
    }

    pub fn stale(&self) -> &[Tab] {
        &self.stale
    }

    /// Bookmarks each stale tab's page, `urls` in the order of `stale`, once in the folder; a tab
    /// whose bookmark failed stays. `discard` drops a tab's live state just before its row goes,
    /// and groups the archive left empty go last; returns those groups. Archives nothing unless
    /// `urls` has one entry per stale tab.
    pub fn run<D: WebsiteData>(self, core: &Core<D>, now: i64, urls: Vec<String>, mut discard: impl FnMut(TabKey)) -> Vec<Id> {
        if urls.len() != self.stale.len() {
            return Vec::new();
        }
        let store = core.store(self.private);
        let mut kept: Vec<String> =
            store.bookmarks(self.profile_id).unwrap_or_default().into_iter().filter(|b| b.parent_id == Some(self.folder)).filter_map(|b| b.url).collect();
        for (tab, url) in self.stale.iter().zip(urls) {
            let title = if tab.title.is_empty() { url.clone() } else { tab.title.clone() };
            if !kept.contains(&url) && store.add_bookmark(self.profile_id, Some(self.folder), Some(&url), &title, now).is_err() {
                continue;
            }
            kept.push(url);
            discard(TabKey { private: self.private, id: tab.id });
            store.delete_tab(tab.id).ok();
        }
        let left = store.tabs(self.space_id).unwrap_or_default();
        let mut emptied = Vec::new();
        for group in empty_groups(&store.groups(self.space_id).unwrap_or_default(), &left) {
            if self.stale.iter().any(|t| t.group_id == Some(group)) && store.delete_group(group).is_ok() {
                emptied.push(group);
            }
        }
        emptied
    }
}

impl Model {
    /// The spaces with unpinned tabs unused for `max_age`, so they outlive a restart as bookmarks;
    /// the shell runs each in turn. The active tab is stamped first, so a tab in use never counts
    /// as unused once left.
    pub fn archive_unused<D: WebsiteData>(&self, core: &Core<D>, max_age: i64, now: i64) -> Vec<Archive> {
        if let Some(key) = self.active {
            core.store(key.private).touch_tab(key.id, now).ok();
        }
        let mut archives = Vec::new();
        for space in &self.spaces {
            let (private, profile_id, space_id) = (space.private, space.space.profile_id, space.space.id);
            let store = core.store(private);
            let tabs = store.tabs(space_id).unwrap_or_default();
            let active = self.active.filter(|k| k.private == private).map(|k| k.id);
            let stale: Vec<Tab> = unused_tabs(&tabs, active, now, max_age).cloned().collect();
            let Some(folder) = stale.first().and_then(|_| archive_folder(store, profile_id, now)) else { continue };
            archives.push(Archive { private, profile_id, space_id, folder, stale });
        }
        archives
    }
}

/// The top-level bookmark folder named `name`, such as Archived Tabs or the Reading List.
pub fn top_folder(bookmarks: &[Bookmark], name: &str) -> Option<Id> {
    bookmarks.iter().find(|b| b.url.is_none() && b.parent_id.is_none() && b.title == name).map(|b| b.id)
}

fn archive_folder(store: &Store, profile_id: Id, now: i64) -> Option<Id> {
    let marks = store.bookmarks(profile_id).ok()?;
    top_folder(&marks, ARCHIVE_FOLDER).or_else(|| store.add_bookmark(profile_id, None, None, ARCHIVE_FOLDER, now).ok().map(|b| b.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestCore;
    use store::Space;

    fn space(private: bool, profile_id: Id, id: Id) -> SpaceRef {
        SpaceRef {
            private,
            space: Space { id, profile_id, name: String::new(), icon: String::new(), position: 0, color: String::new(), schedule: Default::default() },
        }
    }

    #[test]
    fn archiving_is_off_by_default_and_follows_the_stored_choice() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(archive_after(&store), None);
        store.set_setting(ARCHIVE_AFTER_SETTING, "1 day").unwrap();
        assert_eq!((archive_after_index(&store), archive_after(&store)), (2, Some(24 * HOUR)));
        store.set_setting(ARCHIVE_AFTER_SETTING, "forever").unwrap();
        assert_eq!(archive_after(&store), None, "an unknown choice keeps tabs open");
    }

    fn tab(url: &str) -> ClosedTab {
        ClosedTab { url: url.into(), title: String::new(), state: None, position: 0, group: None, web_engine: None }
    }

    fn closed(private: bool, profile_id: Id, space_id: Id, url: &str) -> Closed {
        Closed { private, profile_id, space_id, tabs: vec![tab(url)], group: None }
    }

    fn url(closed: &Closed) -> &str {
        &closed.tabs[0].url
    }

    #[test]
    fn the_stack_keeps_the_newest_tabs() {
        let spaces = [space(false, 1, 1)];
        let mut stack = ClosedTabs::default();
        for n in 0..=MAX_CLOSED_TABS {
            stack.push(closed(false, 1, 1, &n.to_string()));
        }
        let mut urls = Vec::new();
        while let Some((ix, tab)) = stack.pop(&spaces) {
            assert_eq!(ix, 0);
            urls.push(url(&tab).to_owned());
        }
        assert_eq!(urls.len(), MAX_CLOSED_TABS);
        assert_eq!((urls[0].as_str(), urls[MAX_CLOSED_TABS - 1].as_str()), ("20", "1"));
    }

    #[test]
    fn the_signature_follows_only_the_profiles_entries() {
        let spaces = [space(false, 1, 1)];
        let mut stack = ClosedTabs::default();
        assert_eq!(stack.signature(false, 1), (None, 0));
        let first = stack.push(closed(false, 1, 1, "a"));
        let middle = stack.push(closed(false, 1, 1, "b"));
        stack.push(closed(false, 1, 1, "c"));
        let before = stack.signature(false, 1);
        stack.push(closed(false, 2, 2, "other"));
        assert_eq!(stack.signature(false, 1), before, "another profile's tab leaves it alone");
        stack.take(middle, &spaces);
        let taken = stack.signature(false, 1);
        assert_ne!(taken, before, "restoring a middle entry changes it");
        for n in 0..MAX_CLOSED_TABS {
            stack.push(closed(false, 2, 2, &n.to_string()));
        }
        assert_ne!(stack.signature(false, 1), taken, "eviction changes it");
        assert!(stack.signature(false, 1).0.is_none_or(|s| s > first));
    }

    #[test]
    fn tabs_return_only_to_their_own_space() {
        let mut stack = ClosedTabs::default();
        stack.push(closed(false, 1, 7, "home"));
        stack.push(closed(true, 3, 7, "private"));
        stack.push(closed(false, 1, 9, "deleted"));
        let spaces = [space(false, 1, 7), space(false, 2, 9), space(true, 4, 7)];
        let (ix, tab) = stack.pop(&spaces).unwrap();
        assert_eq!((ix, url(&tab)), (0, "home"));
        assert!(stack.is_empty());
    }

    #[test]
    fn deleted_spaces_are_pruned() {
        let mut stack = ClosedTabs::default();
        stack.push(closed(false, 1, 1, "kept"));
        stack.push(closed(false, 1, 2, "gone"));
        stack.retain(&[space(false, 1, 1)]);
        let reused = [space(false, 1, 1), space(false, 1, 2)];
        assert_eq!(stack.pop(&reused).map(|(ix, t)| (ix, url(&t).to_owned())), Some((0, "kept".into())));
        assert!(stack.pop(&reused).is_none());
    }

    #[test]
    fn serials_name_the_newest_entry() {
        let mut stack = ClosedTabs::default();
        let first = stack.push(closed(false, 1, 1, "a"));
        let second = stack.push(closed(false, 1, 1, "b"));
        assert!(second > first);
        assert_eq!(stack.newest(), Some(second));
        stack.pop(&[space(false, 1, 1)]);
        assert_eq!(stack.newest(), Some(first));
    }

    #[test]
    fn entries_are_listed_per_profile_and_taken_by_serial() {
        let mut stack = ClosedTabs::default();
        let first = stack.push(closed(false, 1, 1, "https://a.test/"));
        stack.push(closed(false, 2, 2, "https://other.test/"));
        let mut two = closed(false, 1, 1, "https://b.test/");
        two.tabs.push(tab("https://c.test/"));
        let second = stack.push(two);
        let listed = stack.recent(false, 1);
        assert_eq!(listed, [(second, "2 Tabs".into(), "https://b.test/".into()), (first, "https://a.test/".into(), "https://a.test/".into())]);
        let spaces = [space(false, 1, 1)];
        assert_eq!(stack.take(first, &spaces).map(|(ix, c)| (ix, url(&c).to_owned())), Some((0, "https://a.test/".into())));
        assert!(stack.take(first, &spaces).is_none());
        assert_eq!(stack.recent(false, 1).len(), 1);
    }

    #[test]
    fn a_deleted_group_is_forgotten_in_its_own_store() {
        let mut stack = ClosedTabs::default();
        let closed = |private| Closed {
            private,
            profile_id: 1,
            space_id: 1,
            tabs: vec![ClosedTab { group: Some((5, "Group 2".into())), ..tab("https://a.test/") }],
            group: None,
        };
        stack.push(closed(true));
        stack.push(closed(false));
        stack.forget_group(false, 5);
        let group = |stack: &mut ClosedTabs| stack.entries.pop().map(|(_, c)| c.tabs[0].group.clone());
        assert_eq!(group(&mut stack), Some(None), "a new group with the old id and name is not the old one");
        assert_eq!(group(&mut stack), Some(Some((5, "Group 2".into()))), "the other store's ids are its own");
    }

    #[test]
    fn a_closed_group_comes_back_with_its_tabs() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let store = &core.disk;
        let profile = store.create_profile("P", "u", false).unwrap().id;
        let space_id = store.create_space(profile, "Home", "").unwrap().id;
        let kept = store.create_group(space_id, "Kept").unwrap().id;
        let mut loose = tab("https://c.test/");
        loose.group = Some((kept, "Kept".into()));
        let group = Some(ClosedGroup { name: "Work".into(), color: "green".into(), icon: "🧰".into() });
        let closed = Closed { private: false, profile_id: profile, space_id, tabs: vec![tab("https://a.test/"), tab("https://b.test/")], group };
        let last = closed.restore(&core).unwrap();
        let groups = store.groups(space_id).unwrap();
        let work = groups.iter().find(|g| g.name == "Work").unwrap();
        assert_eq!((work.color.as_str(), work.icon.as_str()), ("green", "🧰"));
        let tabs = store.tabs(space_id).unwrap();
        let urls: Vec<&str> = tabs.iter().filter(|t| t.group_id == Some(work.id)).map(|t| t.url.as_str()).collect();
        assert_eq!(urls, ["https://a.test/", "https://b.test/"]);
        assert_eq!(tabs.last().unwrap().id, last);

        let mut renamed = tab("https://d.test/");
        renamed.group = Some((kept, "Old name".into()));
        let single = Closed { private: false, profile_id: profile, space_id, tabs: vec![loose, renamed], group: None };
        single.restore(&core).unwrap();
        let tabs = store.tabs(space_id).unwrap();
        let group_of = |url: &str| tabs.iter().find(|t| t.url == url).unwrap().group_id;
        assert_eq!((group_of("https://c.test/"), group_of("https://d.test/")), (Some(kept), None));
    }

    #[test]
    fn a_reopened_tab_keeps_its_engine() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let store = core.store(true);
        let profile = store.create_profile("P", "u", true).unwrap().id;
        let space_id = store.create_space(profile, "Home", "").unwrap().id;
        let mut chromium = tab("https://a.test/");
        chromium.web_engine = Some(WebEngine::Chromium);
        let closed = Closed { private: true, profile_id: profile, space_id, tabs: vec![chromium, tab("https://b.test/")], group: None };
        closed.restore(&core).unwrap();
        assert!(core.disk.tabs(space_id).unwrap().is_empty(), "a private tab comes back in the private store");
        let engines: Vec<_> = store.tabs(space_id).unwrap().into_iter().map(|t| (t.url, t.web_engine)).collect();
        assert_eq!(engines, [("https://a.test/".into(), Some(WebEngine::Chromium)), ("https://b.test/".into(), None)]);
    }

    #[test]
    fn only_unused_unpinned_inactive_tabs_are_archived() {
        let tab = |id: Id, kind: TabKind, last_active: i64| Tab {
            id,
            space_id: 1,
            folder_id: None,
            kind,
            url: String::new(),
            title: String::new(),
            position: 0,
            last_active,
            group_id: (id == 6).then_some(9),
            pinned_url: None,
            custom_title: String::new(),
            web_engine: None,
        };
        let tabs = [
            tab(1, TabKind::Ephemeral, 100),
            tab(2, TabKind::Ephemeral, 950),
            tab(3, TabKind::Pinned, 100),
            tab(4, TabKind::Tile, 100),
            tab(5, TabKind::Ephemeral, 100),
            tab(6, TabKind::Ephemeral, 100),
            tab(7, TabKind::Ephemeral, 0),
        ];
        let ids: Vec<Id> = unused_tabs(&tabs, Some(5), 1000, 500).map(|t| t.id).collect();
        assert_eq!(ids, [1, 6]);
        let group = |id| store::Group { id, space_id: 1, name: String::new(), position: 0, collapsed: false, color: String::new(), icon: String::new() };
        assert_eq!(empty_groups(&[group(9), group(11)], &tabs), [11]);
    }

    #[test]
    fn unused_tabs_move_into_the_archive_folder_once() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let mut model = Model::new(4);
        model.reload_spaces(&core);
        model.switch_space(&core, 0).unwrap();
        let (profile, space) = (model.spaces[0].space.profile_id, model.spaces[0].space.id);
        let work = core.disk.create_space(profile, "Work", "").unwrap().id;
        model.reload_spaces(&core);
        let tab = |space, url: &str, title: &str, at| {
            let t = core.disk.create_tab(space, TabKind::Ephemeral, None, url, title).unwrap();
            core.disk.touch_tab(t.id, at).unwrap();
            t
        };
        let (old, twin, fresh, active) = (
            tab(space, "https://a.test/", "", 10),
            tab(space, "https://a.test/", "A", 10),
            tab(space, "https://b.test/", "", 990),
            tab(space, "https://c.test/", "", 10),
        );
        let elsewhere = tab(work, "https://a.test/", "", 10);
        let group = core.disk.create_group(space, "G").unwrap().id;
        core.disk.set_tab_group(old.id, Some(group)).unwrap();
        let waiting = core.disk.create_group(space, "Empty").unwrap().id;
        model.active = Some(TabKey { private: false, id: active.id });
        let mut discarded = Vec::new();
        let archives = model.archive_unused(&core, 500, 1000);
        assert_eq!(archives.len(), 2);
        let mut emptied = Vec::new();
        for archive in archives {
            let urls = archive.stale().iter().map(|t| t.url.clone()).collect();
            emptied.extend(archive.run(&core, 1000, urls, |k| {
                assert!(core.disk.tab(k.id).is_ok(), "the shell drops a tab's live state while its row exists");
                discarded.push(k.id);
            }));
        }
        assert_eq!(emptied, [group]);
        assert_eq!(discarded, [old.id, twin.id, elsewhere.id]);
        let left: Vec<Id> = core.disk.tabs(space).unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(left, [fresh.id, active.id]);
        assert!(core.disk.tabs(work).unwrap().is_empty());
        let groups: Vec<Id> = core.disk.groups(space).unwrap().into_iter().map(|g| g.id).collect();
        assert_eq!(groups, [waiting], "a group the archive emptied goes, one already empty stays");
        let marks = core.disk.bookmarks(profile).unwrap();
        let folder = marks.iter().find(|b| b.title == ARCHIVE_FOLDER).unwrap().id;
        let archived: Vec<(&str, &str)> = marks.iter().filter(|b| b.parent_id == Some(folder)).map(|b| (b.url.as_deref().unwrap(), b.title.as_str())).collect();
        assert_eq!(archived, [("https://a.test/", "https://a.test/")], "one bookmark per page across the profile's spaces, titled by its URL when untitled");
        assert_eq!(core.disk.tab(active.id).unwrap().last_active, 1000, "the active tab is stamped");
        assert!(model.archive_unused(&core, 500, 1000).is_empty(), "nothing is unused");
    }
}
