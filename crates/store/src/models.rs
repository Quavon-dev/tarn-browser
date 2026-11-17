use shield::ShieldLevel;

pub type Id = i64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabKind {
    Tile,
    Pinned,
    Ephemeral,
}

impl TabKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TabKind::Tile => "tile",
            TabKind::Pinned => "pinned",
            TabKind::Ephemeral => "ephemeral",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tile" => Some(TabKind::Tile),
            "pinned" => Some(TabKind::Pinned),
            "ephemeral" => Some(TabKind::Ephemeral),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WebEngine {
    WebKit,
    Chromium,
}

impl WebEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            WebEngine::WebKit => "webkit",
            WebEngine::Chromium => "chromium",
        }
    }

    pub fn resolve(profile: ShieldLevel, site: ShieldLevel, tab: Option<Self>, window: Option<Self>, default: Option<Self>) -> Self {
        if profile == ShieldLevel::Lockdown || site == ShieldLevel::Lockdown {
            return WebEngine::WebKit;
        }
        tab.or(window).or(default).unwrap_or(WebEngine::WebKit)
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "webkit" => Some(WebEngine::WebKit),
            "chromium" => Some(WebEngine::Chromium),
            _ => None,
        }
    }
}

/// What the Shield card counts; every count stays in the profile's store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Counter {
    TrackingParams,
    HttpsUpgrades,
    LocalBlocked,
    /// Pages stopped because a block list names their site.
    ListedStopped,
    PopupsBlocked,
    /// Requests the Chromium engine's request filter cancelled; page loads are not among them.
    ChromiumRequests,
}

impl Counter {
    pub(crate) fn column(self) -> &'static str {
        match self {
            Counter::TrackingParams => "tracking_params",
            Counter::HttpsUpgrades => "https_upgrades",
            Counter::LocalBlocked => "local_blocked",
            Counter::ListedStopped => "listed_stopped",
            Counter::PopupsBlocked => "popups_blocked",
            Counter::ChromiumRequests => "chromium_requests",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShieldStats {
    /// Unix time in milliseconds.
    pub created_at: i64,
    pub seed: i64,
    pub tracking_params: i64,
    pub https_upgrades: i64,
    pub local_blocked: i64,
}

/// One day of the Shield counters; `day` is the local date as `YYYY-MM-DD`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShieldDay {
    pub day: String,
    pub tracking_params: i64,
    pub https_upgrades: i64,
    pub local_blocked: i64,
    pub listed_stopped: i64,
    pub popups_blocked: i64,
    pub chromium_requests: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: Id,
    pub name: String,
    pub data_store_uuid: String,
    pub shield_level: ShieldLevel,
    pub ai_enabled: bool,
    pub is_private: bool,
    pub is_dev: bool,
    pub color: String,
}

/// Opens new tabs of hosts matching `host_pattern` in the space `space_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceRule {
    pub id: Id,
    pub host_pattern: String,
    pub space_id: Id,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub id: Id,
    pub profile_id: Id,
    pub name: String,
    pub icon: String,
    pub position: i64,
    /// A name from the UI's colour list; empty for the default look.
    pub color: String,
    pub schedule: Schedule,
}

/// When a window switches to a space by itself: on `days` (bit 0 Sunday … bit 6 Saturday) from
/// `start` up to `end` o'clock, local time. An `end` at or before `start` runs past midnight into
/// the next day, which still counts as the day it started; no days means no schedule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Schedule {
    pub days: u8,
    pub start: u8,
    pub end: u8,
}

impl Schedule {
    /// Whether the schedule covers `hour` (0–23) of `weekday` (0 Sunday … 6 Saturday).
    pub fn covers(self, weekday: u8, hour: u8) -> bool {
        let on = |day: u8| self.days & (1 << (day % 7)) != 0;
        if self.start < self.end {
            on(weekday) && (self.start..self.end).contains(&hour)
        } else {
            (on(weekday) && hour >= self.start) || (on(weekday + 6) && hour < self.end)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub id: Id,
    pub space_id: Id,
    pub parent_id: Option<Id>,
    pub name: String,
    pub position: i64,
    /// Shown in place of the folder glyph: a symbol name or one emoji (`browser_core::icon`); empty for none.
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: Id,
    pub space_id: Id,
    pub folder_id: Option<Id>,
    pub kind: TabKind,
    pub url: String,
    pub title: String,
    pub position: i64,
    pub last_active: i64,
    pub group_id: Option<Id>,
    /// Where a pinned tab or tile returns to; `None` for unpinned tabs.
    pub pinned_url: Option<String>,
    /// The user's name for the tab, shown instead of the page title; empty for none.
    pub custom_title: String,
    /// The engine chosen for this tab; `None` follows the window and the default.
    pub web_engine: Option<WebEngine>,
}

/// A shell window reopened at launch; it holds no profile data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SavedWindow {
    pub id: Id,
    /// The engine stored on tabs created in the window; `None` leaves them to the default.
    pub web_engine: Option<WebEngine>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub space_id: Option<Id>,
    pub active_tab_id: Option<Id>,
    pub position: i64,
}

/// A named copy of a space's tabs and groups; its data is read only to restore it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub id: Id,
    pub profile_id: Id,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: Id,
    pub space_id: Id,
    pub name: String,
    pub position: i64,
    pub collapsed: bool,
    /// A name from the UI's colour list; empty until the user picks one.
    pub color: String,
    /// Shown in place of the letter: a symbol name or one emoji (`browser_core::icon`); empty for none.
    pub icon: String,
}

/// Where a layout puts a tab: loose, into a group of the space, or into the layout's `n`th new group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    Loose,
    Group(Id),
    New(usize),
}

/// A whole new arrangement of a space's loose and grouped unpinned tabs, applied at once by
/// `Store::apply_layout`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    /// Groups to create, as name, colour and icon.
    pub new_groups: Vec<(String, String, String)>,
    /// Unpinned tabs in their new order, each with its new place.
    pub tabs: Vec<(Id, Slot)>,
    /// Groups in their new order.
    pub groups: Vec<Slot>,
    /// Groups to delete afterwards, but only if no tab is left in them.
    pub drop_if_empty: Vec<Id>,
    /// Groups to collapse afterwards.
    pub collapse: Vec<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    pub id: Id,
    pub profile_id: Id,
    pub parent_id: Option<Id>,
    /// `None` for a bookmark folder.
    pub url: Option<String>,
    pub title: String,
    pub position: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub url: String,
    pub title: String,
    pub visited_at: i64,
    pub visits: i64,
    /// Visits the user opened from the address field or a bookmark.
    pub typed: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub id: Id,
    pub url: String,
    pub path: String,
    pub state: String,
    pub started_at: i64,
}

/// A theme kept by the app: installed, imported or made by the user. `source` is `yours`,
/// `file`, `official:<id>` or `github:<repo id>:<owner>/<repo>`; trust follows it, never the CSS.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstalledTheme {
    pub id: Id,
    pub source: String,
    /// GitHub's numeric ids at install; 0 for other sources.
    pub repo_id: i64,
    pub owner_id: i64,
    /// The GitHub release tag installed; empty for other sources.
    pub tag: String,
    pub name: String,
    pub version: String,
    /// `dark` or `light`, from the theme's header, to pick a fallback of the same kind.
    pub scheme: String,
    pub css: String,
    pub sha256: String,
    /// Whether the theme's page styles may apply; off by default for unverified sources.
    pub page_css: bool,
    pub installed_at: i64,
}

/// One extension installed in a profile. `source` is `appex`, `official:<id>`,
/// `github:<repo id>:<owner>/<repo>` or `file`; trust follows it, never the manifest.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InstalledExtension {
    pub profile_id: Id,
    pub install_id: String,
    pub source: String,
    pub path: String,
    pub name: String,
    pub version: String,
    /// `tree_sha256` of the loaded directory or bundle; empty until first hashed.
    pub sha256: String,
    pub archive_sha256: String,
    /// What the user confirmed, one entry per line.
    pub permissions: String,
    pub enabled: bool,
    pub all_sites: bool,
    /// GitHub's numeric id of the repository owner at install; 0 for other sources.
    pub owner_id: i64,
    /// The release asset installed from GitHub, so an update takes the same build; empty otherwise.
    pub asset: String,
    /// The GitHub release tag installed; an update check with the same tag downloads nothing.
    pub release_tag: String,
}

#[cfg(test)]
mod tests {
    use super::Schedule;

    #[test]
    fn schedules_cover_their_days_and_hours() {
        let work = Schedule { days: 0b011_1110, start: 9, end: 17 };
        assert!(work.covers(1, 9) && work.covers(5, 16));
        assert!(!work.covers(1, 17) && !work.covers(1, 8) && !work.covers(0, 12) && !work.covers(6, 12));
        assert!(!Schedule::default().covers(3, 0) && !Schedule { days: 0, start: 9, end: 17 }.covers(1, 10));
    }

    #[test]
    fn overnight_schedules_belong_to_the_day_they_start() {
        let friday_night = Schedule { days: 1 << 5, start: 22, end: 6 };
        assert!(friday_night.covers(5, 22) && friday_night.covers(5, 23) && friday_night.covers(6, 0) && friday_night.covers(6, 5));
        assert!(!friday_night.covers(5, 5) && !friday_night.covers(6, 6) && !friday_night.covers(6, 22) && !friday_night.covers(4, 23));
        let sunday_night = Schedule { days: 1, start: 20, end: 2 };
        assert!(sunday_night.covers(1, 1) && !sunday_night.covers(0, 1));
        let saturday_night = Schedule { days: 1 << 6, start: 20, end: 2 };
        assert!(saturday_night.covers(0, 1));
        let all_day = Schedule { days: 0b111_1111, start: 0, end: 0 };
        assert!((0..7).all(|d| (0..24).all(|h| all_day.covers(d, h))));
    }
}
