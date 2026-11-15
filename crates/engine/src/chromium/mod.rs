#[cfg(target_os = "macos")]
pub(crate) mod cookies;
#[cfg(windows)]
#[path = "cookies_windows.rs"]
pub(crate) mod cookies;
#[cfg(target_os = "macos")]
mod data;
/// Sites whose clear failed are retried across launches only on macOS, where one profile owns
/// Chromium; on Windows a profile's wipe removes its whole folder.
#[cfg(windows)]
mod data {
    pub(crate) fn record(_host: &str) {}

    pub(crate) fn take_pending_sites() -> Vec<String> {
        Vec::new()
    }
}
#[cfg(windows)]
pub(crate) mod downloads_windows;
mod filter;
pub(crate) mod jar;
#[cfg(windows)]
mod pump_windows;
pub(crate) mod runtime;
pub(crate) mod view;
#[cfg(target_os = "macos")]
mod zone;

#[cfg(target_os = "macos")]
pub use data::{clear_cache, clear_site, wipe};
#[cfg(target_os = "macos")]
pub use filter::set_shield;
#[cfg(not(windows))]
pub use filter::take_blocked;
#[cfg(windows)]
pub(crate) use filter::{drop_shield, set_shield_for, shield_for, take_blocked_for};
#[cfg(windows)]
pub use pump_windows::open_next_window_inactive;
#[cfg(windows)]
pub(crate) use pump_windows::{Watch, unwatch, watch};
pub use runtime::shutdown;
#[cfg(windows)]
pub use runtime::{prepare_browser_process, set_sandbox_info, subprocess};
#[cfg(target_os = "macos")]
pub use zone::register_early_malloc_zone;

use std::sync::atomic::{AtomicI64, Ordering};

/// Milliseconds after `START` from which Chromium is closed; a monotonic clock, so setting the
/// wall clock back does not reopen it. It stops while the Mac sleeps, so the wall-clock deadline
/// in `CLOSES_AT_WALL` closes it as well.
static CLOSES_AT: AtomicI64 = AtomicI64::new(i64::MIN);
static CLOSES_AT_WALL: AtomicI64 = AtomicI64::new(i64::MIN);
static START: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);

fn since_start() -> i64 {
    i64::try_from(START.elapsed().as_millis()).unwrap_or(i64::MAX)
}

/// In `remaining_ms` from now, or at the wall-clock time `wall_deadline_ms`, whichever comes
/// first, every Chromium request is cancelled and main-frame navigations report
/// `Event::WebEngineOutdated`; zero or less closes it at once, as it is until the shell first
/// calls this.
pub fn close_in(remaining_ms: i64, wall_deadline_ms: i64) {
    CLOSES_AT.store(since_start().saturating_add(remaining_ms.max(0)), Ordering::Relaxed);
    CLOSES_AT_WALL.store(if remaining_ms <= 0 { i64::MIN } else { wall_deadline_ms }, Ordering::Relaxed);
}

pub(crate) fn outdated() -> bool {
    let wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    since_start() >= CLOSES_AT.load(Ordering::Relaxed) || wall >= CLOSES_AT_WALL.load(Ordering::Relaxed)
}

/// Whether a Chromium view can open for the profile with this UUID now: Chromium is not shut
/// until the next launch, and no other profile owns it.
#[cfg(target_os = "macos")]
pub fn available(root: &std::path::Path, profile: &str) -> bool {
    !runtime::retired() && crate::chromium_owner::may_claim(root, profile)
}

/// Where Chromium keeps its profiles on Windows; the shell sets it once at launch, before any
/// view opens.
#[cfg(windows)]
pub fn set_root(root: &std::path::Path) {
    crate::data_store_windows::sweep_wiped(root);
    runtime::set_root(root);
}

pub use view::{EditCommand, WebView};
