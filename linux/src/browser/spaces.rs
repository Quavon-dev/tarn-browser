use browser_core::now;
use browser_core::snapshot::{LoadError, SpaceSnapshot};
use browser_core::space_card::{SpaceDraft, Unsaved};
use browser_core::tabs::TabKey;
use store::{Id, Snapshot};
use url::Url;

use super::{Browser, START_URL};

pub fn date(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn snapshot_error(e: LoadError) -> &'static str {
    match e {
        LoadError::Missing => "This snapshot no longer exists.",
        LoadError::Unreadable => "This snapshot could not be read.",
        LoadError::NewerVersion => "This snapshot was saved by a newer version of Tarn.",
    }
}

impl Browser {
    pub fn switch_space_by_user(&mut self, ix: usize) {
        if ix != self.model.space_ix {
            self.switched_at = Some(now());
        }
        self.switch_space(ix);
    }

    pub fn schedule_tick(&mut self) {
        let Ok((weekday, hour)) = self.core.disk.local_time() else { return };
        if let Some(ix) = browser_core::schedule::due(&self.model.spaces, self.model.space_ix, weekday, hour, self.switched_at, now()) {
            self.switch_space(ix);
        }
    }

    pub fn new_space_draft(&self) -> SpaceDraft {
        SpaceDraft::new(&self.model.spaces, self.model.current_space(), &self.core.disk.profiles().unwrap_or_default())
    }

    pub fn save_space(&mut self, draft: &SpaceDraft) -> bool {
        let profiles = self.core.disk.profiles().unwrap_or_default();
        let id = match draft.save(&self.core.disk, self.core.store(draft.private), &self.model.spaces, &profiles) {
            Ok(id) => id,
            Err(Unsaved::Invalid) => return false,
            Err(Unsaved::Failed(_)) => {
                self.show_toast_as(browser_core::toast::Tone::Warning, "Tarn could not save this space", None);
                return false;
            }
        };
        self.model.reload_spaces(&self.core);
        if draft.editing.is_none()
            && let Some(ix) = self.model.spaces.iter().position(|s| s.private == draft.private && s.space.id == id)
        {
            self.switch_space(ix);
        }
        true
    }

    pub fn duplicate_tabs(&self) -> Vec<TabKey> {
        let private = self.model.private();
        let url =
            |t: &store::Tab| self.pages.get(&TabKey { private, id: t.id }).map(|p| p.url.clone()).filter(|u| !u.is_empty()).unwrap_or_else(|| t.url.clone());
        browser_core::tabs::duplicates(&self.model.tabs, self.model.active.map(|k| k.id), url).into_iter().map(|id| TabKey { private, id }).collect()
    }

    pub fn close_duplicates(&mut self) {
        let keys = self.duplicate_tabs();
        self.close_tabs(keys);
    }

    pub fn open_burner(&mut self) {
        if !self.model.burner_shown() {
            self.burn();
            let back = self.model.current_space().map(|s| (s.private, s.space.id));
            let Some(ix) = self.model.open_burner(&self.core) else { return };
            self.burner_return = back;
            self.switch_space(ix);
        }
    }

    pub fn leave_burner(&mut self) {
        let back = self.burner_return.take();
        let burner = self.model.burner;
        let ix = self.model.spaces.iter().position(|s| Some((s.private, s.space.id)) == back && Some(s.space.id) != burner);
        let ix = ix.or_else(|| self.model.spaces.iter().position(|s| !s.private)).unwrap_or(0);
        self.switch_space(ix);
    }

    pub(super) fn burn(&mut self) {
        let keys = self.model.burner_tabs(&self.core);
        let profile = self.model.burner.and_then(|space| self.model.spaces.iter().find(|s| s.private && s.space.id == space)).map(|s| s.space.profile_id);
        if let Some(rt) = profile.and_then(|p| self.core.profiles.borrow().get(&(true, p)).map(|rt| rt.data.key.burner())) {
            self.released.push(rt);
        }
        for key in keys {
            self.forget_tab(key);
        }
        if let Some(p) = profile {
            self.favicons.forget(&(true, p));
        }
        self.model.burn(&self.core);
    }

    pub fn routed_space(&self, url: &Url) -> Option<usize> {
        let from_private = self.model.current_space().is_none_or(|s| s.private);
        let space_id = browser_core::routing::route(&self.core.disk.space_rules().ok()?, url, from_private)?;
        self.model.spaces.iter().position(|s| !s.private && s.space.id == space_id).filter(|ix| *ix != self.model.space_ix)
    }

    pub fn open_routed(&mut self, url: &str) -> bool {
        let Some(ix) = Url::parse(url).ok().and_then(|u| self.routed_space(&u)) else { return false };
        let blank = self.model.active.filter(|k| self.pages.get(k).is_none_or(|p| p.url.is_empty() || p.url == START_URL));
        if let Some(key) = blank {
            self.core.store(key.private).delete_tab(key.id).ok();
            self.forget_tab(key);
            self.model.active = None;
            self.model.reload_space(&self.core);
        }
        self.switch_space(ix);
        let reuse = self.model.active.filter(|k| self.pages.get(k).is_none_or(|p| p.url.is_empty() || p.url == START_URL));
        match reuse {
            Some(key) => self.load(key, url),
            None => {
                self.new_tab(url);
            }
        }
        true
    }

    pub fn route_choices(&self, host: &str) -> (Vec<(Id, String)>, Option<Id>) {
        let spaces = self.model.spaces.iter().filter(|s| !s.private).map(|s| (s.space.id, engine::view_policy::title(&s.space.name))).collect();
        let rules = self.core.disk.space_rules().unwrap_or_default();
        (spaces, rules.into_iter().find(|r| r.host_pattern == host).map(|r| r.space_id))
    }

    pub fn space_rule_rows(&self) -> Vec<(Id, String, String)> {
        let name = |id| self.model.spaces.iter().find(|s| !s.private && s.space.id == id).map(|s| engine::view_policy::title(&s.space.name));
        self.core.disk.space_rules().unwrap_or_default().into_iter().filter_map(|r| Some((r.id, r.host_pattern, name(r.space_id)?))).collect()
    }

    pub fn route_site(&mut self, host: &str, space_id: Option<Id>) {
        let disk = &self.core.disk;
        match space_id {
            Some(id) if disk.set_space_rule(host, id).is_err() => self.show_toast_as(browser_core::toast::Tone::Warning, "Tarn could not save this rule", None),
            Some(_) => {}
            None => {
                for rule in disk.space_rules().unwrap_or_default().into_iter().filter(|r| r.host_pattern == host) {
                    disk.delete_space_rule(rule.id).ok();
                }
            }
        }
    }

    pub fn delete_space_rule(&mut self, id: Id) {
        self.core.disk.delete_space_rule(id).ok();
    }

    pub fn save_snapshot(&mut self) {
        let Some(space) = self.model.current_space().cloned() else { return };
        let name = format!("{} {}", space.space.name, date(now()));
        match browser_core::snapshot::save(self.core.store(space.private), &space, &name, now()) {
            Some(_) => self.show_toast_as(browser_core::toast::Tone::Success, format!("Saved “{}”", engine::view_policy::title(&name)), None),
            None => self.show_toast("This space has no tabs to save", None),
        }
    }

    pub fn snapshots(&self) -> Vec<Snapshot> {
        self.model.current_space().filter(|s| !s.private).and_then(|s| self.core.disk.snapshots(s.space.profile_id).ok()).unwrap_or_default()
    }

    pub fn delete_snapshot(&mut self, saved: &Snapshot) {
        self.core.disk.delete_snapshot(saved.profile_id, saved.id).ok();
    }

    pub fn restore_snapshot(&mut self, saved: &Snapshot, new_space: bool) {
        if let Err(message) = self.try_restore(saved, new_space) {
            self.show_toast_as(browser_core::toast::Tone::Warning, message, None);
        }
    }

    fn try_restore(&mut self, saved: &Snapshot, new_space: bool) -> Result<(), &'static str> {
        let Some(current) = self.model.current_space().filter(|s| !s.private && s.space.profile_id == saved.profile_id).map(|s| s.space.id) else {
            return Err("Switch to a space of this snapshot's profile to restore it.");
        };
        let store = &self.core.disk;
        let snapshot = SpaceSnapshot::load(store, saved.profile_id, saved.id).map_err(snapshot_error)?;
        let target = if new_space {
            let taken: Vec<String> = store.spaces(saved.profile_id).unwrap_or_default().into_iter().map(|s| s.name).collect();
            let mut names = std::iter::once(saved.name.clone()).chain((2..50).map(|n| format!("{} {n}", saved.name)));
            let name = names.find(|n| !taken.contains(n)).unwrap_or_else(|| "Snapshot".to_owned());
            store.create_space(saved.profile_id, &name, "").map_err(|_| "Tarn could not create a space for this snapshot.")?.id
        } else {
            current
        };
        let last = snapshot.restore(store, target);
        if let Some(id) = last {
            store.touch_tab(id, now()).ok();
        }
        self.model.reload_spaces(&self.core);
        if new_space && let Some(ix) = self.model.spaces.iter().position(|s| !s.private && s.space.id == target) {
            self.switch_space_by_user(ix);
        }
        self.model.reload_space(&self.core);
        if let Some(id) = last {
            self.activate(TabKey { private: false, id });
        }
        Ok(())
    }

    pub fn space_commands(&self) -> Vec<(String, SpaceCommand)> {
        let mut rows = Vec::new();
        let active = self.model.active;
        for ix in (0..self.model.spaces.len()).filter(|ix| *ix != self.model.space_ix) {
            let name = self.space_title(ix);
            rows.push((format!("Switch to {name}"), SpaceCommand::Switch(ix)));
            if let Some(key) = active.filter(|k| self.model.move_targets(&self.core, *k).contains(&ix)) {
                rows.push((format!("Move Tab to {name}"), SpaceCommand::Move(key, ix)));
            }
        }
        rows
    }

    fn space_title(&self, ix: usize) -> String {
        let s = &self.model.spaces[ix];
        if s.private { "Private".to_owned() } else { engine::view_policy::title(&s.space.name) }
    }

    /// The spaces the tab can move to, as `model.spaces` indices with their names.
    pub fn move_targets(&self, key: TabKey) -> Vec<(usize, String)> {
        self.model.move_targets(&self.core, key).into_iter().map(|ix| (ix, self.space_title(ix))).collect()
    }

    pub fn run_space_command(&mut self, command: SpaceCommand) {
        match command {
            SpaceCommand::Switch(ix) => self.switch_space_by_user(ix),
            SpaceCommand::Move(key, ix) => self.move_to_space(key, ix),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpaceCommand {
    Switch(usize),
    Move(TabKey, usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_iso() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_709_337_600_000), "2024-03-02");
    }
}
