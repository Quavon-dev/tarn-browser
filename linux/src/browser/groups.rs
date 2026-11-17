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
    pub(super) fn regrouped(&mut self) {
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
        let at = browser_core::sidebar::DropAt::zone(TabKind::Ephemeral, None, Some(id));
        if browser_core::sidebar::drop_tab(&mut self.model, &self.core, key, at).is_some() {
            self.regrouped();
        }
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
        let store = self.core.store(private);
        let (tabs, groups) = match (store.tabs(space_id), store.groups(space_id)) {
            (Ok(tabs), Ok(groups)) => (tabs, groups),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("tarn: Clean Up could not read the space: {e}");
                return self.show_toast_as(browser_core::toast::Tone::Warning, "Could not sort the tabs", None);
            }
        };
        let sort = cleanup::plan(&tabs, &groups, |t| self.current_url(TabKey { private, id: t.id }).unwrap_or_else(|| t.url.clone()));
        let applied = if sort.changed { cleanup::apply(self.core.store(private), space_id, &sort).map(Some) } else { Ok(None) };
        if let Err(e) = &applied {
            eprintln!("tarn: Clean Up could not sort the tabs: {e}");
        }
        let text = sort.summary(applied.is_ok(), archived, closed::archive_after(&self.core.disk).is_some());
        let tone = if applied.is_ok() { browser_core::toast::Tone::Info } else { browser_core::toast::Tone::Warning };
        let undo = applied.ok().flatten();
        if sort.changed {
            self.model.reload_space(&self.core);
            self.regrouped();
        }
        self.show_toast_as(tone, text, undo.map(|undo| super::ToastAction::UndoSort(Box::new((private, undo)))));
    }

    /// Puts back what Clean Up sorted, in whichever space it sorted.
    pub fn undo_sort(&mut self, (private, undo): (bool, cleanup::Undo)) {
        match cleanup::undo(self.core.store(private), &undo) {
            Ok(dropped) => self.forget_groups(private, &dropped),
            Err(e) => {
                eprintln!("tarn: Clean Up could not undo the sort: {e}");
                self.show_toast_as(browser_core::toast::Tone::Warning, "Could not undo the sort", None);
            }
        }
        self.model.reload_space(&self.core);
        self.regrouped();
    }

    fn current_url(&self, key: TabKey) -> Option<String> {
        self.pages.get(&key).map(|p| p.url.clone()).filter(|u| !u.is_empty())
    }
}
