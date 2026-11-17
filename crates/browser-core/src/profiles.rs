use shield::ShieldLevel;
use store::{Id, Store};

use crate::command::{ENGINES, engine_key};
use crate::{share_store, shared_key};

pub struct NewProfile<'a> {
    pub name: &'a str,
    pub color: &'a str,
    pub level: ShieldLevel,
    pub engine: usize,
    pub share: Option<Id>,
}

/// Writes a profile, its settings and its first space, given as `[name, color, icon]`, all or nothing.
pub fn create(disk: &Store, profile: &NewProfile, space: [&str; 3]) -> Result<(Id, Id), store::Error> {
    let shared = profile.share.and_then(|id| disk.profile(id).ok()).map(|mut p| {
        share_store(disk, &mut p);
        p.data_store_uuid
    });
    let engine = ENGINES.get(profile.engine).unwrap_or(&ENGINES[0]).name;
    disk.batch(|disk| {
        let id = disk.create_profile(profile.name, &uuid::Uuid::new_v4().to_string(), false)?.id;
        disk.set_profile(id, profile.name, profile.color)?;
        disk.set_profile_shield_level(id, profile.level)?;
        disk.set_setting(&shared_key(id), shared.as_deref().unwrap_or(""))?;
        disk.set_setting(&engine_key(false, id), engine)?;
        let space_id = disk.create_space(id, space[0], space[2])?.id;
        disk.update_space(space_id, space[0], space[1], space[2])?;
        Ok((id, space_id))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_profile_gets_its_settings_and_first_space() {
        let disk = Store::open_in_memory().unwrap();
        let home = disk.create_profile("Personal", "home-uuid", false).unwrap();
        let new = NewProfile { name: "Work", color: "green", level: ShieldLevel::Strict, engine: 1, share: Some(home.id) };
        let (id, space) = create(&disk, &new, ["Desk", "blue", "briefcase"]).unwrap();
        let mut work = disk.profile(id).unwrap();
        assert_eq!((work.name.as_str(), work.color.as_str(), work.shield_level), ("Work", "green", ShieldLevel::Strict));
        share_store(&disk, &mut work);
        assert_eq!(work.data_store_uuid, "home-uuid");
        assert_eq!(disk.setting(&engine_key(false, id)).unwrap().as_deref(), Some(ENGINES[1].name));
        let desk = disk.spaces(id).unwrap();
        assert_eq!((desk[0].id, desk[0].name.as_str(), desk[0].color.as_str(), desk[0].icon.as_str()), (space, "Desk", "blue", "briefcase"));
    }
}
