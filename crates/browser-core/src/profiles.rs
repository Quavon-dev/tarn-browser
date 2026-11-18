use shield::ShieldLevel;
use store::{Id, Store};

use crate::command::engine_key;
use crate::{share_store, shared_key};

pub const RETIRED_STORES: &str = "retired_stores";

pub const LOCKDOWN_WIPE: &str =
    "Lockdown deletes the cookies and site data this profile has stored, except data it shares with another profile. Open pages reload.";

pub fn lockdown_question(profile: &str) -> String {
    format!("Use Lockdown for the profile \u{201C}{profile}\u{201D}?")
}

/// Whether choosing `chosen` for a profile now at `current` deletes its stored website data:
/// Lockdown entered in a profile that keeps data.
pub fn shield_wipes_data(chosen: ShieldLevel, private: bool, current: ShieldLevel) -> bool {
    chosen == ShieldLevel::Lockdown && !private && current != ShieldLevel::Lockdown
}

pub fn profile_name(disk: &Store, raw: &str, editing: Option<Id>) -> Option<String> {
    let others = disk.profiles().unwrap_or_default().into_iter().filter(|p| Some(p.id) != editing).collect::<Vec<_>>();
    crate::space_card::valid_name(raw, others.iter().map(|p| p.name.as_str()))
}

pub fn name_taken<'a>(raw: &str, taken: impl IntoIterator<Item = &'a str>) -> bool {
    crate::space_card::valid_name(raw, []).is_some() && crate::space_card::valid_name(raw, taken).is_none()
}

pub struct NewProfile<'a> {
    pub name: &'a str,
    pub color: &'a str,
    pub level: ShieldLevel,
    pub engine: &'a str,
    pub share: Option<Id>,
    pub space: [&'a str; 3],
}

pub fn create(disk: &Store, new: &NewProfile, uuid: &str) -> Result<(Id, Id), store::Error> {
    let [space, color, icon] = new.space;
    disk.batch(|disk| {
        let shared = new.share.and_then(|id| disk.profile(id).ok()).map(|mut p| {
            share_store(disk, &mut p);
            p.data_store_uuid
        });
        let id = disk.create_profile(new.name, uuid, false)?.id;
        disk.set_profile(id, new.name, new.color)?;
        disk.set_profile_shield_level(id, new.level)?;
        disk.set_setting(&shared_key(id), shared.as_deref().unwrap_or(""))?;
        disk.set_setting(&engine_key(false, id), new.engine)?;
        let space_id = disk.create_space(id, space, icon)?.id;
        disk.update_space(space_id, space, color, icon)?;
        Ok((id, space_id))
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_entering_lockdown_in_a_profile_that_keeps_data_wipes_it() {
        assert!(shield_wipes_data(ShieldLevel::Lockdown, false, ShieldLevel::Standard));
        assert!(!shield_wipes_data(ShieldLevel::Strict, false, ShieldLevel::Standard));
        assert!(!shield_wipes_data(ShieldLevel::Lockdown, false, ShieldLevel::Lockdown), "already in Lockdown");
        assert!(!shield_wipes_data(ShieldLevel::Lockdown, true, ShieldLevel::Standard), "a private space keeps nothing");
    }

    #[test]
    fn a_name_is_taken_only_when_another_profile_has_it() {
        assert!(name_taken(" work ", ["Home", "Work"]));
        assert!(!name_taken("Work", ["Home"]));
        assert!(!name_taken("  ", ["Home"]), "an empty name is missing, not taken");
    }
    #[test]
    fn profiles_are_created_with_their_first_space_and_deleted_with_their_settings() {
        let disk = Store::open_in_memory().unwrap();
        let first = disk.create_profile("Personal", "uuid-a", false).unwrap();
        assert!(!deletable(&disk, first.id), "the last profile stays");
        assert_eq!(delete(&disk, first.id), None);
        let new =
            NewProfile { name: "Work", color: "blue", level: ShieldLevel::Strict, engine: "Kagi", share: Some(first.id), space: ["Desk", "green", "code"] };
        let (id, _) = create(&disk, &new, "uuid-b").unwrap();
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
        let (id, _) = create(&disk, &own, "uuid-c").unwrap();
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
}
