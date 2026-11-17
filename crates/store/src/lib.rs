#![forbid(unsafe_code)]

mod db;
mod models;

pub use db::{Error, MAX_HISTORY_TEXT, MAX_SITE_CSS, MAX_THEME_CSS, Store, clip_history_text};
pub use models::{
    Bookmark, Counter, Download, Folder, Group, HistoryEntry, Id, InstalledExtension, InstalledTheme, Layout, Profile, SavedWindow, Schedule, ShieldDay,
    ShieldStats, Slot, Snapshot, Space, SpaceRule, Tab, TabKind, WebEngine,
};
