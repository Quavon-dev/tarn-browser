//! When the bundled Chromium is too old to run. Times are
//! milliseconds since the Unix epoch, as `crate::now`.

use crate::{Core, WebsiteData, now};

const DAY: i64 = 86_400_000;
const LOCK: &str = include_str!("../../../packaging/cef.lock");

/// Debug builds, and bundles built with `TARN_LOCAL_CHROMIUM` set, have no update feed to vouch
/// for Chromium: each launch counts as a verified check, so only the 28-day age limit (and seven
/// days within one session) closes it. A release build (`TARN_RELEASE_BUILD`) refuses to compile with it.
pub const LOCAL_BUILD: bool = cfg!(debug_assertions) || LOCAL_CHROMIUM;
const LOCAL_CHROMIUM: bool = matches!(option_env!("TARN_LOCAL_CHROMIUM"), Some(v) if v.len() == 1 && v.as_bytes()[0] == b'1');
const _: () =
    assert!(!LOCAL_CHROMIUM || !matches!(option_env!("TARN_RELEASE_BUILD"), Some(v) if !v.is_empty()), "TARN_LOCAL_CHROMIUM is for local bundles only");

/// The setting that keeps the latest time seen, so turning the clock back after Chromium closed
/// does not open it again.
pub const CLOCK: &str = "chromium_clock";

/// The setting that keeps the time of the last check that verified the signed feed, so a launch
/// without network still has it.
pub const FEED_VERIFIED: &str = "update_feed_verified";

/// The setting that keeps when a critical update was found and the version then running, so a
/// relaunch that installed nothing does not restart its two days.
pub const CRITICAL_FOUND: &str = "update_critical_found";

/// What the updater knows; Sparkle on macOS and `platform::updater` on Windows fill it in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Updates {
    pub checks_on: bool,
    /// The last check that verified the signed feed.
    pub feed_verified_at: Option<i64>,
    /// Since when a critical update waits for a relaunch.
    pub critical_waiting_since: Option<i64>,
}

impl Updates {
    /// Until the updater reports, local builds (`LOCAL_BUILD`) count as checked at `now`; release builds start
    /// from the stored times with checks off until the switch is read. A verified time after `now`
    /// (a clock turned back) is dropped; a critical update found after `now` waits from `now`.
    pub fn at_launch(now: i64, verified: Option<i64>, critical: Option<i64>) -> Self {
        Self::launch(LOCAL_BUILD, now, verified, critical)
    }

    fn launch(development: bool, now: i64, verified: Option<i64>, critical: Option<i64>) -> Self {
        if development {
            return Updates { checks_on: true, feed_verified_at: Some(now), critical_waiting_since: None };
        }
        Updates { checks_on: false, feed_verified_at: verified.filter(|&at| at <= now), critical_waiting_since: critical.map(|at| at.min(now)) }
    }
}

pub fn critical_value(at: i64, version: &str) -> String {
    format!("{at} {version}")
}

/// When the critical update in `value` was found, unless a version other than `version` stored it,
/// which means the update was installed.
pub fn stored_critical(value: &str, version: &str) -> Option<i64> {
    let (at, stored) = value.split_once(' ')?;
    at.parse().ok().filter(|_| stored == version)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Outdated,
    CriticalUpdateWaiting,
    ChecksOff,
    FeedUnverified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Staleness {
    Current,
    /// Two weeks past its release; still available.
    Ageing,
    Unavailable(Reason),
}

impl Staleness {
    pub fn message(self) -> &'static str {
        match self {
            Staleness::Current => "",
            Staleness::Ageing => "Tarn's Chromium is more than two weeks old. Update Tarn to keep using Chromium tabs.",
            Staleness::Unavailable(Reason::Outdated) => "Chromium tabs are closed: Tarn's Chromium is four weeks old. Update Tarn to use them again.",
            Staleness::Unavailable(Reason::CriticalUpdateWaiting) => "Chromium tabs are closed until Tarn relaunches to install a security update.",
            Staleness::Unavailable(Reason::ChecksOff) => "Chromium tabs are closed because automatic update checks are off.",
            Staleness::Unavailable(Reason::FeedUnverified) => "Chromium tabs are closed: Tarn could not verify an update check for seven days.",
        }
    }
}

/// The time to judge by: never earlier than the latest time seen, and within a session never
/// earlier than the last judged time plus the real time elapsed since (`anchor`: that judged time
/// and the milliseconds a monotonic clock counted since), so setting the clock back gains nothing.
pub fn advance(latest_seen: Option<i64>, wall: i64, anchor: Option<(i64, i64)>) -> i64 {
    let moved = anchor.map(|(judged, elapsed)| judged.saturating_add(elapsed));
    latest_seen.into_iter().chain(moved).fold(wall, i64::max)
}

/// The first time at which Chromium is unavailable, unless the updater reports again.
pub fn closes_at(released_at: i64, updates: Updates) -> i64 {
    if !updates.checks_on {
        return i64::MIN;
    }
    let feed = updates.feed_verified_at.map_or(i64::MIN, |at| at.saturating_add(7 * DAY));
    let critical = updates.critical_waiting_since.map_or(i64::MAX, |since| since.saturating_add(2 * DAY));
    (released_at + 28 * DAY).min(feed).min(critical)
}

pub fn assess(now: i64, released_at: i64, updates: Updates) -> Staleness {
    if now < closes_at(released_at, updates) {
        return if now - released_at >= 14 * DAY { Staleness::Ageing } else { Staleness::Current };
    }
    Staleness::Unavailable(if !updates.checks_on {
        Reason::ChecksOff
    } else if updates.critical_waiting_since.is_some_and(|since| now - since >= 2 * DAY) {
        Reason::CriticalUpdateWaiting
    } else if now - released_at >= 28 * DAY {
        Reason::Outdated
    } else {
        Reason::FeedUnverified
    })
}

/// `release_date` of `packaging/cef.lock`, the date this build's Chromium was released.
pub fn released_at() -> i64 {
    LOCK.lines().find_map(|l| l.strip_prefix("release_date=")).and_then(parse_date).expect("packaging/cef.lock has a valid release_date")
}

/// `YYYY-MM-DD` as midnight UTC, by Howard Hinnant's days-from-civil.
fn parse_date(date: &str) -> Option<i64> {
    let mut parts = date.trim().splitn(3, '-').map(str::parse::<i64>);
    let (y, m, d) = (parts.next()?.ok()?, parts.next()?.ok()?, parts.next()?.ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * DAY)
}

impl<D: WebsiteData> Core<D> {
    /// Judges Chromium's age by the latest time this store has seen and keeps the result.
    pub fn check_chromium(&self) -> Staleness {
        let now = self.judged_now();
        let seen = self.disk.setting(CLOCK).ok().flatten().and_then(|v| v.parse::<i64>().ok());
        if seen.is_none_or(|seen| now - seen >= 60_000) {
            self.disk.set_setting(CLOCK, &now.to_string()).ok();
        }
        let result = assess(now, released_at(), self.updates.get());
        self.chromium_staleness.set(result);
        result
    }

    /// The time Chromium's age is judged by; see `advance`.
    pub fn judged_now(&self) -> i64 {
        let seen = self.disk.setting(CLOCK).ok().flatten().and_then(|v| v.parse().ok());
        let anchor = self.judged.get().map(|(at, instant)| (at, i64::try_from(instant.elapsed().as_millis()).unwrap_or(i64::MAX)));
        let now = advance(seen, now(), anchor);
        self.judged.set(Some((now, std::time::Instant::now())));
        now
    }

    pub fn feed_verified(&self, at: i64) {
        self.updates.set(Updates { feed_verified_at: Some(at), ..self.updates.get() });
        self.disk.set_setting(FEED_VERIFIED, &at.to_string()).ok();
    }

    /// A critical update waits from the first report until Tarn runs another version.
    pub fn critical_found(&self, at: i64) {
        let updates = self.updates.get();
        if updates.critical_waiting_since.is_none() {
            self.updates.set(Updates { critical_waiting_since: Some(at), ..updates });
            self.disk.set_setting(CRITICAL_FOUND, &critical_value(at, env!("CARGO_PKG_VERSION"))).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestCore;

    const RELEASED: i64 = 1_000 * DAY;
    const CHECKED: Updates = Updates { checks_on: true, feed_verified_at: Some(RELEASED), critical_waiting_since: None };

    fn day(n: i64) -> i64 {
        RELEASED + n * DAY
    }

    #[test]
    fn dates_parse_to_utc_midnight() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2026-11-07"), Some(1_794_009_600_000));
        assert_eq!(parse_date("2024-03-01"), Some(1_709_251_200_000));
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("soon"), None);
        assert!(released_at() > 0);
    }

    #[test]
    fn the_clock_never_runs_backwards() {
        assert_eq!(advance(None, 5, None), 5);
        assert_eq!(advance(Some(9), 5, None), 9);
        assert_eq!(advance(Some(3), 5, None), 5);
        assert_eq!(advance(Some(900), 100, Some((900, 50))), 950, "a clock set back keeps counting real time");
        assert_eq!(advance(Some(900), 2000, Some((900, 50))), 2000);
    }

    #[test]
    fn age_warns_from_day_fourteen_and_closes_at_day_twenty_eight() {
        let updates = |at| Updates { feed_verified_at: Some(at), ..CHECKED };
        assert_eq!(assess(day(13), RELEASED, updates(day(13))), Staleness::Current);
        assert_eq!(assess(day(14), RELEASED, updates(day(14))), Staleness::Ageing);
        assert_eq!(assess(day(27), RELEASED, updates(day(27))), Staleness::Ageing);
        assert_eq!(assess(day(28), RELEASED, updates(day(28))), Staleness::Unavailable(Reason::Outdated));
    }

    #[test]
    fn a_critical_update_waits_two_days_for_a_relaunch() {
        let waiting = Updates { critical_waiting_since: Some(day(1)), ..CHECKED };
        assert_eq!(assess(day(3) - 1, RELEASED, waiting), Staleness::Current);
        assert_eq!(assess(day(3), RELEASED, waiting), Staleness::Unavailable(Reason::CriticalUpdateWaiting));
    }

    #[test]
    fn closes_at_is_the_earliest_limit() {
        assert_eq!(closes_at(RELEASED, CHECKED), day(7));
        assert_eq!(closes_at(RELEASED, Updates { feed_verified_at: Some(day(25)), ..CHECKED }), day(28));
        assert_eq!(closes_at(RELEASED, Updates { critical_waiting_since: Some(day(1)), ..CHECKED }), day(3));
        assert_eq!(closes_at(RELEASED, Updates { checks_on: false, ..CHECKED }), i64::MIN);
        assert_eq!(closes_at(RELEASED, Updates { feed_verified_at: None, ..CHECKED }), i64::MIN);
    }

    #[test]
    fn checks_off_closes_at_once() {
        assert_eq!(assess(day(0), RELEASED, Updates { checks_on: false, ..CHECKED }), Staleness::Unavailable(Reason::ChecksOff));
    }

    #[test]
    fn the_feed_must_verify_every_seven_days() {
        assert_eq!(assess(day(7) - 1, RELEASED, CHECKED), Staleness::Current);
        assert_eq!(assess(day(7), RELEASED, CHECKED), Staleness::Unavailable(Reason::FeedUnverified));
        let never = Updates { feed_verified_at: None, ..CHECKED };
        assert_eq!(assess(day(0), RELEASED, never), Staleness::Unavailable(Reason::FeedUnverified));
    }

    #[test]
    fn release_builds_fail_closed_until_the_updater_reports() {
        let launch = Updates::at_launch(day(0), None, None);
        let expected = if LOCAL_BUILD { Staleness::Current } else { Staleness::Unavailable(Reason::ChecksOff) };
        assert_eq!(assess(day(0), RELEASED, launch), expected);
    }

    #[test]
    fn a_stored_check_counts_after_a_relaunch_but_never_from_the_future() {
        let development = Updates::launch(true, day(3), Some(day(1)), Some(day(1)));
        assert_eq!((development.feed_verified_at, development.critical_waiting_since), (Some(day(3)), None));
        let launch = |stored| Updates { checks_on: true, ..Updates::launch(false, day(3), stored, None) };
        assert!(!Updates::launch(false, day(3), None, None).checks_on);
        assert_eq!(launch(Some(day(1))).feed_verified_at, Some(day(1)));
        assert_eq!(launch(Some(day(3))).feed_verified_at, Some(day(3)));
        assert_eq!(launch(Some(day(90))).feed_verified_at, None);
        assert_eq!(assess(day(8) - 1, RELEASED, launch(Some(day(1)))), Staleness::Current);
        assert_eq!(assess(day(8), RELEASED, launch(Some(day(1)))), Staleness::Unavailable(Reason::FeedUnverified));
    }

    #[test]
    fn a_found_critical_update_keeps_waiting_until_the_version_changes() {
        let value = critical_value(day(1), "1.2.3");
        assert_eq!(stored_critical(&value, "1.2.3"), Some(day(1)));
        assert_eq!(stored_critical(&value, "1.2.4"), None);
        assert_eq!(stored_critical("soon 1.2.3", "1.2.3"), None);
        assert_eq!(stored_critical("", "1.2.3"), None);
        let relaunched = Updates { checks_on: true, ..Updates::launch(false, day(2), Some(day(2)), stored_critical(&value, "1.2.3")) };
        assert_eq!(assess(day(3), RELEASED, relaunched), Staleness::Unavailable(Reason::CriticalUpdateWaiting));
        assert_eq!(Updates::launch(false, day(2), None, Some(day(9))).critical_waiting_since, Some(day(2)));
    }

    #[test]
    fn a_clock_turned_back_keeps_the_latest_time_seen() {
        use super::{CLOCK, Reason, Staleness, Updates};
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), true);
        core.updates.set(Updates { checks_on: true, feed_verified_at: Some(i64::MAX / 2), critical_waiting_since: None });
        let later = (now() + 400 * 86_400_000).to_string();
        core.disk.set_setting(CLOCK, &later).unwrap();
        assert_eq!(core.check_chromium(), Staleness::Unavailable(Reason::Outdated));
        assert_eq!(core.chromium_staleness.get(), Staleness::Unavailable(Reason::Outdated));
        assert_eq!(core.disk.setting(CLOCK).unwrap(), Some(later));
    }

    #[test]
    fn updater_reports_reach_the_gate_and_the_verified_time_is_kept() {
        use super::{CRITICAL_FOUND, FEED_VERIFIED, critical_value};
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), true);
        core.feed_verified(1_234);
        assert_eq!(core.updates.get().feed_verified_at, Some(1_234));
        assert_eq!(core.disk.setting(FEED_VERIFIED).unwrap().as_deref(), Some("1234"));
        core.critical_found(5);
        core.critical_found(9);
        assert_eq!(core.updates.get().critical_waiting_since, Some(5));
        assert_eq!(core.disk.setting(CRITICAL_FOUND).unwrap(), Some(critical_value(5, env!("CARGO_PKG_VERSION"))));
        drop(core);
        let reopened = TestCore::open(dir.path(), true).updates.get();
        let release = !LOCAL_BUILD;
        assert_eq!(reopened.feed_verified_at.map(|at| at == 1_234), Some(release));
        assert_eq!(reopened.critical_waiting_since, release.then_some(5));
    }
}
