use browser_core::command::host_tail;
use browser_core::tabs::TabKey;
use engine::nav_policy::{self, Decision, Source};
use store::{Id, TabKind};
use url::Url;

use super::{Browser, START_URL};

impl Browser {
    pub fn tab_url(&self, key: TabKey) -> Option<String> {
        let page = self.pages.get(&key).map(|p| p.url.clone()).filter(|u| !u.is_empty());
        page.or_else(|| self.model.tab(key).map(|t| t.url.clone()))
    }

    fn tab_host(&self, key: TabKey) -> Option<String> {
        Url::parse(&self.tab_url(key)?).ok()?.host_str().map(str::to_owned)
    }

    pub fn duplicate_tab(&mut self, key: TabKey) {
        let Some(url) = self.tab_url(key).filter(|u| Url::parse(u).is_ok()) else { return };
        // A copy of a file the user opened shows that file too.
        match Url::parse(&url).ok().filter(|_| self.pages.get(&key).and_then(|p| p.opened_file.as_deref()) == Some(url.as_str())) {
            Some(file) => self.open_file(&file),
            None => drop(self.new_tab(&url)),
        }
    }

    pub fn pinned_drift(&self, key: TabKey) -> Option<Url> {
        self.model.pinned_drift(key, &self.tab_url(key)?)
    }

    pub fn return_to_pinned(&mut self, key: TabKey) {
        let Some(url) = self.pinned_drift(key).filter(|u| nav_policy::decide(u, Source::Sync) == Decision::Allow) else { return };
        self.activate(key);
        self.load(key, url.as_str());
    }

    pub fn set_pinned_url(&mut self, key: TabKey) {
        if let Some(url) = self.tab_url(key) {
            self.core.store(key.private).set_pinned_url(key.id, &url).ok();
            self.model.reload_space(&self.core);
        }
    }

    pub fn rename_tab(&mut self, key: TabKey, typed: &str) {
        let typed = typed.trim();
        let Some(tab) = self.model.tab(key).filter(|t| !typed.is_empty() && t.kind != TabKind::Tile) else { return };
        let title = self.pages.get(&key).map(|p| p.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| tab.title.clone());
        self.core.store(key.private).set_tab_custom_title(key.id, &browser_core::tabs::custom_title(typed, &title)).ok();
        self.model.reload_space(&self.core);
    }

    pub fn reset_tab_title(&mut self, key: TabKey) {
        self.core.store(key.private).set_tab_custom_title(key.id, "").ok();
        self.model.reload_space(&self.core);
    }

    pub fn sleeping(&self, key: TabKey) -> bool {
        self.model.active != Some(key) && !self.elsewhere.contains(&key) && !self.model.lifecycle.live().any(|k| k == key)
    }

    pub fn sleep_tabs(&mut self, keys: Vec<TabKey>) {
        let awake: Vec<TabKey> = keys.into_iter().filter(|k| !self.sleeping(*k) && !self.elsewhere.contains(k) && !self.in_split(*k)).collect();
        for key in awake {
            self.sleep_tab(key);
        }
    }

    pub fn can_sleep_others(&self, key: TabKey) -> bool {
        self.model.other_tabs(key).into_iter().any(|k| !self.sleeping(k) && !self.elsewhere.contains(&k) && self.model.active != Some(k))
    }

    pub fn sleep_inactive_tabs(&mut self) {
        let keys = self.model.tabs.iter().map(|t| self.model.key(t)).collect();
        self.sleep_tabs(keys);
    }

    pub(super) fn unload_tab(&mut self, key: TabKey) {
        self.splits.leave(key);
        self.model.lifecycle.remove(key);
        self.unload(key);
        if self.reader.as_ref().is_some_and(|r| r.key == key) {
            self.close_reader();
        }
    }

    pub fn close_others(&mut self, key: TabKey) {
        let keys = self.model.ephemeral_keys().into_iter().filter(|k| *k != key).collect();
        self.close_tabs(keys);
    }

    pub fn tabs_below(&self, key: TabKey) -> Vec<TabKey> {
        self.model.ephemeral_keys().into_iter().skip_while(|k| *k != key).skip(1).collect()
    }

    pub fn close_below(&mut self, key: TabKey) {
        let keys = self.tabs_below(key);
        self.close_tabs(keys);
    }

    pub fn close_all_tabs(&mut self) {
        let keys = self.model.ephemeral_keys();
        self.close_tabs(keys);
    }

    pub fn reopen_closed(&mut self, serial: u64) {
        let Some((ix, closed)) = self.model.closed_tabs.take(serial, &self.model.spaces) else { return };
        self.restore_closed(ix, closed);
    }

    pub fn recently_closed(&self) -> Vec<(u64, String, String)> {
        self.model.profile().map(|(private, profile)| self.model.closed_tabs.recent(private, profile)).unwrap_or_default()
    }

    pub fn delete_folder(&mut self, id: Id) {
        let Some((private, _)) = self.model.profile() else { return };
        let (keys, tree) = self.model.folder_contents(id);
        let active = self.model.active.filter(|a| keys.contains(a));
        for key in &keys {
            self.forget_tab(*key);
        }
        if self.core.store(private).delete_folder(id).is_err() {
            return;
        }
        self.model.expanded.retain(|f| !tree.contains(f));
        self.model.reload_space(&self.core);
        if active.is_some() {
            self.model.active = None;
            self.pick_next();
        }
    }

    pub(super) fn pick_next(&mut self) {
        match self.model.pick(|k| self.elsewhere.contains(&k)) {
            Some(next) => self.activate(next),
            None => {
                self.new_tab(START_URL);
            }
        }
    }

    fn saved_mutes(&self, key: TabKey) -> Vec<String> {
        self.model.profile_of(&self.core, key).and_then(|p| self.core.store(key.private).muted_sites(p).ok()).unwrap_or_default()
    }

    pub fn muted_with(&self, key: TabKey, saved: &[String]) -> bool {
        let Some(host) = self.tab_host(key) else { return false };
        saved.contains(&host) || self.pages.get(&key).is_some_and(|p| p.muted_host.as_deref() == Some(host.as_str()))
    }

    pub fn tab_muted(&self, key: TabKey) -> bool {
        self.muted_with(key, &self.saved_mutes(key))
    }

    pub fn can_mute(&self, key: TabKey) -> bool {
        self.tab_host(key).is_some()
    }

    pub fn toggle_site_muted(&mut self, key: TabKey) {
        let Some(url) = self.tab_url(key).and_then(|u| Url::parse(&u).ok()) else { return };
        let (Some(profile), Some(host)) = (self.model.profile_of(&self.core, key), url.host_str().map(str::to_owned)) else { return };
        let muted = !self.tab_muted(key);
        let save = browser_core::site::saves_mute(muted, self.level_for(key, url.as_str()));
        self.set_site_muted(key.private, profile, &host, muted, save);
        let shown = host_tail(host.trim_start_matches("www."), 40);
        self.show_toast(if muted { format!("Muted {shown}") } else { format!("Unmuted {shown}") }, None);
    }

    pub fn muted_sites(&self) -> Vec<String> {
        self.model.profile().and_then(|(private, p)| self.core.store(private).muted_sites(p).ok()).unwrap_or_default()
    }

    pub fn unmute_site(&mut self, host: &str) {
        if let Some((private, profile)) = self.model.profile() {
            self.set_site_muted(private, profile, host, false, true);
        }
    }

    fn set_site_muted(&mut self, private: bool, profile: Id, host: &str, muted: bool, save: bool) {
        if save {
            self.core.store(private).set_site_muted(profile, host, muted).ok();
        }
        let keys: Vec<TabKey> = self
            .pages
            .keys()
            .copied()
            .filter(|k| k.private == private && self.model.profile_of(&self.core, *k) == Some(profile) && self.tab_host(*k).as_deref() == Some(host))
            .collect();
        for key in keys {
            if let Some(page) = self.pages.get_mut(&key) {
                page.muted_host = (muted && !save).then(|| host.to_owned());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use shield::ShieldLevel;

    use super::*;

    fn browser() -> (tempfile::TempDir, Browser) {
        let dir = tempfile::tempdir().unwrap();
        let b = Browser::open(dir.path());
        (dir, b)
    }

    fn visit(b: &mut Browser, key: TabKey, url: &str) {
        b.on_event(key, "committed", &json!({ "url": url, "title": "T" }));
        b.on_event(key, "finished", &json!({ "url": url, "title": "T" }));
    }

    #[test]
    fn closing_a_pinned_tab_unloads_it_and_keeps_it() {
        let (_dir, mut b) = browser();
        let pinned = b.model.active.unwrap();
        visit(&mut b, pinned, "https://mail.test/");
        b.pin_toggle(pinned);
        let other = b.new_tab("https://b.test/").unwrap();
        b.activate(pinned);
        b.close_tab(pinned);
        assert!(b.model.tab(pinned).is_some(), "the pinned tab stays in the sidebar");
        assert!(b.sleeping(pinned));
        assert_eq!(b.model.active, Some(other));
        assert!(b.toast.is_none() && b.model.closed_tabs.newest().is_none(), "nothing to reopen");
    }

    #[test]
    fn close_others_below_and_all_spare_pinned_tabs() {
        let (_dir, mut b) = browser();
        let pinned = b.model.active.unwrap();
        b.pin_toggle(pinned);
        let tab = |b: &mut Browser, n: u8| b.new_tab(&format!("https://{n}.test/")).unwrap();
        let (one, two, three) = (tab(&mut b, 1), tab(&mut b, 2), tab(&mut b, 3));
        assert_eq!(b.tabs_below(one), [two, three]);
        b.close_below(two);
        assert!(b.model.tab(three).is_none() && b.model.tab(two).is_some());
        b.close_others(one);
        assert!(b.model.tab(two).is_none() && b.model.tab(one).is_some() && b.model.tab(pinned).is_some());
        b.close_all_tabs();
        assert!(b.model.ephemeral_keys().is_empty() && b.model.tab(pinned).is_some());
    }

    #[test]
    fn a_chosen_closed_tab_reopens_and_the_others_stay_listed() {
        let (_dir, mut b) = browser();
        let a = b.new_tab("https://a.test/").unwrap();
        visit(&mut b, a, "https://a.test/");
        let c = b.new_tab("https://c.test/").unwrap();
        visit(&mut b, c, "https://c.test/");
        b.close_tab(a);
        b.close_tab(c);
        let recent = b.recently_closed();
        assert_eq!(recent.iter().map(|(_, _, url)| url.as_str()).collect::<Vec<_>>(), ["https://c.test/", "https://a.test/"]);
        b.reopen_closed(recent[1].0);
        assert_eq!(b.active_page().map(|p| p.url.as_str()), Some("https://a.test/"));
        assert_eq!(b.recently_closed().len(), 1);
    }

    #[test]
    fn a_tab_is_renamed_reset_duplicated_and_returns_to_its_pinned_url() {
        let (_dir, mut b) = browser();
        let key = b.model.active.unwrap();
        visit(&mut b, key, "https://mail.test/inbox");
        b.rename_tab(key, " Mail ");
        assert_eq!(b.model.tab(key).unwrap().custom_title, "Mail");
        b.rename_tab(key, "T");
        assert_eq!(b.model.tab(key).unwrap().custom_title, "", "the page's own title follows the page");
        b.rename_tab(key, "Mail");
        b.reset_tab_title(key);
        assert_eq!(b.model.tab(key).unwrap().custom_title, "");
        b.pin_toggle(key);
        assert!(b.pinned_drift(key).is_none());
        visit(&mut b, key, "https://mail.test/settings");
        assert_eq!(b.pinned_drift(key).map(String::from).as_deref(), Some("https://mail.test/inbox"));
        b.duplicate_tab(key);
        let copy = b.model.active.unwrap();
        assert_ne!(copy, key);
        assert_eq!(b.pages[&copy].load.as_ref().map(|(u, _)| u.as_str()), Some("https://mail.test/settings"));
        b.return_to_pinned(key);
        assert_eq!(b.model.active, Some(key));
        assert_eq!(b.pages[&key].load.as_ref().map(|(u, _)| u.as_str()), Some("https://mail.test/inbox"));
        b.set_pinned_url(key);
        assert!(b.pinned_drift(key).is_none());
    }

    #[test]
    fn a_muted_site_is_saved_per_profile_and_a_lockdown_mute_lasts_on_the_host() {
        let (_dir, mut b) = browser();
        let key = b.model.active.unwrap();
        visit(&mut b, key, "https://music.test/a");
        b.toggle_site_muted(key);
        assert!(b.tab_muted(key));
        assert_eq!(b.muted_sites(), ["music.test"]);
        let other = b.new_tab("https://music.test/b").unwrap();
        visit(&mut b, other, "https://music.test/b");
        assert!(crate::snapshot::views(&b, true).iter().all(|v| v.muted), "every view of the host is muted");
        b.unmute_site("music.test");
        assert!(!b.tab_muted(key) && b.muted_sites().is_empty());
        b.activate(key);
        b.set_site_level(Some(ShieldLevel::Lockdown));
        b.toggle_site_muted(key);
        assert!(b.tab_muted(key) && b.tab_muted(other), "a Lockdown mute covers the host's open tabs");
        assert!(b.muted_sites().is_empty(), "and is not saved");
        visit(&mut b, key, "https://elsewhere.test/");
        visit(&mut b, key, "https://music.test/a");
        assert!(!b.tab_muted(key), "leaving the host ends it");
    }

    #[test]
    fn sleeping_spares_the_active_tab_and_deleting_a_folder_drops_its_tabs() {
        let (_dir, mut b) = browser();
        let first = b.model.active.unwrap();
        let second = b.new_tab("https://b.test/").unwrap();
        assert!(b.can_sleep_others(second));
        b.sleep_tabs(b.model.other_tabs(second));
        assert!(b.sleeping(first) && !b.sleeping(second));
        assert!(!b.can_sleep_others(second));
        b.sleep_inactive_tabs();
        assert!(!b.sleeping(second));
        b.new_folder(None);
        let folder = b.model.folders[0].id;
        b.move_to_folder(second, folder);
        b.delete_folder(folder);
        assert!(b.model.folders.is_empty() && b.model.tab(second).is_none());
        assert!(b.model.active.is_some_and(|k| b.model.tab(k).is_some()));
    }
}
