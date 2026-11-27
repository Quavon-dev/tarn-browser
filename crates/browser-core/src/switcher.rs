//! The recent-tab switcher: holding the modifier and pressing Tab steps through the current
//! space's tabs, most recently used first; letting go switches to the selected one.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::tabs::{Model, TabKey};

pub const MAX_CARDS: usize = 10;
/// Where the platform reports no modifier release, a pause this long after the last press stands in for it.
pub const COMMIT_DELAY: Duration = Duration::from_millis(800);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Switcher {
    pub order: Vec<TabKey>,
    pub index: usize,
    /// The last press, unique across switchers, so a delayed commit can tell whether another came meanwhile.
    pub presses: u64,
}

impl Switcher {
    /// Starts on the most recent tab other than `active`, or the oldest when going `back`, so one
    /// press and release toggles between the two most recent tabs. `None` when there is nowhere to go.
    pub fn open(order: Vec<TabKey>, active: Option<TabKey>, back: bool) -> Option<Self> {
        let first = usize::from(order.first().is_some_and(|k| Some(*k) == active));
        (order.len() > first).then(|| Self { index: if back { order.len() - 1 } else { first }, order, presses: next_press() })
    }

    /// A press of the switcher's key without a modifier release: opens `open` over `model`'s recent
    /// tabs, or steps it once open.
    pub fn press(open: &mut Option<Self>, model: &Model, forward: bool) {
        match open {
            Some(s) => s.step(forward),
            None => *open = model.open_switcher(forward),
        }
    }

    pub fn step(&mut self, forward: bool) {
        self.presses = next_press();
        let len = self.order.len();
        self.index = if forward { (self.index + 1) % len } else { (self.index + len - 1) % len };
    }

    pub fn pick(&mut self, key: TabKey) {
        if let Some(ix) = self.order.iter().position(|k| *k == key) {
            self.index = ix;
        }
    }

    pub fn selected(&self) -> TabKey {
        self.order[self.index]
    }
}

fn next_press() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Model {
    /// The tabs the sidebar shows, as the other tab keys reach: the active one first, then the most
    /// recently used.
    pub fn recent_order(&self) -> Vec<TabKey> {
        let mut order = self.visible_tab_order();
        order.sort_by_cached_key(|k| (Some(*k) != self.active, std::cmp::Reverse(self.tab(*k).map_or(0, |t| t.last_active))));
        order
    }

    /// The switcher's first press, over the most recent `MAX_CARDS` tabs.
    pub fn open_switcher(&self, forward: bool) -> Option<Switcher> {
        let mut order = self.recent_order();
        order.truncate(MAX_CARDS);
        Switcher::open(order, self.active, !forward)
    }

    /// Where one press and release of the switcher lands, for a client without the strip.
    pub fn recent(&self, forward: bool) -> Option<TabKey> {
        Switcher::open(self.recent_order(), self.active, !forward).map(|s| s.selected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestCore;
    use store::TabKind;

    fn keys(n: i64) -> Vec<TabKey> {
        (1..=n).map(|id| TabKey { private: false, id }).collect()
    }

    #[test]
    fn one_press_toggles_to_the_previous_tab_and_back_starts_at_the_oldest() {
        let active = Some(keys(1)[0]);
        assert_eq!(Switcher::open(keys(3), active, false).unwrap().selected().id, 2);
        assert_eq!(Switcher::open(keys(3), active, true).unwrap().selected().id, 3);
        assert_eq!(Switcher::open(keys(3), None, false).unwrap().selected().id, 1, "with no active tab, the most recent");
        assert!(Switcher::open(keys(1), active, false).is_none());
        assert!(Switcher::open(Vec::new(), None, false).is_none());
    }

    #[test]
    fn steps_wrap_both_ways_and_a_click_picks() {
        let mut s = Switcher::open(keys(3), Some(keys(1)[0]), false).unwrap();
        s.step(true);
        assert_eq!(s.selected().id, 3);
        s.step(true);
        assert_eq!(s.selected().id, 1);
        s.step(false);
        assert_eq!(s.selected().id, 3);
        s.pick(TabKey { private: false, id: 2 });
        assert_eq!(s.selected().id, 2);
    }

    #[test]
    fn recent_order_puts_the_active_tab_first_then_the_most_recently_used() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let mut model = Model::new(4);
        model.reload_spaces(&core);
        model.switch_space(&core, 0).unwrap();
        let s = model.spaces[0].space.id;
        let ids: Vec<_> =
            ["a", "b", "c"].iter().map(|n| core.disk.create_tab(s, TabKind::Ephemeral, None, &format!("https://{n}.test/"), "").unwrap().id).collect();
        model.reload_space(&core);
        let key = |id| TabKey { private: false, id };
        model.touch(&core, key(ids[0]), 10);
        model.touch(&core, key(ids[2]), 30);
        model.touch(&core, key(ids[1]), 20);
        model.active = Some(key(ids[0]));
        assert_eq!(model.recent_order(), vec![key(ids[0]), key(ids[2]), key(ids[1])]);
        assert_eq!(model.recent(true), Some(key(ids[2])));
        assert_eq!(model.recent(false), Some(key(ids[1])));
        let mut open = None;
        Switcher::press(&mut open, &model, true);
        let first = open.as_ref().map(|s| s.presses);
        assert_eq!(open.as_ref().map(|s| s.selected()), Some(key(ids[2])));
        Switcher::press(&mut open, &model, true);
        assert_eq!(open.as_ref().map(|s| s.selected()), Some(key(ids[1])), "a second press steps");
        assert_ne!(open.as_ref().map(|s| s.presses), first, "a step is a new press");
        let mut reopened = None;
        Switcher::press(&mut reopened, &model, true);
        assert_ne!(reopened.map(|s| s.presses), first, "a timer from a closed switcher never matches a new one");
    }
}
