use store::{Id, Store};

use crate::prefs::FEATURE_HINTS;

const DAY: i64 = 86_400_000;
/// How often one hint shows at most.
pub const MAX_SHOWS: i64 = 3;
/// No hint shows in a profile's first day.
pub const FIRST_DAY: i64 = DAY;
/// The least time between two hints of a profile.
pub const QUIET: i64 = 2 * DAY;

/// What Settings says under its switch for hints, on every client.
pub const NOTE: &str = "After a command runs from the command bar, a notice may name its keyboard shortcut: at most three times each, two days apart, never in a profile's first day or a private space.";

/// The toast that names a command's shortcut after it ran from the command bar.
pub fn text(command: &str, keys: &str) -> String {
    format!("{command} also runs with {keys}")
}

/// Whether a hint may show, from a profile's counters: `since` when it was created, `last` when any
/// of its hints last showed and `shown` how often this one did.
pub fn allows(since: i64, last: Option<i64>, shown: i64, now: i64, typing: bool) -> bool {
    !typing && shown < MAX_SHOWS && now - since >= FIRST_DAY && last.is_none_or(|last| now - last >= QUIET)
}

/// Feature hints of one on-disk profile, counted in its `feature_hints` rows, which go with the
/// profile; a private space shows none.
pub struct Throttle<'a> {
    pub store: &'a Store,
    pub profile: Id,
}

impl Throttle<'_> {
    /// Whether the hint for `command` shows now, counting it when it does; never while the user is
    /// `typing` or once Settings turns hints off.
    pub fn offer(&self, command: &str, now: i64, typing: bool) -> bool {
        if typing || !FEATURE_HINTS.on(self.store) {
            return false;
        }
        let Ok((since, last, shown)) = self.store.hint_counts(self.profile, command) else { return false };
        allows(since, last, shown, now, typing) && self.store.count_hint(self.profile, command, now).is_ok()
    }

    /// The toast naming `keys` after `command` ran from the command bar, when `offer`
    /// lets one show; none for a command without keys.
    pub fn hint(&self, command: &str, keys: &str, now: i64, typing: bool) -> Option<String> {
        (!keys.is_empty() && self.offer(command, now, typing)).then(|| text(command, keys))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 100 * DAY;

    #[test]
    fn a_hint_waits_a_day_shows_three_times_and_two_days_apart() {
        assert!(!allows(NOW, None, 0, NOW + DAY - 1, false), "not on day one");
        assert!(allows(NOW, None, 0, NOW + DAY, false));
        assert!(!allows(NOW, None, 0, NOW + DAY, true), "never while typing");
        assert!(!allows(0, Some(NOW - QUIET + 1), 0, NOW, false), "two days apart");
        assert!(allows(0, Some(NOW - QUIET), 2, NOW, false));
        assert!(!allows(0, None, MAX_SHOWS, NOW, false), "three times at most");
    }

    #[test]
    fn the_throttle_counts_per_profile_and_follows_the_switch() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "u", false).unwrap().id;
        let q = store.create_profile("Q", "v", false).unwrap().id;
        let born = store.hint_counts(p, "").unwrap().0;
        let hints = Throttle { store: &store, profile: p };
        assert!(!hints.offer("Copy Link", born, false), "not on day one");
        let mut at = born + DAY;
        for _ in 0..MAX_SHOWS {
            assert!(!hints.offer("Copy Link", at, true));
            assert!(hints.offer("Copy Link", at, false));
            assert!(!hints.offer("Reader", at + 1, false), "two days apart across hints");
            at += QUIET;
        }
        assert!(!hints.offer("Copy Link", at, false));
        assert!(hints.offer("Reader", at, false));
        let other = Throttle { store: &store, profile: q };
        assert!(other.offer("Copy Link", at, false), "counters are per profile");
        FEATURE_HINTS.set(&store, false);
        assert!(!other.offer("Reader", at + 10 * DAY, false), "one switch turns hints off");
    }
}
