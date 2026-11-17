use browser_core::closed::{self, ClosedGroup};
use browser_core::tabs::TabKey;
use browser_core::{cleanup, now};
use store::{Id, TabKind};

use super::Browser;

impl Browser {
    pub fn new_group(&mut self, tab: Option<TabKey>) -> Option<Id> {
        let group = self.model.new_group(&self.core, tab)?;
        self.model.reload_space(&self.core);
        self.regrouped();
        Some(group.id)
    }

    /// Other windows showing this space reload its groups.
    fn regrouped(&mut self) {
        self.regrouped_space = self.model.current_space().map(|s| (s.private, s.space.id));
    }

    pub fn toggle_group(&mut self, id: Id) {
        self.model.toggle_group(&self.core, id);
    }

    pub fn rename_group(&mut self, id: Id, name: &str) {
        let Some(collapsed) = self.model.groups.iter().find(|g| g.id == id).map(|g| g.collapsed) else { return };
        if !name.trim().is_empty() && self.core.store(self.model.private()).update_group(id, name.trim(), collapsed).is_ok() {
            self.model.reload_space(&self.core);
            self.regrouped();
        }
    }

    pub fn set_group_color(&mut self, id: Id, color: &str) {
        if self.core.store(self.model.private()).set_group_color(id, color).is_ok() {
            self.model.reload_space(&self.core);
            self.regrouped();
        }
    }

    pub fn set_tab_group(&mut self, key: TabKey, group: Option<Id>) {
        if self.core.store(key.private).set_tab_group(key.id, group).is_ok() {
            self.model.reload_space(&self.core);
            self.regrouped();
        }
    }

    /// Moves `key` to the end of the unpinned tabs, into group `id`.
    pub fn move_to_group(&mut self, key: TabKey, id: Id) {
        let index = self.model.zone_len(TabKind::Ephemeral, None);
        let store = self.core.store(key.private);
        if store.move_tab(key.id, TabKind::Ephemeral, None, index).is_ok() {
            store.set_tab_group(key.id, Some(id)).ok();
            self.regrouped();
        }
        self.model.reload_space(&self.core);
    }

    pub fn ungroup(&mut self, id: Id) {
        let private = self.model.private();
        if self.core.store(private).ungroup(id).is_ok() {
            self.forget_groups(private, &[id]);
            self.model.reload_space(&self.core);
        }
    }

    pub fn close_group(&mut self, id: Id) {
        let Some(group) =
            self.model.groups.iter().find(|g| g.id == id).map(|g| ClosedGroup { name: g.name.clone(), color: g.color.clone(), icon: g.icon.clone() })
        else {
            return;
        };
        let private = self.model.private();
        let keys = self.model.tabs.iter().filter(|t| t.group_id == Some(id)).map(|t| self.model.key(t)).collect();
        self.close_batch(keys, Some(group));
        self.core.store(private).delete_group(id).ok();
        self.forget_groups(private, &[id]);
        self.model.reload_space(&self.core);
    }

    /// Deleted groups leave every window's reopen stack, since a new group can take their ids.
    fn forget_groups(&mut self, private: bool, ids: &[Id]) {
        for id in ids {
            self.model.closed_tabs.forget_group(private, *id);
            self.forgotten_groups.push((private, *id));
        }
        self.regrouped();
    }

    pub fn archive_after_index(&self) -> usize {
        closed::archive_after_index(&self.core.disk)
    }

    pub fn set_archive_after(&mut self, ix: usize) {
        let Some((name, _)) = closed::ARCHIVE_AFTER.get(ix) else { return };
        if self.core.disk.set_setting(closed::ARCHIVE_AFTER_SETTING, name).is_ok() {
            self.archive_unused_tabs();
        }
    }

    /// Archives the tabs unused for longer than Settings › Tabs allows; the number archived.
    pub fn archive_unused_tabs(&mut self) -> usize {
        let Some(max_age) = closed::archive_after(&self.core.disk) else { return 0 };
        let (core, now) = (self.core.clone(), now());
        let archives = self.model.archive_unused(&core, max_age, now);
        let changed = !archives.is_empty();
        let mut archived = 0;
        for archive in archives {
            archived += archive.stale().len();
            let private = archive.private();
            let urls = archive.stale().iter().map(|t| self.current_url(TabKey { private, id: t.id }).unwrap_or_else(|| t.url.clone())).collect();
            let emptied = archive.run(&core, now, urls, |key| {
                self.ext_tab_closed(key);
                self.forget_tab(key);
            });
            self.forget_groups(private, &emptied);
        }
        if changed {
            self.model.reload_space(&self.core);
        }
        archived
    }

    /// Sorts the space's loose tabs into groups by what each page is and archives tabs unused for
    /// longer than Settings › Tabs allows, all on this device.
    pub fn clean_up_tabs(&mut self) {
        if self.model.burner_shown() {
            return;
        }
        let archived = self.archive_unused_tabs();
        let Some((private, space_id)) = self.model.current_space().map(|s| (s.private, s.space.id)) else { return };
        let plan = cleanup::plan(&self.model.tabs, &self.model.groups, |t| self.current_url(TabKey { private, id: t.id }).unwrap_or_else(|| t.url.clone()));
        let sorted = cleanup::apply(self.core.store(private), space_id, plan);
        if sorted > 0 {
            self.model.reload_space(&self.core);
            self.regrouped();
        }
        let text = cleanup::summary(sorted, archived, closed::archive_after(&self.core.disk).is_some());
        self.show_toast(text, None);
    }

    fn current_url(&self, key: TabKey) -> Option<String> {
        self.pages.get(&key).map(|p| p.url.clone()).filter(|u| !u.is_empty())
    }
}
