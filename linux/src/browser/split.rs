use browser_core::split::{Divider, Split};
use browser_core::tabs::TabKey;

use super::Browser;

impl Browser {
    /// The split on screen: the active tab's.
    pub fn shown_split(&self) -> Option<&Split<TabKey>> {
        self.model.active.and_then(|k| self.splits.of(k))
    }

    pub fn in_split(&self, key: TabKey) -> bool {
        self.shown_split().is_some_and(|s| s.panes().contains(&key))
    }

    /// The other panes of `key`'s split, once those whose tab is gone or shown in another window
    /// have left it.
    pub(super) fn split_partners(&mut self, key: TabKey) -> Vec<TabKey> {
        for pane in self.splits.of(key).map(|s| s.others(key)).unwrap_or_default() {
            if self.model.tab(pane).is_none() || self.elsewhere.contains(&pane) {
                self.splits.leave(pane);
            }
        }
        self.splits.of(key).map(|s| s.others(key)).unwrap_or_default()
    }

    pub fn toggle_split(&mut self) {
        match self.model.active.filter(|a| self.splits.contains(*a)) {
            Some(active) => self.close_split(active),
            None => self.open_split(),
        }
    }

    /// Splits the active tab with the space's most recently used tab outside a split, else with a
    /// copy of the active tab, and focuses the new pane.
    pub fn open_split(&mut self) {
        let Some(active) = self.model.active.filter(|a| !self.splits.contains(*a)) else { return };
        let tabs = self.model.tabs.iter().map(|t| (self.model.key(t), t.last_active)).filter(|(k, _)| !self.elsewhere.contains(k));
        let recent = self.splits.partner(active, tabs);
        let partner = match recent {
            Some(partner) => partner,
            // The burner has one tab, and a copy would load in it instead of beside it.
            None if self.model.burner_shown() => return self.show_toast("Split View needs a second tab", None),
            None => {
                let url = self.pages.get(&active).map(|p| p.url.clone()).filter(|u| !u.is_empty()).or_else(|| self.model.tab(active).map(|t| t.url.clone()));
                let Some(copy) = url.filter(|u| url::Url::parse(u).is_ok()).and_then(|url| self.new_tab(&url)) else { return };
                copy
            }
        };
        self.join_split(active, partner);
    }

    /// Adds `key` to the active tab's split or starts one with the two; the active tab itself gets
    /// a partner.
    pub fn split_with(&mut self, key: TabKey) {
        match self.model.active.filter(|a| *a != key) {
            Some(active) => self.join_split(active, key),
            None => self.open_split(),
        }
    }

    fn join_split(&mut self, anchor: TabKey, key: TabKey) {
        if !self.elsewhere.contains(&key) && self.splits.join(anchor, key) {
            self.activate(key);
        }
    }

    /// A tab dropped on a pane, or on the middle of its row: two panes of one split swap, any
    /// other tab joins.
    pub fn drop_on_pane(&mut self, target: TabKey, dragged: TabKey) {
        if !self.elsewhere.contains(&dragged) && !self.splits.swap(dragged, target) {
            self.join_split(target, dragged);
        }
    }

    /// Takes `key` out of its split; the tab stays open and a split left with one pane ends.
    pub fn leave_split(&mut self, key: TabKey) {
        if let Some(heir) = self.splits.leave(key).filter(|_| self.model.active == Some(key)) {
            self.activate(heir);
        }
    }

    pub fn close_split(&mut self, key: TabKey) {
        self.splits.close(key);
    }

    pub fn rotate_split(&mut self, key: TabKey) {
        self.splits.rotate(key);
    }

    pub fn swap_panes(&mut self) {
        if let Some((active, next)) = self.model.active.and_then(|a| Some((a, self.splits.of(a)?.next(a, true)?))) {
            self.splits.swap(active, next);
        }
    }

    pub fn focus_pane(&mut self, forward: bool) {
        if let Some(next) = self.model.active.and_then(|a| self.splits.of(a)?.next(a, forward)) {
            self.activate(next);
        }
    }

    pub fn resize_split(&mut self, divider: Divider, share: f32) -> bool {
        self.model.active.is_some_and(|a| self.splits.resize(a, divider, share))
    }

    pub fn equalise_split(&mut self) {
        if let Some(active) = self.model.active {
            self.splits.equalise(active);
        }
    }
}
