//! Decides which tabs keep a live web view. The active tab and the most recently used ones stay
//! live up to `max_live`; the rest are hibernated to their saved interaction state.

use std::collections::VecDeque;
use std::hash::Hash;
use std::time::Duration;

/// How long an awake background tab must have gone unused before it counts as heavy.
pub const HEAVY_IDLE: Duration = Duration::from_secs(60);

pub struct Lifecycle<T> {
    max_live: usize,
    live: VecDeque<T>,
}

impl<T: Copy + Eq + Hash> Lifecycle<T> {
    pub fn new(max_live: usize) -> Self {
        Lifecycle { max_live: max_live.max(1), live: VecDeque::new() }
    }

    /// Marks `tab` active and returns the tabs that must be hibernated now.
    pub fn activate(&mut self, tab: T) -> Vec<T> {
        self.live.retain(|t| *t != tab);
        self.live.push_front(tab);
        self.live.split_off(self.max_live.min(self.live.len())).into()
    }

    /// Puts back, as the least recently used, a tab whose hibernation was put off; the next
    /// activation evicts it again.
    pub fn keep(&mut self, tab: T) {
        if !self.live.contains(&tab) {
            self.live.push_back(tab);
        }
    }

    pub fn remove(&mut self, tab: T) {
        self.live.retain(|t| *t != tab);
    }

    pub fn live(&self) -> impl Iterator<Item = T> + '_ {
        self.live.iter().copied()
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

/// How much measured memory the awake background tabs may hold before the heaviest sleep.
pub const BACKGROUND_BUDGET: u64 = 1 << 30;

/// How long a pinned tab must have gone unused before the memory budget may sleep it.
pub const PINNED_IDLE: Duration = Duration::from_secs(30 * 60);

/// The idle time `over_budget` should see: a pinned tab counts as just used until `PINNED_IDLE`.
pub fn budget_idle(idle: Duration, pinned: bool) -> Duration {
    if pinned && idle < PINNED_IDLE { Duration::ZERO } else { idle }
}

/// The awake background tabs, given as for `heaviest`, that must sleep, the most memory first, so
/// the rest fit `budget`; a tab playing media or used within `HEAVY_IDLE` stays awake.
pub fn over_budget<T>(awake: impl IntoIterator<Item = (T, u64, bool, Duration)>, budget: u64) -> Vec<T> {
    let awake: Vec<_> = awake.into_iter().collect();
    let mut total: u64 = awake.iter().map(|(_, memory, ..)| memory).sum();
    let mut ranked: Vec<_> = awake.into_iter().filter(|(_, _, playing, idle)| !playing && *idle >= HEAVY_IDLE).collect();
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
        let (idle, fresh) = (HEAVY_IDLE, Duration::from_secs(5));
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
