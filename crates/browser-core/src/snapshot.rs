//! Named snapshots of a space's open tabs and groups, saved per profile and restored on request.
//! A snapshot holds addresses, titles and grouping only: no page state, no cookies.

use engine::nav_policy::{self, Decision, Source};
use serde::{Deserialize, Serialize};
use store::{Id, Store, TabKind};
use url::Url;

use crate::tabs::SpaceRef;

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotTab {
    pub url: String,
    pub title: String,
    /// An index into the snapshot's groups.
    pub group: Option<usize>,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotGroup {
    pub name: String,
    pub color: String,
    #[serde(default)]
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceSnapshot {
    version: u32,
    pub tabs: Vec<SnapshotTab>,
    pub groups: Vec<SnapshotGroup>,
}

#[derive(Deserialize)]
struct Header {
    version: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LoadError {
    Missing,
    Unreadable,
    /// Saved by a newer Tarn whose format this one does not know.
    NewerVersion,
}

impl SpaceSnapshot {
    /// The space's unpinned tabs in sidebar order and the groups they are in.
    pub fn capture(store: &Store, space_id: Id) -> Result<Self, store::Error> {
        let groups = store.groups(space_id)?;
        let mut tabs: Vec<_> = store.tabs(space_id)?.into_iter().filter(|t| t.kind == TabKind::Ephemeral && !t.url.is_empty()).collect();
        tabs.sort_by_key(|t| t.position);
        let tabs = tabs
            .into_iter()
            .map(|t| SnapshotTab { group: t.group_id.and_then(|id| groups.iter().position(|g| g.id == id)), url: t.url, title: t.title, position: t.position })
            .collect();
        let groups = groups.into_iter().map(|g| SnapshotGroup { name: g.name, color: g.color, icon: g.icon }).collect();
        Ok(SpaceSnapshot { version: VERSION, tabs, groups })
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(data: &[u8]) -> Result<Self, LoadError> {
        let header: Header = serde_json::from_slice(data).map_err(|_| LoadError::Unreadable)?;
        if header.version > VERSION {
            return Err(LoadError::NewerVersion);
        }
        if header.version != VERSION {
            return Err(LoadError::Unreadable);
        }
        let snapshot: Self = serde_json::from_slice(data).map_err(|_| LoadError::Unreadable)?;
        if snapshot.tabs.iter().any(|t| t.group.is_some_and(|g| g >= snapshot.groups.len())) {
            return Err(LoadError::Unreadable);
        }
        Ok(snapshot)
    }

    pub fn load(store: &Store, profile_id: Id, id: Id) -> Result<Self, LoadError> {
        Self::decode(&store.snapshot_data(profile_id, id).map_err(|_| LoadError::Missing)?)
    }

    /// Appends the tabs to the space in their saved order, each saved group as a new group holding
    /// its tabs, and returns the last tab. Addresses pass the same gate as other restored tabs, so
    /// a snapshot never opens what sync could not.
    pub fn restore(&self, store: &Store, space_id: Id) -> Option<Id> {
        let mut tabs: Vec<(&SnapshotTab, Url)> = self
            .tabs
            .iter()
            .filter_map(|t| Url::parse(&t.url).ok().filter(|u| nav_policy::decide(u, Source::Sync) == Decision::Allow).map(|u| (t, u)))
            .collect();
        tabs.sort_by_key(|(t, _)| t.position);
        let mut groups: Vec<Option<Id>> = vec![None; self.groups.len()];
        let mut last = None;
        for (saved, url) in tabs {
            let Ok(tab) = store.create_tab(space_id, TabKind::Ephemeral, None, url.as_str(), &saved.title) else { continue };
            if let Some(ix) = saved.group {
                let (made, group) = (&mut groups[ix], &self.groups[ix]);
                if made.is_none()
                    && let Ok(created) = store.create_group(space_id, &group.name)
                {
                    store.set_group_color(created.id, &group.color).ok();
                    crate::icon::set_group(store, created.id, &group.icon).ok();
                    *made = Some(created.id);
                }
                if let Some(id) = *made {
                    store.set_tab_group(tab.id, Some(id)).ok();
                }
            }
            last = Some(tab.id);
        }
        last
    }
}

/// Saves the space's tabs under `name`; private spaces and spaces without tabs save nothing.
pub fn save(store: &Store, space: &SpaceRef, name: &str, now: i64) -> Option<store::Snapshot> {
    if space.private {
        return None;
    }
    let snapshot = SpaceSnapshot::capture(store, space.space.id).ok().filter(|s| !s.tabs.is_empty())?;
    store.create_snapshot(space.space.profile_id, name, now, &snapshot.encode()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (Store, SpaceRef) {
        let store = Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "u", false).unwrap().id;
        let space = store.create_space(profile, "Home", "").unwrap();
        (store, SpaceRef { private: false, space })
    }

    fn listing(store: &Store, space_id: Id) -> Vec<(String, Option<[String; 3]>)> {
        let groups = store.groups(space_id).unwrap();
        let mut tabs: Vec<_> = store.tabs(space_id).unwrap().into_iter().filter(|t| t.kind == TabKind::Ephemeral).collect();
        tabs.sort_by_key(|t| t.position);
        tabs.into_iter()
            .map(|t| (t.url, t.group_id.map(|id| groups.iter().find(|g| g.id == id).map(|g| [g.name.clone(), g.color.clone(), g.icon.clone()]).unwrap())))
            .collect()
    }

    #[test]
    fn a_saved_space_comes_back_with_the_same_tabs_order_and_groups() {
        let (store, space) = setup();
        let id = space.space.id;
        let work = store.create_group(id, "Work").unwrap();
        store.set_group_color(work.id, "green").unwrap();
        crate::icon::set_group(&store, work.id, "briefcase").unwrap();
        store.create_group(id, "Empty").unwrap();
        for url in ["https://a.test/", "https://b.test/", "https://c.test/"] {
            store.create_tab(id, TabKind::Ephemeral, None, url, "T").unwrap();
        }
        let tabs = store.tabs(id).unwrap();
        store.set_tab_group(tabs[0].id, Some(work.id)).unwrap();
        store.set_tab_group(tabs[2].id, Some(work.id)).unwrap();
        store.move_tab(tabs[2].id, TabKind::Ephemeral, None, 0).unwrap();
        store.create_tab(id, TabKind::Pinned, None, "https://pinned.test/", "P").unwrap();
        store.set_interaction_state(tabs[1].id, Some(b"state")).unwrap();

        let saved = save(&store, &space, "Morning", 5).unwrap();
        let data = store.snapshot_data(space.space.profile_id, saved.id).unwrap();
        assert!(!String::from_utf8_lossy(&data).contains("pinned.test"), "pinned tabs stay with their space");
        let snapshot = SpaceSnapshot::load(&store, space.space.profile_id, saved.id).unwrap();
        assert_eq!(SpaceSnapshot::decode(&snapshot.encode()).unwrap(), snapshot);

        let copy = store.create_space(space.space.profile_id, "Copy", "").unwrap().id;
        let last = snapshot.restore(&store, copy).unwrap();
        assert_eq!(listing(&store, copy), listing(&store, id));
        assert_eq!(store.tab(last).unwrap().url, "https://b.test/");
        assert_eq!(store.groups(copy).unwrap().len(), 1, "a group no saved tab is in is not recreated");
        assert!(store.tabs(copy).unwrap().iter().all(|t| store.interaction_state(t.id).unwrap().is_none()));

        snapshot.restore(&store, id).unwrap();
        assert_eq!(listing(&store, id).len(), 6, "restoring into a space adds to its tabs");
    }

    #[test]
    fn restored_addresses_pass_the_navigation_gate() {
        let (store, space) = setup();
        let tab = |url: &str| SnapshotTab { url: url.into(), title: String::new(), group: None, position: 0 };
        let snapshot = SpaceSnapshot {
            version: VERSION,
            tabs: vec![tab("javascript:alert(1)"), tab("file:///etc/passwd"), tab("not a url"), tab("https://ok.test/"), tab("tarn://settings")],
            groups: Vec::new(),
        };
        snapshot.restore(&store, space.space.id).unwrap();
        assert_eq!(listing(&store, space.space.id), [("https://ok.test/".to_owned(), None)]);
        let blocked = SpaceSnapshot { tabs: vec![tab("data:text/html,x")], ..snapshot };
        assert_eq!(blocked.restore(&store, space.space.id), None);
    }

    #[test]
    fn unknown_versions_and_broken_data_are_refused() {
        assert_eq!(SpaceSnapshot::decode(br#"{"version":2,"tabs":[],"groups":[],"split":true}"#), Err(LoadError::NewerVersion));
        assert_eq!(SpaceSnapshot::decode(br#"{"version":0,"tabs":[],"groups":[]}"#), Err(LoadError::Unreadable));
        assert_eq!(SpaceSnapshot::decode(br#"{"tabs":[],"groups":[]}"#), Err(LoadError::Unreadable));
        assert_eq!(SpaceSnapshot::decode(b"\xff"), Err(LoadError::Unreadable));
        let bad_group = br#"{"version":1,"tabs":[{"url":"https://a.test/","title":"","group":0,"position":0}],"groups":[]}"#;
        assert_eq!(SpaceSnapshot::decode(bad_group), Err(LoadError::Unreadable));
        assert!(SpaceSnapshot::decode(br#"{"version":1,"tabs":[],"groups":[]}"#).is_ok());
        let (store, _) = setup();
        assert_eq!(SpaceSnapshot::load(&store, 1, 99), Err(LoadError::Missing));
    }

    #[test]
    fn private_and_empty_spaces_save_nothing() {
        let (store, space) = setup();
        assert!(save(&store, &space, "Empty", 0).is_none());
        store.create_tab(space.space.id, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
        let private = SpaceRef { private: true, space: space.space.clone() };
        assert!(save(&store, &private, "Private", 0).is_none());
        assert!(store.snapshots(space.space.profile_id).unwrap().is_empty());
        assert!(save(&store, &space, "Kept", 0).is_some());
    }
}
