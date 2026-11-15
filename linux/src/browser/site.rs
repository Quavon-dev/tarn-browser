use browser_core::tabs::TabKey;
use serde_json::{Map, Value, json};
use shield::lists;

use super::{Browser, pages::Request};

impl Browser {
    pub fn styles(&self, key: TabKey) -> Value {
        let Some(profile) = self.model.profile_of(&self.core, key) else { return json!({}) };
        let store = self.core.store(key.private);
        let styles = engine::page_scripts::site_styles(store.site_css(profile).unwrap_or_default(), store.dark_sites(profile).unwrap_or_default());
        Value::Object(styles.into_iter().map(|(h, css)| (h, Value::from(css))).collect())
    }

    pub fn zaps(&self, key: TabKey) -> Value {
        let Some(profile) = self.model.profile_of(&self.core, key).filter(|_| !key.private) else { return json!({}) };
        let mut map = Map::new();
        for (host, selector) in self.core.disk.zapped_elements(profile).unwrap_or_default() {
            if let Value::Array(list) = map.entry(host).or_insert_with(|| Value::Array(Vec::new())) {
                list.push(Value::from(selector));
            }
        }
        Value::Object(map)
    }

    fn site_profile(&self) -> Option<(TabKey, store::Id, String)> {
        let key = self.model.active?;
        Some((key, self.model.profile_of(&self.core, key)?, self.site_host()?))
    }

    pub fn site_dark(&self) -> bool {
        self.site_profile().is_some_and(|(key, p, host)| self.core.store(key.private).dark_sites(p).unwrap_or_default().contains(&host))
    }

    pub fn site_css(&self) -> String {
        self.site_profile().and_then(|(key, p, host)| self.core.store(key.private).site_css_for(p, &host).ok()).unwrap_or_default()
    }

    pub fn toggle_site_dark(&mut self) {
        let Some((key, profile, host)) = self.site_profile() else { return };
        let dark = !self.site_dark();
        if self.core.store(key.private).set_site_dark(profile, &host, dark).is_ok() {
            self.reload_host(&host);
        }
    }

    pub fn save_site_css(&mut self, css: &str) -> Result<(), String> {
        let Some((key, profile, host)) = self.site_profile() else { return Ok(()) };
        if css.len() > store::MAX_SITE_CSS {
            return Err(format!("CSS over {} KiB", store::MAX_SITE_CSS / 1024));
        }
        self.core.store(key.private).set_site_css(profile, &host, css).map_err(|e| format!("Could not save the site's CSS ({e})."))?;
        self.reload_host(&host);
        Ok(())
    }

    fn reload_host(&mut self, host: &str) {
        let keys: Vec<TabKey> = self
            .model
            .lifecycle
            .live()
            .filter(|k| self.page_url(*k).and_then(|u| browser_core::site::site_host(&u).map(str::to_owned)).as_deref() == Some(host))
            .collect();
        for key in keys {
            self.outbox.push((key, "reload", Value::Null));
        }
    }

    pub fn zappable(&self) -> bool {
        self.model.active.is_some_and(|k| !k.private && self.unlocked(k).is_some())
    }

    pub fn zap_element(&mut self) {
        let Some(key) = self.model.active.filter(|_| self.zappable()) else {
            return self.show_toast("Zap Element works on web pages outside private spaces and Lockdown", None);
        };
        let seq = self.next_seq();
        self.requests.insert(seq, Request::Pick(key));
        self.outbox.push((key, "pick", json!({ "seq": seq })));
    }

    pub(super) fn zap_picked(&mut self, key: TabKey, selector: &str) {
        let Some(url) = self.unlocked(key).filter(|_| !key.private && lists::valid_zap(selector)) else { return };
        let (Some(host), Some(profile)) = (url.host_str(), self.model.profile_of(&self.core, key)) else { return };
        let disk = &self.core.disk;
        let count = disk.zapped_elements(profile).unwrap_or_default().iter().filter(|(h, _)| h == host).count();
        if count < lists::MAX_ZAPS_PER_SITE {
            disk.zap_element(profile, host, selector).ok();
        }
    }

    pub fn zapped(&self, host: Option<&str>) -> Vec<(String, String)> {
        let Some((false, profile)) = self.model.profile() else { return Vec::new() };
        let mut zaps = self.core.disk.zapped_elements(profile).unwrap_or_default();
        zaps.retain(|(h, _)| host.is_none_or(|host| h == host));
        zaps
    }

    pub fn unzap(&mut self, host: &str, selector: Option<&str>) {
        if let Some((false, profile)) = self.model.profile() {
            self.core.disk.unzap_element(profile, host, selector).ok();
        }
    }
}
