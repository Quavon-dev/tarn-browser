use std::time::Duration;

use engine::lifecycle::Lifecycle;
use gpui::{Context, Window};
use store::Store;

use crate::browser::{Browser, CommandMode};
use crate::sidebar::SIDEBAR_WIDTH;

/// App-wide appearance and behaviour switches, stored as text in the `settings` table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pref {
    CompactToolbar,
    AutoHideSidebar,
    Gestures,
    ReduceMotion,
    Suggestions,
    NewTabStartPage,
    ClosePrevious,
    BlockAds,
    BlockTrackers,
    BlockAnnoyances,
    BlockMalware,
    BlockAdult,
    BlockPopups,
    YouTubeAds,
    UpdateChecks,
    NightlyUpdates,
    ExtensionsEverySite,
    HistoryText,
    SortDownloads,
    LocalhostFromApps,
}

impl Pref {
    pub const ALL: [Pref; 20] = [
        Pref::CompactToolbar,
        Pref::AutoHideSidebar,
        Pref::Gestures,
        Pref::ReduceMotion,
        Pref::Suggestions,
        Pref::NewTabStartPage,
        Pref::ClosePrevious,
        Pref::BlockAds,
        Pref::BlockTrackers,
        Pref::BlockAnnoyances,
        Pref::BlockMalware,
        Pref::BlockAdult,
        Pref::BlockPopups,
        Pref::YouTubeAds,
        Pref::UpdateChecks,
        Pref::NightlyUpdates,
        Pref::ExtensionsEverySite,
        Pref::HistoryText,
        Pref::SortDownloads,
        Pref::LocalhostFromApps,
    ];

    fn key(self) -> &'static str {
        match self {
            Pref::CompactToolbar => "compact_toolbar",
            Pref::AutoHideSidebar => "auto_hide_sidebar",
            Pref::Gestures => "gestures",
            Pref::ReduceMotion => "reduce_motion",
            Pref::Suggestions => "suggestions",
            Pref::NewTabStartPage => "new_tab_start_page",
            Pref::ClosePrevious => "close_selects_previous",
            Pref::BlockAds => "block_ads",
            Pref::BlockTrackers => "block_trackers",
            Pref::BlockAnnoyances => "block_annoyances",
            Pref::BlockMalware => "block_malware",
            Pref::BlockAdult => "block_adult",
            Pref::BlockPopups => "block_popups",
            Pref::YouTubeAds => "block_youtube_ads",
            Pref::UpdateChecks => "update_checks",
            Pref::NightlyUpdates => "nightly_updates",
            Pref::ExtensionsEverySite => "extensions_every_site",
            Pref::HistoryText => "history_text",
            Pref::SortDownloads => "sort_downloads",
            Pref::LocalhostFromApps => "localhost_from_apps",
        }
    }

    /// Every blocking switch is on by default except adult content, which is the user's choice.
    fn default(self) -> bool {
        matches!(
            self,
            Pref::Gestures
                | Pref::Suggestions
                | Pref::BlockAds
                | Pref::BlockTrackers
                | Pref::BlockAnnoyances
                | Pref::BlockMalware
                | Pref::BlockPopups
                | Pref::YouTubeAds
                | Pref::UpdateChecks
                | Pref::ExtensionsEverySite
                | Pref::LocalhostFromApps
        )
    }

    pub fn blocking(self) -> bool {
        matches!(
            self,
            Pref::BlockAds | Pref::BlockTrackers | Pref::BlockAnnoyances | Pref::BlockMalware | Pref::BlockAdult | Pref::BlockPopups | Pref::YouTubeAds
        )
    }
}

/// Enum-valued settings, stored by option name so reordering options keeps choices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Accent,
    LiveTabs,
    SleepAfter,
    ArchiveAfter,
}

pub const LIVE_TABS: [&str; 4] = ["3", "6", "10", "16"];
pub const SLEEP_AFTER: [(&str, Option<Duration>); 4] = [
    ("Never", None),
    ("10 minutes", Some(Duration::from_secs(600))),
    ("30 minutes", Some(Duration::from_secs(1_800))),
    ("2 hours", Some(Duration::from_secs(7_200))),
];
pub const ARCHIVE_AFTER: [(&str, Option<i64>); 4] =
    [("Never", None), ("12 hours", Some(12 * HOUR)), ("1 day", Some(24 * HOUR)), ("7 days", Some(7 * 24 * HOUR))];
const HOUR: i64 = 3_600_000;
const SIDEBAR_SETTING: &str = "sidebar_open";
const SIDEBAR_WIDTH_SETTING: &str = "sidebar_width";
pub const SIDEBAR_MIN_WIDTH: f32 = 200.;
pub const SIDEBAR_MAX_WIDTH: f32 = 420.;

pub fn clamp_sidebar_width(width: f32) -> f32 {
    if width.is_finite() { width.round().clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH) } else { SIDEBAR_WIDTH }
}

pub fn sidebar_width(store: &Store) -> f32 {
    store.setting(SIDEBAR_WIDTH_SETTING).ok().flatten().and_then(|w| w.parse().ok()).map_or(SIDEBAR_WIDTH, clamp_sidebar_width)
}

/// How long a tab may stay off screen before it sleeps; `None` leaves sleeping to the tab limit.
pub fn sleep_after(store: &Store) -> Option<Duration> {
    SLEEP_AFTER[Choice::SleepAfter.load(store)].1
}

/// How long an unused tab stays open, in milliseconds; `None` keeps tabs open.
pub fn archive_after(store: &Store) -> Option<i64> {
    ARCHIVE_AFTER[Choice::ArchiveAfter.load(store)].1
}

pub fn sidebar_open(store: &Store) -> bool {
    store.setting(SIDEBAR_SETTING).ok().flatten().as_deref() != Some("off")
}

impl Choice {
    pub fn options(self) -> Vec<&'static str> {
        match self {
            Choice::Accent => std::iter::once(THEME_ACCENT).chain(ui::theme::ACCENTS.iter().map(|(name, _)| *name)).collect(),
            Choice::LiveTabs => LIVE_TABS.to_vec(),
            Choice::SleepAfter => SLEEP_AFTER.iter().map(|(name, _)| *name).collect(),
            Choice::ArchiveAfter => ARCHIVE_AFTER.iter().map(|(name, _)| *name).collect(),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Choice::Accent => "accent",
            Choice::LiveTabs => "live_tabs",
            Choice::SleepAfter => "sleep_after",
            Choice::ArchiveAfter => "archive_after",
        }
    }

    fn default(self) -> usize {
        match self {
            Choice::Accent | Choice::ArchiveAfter => 0,
            Choice::LiveTabs => 1,
            Choice::SleepAfter => 2,
        }
    }

    pub fn load(self, store: &Store) -> usize {
        option_index(&self.options(), store.setting(self.key()).ok().flatten().as_deref()).unwrap_or(self.default())
    }
}

pub fn option_index(options: &[&str], stored: Option<&str>) -> Option<usize> {
    options.iter().position(|o| Some(*o) == stored)
}

pub fn live_tabs(store: &Store) -> usize {
    LIVE_TABS[Choice::LiveTabs.load(store)].parse().unwrap_or(6)
}

/// The accent choice that keeps the theme's own accent.
pub const THEME_ACCENT: &str = "Theme";

/// Puts the user's accent over the theme's; "Theme" keeps the theme's.
pub fn apply_accent(store: &Store) {
    if let Some(ix) = Choice::Accent.load(store).checked_sub(1) {
        ui::theme::Theme::set_accent(ui::theme::ACCENTS[ix].1);
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Prefs(u32);

impl Prefs {
    pub fn load(store: &Store) -> Self {
        Pref::ALL.into_iter().fold(Prefs(0), |prefs, pref| {
            let on = match store.setting(pref.key()).ok().flatten().as_deref() {
                Some("on") => true,
                Some("off") => false,
                _ => pref.default(),
            };
            prefs.with(pref, on)
        })
    }

    pub fn get(self, pref: Pref) -> bool {
        self.0 & (1 << pref as u32) != 0
    }

    fn with(self, pref: Pref, on: bool) -> Self {
        if on { Prefs(self.0 | 1 << pref as u32) } else { Prefs(self.0 & !(1 << pref as u32)) }
    }
}

impl crate::shared::AppCore {
    pub fn pref(&self, pref: Pref) -> bool {
        self.prefs.get().get(pref) || (pref == Pref::ReduceMotion && engine::reduce_motion())
    }

    /// Whether the updater checks for updates on its own; Chromium may be the default engine only then.
    pub fn update_checks(&self) -> bool {
        self.updater.is_some() && self.pref(Pref::UpdateChecks)
    }

    /// Chromium may be the default engine; local builds allow it without an updater.
    pub fn chromium_allowed(&self) -> bool {
        self.update_checks() || browser_core::staleness::LOCAL_BUILD
    }

    /// Chromium's staleness gate follows the switch; local builds keep it on without an updater.
    pub(crate) fn set_update_checks(&self, on: bool) {
        let mut updates = self.updates.get();
        updates.checks_on = on || browser_core::staleness::LOCAL_BUILD;
        self.updates.set(updates);
    }
}

impl Browser {
    pub fn pref(&self, pref: Pref) -> bool {
        self.core.pref(pref)
    }

    pub fn set_pref(&mut self, pref: Pref, on: bool, cx: &mut Context<Self>) {
        if self.core.disk.set_setting(pref.key(), if on { "on" } else { "off" }).is_ok() {
            self.core.prefs.set(self.core.prefs.get().with(pref, on));
            if pref == Pref::Gestures {
                #[cfg(windows)]
                engine::set_swipe_navigation(on);
                self.apply_gestures();
                crate::shared::others(cx, |b, _, _| b.apply_gestures());
            }
            if let (Pref::UpdateChecks, Some(updater)) = (pref, &self.core.updater) {
                updater.set_automatic(on);
                self.core.set_update_checks(on);
                #[cfg(feature = "chromium")]
                crate::shared::check_chromium(cx);
                if !self.core.chromium_allowed() && self.core.default_web_engine.get() == store::WebEngine::Chromium {
                    self.core.disk.set_setting(browser_core::site::DEFAULT_WEB_ENGINE, store::WebEngine::WebKit.as_str()).ok();
                    self.core.default_web_engine.set(store::WebEngine::WebKit);
                }
            }
            if let (Pref::NightlyUpdates, Some(updater)) = (pref, &self.core.updater) {
                updater.set_nightly(on);
            }
            if pref == Pref::AutoHideSidebar {
                self.set_sidebar_open(!on, cx);
                crate::shared::others(cx, |b, _, cx| b.set_sidebar_open(!on, cx));
            }
            if pref == Pref::HistoryText && !on {
                self.core.disk.clear_history_text().ok();
                self.forget_text_matches();
                crate::shared::others(cx, |b, _, _| b.forget_text_matches());
            }
            if pref.blocking() {
                self.blocking_changed(pref, cx);
            }
            if pref == Pref::SortDownloads {
                self.core.profiles.borrow().values().for_each(|rt| rt.data.set_sort_downloads(on));
            }
            if pref == Pref::ExtensionsEverySite {
                let profiles: Vec<_> = self.core.profiles.borrow().keys().filter(|(private, _)| !private).map(|(_, id)| *id).collect();
                for id in &profiles {
                    self.core.apply_extensions(false, *id);
                    crate::tarn_ext::apply_rules(&self.core, *id);
                }
                cx.defer(move |cx| {
                    crate::shared::each(cx, |b, window, cx| profiles.iter().for_each(|id| b.restart_views_here(false, *id, None, false, window, cx)));
                });
            }
            cx.refresh_windows();
        }
    }

    pub fn set_sidebar_width(&mut self, width: f32, save: bool, cx: &mut Context<Self>) {
        self.sidebar_width = clamp_sidebar_width(width);
        if save {
            self.core.disk.set_setting(SIDEBAR_WIDTH_SETTING, &self.sidebar_width.to_string()).ok();
        }
        cx.notify();
    }

    pub fn set_sidebar_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.sidebar_open != open {
            self.sidebar_toggled = Some(std::time::Instant::now());
        }
        self.sidebar_open = open;
        self.core.disk.set_setting(SIDEBAR_SETTING, if open { "on" } else { "off" }).ok();
        self.close_peek(cx);
        self.hide_hover_card(cx);
        cx.notify();
    }

    /// Rebuilds the live set oldest first, so the most recent tabs stay awake.
    fn set_live_tabs(&mut self, max: usize, cx: &mut Context<Self>) {
        let live: Vec<_> = self.model.lifecycle.live().collect();
        self.model.lifecycle = Lifecycle::new(max);
        let evicted: Vec<_> = live.into_iter().rev().flat_map(|key| self.model.lifecycle.activate(key)).collect();
        for key in evicted {
            self.sleep_unless_playing(key, cx);
        }
    }

    /// The floating sidebar steps aside and the field takes the keyboard, on the start page too.
    pub fn new_tab_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let replaced = self.model.active.filter(|_| self.pref(Pref::NewTabStartPage) && !self.overlay_open() && !self.inline_editing());
        if replaced.is_some() {
            if let Some(view) = self.active_view() {
                view.set_visible(false);
            }
            self.model.active = None;
        }
        // Before the sidebar closes, so a click in it still hands the field to the main window.
        self.open_command(CommandMode::NewTab, window, cx);
        if replaced.is_some() {
            self.launcher_from = replaced;
        }
        if self.peek.window.is_some() {
            self.close_peek(cx);
        }
    }

    pub fn choice(&self, choice: Choice) -> usize {
        choice.load(&self.core.disk)
    }

    pub fn set_choice(&mut self, choice: Choice, ix: usize, cx: &mut Context<Self>) {
        let Some(name) = choice.options().get(ix).copied() else { return };
        let was_never = sleep_after(&self.core.disk).is_none();
        if self.core.disk.set_setting(choice.key(), name).is_err() {
            return;
        }
        match choice {
            Choice::Accent => crate::themes::apply(&self.core, crate::themes::system_light(cx)),
            Choice::LiveTabs => {
                let max = live_tabs(&self.core.disk);
                self.set_live_tabs(max, cx);
                crate::shared::others(cx, |b, _, cx| b.set_live_tabs(max, cx));
            }
            Choice::SleepAfter => {
                // Idle times keep counting under Never for the heavy marker; turning sleep on starts them afresh.
                if was_never && sleep_after(&self.core.disk).is_some() {
                    let now = std::time::Instant::now();
                    self.seen.values_mut().for_each(|seen| *seen = now);
                }
                self.sleep_idle_tabs(cx);
            }
            Choice::ArchiveAfter => {
                self.archive_unused_tabs(cx);
            }
        }
        cx.refresh_windows();
    }
}

#[test]
fn prefs_default_and_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    let prefs = Prefs::load(&store);
    assert!(prefs.get(Pref::Gestures) && !prefs.get(Pref::CompactToolbar));
    assert!(prefs.get(Pref::BlockAds) && prefs.get(Pref::BlockMalware) && prefs.get(Pref::YouTubeAds) && !prefs.get(Pref::BlockAdult));
    assert!(prefs.get(Pref::UpdateChecks) && !prefs.get(Pref::HistoryText) && !prefs.get(Pref::SortDownloads));
    store.set_setting(Pref::Gestures.key(), "off").unwrap();
    store.set_setting(Pref::CompactToolbar.key(), "on").unwrap();
    let prefs = Prefs::load(&store);
    assert!(!prefs.get(Pref::Gestures) && prefs.get(Pref::CompactToolbar));
}

#[test]
fn sidebar_state_persists() {
    let store = Store::open_in_memory().unwrap();
    assert!(sidebar_open(&store));
    store.set_setting(SIDEBAR_SETTING, "off").unwrap();
    assert!(!sidebar_open(&store));
    store.set_setting(SIDEBAR_SETTING, "on").unwrap();
    assert!(sidebar_open(&store));
}

#[test]
fn choices_default_and_ignore_unknown_values() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(live_tabs(&store), 6);
    store.set_setting(Choice::LiveTabs.key(), "16").unwrap();
    assert_eq!(live_tabs(&store), 16);
    store.set_setting(Choice::LiveTabs.key(), "999").unwrap();
    assert_eq!(live_tabs(&store), 6);
    assert_eq!(Choice::Accent.options()[Choice::Accent.load(&store)], THEME_ACCENT, "the theme's accent by default");
    store.set_setting(Choice::Accent.key(), "Green").unwrap();
    assert_eq!(Choice::Accent.options()[Choice::Accent.load(&store)], "Green", "a saved accent keeps its colour");
    store.set_setting(Choice::Accent.key(), "Tarn").unwrap();
    assert_eq!(ui::theme::ACCENTS[Choice::Accent.load(&store) - 1].0, "Tarn");
}

#[test]
fn sidebar_width_is_clamped_and_persists() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(sidebar_width(&store), SIDEBAR_WIDTH);
    assert_eq!(clamp_sidebar_width(120.), SIDEBAR_MIN_WIDTH);
    assert_eq!(clamp_sidebar_width(900.), SIDEBAR_MAX_WIDTH);
    assert_eq!(clamp_sidebar_width(f32::NAN), SIDEBAR_WIDTH);
    store.set_setting(SIDEBAR_WIDTH_SETTING, &clamp_sidebar_width(311.6).to_string()).unwrap();
    assert_eq!(sidebar_width(&store), 312.);
    store.set_setting(SIDEBAR_WIDTH_SETTING, "5000").unwrap();
    assert_eq!(sidebar_width(&store), SIDEBAR_MAX_WIDTH);
    store.set_setting(SIDEBAR_WIDTH_SETTING, "wide").unwrap();
    assert_eq!(sidebar_width(&store), SIDEBAR_WIDTH);
}

#[test]
fn background_tabs_sleep_after_half_an_hour_by_default() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(sleep_after(&store), Some(Duration::from_secs(1_800)));
    store.set_setting(Choice::SleepAfter.key(), "Never").unwrap();
    assert_eq!(sleep_after(&store), None);
}

#[test]
fn archiving_is_off_by_default() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(archive_after(&store), None);
    store.set_setting(Choice::ArchiveAfter.key(), "1 day").unwrap();
    assert_eq!(archive_after(&store), Some(24 * HOUR));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_every_site_switch_is_the_setting_every_client_reads() {
        assert_eq!(Pref::ExtensionsEverySite.key(), browser_core::extensions::EVERY_SITE_SETTING);
        assert_eq!(Pref::ExtensionsEverySite.default(), browser_core::extensions::every_site_setting(None));
        assert!(!browser_core::extensions::every_site_setting(Some("off")));
        assert!(browser_core::extensions::every_site_setting(Some("on")));
    }
}
