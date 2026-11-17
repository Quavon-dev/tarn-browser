use shield::ShieldLevel;
use store::{Id, Schedule, Store};

use crate::command::engine_key;
use crate::tabs::SpaceRef;
use crate::{share_store, shared_key};

pub const MAX_NAME: usize = 48;

pub const RETIRED_STORES: &str = "retired_stores";

pub fn valid_name<'a>(raw: &str, taken: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let name: String = raw.trim().chars().filter(|c| !c.is_control() && !crate::text::is_bidi(*c)).take(MAX_NAME).collect();
    let name = name.trim_end().to_owned();
    let free = taken.into_iter().all(|t| !t.trim().eq_ignore_ascii_case(&name));
    (!name.is_empty() && free).then_some(name)
}

pub fn space_name(raw: &str, spaces: &[SpaceRef], private: bool, profile: Option<Id>, editing: Option<Id>) -> Option<String> {
    let taken = spaces.iter().filter(|s| s.private == private && Some(s.space.profile_id) == profile && Some(s.space.id) != editing);
    valid_name(raw, taken.map(|s| s.space.name.as_str()))
}

pub fn next_space_name(spaces: &[SpaceRef], profile: Option<Id>) -> String {
    let count = spaces.iter().filter(|s| !s.private && Some(s.space.profile_id) == profile).count();
    let mut names = (count + 1..).map(|n| format!("Space {n}"));
    names.find(|n| space_name(n, spaces, false, profile, None).is_some()).unwrap_or_default()
}

pub fn profile_name(disk: &Store, raw: &str, editing: Option<Id>) -> Option<String> {
    let others = disk.profiles().unwrap_or_default().into_iter().filter(|p| Some(p.id) != editing).collect::<Vec<_>>();
    valid_name(raw, others.iter().map(|p| p.name.as_str()))
}

pub struct NewProfile<'a> {
    pub name: &'a str,
    pub color: &'a str,
    pub level: ShieldLevel,
    pub engine: &'a str,
    pub share: Option<Id>,
    pub space: [&'a str; 3],
}

pub fn create(disk: &Store, new: &NewProfile, uuid: &str) -> Result<Id, store::Error> {
    let shared = new.share.and_then(|id| disk.profile(id).ok()).map(|mut p| {
        share_store(disk, &mut p);
        p.data_store_uuid
    });
    let id = disk.create_profile(new.name, uuid, false)?.id;
    let [space, color, icon] = new.space;
    let written = disk
        .set_profile(id, new.name, new.color)
        .and_then(|_| disk.set_profile_shield_level(id, new.level))
        .and_then(|_| disk.set_setting(&shared_key(id), shared.as_deref().unwrap_or("")))
        .and_then(|_| disk.set_setting(&engine_key(false, id), new.engine))
        .and_then(|_| disk.create_space(id, space, icon))
        .and_then(|s| disk.update_space(s.id, space, color, icon));
    if let Err(e) = written {
        disk.delete_profile(id).ok();
        return Err(e);
    }
    Ok(id)
}

pub fn deletable(disk: &Store, id: Id) -> bool {
    let profiles = disk.profiles().unwrap_or_default();
    profiles.len() > 1 && profiles.iter().any(|p| p.id == id)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Deleted {
    pub own_store: String,
    pub store: String,
    pub still_used: bool,
}

pub fn delete(disk: &Store, id: Id) -> Option<Deleted> {
    let profiles = disk.profiles().ok()?;
    let mut profile = profiles.iter().find(|p| p.id == id).cloned().filter(|_| profiles.len() > 1)?;
    let own_store = profile.data_store_uuid.clone();
    share_store(disk, &mut profile);
    disk.delete_profile(id).ok()?;
    for key in [shared_key(id), engine_key(false, id), crate::theme::pages_setting(id)] {
        disk.delete_setting(&key).ok();
    }
    let still_used = profiles.into_iter().filter(|p| p.id != id).any(|mut p| {
        share_store(disk, &mut p);
        p.data_store_uuid == profile.data_store_uuid
    });
    Some(Deleted { own_store, store: profile.data_store_uuid, still_used })
}

pub fn retired_stores(disk: &Store) -> Vec<String> {
    let listed = disk.setting(RETIRED_STORES).ok().flatten().unwrap_or_default();
    listed.lines().filter(|u| !u.is_empty()).map(str::to_owned).collect()
}

pub fn set_retired(disk: &Store, uuid: &str, on: bool) {
    let listed = disk.setting(RETIRED_STORES).ok().flatten().unwrap_or_default();
    disk.set_setting(RETIRED_STORES, &toggled(&listed, uuid, on)).ok();
}

fn toggled(stored: &str, entry: &str, on: bool) -> String {
    let mut entries: Vec<&str> = stored.lines().filter(|p| !p.is_empty() && *p != entry).collect();
    if on {
        entries.push(entry);
    }
    entries.join("\n")
}

pub struct SpaceEdit<'a> {
    pub editing: Option<Id>,
    pub profile: Id,
    pub name: &'a str,
    pub color: &'a str,
    pub icon: &'a str,
    pub schedule: Option<Schedule>,
}

pub fn save_space(store: &Store, edit: &SpaceEdit) -> Result<Id, store::Error> {
    match edit.editing {
        Some(id) => {
            store.update_space(id, edit.name, edit.color, edit.icon).and_then(|_| edit.schedule.map_or(Ok(()), |s| store.set_space_schedule(id, s))).map(|_| id)
        }
        None => store.create_space(edit.profile, edit.name, edit.icon).and_then(|s| store.update_space(s.id, edit.name, edit.color, edit.icon).map(|_| s.id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::Space;

    fn space(id: Id, profile_id: Id, name: &str, private: bool) -> SpaceRef {
        SpaceRef {
            private,
            space: Space { id, profile_id, name: name.into(), icon: String::new(), position: 0, color: String::new(), schedule: Schedule::default() },
        }
    }

    #[test]
    fn names_are_trimmed_cleaned_and_unique() {
        assert_eq!(valid_name("  Work  ", []), Some("Work".into()));
        assert_eq!(valid_name("   ", []), None);
        assert_eq!(valid_name("Wo\u{202e}rk\n", []), Some("Work".into()));
        assert_eq!(valid_name("work", ["Personal", " Work"]), None);
        assert_eq!(valid_name(&"x".repeat(200), []).map(|n| n.chars().count()), Some(MAX_NAME));
    }

    #[test]
    fn space_names_are_unique_within_their_profile_only() {
        let spaces = [space(1, 1, "Work", false), space(2, 2, "Home", false), space(1, 9, "Private", true)];
        assert_eq!(space_name(" work ", &spaces, false, Some(1), None), None);
        assert_eq!(space_name("Work", &spaces, false, Some(1), Some(1)), Some("Work".into()));
        assert_eq!(space_name("Work", &spaces, false, Some(2), None), Some("Work".into()));
        assert_eq!(space_name("Work", &spaces, false, None, None), Some("Work".into()));
        assert_eq!(space_name("private", &spaces, false, Some(9), None), Some("private".into()));
        assert_eq!(space_name("private", &spaces, true, Some(9), None), None);
        assert_eq!(space_name("  ", &spaces, false, Some(1), None), None);
        let named = [space(1, 1, "Home", false), space(2, 1, "Space 3", false)];
        assert_eq!(next_space_name(&named, Some(1)), "Space 4");
        assert_eq!(next_space_name(&named, Some(2)), "Space 1");
    }

    #[test]
    fn profiles_are_created_with_their_first_space_and_deleted_with_their_settings() {
        let disk = Store::open_in_memory().unwrap();
        let first = disk.create_profile("Personal", "uuid-a", false).unwrap();
        assert!(!deletable(&disk, first.id), "the last profile stays");
        assert_eq!(delete(&disk, first.id), None);
        let new =
            NewProfile { name: "Work", color: "blue", level: ShieldLevel::Strict, engine: "Kagi", share: Some(first.id), space: ["Desk", "green", "code"] };
        let id = create(&disk, &new, "uuid-b").unwrap();
        let made = disk.profile(id).unwrap();
        assert_eq!((made.name.as_str(), made.color.as_str(), made.shield_level), ("Work", "blue", ShieldLevel::Strict));
        assert_eq!(disk.setting(&engine_key(false, id)).unwrap().as_deref(), Some("Kagi"));
        let spaces = disk.spaces(id).unwrap();
        assert_eq!(spaces.len(), 1);
        assert_eq!((spaces[0].name.as_str(), spaces[0].color.as_str(), spaces[0].icon.as_str()), ("Desk", "green", "code"));
        assert_eq!(profile_name(&disk, " work ", None), None);
        assert_eq!(profile_name(&disk, "Work", Some(id)), Some("Work".into()));
        assert!(deletable(&disk, id) && !deletable(&disk, 999));
        assert_eq!(delete(&disk, id), Some(Deleted { own_store: "uuid-b".into(), store: "uuid-a".into(), still_used: true }));
        assert!(disk.profile(id).is_err());
        assert_eq!(disk.setting(&shared_key(id)).unwrap(), None);
        assert_eq!(disk.setting(&engine_key(false, id)).unwrap(), None);
        let own = NewProfile { share: None, ..new };
        let id = create(&disk, &own, "uuid-c").unwrap();
        assert_eq!(delete(&disk, id), Some(Deleted { own_store: "uuid-c".into(), store: "uuid-c".into(), still_used: false }));
    }

    #[test]
    fn retired_stores_are_listed_until_removed() {
        let disk = Store::open_in_memory().unwrap();
        set_retired(&disk, "a", true);
        set_retired(&disk, "b", true);
        set_retired(&disk, "a", true);
        assert_eq!(retired_stores(&disk), ["b", "a"]);
        set_retired(&disk, "b", false);
        assert_eq!(retired_stores(&disk), ["a"]);
    }

    #[test]
    fn spaces_are_created_and_edited_with_their_look() {
        let disk = Store::open_in_memory().unwrap();
        let p = disk.create_profile("Personal", "uuid-a", false).unwrap();
        let edit = SpaceEdit { editing: None, profile: p.id, name: "Work", color: "red", icon: "briefcase", schedule: None };
        let id = save_space(&disk, &edit).unwrap();
        let schedule = Schedule { days: 0b011_1110, start: 9, end: 17 };
        save_space(&disk, &SpaceEdit { editing: Some(id), name: "Office", color: "", icon: "code", schedule: Some(schedule), ..edit }).unwrap();
        let saved = disk.spaces(p.id).unwrap().into_iter().find(|s| s.id == id).unwrap();
        assert_eq!((saved.name.as_str(), saved.color.as_str(), saved.icon.as_str(), saved.schedule), ("Office", "", "code", schedule));
    }
}
