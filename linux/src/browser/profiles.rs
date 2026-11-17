use browser_core::profiles::{self, NewProfile};
use browser_core::tabs::TabKey;
use store::{Id, Profile};

use super::Browser;

pub type SpaceKey = (bool, Id);

impl Browser {
    pub fn saved_profiles(&self) -> Vec<Profile> {
        self.core.disk.profiles().unwrap_or_default()
    }

    pub fn current_profile(&self) -> Option<Id> {
        self.model.current_space().filter(|s| !s.private).map(|s| s.space.profile_id)
    }

    pub fn create_profile(&mut self, new: &NewProfile) -> Result<(), String> {
        let named = browser_core::space_card::valid_name(new.name, []).is_some();
        let name =
            profiles::profile_name(&self.core.disk, new.name, None).ok_or(if named { "Choose a name no other profile has." } else { "Name the profile." })?;
        let space = browser_core::space_card::valid_name(new.space[0], []).ok_or("Name the profile's first space.")?;
        let new = NewProfile { name: name.as_str(), space: [space.as_str(), new.space[1], new.space[2]], ..*new };
        let id = profiles::create(&self.core.disk, &new, &uuid::Uuid::new_v4().to_string()).map_err(|e| format!("The profile could not be created ({e})."))?;
        self.model.reload_spaces(&self.core);
        if let Some(ix) = self.model.spaces.iter().position(|s| !s.private && s.space.profile_id == id) {
            self.switch_space_by_user(ix);
        }
        Ok(())
    }

    pub fn edit_profile(&mut self, id: Id, name: &str, color: &str) -> bool {
        let Some(name) = profiles::profile_name(&self.core.disk, name, Some(id)) else { return false };
        if self.core.disk.set_profile(id, &name, color).is_err() {
            return false;
        }
        if let Some(rt) = self.core.profiles.borrow_mut().get_mut(&(false, id)) {
            rt.profile.name = name;
            rt.profile.color = color.into();
        }
        self.model.reload_spaces(&self.core);
        true
    }

    pub fn switch_profile(&mut self, id: Id) {
        if let Some(ix) = self.model.spaces.iter().position(|s| !s.private && s.space.profile_id == id) {
            self.switch_space_by_user(ix);
        }
    }

    pub fn delete_profile(&mut self, id: Id) -> bool {
        if !profiles::deletable(&self.core.disk, id) {
            return false;
        }
        let keys: Vec<TabKey> = self.pages.keys().copied().filter(|k| !k.private && self.model.profile_of(&self.core, *k) == Some(id)).collect();
        let Some(deleted) = profiles::delete(&self.core.disk, id) else { return false };
        for key in keys {
            self.forget_tab(key);
        }
        crate::extensions::unload(id, &deleted.own_store);
        browser_core::ext_package::remove_tree(&self.extensions_root.join(id.to_string())).ok();
        if let Some(rt) = self.core.profiles.borrow_mut().remove(&(false, id))
            && !deleted.still_used
        {
            self.released.push(rt.data.key);
        }
        self.favicons.forget(&(false, id));
        self.core.wiped_at.borrow_mut().remove(&(false, id));
        if !deleted.still_used {
            profiles::set_retired(&self.core.disk, &deleted.store, true);
            self.retired.push(deleted.store);
        }
        self.model.reload_spaces(&self.core);
        self.model.closed_tabs.retain(&self.model.spaces);
        self.after_removal(0);
        true
    }

    pub fn delete_space(&mut self, (private, id): SpaceKey) {
        let Some((ix, keys)) = self.model.space_to_delete(&self.core, private, id) else { return };
        for key in keys {
            self.forget_tab(key);
        }
        if self.core.store(private).delete_space(id).is_err() {
            return;
        }
        self.model.reload_spaces(&self.core);
        self.model.closed_tabs.retain(&self.model.spaces);
        self.after_removal(ix.saturating_sub(1));
    }

    fn after_removal(&mut self, fallback: usize) {
        if self.model.current_space().is_some() {
            self.model.reload_space(&self.core);
        } else {
            self.switch_space(fallback);
        }
    }

    pub fn space_number(&mut self, n: usize) {
        if let Some(ix) = n.checked_sub(1).filter(|ix| *ix < self.model.spaces.len()) {
            self.switch_space_by_user(ix);
        }
    }

    #[cfg(test)]
    pub fn new_profile(&mut self) {
        let n = self.saved_profiles().len() + 1;
        let name = format!("Profile {n}");
        let new = NewProfile {
            name: &name,
            color: "blue",
            level: shield::ShieldLevel::Standard,
            engine: browser_core::command::ENGINES[0].name,
            share: None,
            space: ["Home", "", ""],
        };
        self.create_profile(&new).ok();
    }
}
