use browser_core::tabs::TabKey;
use serde_json::json;
use shield::ShieldLevel;
use store::Id;

use super::{Browser, Notice};
use crate::data::StoreKey;

#[derive(Clone, Debug, PartialEq)]
pub struct Wipe {
    pub key: StoreKey,
    pub remove: Option<String>,
}

impl Browser {
    fn on(&self, key: TabKey, private: bool, profile: Id, host: Option<&str>) -> bool {
        key.private == private
            && self.model.profile_of(&self.core, key) == Some(profile)
            && host.is_none_or(|h| self.page_url(key).and_then(|u| browser_core::site::site_host(&u).map(str::to_owned)).as_deref() == Some(h))
    }

    fn live_on(&self, private: bool, profile: Id, host: Option<&str>) -> Vec<TabKey> {
        self.model.lifecycle.live().filter(|k| self.on(*k, private, profile, host)).collect()
    }

    pub(super) fn store_site_level(&self, private: bool, profile: Id, host: &str, level: Option<ShieldLevel>) -> bool {
        if self.core.set_site_override(private, profile, host, level).is_err() {
            return false;
        }
        if !private {
            crate::extensions::levels_changed(profile);
        }
        true
    }

    pub fn site_overrides(&self) -> Vec<(String, ShieldLevel)> {
        let Some(profile) = self.model.profile() else { return Vec::new() };
        let profiles = self.core.profiles.borrow();
        let Some(rt) = profiles.get(&profile) else { return Vec::new() };
        let mut list: Vec<_> = rt.overrides.borrow().iter().map(|(h, l)| (h.clone(), *l)).collect();
        list.sort();
        list
    }

    pub fn remove_site_override(&mut self, host: &str) {
        let Some((private, profile)) = self.model.profile() else { return };
        if !self.store_site_level(private, profile, host, None) {
            return;
        }
        self.reload_level(private, profile, host);
    }

    pub fn reload_level(&mut self, private: bool, profile: Id, host: &str) {
        for key in self.live_on(private, profile, Some(host)) {
            let url = self.pages.get(&key).map(|p| p.url.clone()).unwrap_or_default();
            self.load(key, &url);
        }
    }

    pub fn site_zooms(&self) -> Vec<(String, f64)> {
        let Some((private, profile)) = self.model.profile() else { return Vec::new() };
        self.core.store(private).site_zooms(profile).unwrap_or_default()
    }

    pub fn remove_site_zoom(&mut self, host: Option<&str>) {
        let Some((private, profile)) = self.model.profile() else { return };
        let store = self.core.store(private);
        match host {
            Some(host) => store.set_site_zoom(profile, host, None),
            None => store.clear_site_zooms(profile),
        }
        .ok();
        self.reset_zooms(private, profile, host);
    }

    pub fn reset_zooms(&mut self, private: bool, profile: Id, host: Option<&str>) {
        let keys: Vec<TabKey> = self.pages.keys().copied().filter(|k| self.on(*k, private, profile, host)).collect();
        for key in keys {
            if let Some(page) = self.pages.get_mut(&key).filter(|p| p.zoom > 0.0 && (p.zoom - 1.0).abs() > 0.001) {
                page.zoom = 1.0;
                self.outbox.push((key, "zoom", json!({ "factor": 1.0 })));
            }
        }
    }

    pub fn export_profile(&self) -> Result<(String, String), String> {
        let Some((false, profile)) = self.model.profile() else { return Err("private data is not saved".into()) };
        let name = self.core.disk.profile(profile).map_err(|e| e.to_string())?.name;
        let json = self.core.disk.export_profile(profile).map_err(|e| e.to_string())?;
        Ok((browser_core::profile_data::export_name(&name), json))
    }

    pub fn forget_profile(&mut self, private: bool, profile: Id) {
        self.favicons.forget(&(private, profile));
        self.text_found.clear();
        self.model.closed_tabs.clear_profile(private, profile);
        self.sync_shared();
    }

    pub fn delete_profile_data(&mut self) -> Vec<Wipe> {
        let Some((private, profile)) = self.model.profile() else { return Vec::new() };
        let current = self.core.profiles.borrow().get(&(private, profile)).map(|rt| {
            let locked = (!private && rt.profile.shield_level == ShieldLevel::Lockdown).then(|| StoreKey::Persistent(rt.profile.data_store_uuid.clone()));
            (rt.data.key.clone(), locked)
        });
        let reset = match self.core.reset_profile_data(private, profile) {
            Ok(reset) => reset,
            Err(e) => {
                self.notice = Some(Notice::Message(format!("Deleting failed ({e}).")));
                return Vec::new();
            }
        };
        let wipes = if reset.fresh_store.is_some() {
            self.core.profiles.borrow_mut().remove(&(false, profile));
            let key = current.map_or_else(|| StoreKey::Persistent(reset.own_store.clone()), |(key, _)| key);
            vec![Wipe { key, remove: Some(reset.own_store) }]
        } else {
            if let Some(rt) = self.core.profiles.borrow().get(&(private, profile)) {
                rt.overrides.borrow_mut().clear();
                rt.rules.clear();
            }
            current.into_iter().flat_map(|(key, locked)| [Some(key), locked]).flatten().map(|key| Wipe { key, remove: None }).collect()
        };
        self.downloads.retain(|d| d.state == "started");
        self.forget_profile(private, profile);
        if !private {
            crate::extensions::restart(profile);
        }
        if let Some(ix) = self.model.spaces.iter().position(|s| s.private == private && s.space.id == reset.space.id) {
            self.switch_space(ix);
        }
        self.notice = Some(Notice::Message("All data of this profile was deleted.".into()));
        wipes
    }
}
