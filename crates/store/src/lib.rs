#![forbid(unsafe_code)]

mod db;
mod models;

pub use db::{
    Error, MAX_AI_NAME, MAX_AI_TEMPLATE, MAX_CHAT, MAX_CHATS, MAX_HISTORY_TEXT, MAX_NOTE, MAX_NOTE_TITLE, MAX_NOTES, MAX_SITE_CSS, MAX_SNIPPET,
    MAX_SNIPPET_NAME, MAX_THEME_CSS, MISSING_CHAT, Store, clip_history_text, valid_snippet_name,
};
pub use models::{
    AiCommand, Bookmark, Chat, ChatTitle, ChatTurn, Counter, Download, Folder, Group, HistoryEntry, Id, InstalledExtension, InstalledTheme, Layout, Note,
    Profile, SavedWindow, Schedule, ShieldDay, ShieldStats, Slot, Snapshot, Snippet, Space, SpaceRule, Tab, TabKind, WebEngine,
};

pub fn is_bidi(c: char) -> bool {
    // Kept in sync with engine::downloads, ai::page_context and devkit::json_view, which cannot depend on `store`.
    matches!(c, '\u{061C}' | '\u{115F}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}' | '\u{E0000}'..='\u{E007F}')
}

/// Unicode's Default_Ignorable_Code_Point, the bidi controls included: characters that render as nothing.
pub fn is_invisible(c: char) -> bool {
    is_bidi(c)
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{1160}'
                | '\u{17B4}'..='\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{206A}'..='\u{206F}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FFF0}'..='\u{FFF8}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0080}'..='\u{E0FFF}'
        )
}
