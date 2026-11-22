use shield::ShieldLevel;
use store::{Id, Space, Store};

use crate::{Core, WebsiteData, now, shared_key};

pub fn store_is_shared(disk: &Store, id: Id, own_store: &str) -> bool {
    disk.setting(&shared_key(id)).ok().flatten().is_some_and(|u| !u.is_empty())
        || disk.profiles().unwrap_or_default().iter().any(|p| p.id != id && disk.setting(&shared_key(p.id)).ok().flatten().as_deref() == Some(own_store))
}

pub fn export_name(profile_name: &str) -> String {
    engine::downloads::sanitize_filename(&format!("Tarn export {profile_name}.json"))
}

#[derive(Debug)]
pub struct Reset {
    pub space: Space,
    pub own_store: String,
    pub fresh_store: Option<String>,
}

impl<D: WebsiteData> Core<D> {
    pub fn reset_profile_data(&self, private: bool, profile_id: Id) -> Result<Reset, store::Error> {
        let store = self.store(private);
        let space = store.reset_profile(profile_id, if private { "Private" } else { "Home" })?;
        self.wiped_at.borrow_mut().insert((private, profile_id), now());
        self.snippets_changed();
        let own_store = store.profile(profile_id).map(|p| p.data_store_uuid).unwrap_or_default();
        let fresh_store = (!private && !store_is_shared(&self.disk, profile_id, &own_store))
            .then(|| uuid::Uuid::new_v4().to_string())
            .filter(|fresh| self.disk.set_profile_store(profile_id, fresh).is_ok());
        Ok(Reset { space, own_store, fresh_store })
    }

    pub fn set_site_override(&self, private: bool, profile_id: Id, host: &str, level: Option<ShieldLevel>) -> Result<(), store::Error> {
        self.store(private).set_site_override(profile_id, host, level)?;
        if let Some(rt) = self.profiles.borrow().get(&(private, profile_id)) {
            let mut overrides = rt.overrides.borrow_mut();
            match level {
                Some(level) => overrides.insert(host.to_owned(), level),
                None => overrides.remove(host),
            };
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestCore;

    #[test]
    fn a_reset_keeps_the_profile_and_moves_an_unshared_store() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let p = core.disk.profiles().unwrap().remove(0);
        core.disk.add_history(p.id, "https://a.test/", "A", now(), false).unwrap();
        core.disk.set_site_zoom(p.id, "a.test", Some(1.5)).unwrap();
        let reset = core.reset_profile_data(false, p.id).unwrap();
        assert_eq!(reset.own_store, p.data_store_uuid);
        let fresh = reset.fresh_store.unwrap();
        assert_ne!(fresh, p.data_store_uuid);
        assert_eq!(core.disk.profile(p.id).unwrap().data_store_uuid, fresh);
        assert!(core.disk.history(p.id, 10).unwrap().is_empty());
        assert!(core.disk.site_zooms(p.id).unwrap().is_empty());
        assert_eq!(core.disk.spaces(p.id).unwrap().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Home"]);
        assert!(core.wiped_at.borrow().contains_key(&(false, p.id)));
    }

    #[test]
    fn a_shared_or_private_store_stays() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let a = core.disk.profiles().unwrap().remove(0);
        let b = core.disk.create_profile("B", "uuid-b", false).unwrap();
        core.disk.set_setting(&shared_key(b.id), &a.data_store_uuid).unwrap();
        assert!(store_is_shared(&core.disk, a.id, &a.data_store_uuid) && store_is_shared(&core.disk, b.id, "uuid-b"));
        assert!(core.reset_profile_data(false, a.id).unwrap().fresh_store.is_none(), "another profile borrows it");
        assert!(core.reset_profile_data(false, b.id).unwrap().fresh_store.is_none(), "it borrows another's");
        let private = core.store(true).create_profile("P", "uuid-p", true).unwrap();
        let reset = core.reset_profile_data(true, private.id).unwrap();
        assert!(reset.fresh_store.is_none());
        assert_eq!(reset.space.name, "Private");
    }

    #[test]
    fn a_site_override_reaches_the_store_and_an_open_profile() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let p = core.disk.profiles().unwrap().remove(0);
        core.open_profile(false, p.id, || std::rc::Rc::new(|_| {})).unwrap();
        core.set_site_override(false, p.id, "a.test", Some(ShieldLevel::Strict)).unwrap();
        assert!(core.has_override(false, p.id, "a.test"));
        assert_eq!(core.disk.site_overrides(p.id).unwrap(), [("a.test".to_owned(), ShieldLevel::Strict)]);
        core.set_site_override(false, p.id, "a.test", None).unwrap();
        assert!(!core.has_override(false, p.id, "a.test") && core.disk.site_overrides(p.id).unwrap().is_empty());
    }

    #[test]
    fn export_names_are_safe_file_names() {
        assert_eq!(export_name("Work"), "Tarn export Work.json");
        assert!(!export_name("../x/y").contains('/'));
    }
}
