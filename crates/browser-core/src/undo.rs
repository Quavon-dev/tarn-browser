use store::{Id, TabKind};

use crate::cleanup;
use crate::tabs::{Model, TabKey};
use crate::{Core, WebsiteData};

/// The most steps a window keeps to undo.
pub const MAX_STEPS: usize = 50;

/// Something the window can undo: a close (its entry in `ClosedTabs`), a Clean Up, or a tab moved
/// in the sidebar with the place it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Closed(u64),
    CleanUp(cleanup::Undo),
    Moved(Move),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub key: TabKey,
    pub kind: TabKind,
    pub folder: Option<Id>,
    pub group: Option<Id>,
    pub index: usize,
}

impl Move {
    /// Where `key` sits now, to move it back to.
    pub fn of(model: &Model, key: TabKey) -> Option<Move> {
        let tab = model.tab(key)?;
        Some(Move { key, kind: tab.kind, folder: tab.folder_id, group: tab.group_id, index: model.zone_index(tab) })
    }

    /// Puts the tab back where it was; false when it is gone.
    pub fn revert<D: WebsiteData>(&self, model: &mut Model, core: &Core<D>) -> bool {
        let group = (self.kind == TabKind::Ephemeral).then_some(self.group);
        let moved = core.store(self.key.private).move_tab_to(self.key.id, self.kind, self.folder, self.index, group).is_ok();
        model.reload_space(core);
        moved
    }
}

/// One window's undo steps, newest last, in memory only; nothing of a private space or a burner
/// tab is kept.
#[derive(Debug, Default)]
pub struct Stack {
    steps: Vec<Step>,
}

impl Stack {
    pub fn push(&mut self, step: Step, private: bool) {
        if private {
            return;
        }
        if self.steps.len() == MAX_STEPS {
            self.steps.remove(0);
        }
        self.steps.push(step);
    }

    /// The newest step `live` still accepts; older dead steps on the way are dropped.
    pub fn pop(&mut self, live: impl Fn(&Step) -> bool) -> Option<Step> {
        while let Some(step) = self.steps.pop() {
            if live(&step) {
                return Some(step);
            }
        }
        None
    }

    /// Drops `step`, which a toast's Undo runs itself.
    pub fn forget(&mut self, step: &Step) {
        self.steps.retain(|s| s != step);
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pops_the_newest_live_step_and_drops_dead_ones() {
        let mut stack = Stack::default();
        for serial in 1..=3 {
            stack.push(Step::Closed(serial), false);
        }
        assert_eq!(stack.pop(|s| *s != Step::Closed(3)), Some(Step::Closed(2)));
        assert_eq!(stack.pop(|_| true), Some(Step::Closed(1)));
        assert_eq!(stack.pop(|_| true), None);
    }

    #[test]
    fn keeps_nothing_private_and_at_most_max_steps() {
        let mut stack = Stack::default();
        stack.push(Step::Closed(1), true);
        assert!(stack.is_empty(), "a private space or a burner tab is never kept");
        for serial in 0..MAX_STEPS as u64 + 5 {
            stack.push(Step::Closed(serial), false);
        }
        let mut popped = 0;
        while stack.pop(|_| true).is_some() {
            popped += 1;
        }
        assert_eq!(popped, MAX_STEPS);
    }

    #[test]
    fn a_toast_forgets_the_step_it_ran() {
        let mut stack = Stack::default();
        stack.push(Step::Closed(1), false);
        stack.push(Step::Closed(2), false);
        stack.forget(&Step::Closed(2));
        assert_eq!(stack.pop(|_| true), Some(Step::Closed(1)));
    }
}
