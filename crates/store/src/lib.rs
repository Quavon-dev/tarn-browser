#![forbid(unsafe_code)]

mod db;
mod models;

pub use db::{
    Error, MAX_AI_TEMPLATE, MAX_HISTORY_TEXT, MAX_NOTE, MAX_NOTE_TITLE, MAX_NOTES, MAX_SITE_CSS, MAX_SNIPPET, MAX_SNIPPET_NAME, MAX_THEME_CSS, Store,
    clip_history_text, is_invisible, valid_snippet_name,
};
pub use models::{
    AiCommand, Bookmark, Counter, Download, Folder, Group, HistoryEntry, Id, InstalledExtension, InstalledTheme, Layout, Note, Profile, SavedWindow, Schedule,
    ShieldDay, ShieldStats, Slot, Snapshot, Snippet, Space, SpaceRule, Tab, TabKind, WebEngine,
};
