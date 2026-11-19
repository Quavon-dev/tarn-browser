//! Decides which tabs keep a live web view. The active tab and the most recently used ones stay
//! live up to `max_live`; the rest are hibernated to their saved interaction state.

use std::collections::VecDeque;
use std::hash::Hash;
use std::time::Duration;

/// How long an awake background tab must have gone unused before it counts as heavy.
pub const HEAVY_IDLE: Duration = Duration::from_secs(60);

pub struct Lifecycle<T> {
    max_live: usize,
    /// Each live tab with the number it got when it became live.
    live: VecDeque<(T, u64)>,
    next: u64,
}

impl<T: Copy + Eq + Hash> Lifecycle<T> {
    pub fn new(max_live: usize) -> Self {
        Lifecycle { max_live: max_live.max(1), live: VecDeque::new(), next: 0 }
    }

    /// Marks `tab` active and returns the tabs that must be hibernated now.
    pub fn activate(&mut self, tab: T) -> Vec<T> {
        let born = self.born(tab).unwrap_or_else(|| self.take_number());
        self.live.retain(|(t, _)| *t != tab);
        self.live.push_front((tab, born));
        self.live.split_off(self.max_live.min(self.live.len())).into_iter().map(|(t, _)| t).collect()
    }

    /// Activates the panes `others` shown beside `tab`, then `tab`: the live ones first, so waking
    /// a sleeping pane cannot evict one that is still live and renumber its view.
    pub fn activate_with(&mut self, others: &[T], tab: T) -> Vec<T> {
        let (live, asleep): (Vec<T>, Vec<T>) = others.iter().copied().chain([tab]).partition(|t| self.born(*t).is_some());
        live.into_iter().chain(asleep).chain([tab]).flat_map(|t| self.activate(t)).collect()
    }

    /// Puts back, as the least recently used, a tab whose hibernation was put off; the next
    /// activation evicts it again.
    pub fn keep(&mut self, tab: T) {
        if self.born(tab).is_none() {
            let born = self.take_number();
            self.live.push_back((tab, born));
        }
    }

    /// Changes how many tabs stay live and returns the least recently used ones beyond it that must
    /// sleep; tabs that stay live, `awake` ones beyond the limit too, keep their numbers.
    pub fn set_max_live(&mut self, max_live: usize, awake: impl Fn(T) -> bool) -> Vec<T> {
        self.max_live = max_live.max(1);
        let mut evicted = Vec::new();
        let mut kept = 0;
        self.live.retain(|(t, _)| {
            kept += 1;
            let stays = kept <= self.max_live || awake(*t);
            if !stays {
                evicted.push(*t);
            }
            stays
        });
        evicted
    }

    pub fn remove(&mut self, tab: T) {
        self.live.retain(|(t, _)| *t != tab);
    }

    pub fn live(&self) -> impl Iterator<Item = T> + '_ {
        self.live.iter().map(|(t, _)| *t)
    }

    /// A number `tab` keeps while it stays live and that no later wake repeats, so a shell keying
    /// its web views by it never hands a closed tab's view to a new tab that reuses the tab's id.
    pub fn born(&self, tab: T) -> Option<u64> {
        self.live.iter().find(|(t, _)| *t == tab).map(|(_, born)| *born)
    }

    fn take_number(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}

/// The `n` awake background tabs most worth sleeping, given as (tab, measured memory in bytes or 0,
/// playing media, time since last active): the most memory first, then playing tabs, then the
/// longest unused, so unmeasured tabs fall back to those guesses; a tab used within `HEAVY_IDLE`
/// and not playing is never heavy.
pub fn heaviest<T>(awake: impl IntoIterator<Item = (T, u64, bool, Duration)>, n: usize) -> Vec<T> {
    let mut ranked: Vec<_> = awake.into_iter().filter(|(_, _, playing, idle)| *playing || *idle >= HEAVY_IDLE).collect();
    ranked.sort_by_key(|(_, memory, playing, idle)| std::cmp::Reverse((*memory, *playing, *idle)));
    ranked.into_iter().take(n).map(|(t, ..)| t).collect()
}

/// How long an awake background tab must have gone unused before the memory budget may sleep it.
pub const BUDGET_IDLE: Duration = Duration::from_secs(10 * 60);

/// How much measured memory the awake background tabs may hold before the heaviest sleep.
pub const BACKGROUND_BUDGET: u64 = 1 << 30;

/// How long a pinned tab must have gone unused before the memory budget may sleep it.
pub const PINNED_IDLE: Duration = Duration::from_secs(30 * 60);

/// The idle time `over_budget` should see: a pinned tab counts as just used until `PINNED_IDLE`.
pub fn budget_idle(idle: Duration, pinned: bool) -> Duration {
    if pinned && idle < PINNED_IDLE { Duration::ZERO } else { idle }
}

/// The awake background tabs, given as for `heaviest`, that must sleep, the most memory first, so
/// the rest fit `budget`; a tab playing media or used within `BUDGET_IDLE` stays awake.
pub fn over_budget<T>(awake: impl IntoIterator<Item = (T, u64, bool, Duration)>, budget: u64) -> Vec<T> {
    let awake: Vec<_> = awake.into_iter().collect();
    let mut total: u64 = awake.iter().map(|(_, memory, ..)| memory).sum();
    let mut ranked: Vec<_> = awake.into_iter().filter(|(_, _, playing, idle)| !playing && *idle >= BUDGET_IDLE).collect();
    ranked.sort_by_key(|(_, memory, ..)| std::cmp::Reverse(*memory));
    let mut sleep = Vec::new();
    for (tab, memory, ..) in ranked {
        if total <= budget {
            break;
        }
        total -= memory;
        sleep.push(tab);
    }
    sleep
}

#[cfg(test)]
mod tests {
    #[test]
    fn waking_a_pane_keeps_the_live_panes_numbers() {
        let mut life = Lifecycle::new(3);
        for tab in [1, 2] {
            life.activate(tab);
        }
        life.activate(5);
        life.activate(6);
        let born = life.born(5);
        assert!(life.born(1).is_none() && born.is_some(), "1 sleeps, 5 is the oldest live tab");
        life.activate_with(&[1], 5);
        assert_eq!(life.born(5), born, "waking 1 does not evict 5");
        assert!(life.born(1).is_some());
        assert_eq!(life.live().next(), Some(5), "the focused pane comes first");
    }

    use super::*;

    #[test]
    fn memory_ranks_first_then_playing_then_the_longest_unused() {
        let min = |m: u64| Duration::from_secs(60 * m);
        let awake =
            [(1, 0, false, min(5)), (2, 0, true, Duration::ZERO), (3, 0, false, min(30)), (4, 0, false, Duration::from_secs(10)), (5, 0, false, min(2))];
        assert_eq!(heaviest(awake, 3), [2, 3, 1]);
        assert_eq!(heaviest(awake, 10), [2, 3, 1, 5], "a tab used seconds ago is never heavy");
        assert!(heaviest(awake, 0).is_empty());
        let measured = [(1, 100, false, min(5)), (2, 0, true, Duration::ZERO), (3, 500, false, min(2)), (4, 900, false, Duration::from_secs(10))];
        assert_eq!(heaviest(measured, 2), [3, 1]);
    }

    #[test]
    fn the_heaviest_idle_tabs_sleep_until_the_rest_fit_the_budget() {
        let (idle, fresh) = (BUDGET_IDLE, Duration::from_secs(5 * 60));
        let awake = [(1, 300, false, idle), (2, 900, false, idle), (3, 500, true, idle), (4, 800, false, fresh), (5, 400, false, idle)];
        assert_eq!(over_budget(awake, 2_000), [2], "2900 minus 900 fits");
        assert_eq!(over_budget(awake, 1_600), [2, 5]);
        assert_eq!(over_budget(awake, 0), [2, 5, 1], "playing and recently used tabs stay awake");
        assert!(over_budget(awake, 2_900).is_empty());
    }

    #[test]
    fn a_pinned_tab_waits_longer_for_the_budget() {
        let minutes = |m: u64| Duration::from_secs(60 * m);
        assert_eq!(budget_idle(minutes(5), false), minutes(5));
        assert_eq!(budget_idle(minutes(29), true), Duration::ZERO);
        assert_eq!(budget_idle(PINNED_IDLE, true), PINNED_IDLE);
        let awake = [(1, 900, false, budget_idle(minutes(29), true)), (2, 300, false, budget_idle(minutes(31), true))];
        assert_eq!(over_budget(awake, 500), [2]);
    }

    #[test]
    fn fifty_tabs_stay_within_budget() {
        let mut lc = Lifecycle::new(4);
        let mut hibernated = Vec::new();
        for id in 0..50 {
            hibernated.extend(lc.activate(id));
            assert!(lc.live().count() <= 4);
        }
        assert_eq!(hibernated.len(), 46);
        assert_eq!(lc.live().collect::<Vec<_>>(), [49, 48, 47, 46]);
    }

    #[test]
    fn a_lower_limit_sleeps_the_least_recent_and_keeps_numbers() {
        let mut lc = Lifecycle::new(6);
        for id in [1, 2, 3, 4] {
            lc.activate(id);
        }
        let numbers = [4, 3, 1].map(|t| lc.born(t));
        assert_eq!(lc.set_max_live(2, |t| t == 1), [2]);
        assert_eq!([4, 3, 1].map(|t| lc.born(t)), numbers, "live and awake tabs keep their numbers");
        assert!(lc.set_max_live(10, |_| false).is_empty());
        lc.activate(5);
        lc.activate(6);
        assert_eq!(lc.live().collect::<Vec<_>>(), [6, 5, 4, 3, 1]);
    }

    #[test]
    fn reactivating_moves_to_front_without_eviction() {
        let mut lc = Lifecycle::new(3);
        for id in [1, 2, 3] {
            lc.activate(id);
        }
        assert!(lc.activate(1).is_empty());
        assert_eq!(lc.activate(4), [2]);
        lc.remove(4);
        assert_eq!(lc.live().collect::<Vec<_>>(), [1, 3]);
    }

    #[test]
    fn a_tab_live_again_after_removal_gets_a_new_number() {
        let mut lc = Lifecycle::new(3);
        lc.activate(1);
        let first = lc.born(1);
        lc.activate(2);
        lc.activate(1);
        assert_eq!(lc.born(1), first, "staying live keeps the number");
        lc.remove(1);
        assert_eq!(lc.born(1), None);
        lc.activate(1);
        assert!(lc.born(1).is_some() && lc.born(1) != first, "a reused id is a new view");
        lc.keep(3);
        assert!(lc.born(3).is_some_and(|n| Some(n) != lc.born(1)));
    }

    #[test]
    fn a_kept_tab_is_evicted_again_on_the_next_activation() {
        let mut lc = Lifecycle::new(2);
        lc.activate(1);
        lc.activate(2);
        assert_eq!(lc.activate(3), [1]);
        lc.keep(1);
        lc.keep(1);
        assert_eq!(lc.live().collect::<Vec<_>>(), [3, 2, 1]);
        assert_eq!(lc.activate(2), [1]);
    }
}
