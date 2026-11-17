use browser_core::profiles::{self, NewProfile, SpaceEdit};
use browser_core::tabs::TabKey;
use store::{Id, Profile};

use super::Browser;

pub type SpaceKey = (bool, Id);

pub struct SpaceForm<'a> {
    pub editing: Option<SpaceKey>,
    pub profile: Id,
    pub name: &'a str,
    pub color: &'a str,
    pub icon: &'a str,
}

impl Browser {
    pub fn saved_profiles(&self) -> Vec<Profile> {
        self.core.disk.profiles().unwrap_or_default()
    }

    pub fn current_profile(&self) -> Option<Id> {
        self.model.current_space().filter(|s| !s.private).map(|s| s.space.profile_id)
    }

    pub fn next_space_name(&self, profile: Option<Id>) -> String {
        profiles::next_space_name(&self.model.spaces, profile)
    }

    pub fn create_profile(&mut self, new: &NewProfile) -> Result<(), String> {
        let named = profiles::valid_name(new.name, []).is_some();
        let name =
            profiles::profile_name(&self.core.disk, new.name, None).ok_or(if named { "Choose a name no other profile has." } else { "Name the profile." })?;
        let space = profiles::space_name(new.space[0], &[], false, None, None).ok_or("Name the profile's first space.")?;
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

    pub fn save_space(&mut self, form: &SpaceForm) -> bool {
        let editing = form.editing.filter(|key| self.model.spaces.iter().any(|s| (s.private, s.space.id) == *key));
        if form.editing.is_some() && editing.is_none() {
            return false;
        }
        let private = editing.is_some_and(|(p, _)| p);
        let Some(name) = profiles::space_name(form.name, &self.model.spaces, private, Some(form.profile), editing.map(|(_, id)| id)) else { return false };
        let edit = SpaceEdit { editing: editing.map(|(_, id)| id), profile: form.profile, name: &name, color: form.color, icon: form.icon, schedule: None };
        let Ok(id) = profiles::save_space(self.core.store(private), &edit) else { return false };
        self.model.reload_spaces(&self.core);
        if editing.is_none()
            && let Some(ix) = self.model.spaces.iter().position(|s| !s.private && s.space.id == id)
        {
            self.switch_space_by_user(ix);
        }
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
    pub fn new_space(&mut self) {
        let Some(profile) = self.current_profile() else { return };
        let name = self.next_space_name(Some(profile));
        self.save_space(&SpaceForm { editing: None, profile, name: &name, color: "", icon: "" });
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
