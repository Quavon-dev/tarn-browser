use std::collections::HashSet;
#[cfg(unix)]
use std::fs::{self, Permissions};
use std::fs::{DirBuilder, OpenOptions};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};
use shield::ShieldLevel;

use crate::models::{
    AiCommand, Bookmark, Counter, Download, Folder, Group, HistoryEntry, Id, InstalledExtension, InstalledTheme, Layout, Note, Profile, SavedWindow, Schedule,
    ShieldDay, ShieldStats, Slot, Snapshot, Snippet, Space, SpaceRule, Tab, TabKind, WebEngine,
};

const MIGRATIONS: &[&str] = &[
    include_str!("schema.sql"),
    "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    "ALTER TABLE profiles ADD COLUMN is_dev INTEGER NOT NULL DEFAULT 0;",
    "ALTER TABLE profiles ADD COLUMN color TEXT NOT NULL DEFAULT '';
     CREATE TABLE groups (
         id INTEGER PRIMARY KEY,
         space_id INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
         name TEXT NOT NULL,
         position INTEGER NOT NULL,
         collapsed INTEGER NOT NULL DEFAULT 0
     );
     ALTER TABLE tabs ADD COLUMN group_id INTEGER REFERENCES groups(id) ON DELETE CASCADE;
     CREATE TABLE bookmarks (
         id INTEGER PRIMARY KEY,
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         parent_id INTEGER REFERENCES bookmarks(id) ON DELETE CASCADE,
         url TEXT,
         title TEXT NOT NULL DEFAULT '',
         position INTEGER NOT NULL,
         created_at INTEGER NOT NULL
     );
     CREATE INDEX bookmarks_profile_parent ON bookmarks(profile_id, parent_id, position);",
    "ALTER TABLE groups ADD COLUMN color TEXT NOT NULL DEFAULT '';",
    "CREATE TABLE site_zoom (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         host TEXT NOT NULL,
         zoom REAL NOT NULL,
         PRIMARY KEY (profile_id, host)
     );",
    "ALTER TABLE spaces ADD COLUMN color TEXT NOT NULL DEFAULT '';",
    "CREATE TABLE muted_sites (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         host TEXT NOT NULL,
         PRIMARY KEY (profile_id, host)
     );",
    "ALTER TABLE tabs ADD COLUMN pinned_url TEXT;
     ALTER TABLE tabs ADD COLUMN custom_title TEXT NOT NULL DEFAULT '';
     UPDATE tabs SET pinned_url = url WHERE kind != 'ephemeral';",
    "CREATE INDEX tabs_space ON tabs(space_id, kind, folder_id, position);
     CREATE INDEX tabs_folder ON tabs(folder_id);
     CREATE INDEX tabs_group ON tabs(group_id);
     CREATE INDEX folders_space ON folders(space_id);
     CREATE INDEX folders_parent ON folders(parent_id);
     CREATE INDEX groups_space ON groups(space_id);
     CREATE INDEX spaces_profile ON spaces(profile_id);
     CREATE INDEX bookmarks_parent ON bookmarks(parent_id);
     CREATE INDEX downloads_profile ON downloads(profile_id);",
    "UPDATE OR IGNORE site_overrides SET host = rtrim(host, '.') WHERE host LIKE '%.';
     DELETE FROM site_overrides WHERE host LIKE '%.';",
    "CREATE TABLE paused_sites (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         host TEXT NOT NULL,
         PRIMARY KEY (profile_id, host)
     );",
    "ALTER TABLE tabs ADD COLUMN web_engine TEXT CHECK (web_engine IN ('webkit', 'chromium'));",
    "CREATE TABLE windows (
         id INTEGER PRIMARY KEY,
         web_engine TEXT CHECK (web_engine IN ('webkit', 'chromium')),
         x REAL NOT NULL,
         y REAL NOT NULL,
         width REAL NOT NULL,
         height REAL NOT NULL,
         space_id INTEGER REFERENCES spaces(id) ON DELETE SET NULL,
         active_tab_id INTEGER REFERENCES tabs(id) ON DELETE SET NULL,
         position INTEGER NOT NULL
     );",
    "ALTER TABLE extensions ADD COLUMN install_id TEXT NOT NULL DEFAULT '';
     ALTER TABLE extensions ADD COLUMN source TEXT NOT NULL DEFAULT '';
     ALTER TABLE extensions ADD COLUMN path TEXT NOT NULL DEFAULT '';
     ALTER TABLE extensions ADD COLUMN archive_sha256 TEXT NOT NULL DEFAULT '';
     ALTER TABLE extensions ADD COLUMN permissions TEXT NOT NULL DEFAULT '';
     ALTER TABLE extensions ADD COLUMN all_sites INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE extensions ADD COLUMN owner_id INTEGER NOT NULL DEFAULT 0;
     DELETE FROM extensions;
     CREATE UNIQUE INDEX extensions_install ON extensions(profile_id, install_id);
     CREATE TABLE extension_permissions (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         install_id TEXT NOT NULL,
         host TEXT NOT NULL,
         decision TEXT NOT NULL CHECK (decision IN ('allow', 'deny')),
         PRIMARY KEY (profile_id, install_id, host)
     );
     WITH RECURSIVE split(profile_id, rest, path) AS (
         SELECT CAST(substr(key, 12) AS INTEGER), value || char(10), NULL FROM settings WHERE key LIKE 'extensions.%'
         UNION ALL
         SELECT profile_id, substr(rest, instr(rest, char(10)) + 1), substr(rest, 1, instr(rest, char(10)) - 1) FROM split WHERE rest != ''
     )
     INSERT OR IGNORE INTO extensions (profile_id, name, version, sha256, enabled, install_id, source, path, all_sites)
         SELECT DISTINCT profile_id, '', '', '', 1, lower(hex(randomblob(16))), 'appex', path, 1 FROM split
         WHERE path != '' AND profile_id IN (SELECT id FROM profiles);
     INSERT OR REPLACE INTO settings (key, value) SELECT 'extension_all_sites_notice', '1' WHERE EXISTS (SELECT 1 FROM extensions);
     DELETE FROM settings WHERE key LIKE 'extensions.%';",
    "ALTER TABLE extensions ADD COLUMN asset TEXT NOT NULL DEFAULT '';
     ALTER TABLE extensions ADD COLUMN release_tag TEXT NOT NULL DEFAULT '';",
    "ALTER TABLE profiles ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;
     UPDATE profiles SET created_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER);
     CREATE TABLE shield_stats (
         profile_id INTEGER PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE,
         seed INTEGER NOT NULL,
         tracking_params INTEGER NOT NULL DEFAULT 0,
         https_upgrades INTEGER NOT NULL DEFAULT 0,
         local_blocked INTEGER NOT NULL DEFAULT 0
     );",
    "ALTER TABLE favicons ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
     UPDATE favicons SET updated_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER);",
    "CREATE VIRTUAL TABLE history_text USING fts5(text, tokenize = 'unicode61 remove_diacritics 2');
     INSERT INTO history_text (history_text, rank) VALUES ('secure-delete', 1);
     CREATE INDEX history_profile_url ON history(profile_id, url);
     CREATE TRIGGER history_text_forget AFTER DELETE ON history BEGIN
         DELETE FROM history_text WHERE rowid = old.id;
     END;",
    "CREATE TABLE shield_daily (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         day TEXT NOT NULL,
         counter TEXT NOT NULL,
         n INTEGER NOT NULL,
         PRIMARY KEY (profile_id, day, counter)
     );",
    "CREATE TABLE shield_sites (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         host TEXT NOT NULL,
         n INTEGER NOT NULL,
         PRIMARY KEY (profile_id, host)
     );",
    "CREATE TABLE site_css (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         host TEXT NOT NULL,
         css TEXT NOT NULL,
         PRIMARY KEY (profile_id, host)
     );",
    "CREATE TABLE site_dark (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         host TEXT NOT NULL,
         PRIMARY KEY (profile_id, host)
     );",
    "ALTER TABLE spaces ADD COLUMN schedule_days INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE spaces ADD COLUMN schedule_start INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE spaces ADD COLUMN schedule_end INTEGER NOT NULL DEFAULT 0;",
    "CREATE TABLE snapshots (
         id INTEGER PRIMARY KEY,
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         name TEXT NOT NULL,
         created_at INTEGER NOT NULL,
         data BLOB NOT NULL
     );
     CREATE INDEX snapshots_profile ON snapshots(profile_id);",
    "CREATE TABLE space_rules (
         id INTEGER PRIMARY KEY,
         host_pattern TEXT NOT NULL UNIQUE,
         space_id INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE
     );",
    "CREATE TABLE zapped_elements (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         host TEXT NOT NULL,
         selector TEXT NOT NULL,
         PRIMARY KEY (profile_id, host, selector)
     );",
    "ALTER TABLE shield_stats ADD COLUMN listed_stopped INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE shield_stats ADD COLUMN popups_blocked INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE shield_stats ADD COLUMN chromium_requests INTEGER NOT NULL DEFAULT 0;",
    "ALTER TABLE history ADD COLUMN typed INTEGER NOT NULL DEFAULT 0;",
    "CREATE TABLE themes (
         id INTEGER PRIMARY KEY,
         source TEXT NOT NULL,
         repo_id INTEGER NOT NULL DEFAULT 0,
         owner_id INTEGER NOT NULL DEFAULT 0,
         tag TEXT NOT NULL DEFAULT '',
         name TEXT NOT NULL,
         version TEXT NOT NULL DEFAULT '',
         scheme TEXT NOT NULL DEFAULT 'dark',
         css TEXT NOT NULL,
         sha256 TEXT NOT NULL DEFAULT '',
         page_css INTEGER NOT NULL DEFAULT 0,
         installed_at INTEGER NOT NULL
     );",
    "CREATE TABLE extension_storage (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         install_id TEXT NOT NULL,
         key TEXT NOT NULL,
         value TEXT NOT NULL,
         PRIMARY KEY (profile_id, install_id, key)
     );",
    "ALTER TABLE groups ADD COLUMN icon TEXT NOT NULL DEFAULT '';
     ALTER TABLE folders ADD COLUMN icon TEXT NOT NULL DEFAULT '';",
    "ALTER TABLE spaces ADD COLUMN temporary INTEGER NOT NULL DEFAULT 0;",
    "CREATE TABLE ai_commands (
         id INTEGER PRIMARY KEY,
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         name TEXT NOT NULL,
         template TEXT NOT NULL
     );
     CREATE INDEX ai_commands_profile ON ai_commands(profile_id);",
    "CREATE TABLE feature_hints (
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         hint TEXT NOT NULL,
         shown INTEGER NOT NULL,
         last_at INTEGER NOT NULL,
         PRIMARY KEY (profile_id, hint)
     );",
    "CREATE TABLE snippets (
         id INTEGER PRIMARY KEY,
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         name TEXT NOT NULL,
         body TEXT NOT NULL,
         UNIQUE (profile_id, name)
     );",
    "CREATE TABLE notes (
         id INTEGER PRIMARY KEY,
         profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
         title TEXT NOT NULL,
         body TEXT NOT NULL,
         updated INTEGER NOT NULL
     );",
];
/// Tables that belong to no profile: never exported, reset or synced with one.
#[cfg(test)]
const PROFILE_NEUTRAL: [&str; 3] = ["settings", "themes", "windows"];
/// Every table holding profile data, with the filter that selects one profile's rows.
const PROFILE_TABLES: [(&str, &str); 30] = [
    ("profiles", "id = ?1"),
    ("spaces", "profile_id = ?1"),
    ("folders", "space_id IN (SELECT id FROM spaces WHERE profile_id = ?1)"),
    ("groups", "space_id IN (SELECT id FROM spaces WHERE profile_id = ?1)"),
    ("tabs", "space_id IN (SELECT id FROM spaces WHERE profile_id = ?1)"),
    ("history", "profile_id = ?1"),
    ("site_overrides", "profile_id = ?1"),
    ("permissions", "profile_id = ?1"),
    ("downloads", "profile_id = ?1"),
    ("favicons", "profile_id = ?1"),
    ("extensions", "profile_id = ?1"),
    ("bookmarks", "profile_id = ?1"),
    ("site_zoom", "profile_id = ?1"),
    ("muted_sites", "profile_id = ?1"),
    ("paused_sites", "profile_id = ?1"),
    ("extension_permissions", "profile_id = ?1"),
    ("shield_stats", "profile_id = ?1"),
    ("history_text", "rowid IN (SELECT id FROM history WHERE profile_id = ?1)"),
    ("shield_daily", "profile_id = ?1"),
    ("shield_sites", "profile_id = ?1"),
    ("site_css", "profile_id = ?1"),
    ("site_dark", "profile_id = ?1"),
    ("snapshots", "profile_id = ?1"),
    ("space_rules", "space_id IN (SELECT id FROM spaces WHERE profile_id = ?1)"),
    ("zapped_elements", "profile_id = ?1"),
    ("extension_storage", "profile_id = ?1"),
    ("ai_commands", "profile_id = ?1"),
    ("feature_hints", "profile_id = ?1"),
    ("snippets", "profile_id = ?1"),
    ("notes", "profile_id = ?1"),
];
/// Sites kept per profile for the privacy report's list.
const MAX_SHIELD_SITES: i64 = 200;
/// Bytes of page text kept per URL for full-text history search.
pub const MAX_HISTORY_TEXT: usize = 4096;

/// The longest start of `text` that `Store::set_history_text` keeps, cut at a character boundary.
pub fn clip_history_text(text: &str) -> &str {
    let mut end = text.len().min(MAX_HISTORY_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
/// The longest custom assistant command template kept, in bytes.
pub const MAX_AI_TEMPLATE: usize = 4096;
/// The longest snippet text kept, in bytes, and the longest snippet name.
pub const MAX_SNIPPET: usize = 16 * 1024;
pub const MAX_SNIPPET_NAME: usize = 32;

/// A snippet name: 1 to `MAX_SNIPPET_NAME` lowercase ASCII letters, digits, `-` or `_`.
pub fn valid_snippet_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_SNIPPET_NAME && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}
/// The longest note title kept, in characters, the longest note text, in bytes, and the most notes a
/// profile keeps.
pub const MAX_NOTE_TITLE: usize = 200;
pub const MAX_NOTE: usize = 64 * 1024;
pub const MAX_NOTES: usize = 1000;

/// A character a note never keeps: a control character other than a line break or tab, or a bidi,
/// zero-width or tag character (kept in sync with `browser_core::text::is_bidi`).
pub fn is_invisible(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(c, '\u{061C}' | '\u{115F}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}' | '\u{E0000}'..='\u{E007F}')
}

/// The most CSS one site may keep, in bytes.
pub const MAX_SITE_CSS: usize = 64 * 1024;
/// The largest theme file kept, in bytes (`browser_core::theme::MAX_THEME_BYTES`).
pub const MAX_THEME_CSS: usize = 256 * 1024;
const THEME_COLUMNS: &str = "id, source, repo_id, owner_id, tag, name, version, scheme, css, sha256, page_css, installed_at";
/// Kept by "Delete everything": the data goes, the installs stay.
const KEPT_ON_RESET: [&str; 1] = ["extensions"];
const EXTENSION_COLUMNS: &str =
    "profile_id, install_id, source, path, name, version, sha256, archive_sha256, permissions, enabled, all_sites, owner_id, asset, release_tag";
/// Leaves out the saved page state, which can be large; `interaction_state` reads it for one tab.
const TAB_COLUMNS: &str = "id, space_id, folder_id, kind, url, title, position, last_active, group_id, pinned_url, custom_title, web_engine";

#[derive(Debug)]
pub enum Error {
    Sql(rusqlite::Error),
    Io(std::io::Error),
    Invalid(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Sql(e) => write!(f, "database: {e}"),
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Invalid(what) => write!(f, "invalid: {what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct Store {
    conn: Connection,
    settings_rev: std::sync::atomic::AtomicU64,
}

impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        let path = dir.join("tarn.db");
        // Windows: the data folder lives under the user's Local AppData, whose inherited ACL
        // already admits only the user, SYSTEM and Administrators.
        #[cfg(unix)]
        {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
            fs::set_permissions(dir, Permissions::from_mode(0o700))?;
            OpenOptions::new().create(true).append(true).mode(0o600).open(&path)?;
            fs::set_permissions(&path, Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        {
            DirBuilder::new().recursive(true).create(dir)?;
            OpenOptions::new().create(true).append(true).open(&path)?;
        }
        let conn = Connection::open(&path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // Safe in WAL mode: a crash can lose the last commits but never corrupts the database,
        // and writes stop paying an fsync each.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    /// False for the in-memory stand-in `Core::open` uses when the database cannot be opened.
    pub fn on_disk(&self) -> bool {
        self.conn.path().is_some_and(|p| !p.is_empty())
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        conn.pragma_update(None, "secure_delete", true)?;
        let mut store = Store { conn, settings_rev: Default::default() };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let tx = self.conn.transaction()?;
        for (i, sql) in (0..).zip(MIGRATIONS).skip(version as usize) {
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", i + 1i64)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn create_profile(&self, name: &str, data_store_uuid: &str, is_private: bool) -> Result<Profile> {
        self.conn.execute(
            "INSERT INTO profiles (name, data_store_uuid, is_private, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![name, data_store_uuid, is_private, now_ms()],
        )?;
        self.settings_changed();
        self.profile(self.conn.last_insert_rowid())
    }

    pub fn profile(&self, id: Id) -> Result<Profile> {
        Ok(self.conn.query_row(
            "SELECT id, name, data_store_uuid, shield_level, ai_enabled, is_private, is_dev, color FROM profiles WHERE id = ?1",
            [id],
            profile_row,
        )?)
    }

    pub fn profiles(&self) -> Result<Vec<Profile>> {
        let mut stmt = self.conn.prepare("SELECT id, name, data_store_uuid, shield_level, ai_enabled, is_private, is_dev, color FROM profiles ORDER BY id")?;
        let rows = stmt.query_map([], profile_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_profile_shield_level(&self, id: Id, level: ShieldLevel) -> Result<()> {
        self.conn.execute("UPDATE profiles SET shield_level = ?2 WHERE id = ?1", params![id, level.as_str()])?;
        self.settings_changed();
        Ok(())
    }

    pub fn set_profile_ai(&self, id: Id, enabled: bool) -> Result<()> {
        self.conn.execute("UPDATE profiles SET ai_enabled = ?2 WHERE id = ?1", params![id, enabled])?;
        self.settings_changed();
        Ok(())
    }

    pub fn create_dev_profile(&self, data_store_uuid: &str) -> Result<(Profile, Space)> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO profiles (name, data_store_uuid, is_private, is_dev, created_at) VALUES ('Developer', ?1, 0, 1, ?2)",
            params![data_store_uuid, now_ms()],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute("INSERT INTO spaces (profile_id, name, icon, position) VALUES (?1, 'Dev', '', 0)", [id])?;
        tx.commit()?;
        let space = self.spaces(id)?.into_iter().next().ok_or(Error::Invalid("space"))?;
        self.settings_changed();
        Ok((self.profile(id)?, space))
    }

    /// The profile's Shield card: its creation time and local counters, with a random design
    /// value picked on first use and again after "Delete everything".
    pub fn shield_stats(&self, profile_id: Id) -> Result<ShieldStats> {
        self.conn
            .execute("INSERT OR IGNORE INTO shield_stats (profile_id, seed) SELECT id, random() & 0x7fffffffffff FROM profiles WHERE id = ?1", [profile_id])?;
        Ok(self.conn.query_row(
            "SELECT p.created_at, s.seed, s.tracking_params, s.https_upgrades, s.local_blocked
             FROM profiles p JOIN shield_stats s ON s.profile_id = p.id WHERE p.id = ?1",
            [profile_id],
            |r| Ok(ShieldStats { created_at: r.get(0)?, seed: r.get(1)?, tracking_params: r.get(2)?, https_upgrades: r.get(3)?, local_blocked: r.get(4)? }),
        )?)
    }

    /// Adds `n` to the counter's total, to today's bucket (local time) and, with `host`, to that
    /// site's total; every profile's buckets older than the last seven days are dropped.
    pub fn count(&self, profile_id: Id, counter: Counter, n: i64, host: Option<&str>) -> Result<()> {
        let column = counter.column();
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            &format!(
                "INSERT INTO shield_stats (profile_id, seed, {column}) SELECT id, random() & 0x7fffffffffff, ?2 FROM profiles WHERE id = ?1
                 ON CONFLICT(profile_id) DO UPDATE SET {column} = {column} + ?2"
            ),
            params![profile_id, n],
        )?;
        tx.execute(
            "INSERT INTO shield_daily (profile_id, day, counter, n) SELECT id, date('now', 'localtime'), ?2, ?3 FROM profiles WHERE id = ?1
             ON CONFLICT DO UPDATE SET n = n + excluded.n",
            params![profile_id, column, n],
        )?;
        tx.execute("DELETE FROM shield_daily WHERE day < date('now', 'localtime', '-6 days')", [])?;
        if let Some(host) = host.filter(|h| !h.is_empty() && h.len() <= 253) {
            tx.execute(
                "INSERT INTO shield_sites (profile_id, host, n) SELECT id, ?2, ?3 FROM profiles WHERE id = ?1
                 ON CONFLICT DO UPDATE SET n = n + excluded.n",
                params![profile_id, host, n],
            )?;
            // A page can choose the hosts it is upgraded to, so only the busiest sites are kept.
            tx.execute(
                "DELETE FROM shield_sites WHERE profile_id = ?1 AND host NOT IN
                 (SELECT host FROM shield_sites WHERE profile_id = ?1 ORDER BY n DESC, host LIMIT ?2)",
                params![profile_id, MAX_SHIELD_SITES],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The sites with the most protections, most first.
    pub fn shield_sites(&self, profile_id: Id, limit: usize) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare("SELECT host, n FROM shield_sites WHERE profile_id = ?1 ORDER BY n DESC, host LIMIT ?2")?;
        let rows = stmt.query_map(params![profile_id, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn shield_site_total(&self, profile_id: Id, host: &str) -> Result<i64> {
        let n = self.conn.query_row("SELECT n FROM shield_sites WHERE profile_id = ?1 AND host = ?2", params![profile_id, host], |r| r.get(0)).optional()?;
        Ok(n.unwrap_or(0))
    }

    /// The last seven days (local time), oldest first, each with its counts; days without any are zero.
    pub fn shield_week(&self, profile_id: Id) -> Result<Vec<ShieldDay>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE days(i, day) AS (
                 SELECT 0, date('now', 'localtime', '-6 days') UNION ALL SELECT i + 1, date(day, '+1 day') FROM days WHERE i < 6
             )
             SELECT days.day,
                 COALESCE(SUM(CASE WHEN d.counter = 'tracking_params' THEN d.n END), 0),
                 COALESCE(SUM(CASE WHEN d.counter = 'https_upgrades' THEN d.n END), 0),
                 COALESCE(SUM(CASE WHEN d.counter = 'local_blocked' THEN d.n END), 0),
                 COALESCE(SUM(CASE WHEN d.counter = 'listed_stopped' THEN d.n END), 0),
                 COALESCE(SUM(CASE WHEN d.counter = 'popups_blocked' THEN d.n END), 0),
                 COALESCE(SUM(CASE WHEN d.counter = 'chromium_requests' THEN d.n END), 0)
             FROM days LEFT JOIN shield_daily d ON d.profile_id = ?1 AND d.day = days.day
             GROUP BY days.day ORDER BY days.day",
        )?;
        let rows = stmt.query_map([profile_id], |r| {
            Ok(ShieldDay {
                day: r.get(0)?,
                tracking_params: r.get(1)?,
                https_upgrades: r.get(2)?,
                local_blocked: r.get(3)?,
                listed_stopped: r.get(4)?,
                popups_blocked: r.get(5)?,
                chromium_requests: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn new_shield_design(&self, profile_id: Id) -> Result<()> {
        self.shield_stats(profile_id)?;
        self.conn.execute("UPDATE shield_stats SET seed = random() & 0x7fffffffffff WHERE profile_id = ?1", [profile_id])?;
        Ok(())
    }

    pub fn delete_profile(&self, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM profiles WHERE id = ?1", [id])?;
        self.settings_changed();
        self.checkpoint().ok();
        Ok(())
    }

    pub fn create_space(&self, profile_id: Id, name: &str, icon: &str) -> Result<Space> {
        self.conn.execute(
            "INSERT INTO spaces (profile_id, name, icon, position)
             VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position) + 1, 0) FROM spaces WHERE profile_id = ?1))",
            params![profile_id, name, icon],
        )?;
        Ok(self.conn.query_row(
            "SELECT id, profile_id, name, icon, position, color, schedule_days, schedule_start, schedule_end FROM spaces WHERE id = ?1",
            [self.conn.last_insert_rowid()],
            space_row,
        )?)
    }

    pub fn spaces(&self, profile_id: Id) -> Result<Vec<Space>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, profile_id, name, icon, position, color, schedule_days, schedule_start, schedule_end FROM spaces WHERE profile_id = ?1 AND temporary = 0 ORDER BY position",
        )?;
        let rows = stmt.query_map([profile_id], space_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn create_temporary_space(&self, profile_id: Id, name: &str) -> Result<Space> {
        self.conn.execute("INSERT INTO spaces (profile_id, name, position, temporary) VALUES (?1, ?2, 0, 1)", params![profile_id, name])?;
        self.temporary_space(self.conn.last_insert_rowid())
    }

    pub fn every_space(&self, profile_id: Id) -> Result<Vec<Space>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, profile_id, name, icon, position, color, schedule_days, schedule_start, schedule_end FROM spaces WHERE profile_id = ?1 ORDER BY temporary, position",
        )?;
        let rows = stmt.query_map([profile_id], space_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn temporary_space(&self, id: Id) -> Result<Space> {
        Ok(self.conn.query_row(
            "SELECT id, profile_id, name, icon, position, color, schedule_days, schedule_start, schedule_end FROM spaces WHERE id = ?1 AND temporary = 1",
            [id],
            space_row,
        )?)
    }

    pub fn delete_orphan_temporary_spaces(&self) -> Result<()> {
        let deleted =
            self.conn.execute("DELETE FROM spaces WHERE temporary = 1 AND id NOT IN (SELECT space_id FROM windows WHERE space_id IS NOT NULL)", [])?;
        if deleted > 0 {
            self.checkpoint().ok();
        }
        Ok(())
    }

    pub fn delete_space(&self, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM spaces WHERE id = ?1", [id])?;
        self.checkpoint().ok();
        Ok(())
    }

    pub fn space_rules(&self) -> Result<Vec<SpaceRule>> {
        let mut stmt = self.conn.prepare("SELECT id, host_pattern, space_id FROM space_rules ORDER BY host_pattern")?;
        let rows = stmt.query_map([], |r| Ok(SpaceRule { id: r.get(0)?, host_pattern: r.get(1)?, space_id: r.get(2)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Sends `host_pattern` (a host, or `*.host` for it and its subdomains) to `space_id`,
    /// replacing any rule for the same pattern.
    pub fn set_space_rule(&self, host_pattern: &str, space_id: Id) -> Result<()> {
        let pattern = host_pattern.to_ascii_lowercase();
        let host = pattern.strip_prefix("*.").unwrap_or(&pattern);
        let label_ok = |l: &str| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if host.len() > 253 || !host.split('.').all(label_ok) {
            return Err(Error::Invalid("host pattern"));
        }
        self.conn.execute(
            "INSERT INTO space_rules (host_pattern, space_id) VALUES (?1, ?2)
             ON CONFLICT (host_pattern) DO UPDATE SET space_id = excluded.space_id",
            params![pattern, space_id],
        )?;
        Ok(())
    }

    pub fn delete_space_rule(&self, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM space_rules WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn create_folder(&self, space_id: Id, parent_id: Option<Id>, name: &str) -> Result<Folder> {
        self.conn.execute(
            "INSERT INTO folders (space_id, parent_id, name, position)
             VALUES (?1, ?2, ?3, (SELECT COALESCE(MAX(position) + 1, 0) FROM folders
                                  WHERE space_id = ?1 AND parent_id IS ?2))",
            params![space_id, parent_id, name],
        )?;
        Ok(self.conn.query_row(
            "SELECT id, space_id, parent_id, name, position, icon FROM folders WHERE id = ?1",
            [self.conn.last_insert_rowid()],
            folder_row,
        )?)
    }

    pub fn folders(&self, space_id: Id) -> Result<Vec<Folder>> {
        let mut stmt =
            self.conn.prepare("SELECT id, space_id, parent_id, name, position, icon FROM folders WHERE space_id = ?1 ORDER BY parent_id, position")?;
        let rows = stmt.query_map([space_id], folder_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Moves a folder to the end of `parent_id` in its space; refused for a folder of another
    /// space or one inside the moved folder, so folders never form a cycle.
    pub fn move_folder(&self, id: Id, parent_id: Option<Id>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE folders SET parent_id = ?2,
                 position = (SELECT COALESCE(MAX(f.position) + 1, 0) FROM folders f WHERE f.space_id = folders.space_id AND f.parent_id IS ?2)
             WHERE id = ?1 AND (?2 IS NULL OR (
                 EXISTS (SELECT 1 FROM folders p WHERE p.id = ?2 AND p.space_id = folders.space_id)
                 AND ?2 NOT IN (WITH RECURSIVE inside(id) AS (SELECT ?1 UNION ALL SELECT f.id FROM folders f JOIN inside ON f.parent_id = inside.id)
                                SELECT id FROM inside)))",
            params![id, parent_id],
        )?;
        if changed == 0 {
            return Err(Error::Invalid("folder"));
        }
        Ok(())
    }

    pub fn delete_folder(&self, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn create_tab(&self, space_id: Id, kind: TabKind, folder_id: Option<Id>, url: &str, title: &str) -> Result<Tab> {
        self.conn.execute(
            "INSERT INTO tabs (space_id, folder_id, kind, url, title, position, pinned_url)
             VALUES (?1, ?2, ?3, ?4, ?5, (SELECT COALESCE(MAX(position) + 1, 0) FROM tabs
                                          WHERE space_id = ?1 AND kind = ?3 AND folder_id IS ?2),
                     CASE WHEN ?3 != 'ephemeral' THEN ?4 END)",
            params![space_id, folder_id, kind.as_str(), url, title],
        )?;
        self.tab(self.conn.last_insert_rowid())
    }

    pub fn tab(&self, id: Id) -> Result<Tab> {
        Ok(self.conn.prepare_cached(&format!("SELECT {TAB_COLUMNS} FROM tabs WHERE id = ?1"))?.query_row([id], tab_row)?)
    }

    /// Ids of every stored tab, for checking many tabs at once.
    pub fn tab_ids(&self) -> Result<HashSet<Id>> {
        let mut stmt = self.conn.prepare_cached("SELECT id FROM tabs")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn tabs(&self, space_id: Id) -> Result<Vec<Tab>> {
        let mut stmt = self.conn.prepare_cached(&format!("SELECT {TAB_COLUMNS} FROM tabs WHERE space_id = ?1 ORDER BY kind, folder_id, position"))?;
        let rows = stmt.query_map([space_id], tab_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Moves a tab into the zone `(kind, folder_id)` of its space at `index`, renumbering both zones;
    /// refused for a folder of another space.
    pub fn move_tab(&self, id: Id, kind: TabKind, folder_id: Option<Id>, index: usize) -> Result<()> {
        self.move_tab_to(id, kind, folder_id, index, None)
    }

    pub fn move_tab_to(&self, id: Id, kind: TabKind, folder_id: Option<Id>, index: usize, group: Option<Option<Id>>) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let (space_id, old_kind, old_folder): (Id, String, Option<Id>) =
            tx.query_row("SELECT space_id, kind, folder_id FROM tabs WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        if let Some(folder) = folder_id
            && !tx.query_row("SELECT EXISTS (SELECT 1 FROM folders WHERE id = ?1 AND space_id = ?2)", params![folder, space_id], |r| r.get::<_, bool>(0))?
        {
            return Err(Error::Invalid("folder"));
        }
        tx.execute(
            "UPDATE tabs SET kind = ?2, folder_id = ?3, group_id = CASE WHEN ?2 = 'ephemeral' THEN group_id END,
                             pinned_url = CASE WHEN ?2 != 'ephemeral' THEN COALESCE(pinned_url, url) END
             WHERE id = ?1",
            params![id, kind.as_str(), folder_id],
        )?;
        if let Some(group) = group
            && tx.execute(
                "UPDATE tabs SET group_id = ?2 WHERE id = ?1 AND kind = 'ephemeral'
                 AND (?2 IS NULL OR space_id = (SELECT space_id FROM groups WHERE id = ?2))",
                params![id, group],
            )? == 0
        {
            return Err(Error::Invalid("tab group"));
        }
        let zone_ids = |kind: &str, folder: Option<Id>| -> rusqlite::Result<Vec<Id>> {
            let mut stmt = tx.prepare(
                "SELECT id FROM tabs WHERE space_id = ?1 AND kind = ?2 AND folder_id IS ?3 AND id != ?4
                 ORDER BY position",
            )?;
            let rows = stmt.query_map(params![space_id, kind, folder, id], |r| r.get(0))?;
            rows.collect()
        };
        let mut target = zone_ids(kind.as_str(), folder_id)?;
        target.insert(index.min(target.len()), id);
        let source = zone_ids(&old_kind, old_folder)?;
        let mut renumber = tx.prepare("UPDATE tabs SET position = ?2 WHERE id = ?1")?;
        for zone in [&source, &target] {
            for (pos, tab_id) in zone.iter().enumerate() {
                renumber.execute(params![tab_id, pos as i64])?;
            }
        }
        drop(renumber);
        tx.commit()?;
        Ok(())
    }

    pub fn update_tab_page(&self, id: Id, url: &str, title: &str) -> Result<()> {
        self.conn.execute("UPDATE tabs SET url = ?2, title = ?3 WHERE id = ?1", params![id, url, title])?;
        Ok(())
    }

    pub fn set_tab_title(&self, id: Id, title: &str) -> Result<()> {
        self.conn.execute("UPDATE tabs SET title = ?2 WHERE id = ?1", params![id, title])?;
        Ok(())
    }

    /// Only pinned tabs and tiles keep a pinned URL.
    pub fn set_pinned_url(&self, id: Id, url: &str) -> Result<()> {
        self.conn.execute("UPDATE tabs SET pinned_url = ?2 WHERE id = ?1 AND kind != 'ephemeral'", params![id, url])?;
        Ok(())
    }

    /// An empty title shows the page's own title again.
    pub fn set_tab_custom_title(&self, id: Id, title: &str) -> Result<()> {
        self.conn.execute("UPDATE tabs SET custom_title = ?2 WHERE id = ?1", params![id, title])?;
        Ok(())
    }

    pub fn set_tab_web_engine(&self, id: Id, engine: Option<WebEngine>) -> Result<()> {
        self.conn.execute("UPDATE tabs SET web_engine = ?2 WHERE id = ?1", params![id, engine.map(WebEngine::as_str)])?;
        Ok(())
    }

    pub fn interaction_state(&self, id: Id) -> Result<Option<Vec<u8>>> {
        Ok(self.conn.query_row("SELECT interaction_state FROM tabs WHERE id = ?1", [id], |r| r.get(0))?)
    }

    pub fn set_interaction_state(&self, id: Id, state: Option<&[u8]>) -> Result<()> {
        self.conn.execute("UPDATE tabs SET interaction_state = ?2 WHERE id = ?1", params![id, state])?;
        Ok(())
    }

    pub fn touch_tab(&self, id: Id, at: i64) -> Result<()> {
        self.conn.execute("UPDATE tabs SET last_active = ?2 WHERE id = ?1", params![id, at])?;
        Ok(())
    }

    /// Moves a tab to the end of its zone's top level in another space, keeping its saved state,
    /// custom title and pinned URL; it leaves its folder and group behind.
    pub fn move_tab_to_space(&self, id: Id, space_id: Id) -> Result<()> {
        self.conn.execute(
            "UPDATE tabs SET space_id = ?2, folder_id = NULL, group_id = NULL,
                 position = (SELECT COALESCE(MAX(t.position) + 1, 0) FROM tabs t
                             WHERE t.space_id = ?2 AND t.kind = tabs.kind AND t.folder_id IS NULL)
             WHERE id = ?1",
            params![id, space_id],
        )?;
        Ok(())
    }

    pub fn delete_tab(&self, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM tabs WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn set_profile(&self, id: Id, name: &str, color: &str) -> Result<()> {
        self.conn.execute("UPDATE profiles SET name = ?2, color = ?3 WHERE id = ?1", params![id, name, color])?;
        self.settings_changed();
        Ok(())
    }

    /// Points the profile at a new WebKit store, so the old one can be deleted as a whole.
    pub fn set_profile_store(&self, id: Id, data_store_uuid: &str) -> Result<()> {
        self.conn.execute("UPDATE profiles SET data_store_uuid = ?2 WHERE id = ?1", params![id, data_store_uuid])?;
        self.settings_changed();
        Ok(())
    }

    pub fn rename_space(&self, id: Id, name: &str) -> Result<()> {
        self.conn.execute("UPDATE spaces SET name = ?2 WHERE id = ?1", params![id, name])?;
        Ok(())
    }

    pub fn update_space(&self, id: Id, name: &str, color: &str, icon: &str) -> Result<()> {
        self.conn.execute("UPDATE spaces SET name = ?2, color = ?3, icon = ?4 WHERE id = ?1", params![id, name, color, icon])?;
        Ok(())
    }

    pub fn set_space_schedule(&self, id: Id, schedule: Schedule) -> Result<()> {
        let Schedule { days, start, end } = schedule;
        if days > 0x7f || start > 23 || end > 23 {
            return Err(Error::Invalid("schedule"));
        }
        self.conn.execute("UPDATE spaces SET schedule_days = ?2, schedule_start = ?3, schedule_end = ?4 WHERE id = ?1", params![id, days, start, end])?;
        Ok(())
    }

    /// The weekday (0 Sunday … 6 Saturday) and hour now in the Mac's time zone.
    pub fn local_time(&self) -> Result<(u8, u8)> {
        Ok(self.conn.query_row("SELECT CAST(strftime('%w', 'now', 'localtime') AS INTEGER), CAST(strftime('%H', 'now', 'localtime') AS INTEGER)", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?)
    }

    pub fn rename_folder(&self, id: Id, name: &str) -> Result<()> {
        self.conn.execute("UPDATE folders SET name = ?2 WHERE id = ?1", params![id, name])?;
        Ok(())
    }

    /// Stores `icon` as given; `browser_core::icon::set_folder` checks it first.
    pub fn set_folder_icon(&self, id: Id, icon: &str) -> Result<()> {
        self.conn.execute("UPDATE folders SET icon = ?2 WHERE id = ?1", params![id, icon])?;
        Ok(())
    }

    pub fn create_group(&self, space_id: Id, name: &str) -> Result<Group> {
        self.conn.execute(
            "INSERT INTO groups (space_id, name, position)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(position) + 1, 0) FROM groups WHERE space_id = ?1))",
            params![space_id, name],
        )?;
        Ok(self.conn.query_row(
            "SELECT id, space_id, name, position, collapsed, color, icon FROM groups WHERE id = ?1",
            [self.conn.last_insert_rowid()],
            group_row,
        )?)
    }

    pub fn groups(&self, space_id: Id) -> Result<Vec<Group>> {
        let mut stmt = self.conn.prepare("SELECT id, space_id, name, position, collapsed, color, icon FROM groups WHERE space_id = ?1 ORDER BY position")?;
        let rows = stmt.query_map([space_id], group_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn update_group(&self, id: Id, name: &str, collapsed: bool) -> Result<()> {
        self.conn.execute("UPDATE groups SET name = ?2, collapsed = ?3 WHERE id = ?1", params![id, name, collapsed])?;
        Ok(())
    }

    pub fn set_group_color(&self, id: Id, color: &str) -> Result<()> {
        self.conn.execute("UPDATE groups SET color = ?2 WHERE id = ?1", params![id, color])?;
        Ok(())
    }

    /// Stores `icon` as given; `browser_core::icon::set_group` checks it first.
    pub fn set_group_icon(&self, id: Id, icon: &str) -> Result<()> {
        self.conn.execute("UPDATE groups SET icon = ?2 WHERE id = ?1", params![id, icon])?;
        Ok(())
    }

    /// Moves a group to `index` among the groups of its space.
    pub fn move_group(&self, id: Id, index: usize) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let space_id: Id = tx.query_row("SELECT space_id FROM groups WHERE id = ?1", [id], |r| r.get(0))?;
        let mut ids: Vec<Id> = {
            let mut stmt = tx.prepare("SELECT id FROM groups WHERE space_id = ?1 AND id != ?2 ORDER BY position")?;
            stmt.query_map(params![space_id, id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
        };
        ids.insert(index.min(ids.len()), id);
        for (pos, group_id) in ids.iter().enumerate() {
            tx.execute("UPDATE groups SET position = ?2 WHERE id = ?1", params![group_id, pos as i64])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Deletes the group and, through the cascade, every tab in it.
    pub fn delete_group(&self, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM groups WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Deletes the group and keeps its tabs, ungrouped.
    pub fn ungroup(&self, id: Id) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("UPDATE tabs SET group_id = NULL WHERE group_id = ?1", [id])?;
        tx.execute("DELETE FROM groups WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    /// Puts an unpinned tab into a group of its own space, or takes it out with `None`.
    pub fn set_tab_group(&self, id: Id, group_id: Option<Id>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE tabs SET group_id = ?2 WHERE id = ?1 AND kind = 'ephemeral'
             AND (?2 IS NULL OR space_id = (SELECT space_id FROM groups WHERE id = ?2))",
            params![id, group_id],
        )?;
        if changed == 0 {
            return Err(Error::Invalid("tab group"));
        }
        Ok(())
    }

    /// Applies `layout` to the space's top-level unpinned tabs and its groups in one transaction;
    /// the ids of the groups it created. Tabs and groups gone since the layout was made are skipped,
    /// ones of another space refuse the whole layout, and unlisted ones keep their order after the
    /// listed ones. A tab only moves into a group while loose, or out of one, never from one group
    /// into another, so a layout made from an older view of the space cannot regroup it. A
    /// collapsed group that receives a tab is expanded.
    pub fn apply_layout(&self, space_id: Id, layout: &Layout) -> Result<Vec<Id>> {
        let tx = self.conn.unchecked_transaction()?;
        let mut created = Vec::with_capacity(layout.new_groups.len());
        for (name, color, icon) in &layout.new_groups {
            let group = self.create_group(space_id, name)?;
            self.set_group_color(group.id, color)?;
            self.set_group_icon(group.id, icon)?;
            created.push(group.id);
        }
        let ids = |sql: &str| -> Result<Vec<Id>> {
            let mut stmt = tx.prepare(sql)?;
            let rows = stmt.query_map([space_id], |r| r.get(0))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        };
        let zone = ids("SELECT id FROM tabs WHERE space_id = ?1 AND kind = 'ephemeral' AND folder_id IS NULL ORDER BY position")?;
        let groups = ids("SELECT id FROM groups WHERE space_id = ?1 ORDER BY position")?;
        let in_zone: HashSet<Id> = zone.iter().copied().collect();
        let space_of = |table: &str, id: Id| -> Result<Option<Id>> {
            Ok(tx.query_row(&format!("SELECT space_id FROM {table} WHERE id = ?1"), [id], |r| r.get(0)).optional()?)
        };
        let group = |slot: Slot| -> Result<Option<Option<Id>>> {
            match slot {
                Slot::Loose => Ok(Some(None)),
                Slot::New(i) => created.get(i).map(|&id| Some(Some(id))).ok_or(Error::Invalid("layout group")),
                Slot::Group(id) => match space_of("groups", id)? {
                    Some(space) if space == space_id => Ok(Some(Some(id))),
                    Some(_) => Err(Error::Invalid("layout group")),
                    None => Ok(None),
                },
            }
        };
        let (mut order, mut placed) = (Vec::with_capacity(zone.len()), HashSet::new());
        for &(id, slot) in &layout.tabs {
            if !in_zone.contains(&id) || placed.contains(&id) {
                if space_of("tabs", id)?.is_some_and(|space| space != space_id) {
                    return Err(Error::Invalid("layout tab"));
                }
                continue;
            }
            let Some(group_id) = group(slot)? else { continue };
            let moved =
                tx.execute("UPDATE tabs SET group_id = ?2 WHERE id = ?1 AND group_id IS NOT ?2 AND (group_id IS NULL OR ?2 IS NULL)", params![id, group_id])?;
            if moved > 0 && group_id.is_some() {
                tx.execute("UPDATE groups SET collapsed = 0 WHERE id = ?1", [group_id])?;
            } else if moved == 0 && tx.query_row("SELECT group_id FROM tabs WHERE id = ?1", [id], |r| r.get::<_, Option<Id>>(0))? != group_id {
                continue;
            }
            placed.insert(id);
            order.push(id);
        }
        order.extend(zone.iter().filter(|id| !placed.contains(id)));
        let (mut group_order, mut listed) = (Vec::with_capacity(groups.len() + created.len()), HashSet::new());
        for &slot in &layout.groups {
            match group(slot)? {
                Some(Some(id)) if listed.insert(id) => group_order.push(id),
                Some(None) => return Err(Error::Invalid("layout group")),
                _ => {}
            }
        }
        group_order.extend(groups.iter().filter(|id| !listed.contains(id)));
        for &id in &layout.drop_if_empty {
            tx.execute("DELETE FROM groups WHERE id = ?1 AND space_id = ?2 AND NOT EXISTS (SELECT 1 FROM tabs WHERE group_id = ?1)", params![id, space_id])?;
        }
        for &id in &layout.collapse {
            tx.execute("UPDATE groups SET collapsed = 1 WHERE id = ?1 AND space_id = ?2", params![id, space_id])?;
        }
        for (table, ids) in [("tabs", &order), ("groups", &group_order)] {
            let mut renumber = tx.prepare(&format!("UPDATE {table} SET position = ?2 WHERE id = ?1"))?;
            for (pos, id) in ids.iter().enumerate() {
                renumber.execute(params![id, pos as i64])?;
            }
        }
        tx.commit()?;
        Ok(created)
    }

    /// Adds a bookmark, or a bookmark folder when `url` is `None`, at the end of `parent_id`.
    pub fn add_bookmark(&self, profile_id: Id, parent_id: Option<Id>, url: Option<&str>, title: &str, at: i64) -> Result<Bookmark> {
        self.conn.execute(
            "INSERT INTO bookmarks (profile_id, parent_id, url, title, position, created_at)
             SELECT ?1, ?2, ?3, ?4, (SELECT COALESCE(MAX(position) + 1, 0) FROM bookmarks WHERE profile_id = ?1 AND parent_id IS ?2), ?5
             WHERE ?2 IS NULL OR EXISTS (SELECT 1 FROM bookmarks WHERE id = ?2 AND profile_id = ?1 AND url IS NULL)",
            params![profile_id, parent_id, url, title, at],
        )?;
        if self.conn.changes() == 0 {
            return Err(Error::Invalid("bookmark folder"));
        }
        Ok(self.conn.query_row(
            "SELECT id, profile_id, parent_id, url, title, position, created_at FROM bookmarks WHERE id = ?1",
            [self.conn.last_insert_rowid()],
            bookmark_row,
        )?)
    }

    /// Every bookmark and bookmark folder of a profile, ordered by parent and position.
    pub fn bookmarks(&self, profile_id: Id) -> Result<Vec<Bookmark>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, profile_id, parent_id, url, title, position, created_at FROM bookmarks
             WHERE profile_id = ?1 ORDER BY parent_id, position",
        )?;
        let rows = stmt.query_map([profile_id], bookmark_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn rename_bookmark(&self, id: Id, title: &str) -> Result<()> {
        self.conn.execute("UPDATE bookmarks SET title = ?2 WHERE id = ?1", params![id, title])?;
        Ok(())
    }

    /// Moves a bookmark or folder to the end of `parent_id`, a folder of the same profile outside it.
    pub fn move_bookmark(&self, id: Id, parent_id: Option<Id>) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE bookmarks SET parent_id = ?2,
                 position = (SELECT COALESCE(MAX(b.position) + 1, 0) FROM bookmarks b WHERE b.profile_id = bookmarks.profile_id AND b.parent_id IS ?2)
             WHERE id = ?1 AND (?2 IS NULL OR (
                 EXISTS (SELECT 1 FROM bookmarks f WHERE f.id = ?2 AND f.url IS NULL AND f.profile_id = bookmarks.profile_id)
                 AND ?2 NOT IN (WITH RECURSIVE inside(id) AS (SELECT ?1 UNION ALL SELECT b.id FROM bookmarks b JOIN inside ON b.parent_id = inside.id)
                                SELECT id FROM inside)))",
            params![id, parent_id],
        )?;
        if changed == 0 {
            return Err(Error::Invalid("bookmark folder"));
        }
        Ok(())
    }

    pub fn delete_bookmark(&self, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn create_snapshot(&self, profile_id: Id, name: &str, at: i64, data: &[u8]) -> Result<Snapshot> {
        self.conn.execute("INSERT INTO snapshots (profile_id, name, created_at, data) VALUES (?1, ?2, ?3, ?4)", params![profile_id, name, at, data])?;
        Ok(Snapshot { id: self.conn.last_insert_rowid(), profile_id, name: name.to_owned(), created_at: at })
    }

    /// Newest first, without their data.
    pub fn snapshots(&self, profile_id: Id) -> Result<Vec<Snapshot>> {
        let mut stmt = self.conn.prepare("SELECT id, profile_id, name, created_at FROM snapshots WHERE profile_id = ?1 ORDER BY created_at DESC, id DESC")?;
        let rows = stmt.query_map([profile_id], |r| Ok(Snapshot { id: r.get(0)?, profile_id: r.get(1)?, name: r.get(2)?, created_at: r.get(3)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn snapshot_data(&self, profile_id: Id, id: Id) -> Result<Vec<u8>> {
        Ok(self.conn.query_row("SELECT data FROM snapshots WHERE profile_id = ?1 AND id = ?2", params![profile_id, id], |r| r.get(0))?)
    }

    pub fn rename_snapshot(&self, profile_id: Id, id: Id, name: &str) -> Result<()> {
        self.conn.execute("UPDATE snapshots SET name = ?3 WHERE profile_id = ?1 AND id = ?2", params![profile_id, id, name])?;
        Ok(())
    }

    pub fn ai_commands(&self, profile_id: Id) -> Result<Vec<AiCommand>> {
        let mut stmt = self.conn.prepare("SELECT id, profile_id, name, template FROM ai_commands WHERE profile_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([profile_id], |r| Ok(AiCommand { id: r.get(0)?, profile_id: r.get(1)?, name: r.get(2)?, template: r.get(3)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A blank name or a template over `MAX_AI_TEMPLATE` bytes is refused.
    pub fn create_ai_command(&self, profile_id: Id, name: &str, template: &str) -> Result<AiCommand> {
        check_ai_command(name, template)?;
        self.conn.execute("INSERT INTO ai_commands (profile_id, name, template) VALUES (?1, ?2, ?3)", params![profile_id, name, template])?;
        Ok(AiCommand { id: self.conn.last_insert_rowid(), profile_id, name: name.to_owned(), template: template.to_owned() })
    }

    pub fn update_ai_command(&self, profile_id: Id, id: Id, name: &str, template: &str) -> Result<()> {
        check_ai_command(name, template)?;
        self.conn.execute("UPDATE ai_commands SET name = ?3, template = ?4 WHERE profile_id = ?1 AND id = ?2", params![profile_id, id, name, template])?;
        Ok(())
    }

    pub fn delete_ai_command(&self, profile_id: Id, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM ai_commands WHERE profile_id = ?1 AND id = ?2", params![profile_id, id])?;
        self.checkpoint().ok();
        Ok(())
    }

    pub fn snippets(&self, profile_id: Id) -> Result<Vec<Snippet>> {
        let mut stmt = self.conn.prepare("SELECT id, profile_id, name, body FROM snippets WHERE profile_id = ?1 ORDER BY name")?;
        let rows = stmt.query_map([profile_id], |r| Ok(Snippet { id: r.get(0)?, profile_id: r.get(1)?, name: r.get(2)?, body: r.get(3)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Saves `body` under `name`, replacing the profile's snippet of that name. An invalid name
    /// (`valid_snippet_name`), an empty body or one over `MAX_SNIPPET` bytes is refused.
    pub fn save_snippet(&self, profile_id: Id, name: &str, body: &str) -> Result<Snippet> {
        if !valid_snippet_name(name) || body.trim().is_empty() || body.len() > MAX_SNIPPET {
            return Err(Error::Invalid("snippet"));
        }
        let id = self.conn.query_row(
            "INSERT INTO snippets (profile_id, name, body) VALUES (?1, ?2, ?3)
             ON CONFLICT (profile_id, name) DO UPDATE SET body = ?3 RETURNING id",
            params![profile_id, name, body],
            |r| r.get(0),
        )?;
        Ok(Snippet { id, profile_id, name: name.to_owned(), body: body.to_owned() })
    }

    pub fn delete_snippet(&self, profile_id: Id, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM snippets WHERE profile_id = ?1 AND id = ?2", params![profile_id, id])?;
        self.checkpoint().ok();
        Ok(())
    }

    pub fn notes(&self, profile_id: Id) -> Result<Vec<Note>> {
        let mut stmt = self.conn.prepare("SELECT id, profile_id, title, body, updated FROM notes WHERE profile_id = ?1 ORDER BY updated DESC, id DESC")?;
        let rows =
            stmt.query_map([profile_id], |r| Ok(Note { id: r.get(0)?, profile_id: r.get(1)?, title: r.get(2)?, body: r.get(3)?, updated: r.get(4)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds a note (`id` none) or replaces the profile's note `id`, without invisible characters
    /// (`is_invisible`) and with the title on one line. Refused for a private profile, for an empty
    /// note, a title past `MAX_NOTE_TITLE` characters, text past `MAX_NOTE` bytes, or for a new note past `MAX_NOTES`.
    pub fn save_note(&self, profile_id: Id, id: Option<Id>, title: &str, body: &str) -> Result<Note> {
        let visible = |s: &str| s.chars().filter(|c| !is_invisible(*c)).collect::<String>();
        let title = visible(title).split_whitespace().collect::<Vec<_>>().join(" ");
        let body = visible(body);
        if (title.is_empty() && body.trim().is_empty()) || title.chars().count() > MAX_NOTE_TITLE || body.len() > MAX_NOTE {
            return Err(Error::Invalid("note"));
        }
        let updated = now_ms();
        let id = match id {
            None => self.conn.query_row(
                "INSERT INTO notes (profile_id, title, body, updated)
                 SELECT id, ?2, ?3, ?4 FROM profiles
                 WHERE id = ?1 AND NOT is_private AND (SELECT COUNT(*) FROM notes WHERE profile_id = ?1) < ?5
                 RETURNING id",
                params![profile_id, title, body, updated, MAX_NOTES as i64],
                |r| r.get(0),
            ),
            Some(id) => self.conn.query_row(
                "UPDATE notes SET title = ?3, body = ?4, updated = ?5
                 WHERE id = ?2 AND profile_id = ?1 AND profile_id IN (SELECT id FROM profiles WHERE NOT is_private)
                 RETURNING id",
                params![profile_id, id, title, body, updated],
                |r| r.get(0),
            ),
        };
        let id = match id {
            Err(rusqlite::Error::QueryReturnedNoRows) => return Err(Error::Invalid("note")),
            other => other?,
        };
        Ok(Note { id, profile_id, title, body, updated })
    }

    pub fn delete_note(&self, profile_id: Id, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM notes WHERE profile_id = ?1 AND id = ?2", params![profile_id, id])?;
        self.checkpoint().ok();
        Ok(())
    }

    /// When the profile was created, when it last showed any feature hint, and how often it
    /// showed `hint`.
    pub fn hint_counts(&self, profile_id: Id, hint: &str) -> Result<(i64, Option<i64>, i64)> {
        Ok(self.conn.query_row(
            "SELECT created_at, (SELECT MAX(last_at) FROM feature_hints WHERE profile_id = ?1),
                    COALESCE((SELECT shown FROM feature_hints WHERE profile_id = ?1 AND hint = ?2), 0)
             FROM profiles WHERE id = ?1",
            params![profile_id, hint],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }

    pub fn count_hint(&self, profile_id: Id, hint: &str, now: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO feature_hints (profile_id, hint, shown, last_at) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT (profile_id, hint) DO UPDATE SET shown = shown + 1, last_at = ?3",
            params![profile_id, hint, now],
        )?;
        Ok(())
    }

    pub fn delete_snapshot(&self, profile_id: Id, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM snapshots WHERE profile_id = ?1 AND id = ?2", params![profile_id, id])?;
        self.checkpoint().ok();
        Ok(())
    }

    /// Gives one recorded visit the page's current title; single-page apps set it after their URL
    /// changes. Profile and URL must match too, since SQLite reuses the id of a deleted last row.
    pub fn retitle_visit(&self, profile_id: Id, visit: Id, url: &str, title: &str) -> Result<()> {
        self.conn.execute("UPDATE history SET title = ?4 WHERE id = ?2 AND profile_id = ?1 AND url = ?3", params![profile_id, visit, url, title])?;
        Ok(())
    }

    /// Records a visit and returns its id; `typed` when the user opened the address themselves
    /// (address field or bookmark) rather than a page leading there.
    pub fn add_history(&self, profile_id: Id, url: &str, title: &str, at: i64, typed: bool) -> Result<Id> {
        self.conn.execute(
            "INSERT INTO history (profile_id, url, title, visited_at, typed) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![profile_id, url, title, at, typed],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Records many visits in one transaction, leaving out any the profile already has at the
    /// same time; returns how many were added.
    pub fn add_history_bulk<'a>(&self, profile_id: Id, visits: impl IntoIterator<Item = (&'a str, &'a str, i64)>) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut added = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO history (profile_id, url, title, visited_at) SELECT ?1, ?2, ?3, ?4
                 WHERE NOT EXISTS (SELECT 1 FROM history WHERE profile_id = ?1 AND visited_at = ?4 AND url = ?2)",
            )?;
            for (url, title, at) in visits {
                added += stmt.execute(params![profile_id, url, title, at])?;
            }
        }
        tx.commit()?;
        Ok(added)
    }

    /// Runs `f` in one transaction, committed only when it succeeds.
    pub fn batch<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let out = f(self)?;
        tx.commit()?;
        Ok(out)
    }

    /// Most recent visit per URL, newest first, for at most `limit` URLs. Only the latest
    /// `limit * 50` visits are read, so the cost follows the request rather than the size of the
    /// history; a URL last visited before that window is left out.
    pub fn history(&self, profile_id: Id, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT url, title, MAX(visited_at) AS v, COUNT(*), SUM(typed) FROM (
                 SELECT url, title, visited_at, typed FROM history WHERE profile_id = ?1
                 ORDER BY visited_at DESC LIMIT ?3
             ) GROUP BY url ORDER BY v DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![profile_id, limit as i64, limit.saturating_mul(50) as i64], |r| {
            Ok(HistoryEntry { url: r.get(0)?, title: r.get(1)?, visited_at: r.get(2)?, visits: r.get(3)?, typed: r.get(4)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Keeps `text` (at most `MAX_HISTORY_TEXT` bytes) for one recorded visit, replacing the text
    /// of the profile's earlier visits to the same URL. Profile and URL must match the visit, as
    /// in `retitle_visit`.
    pub fn set_history_text(&self, profile_id: Id, visit: Id, url: &str, text: &str) -> Result<()> {
        if text.len() > MAX_HISTORY_TEXT {
            return Err(Error::Invalid("history text"));
        }
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM history_text WHERE rowid IN (SELECT id FROM history WHERE profile_id = ?1 AND url = ?2)", params![profile_id, url])?;
        tx.execute(
            "INSERT INTO history_text (rowid, text) SELECT id, ?4 FROM history WHERE id = ?3 AND profile_id = ?1 AND url = ?2",
            params![profile_id, url, visit, text],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Visits whose kept text contains every word of `query` (the last one as a prefix), best
    /// match first. The words are quoted, so `query` cannot use FTS5 syntax.
    pub fn search_history_text(&self, profile_id: Id, query: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let terms: Vec<String> = query
            .split_whitespace()
            .filter(|t| t.chars().any(char::is_alphanumeric))
            .take(8)
            .map(|t| format!("\"{}\"", t.chars().take(64).collect::<String>().replace('"', "\"\"")))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare_cached(
            "SELECT h.url, h.title, h.visited_at,
                 (SELECT COUNT(*) FROM history v WHERE v.profile_id = h.profile_id AND v.url = h.url),
                 (SELECT SUM(typed) FROM history v WHERE v.profile_id = h.profile_id AND v.url = h.url)
             FROM history_text t JOIN history h ON h.id = t.rowid
             WHERE history_text MATCH ?2 AND h.profile_id = ?1 ORDER BY bm25(history_text) LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![profile_id, terms.join(" ") + "*", limit as i64], |r| {
            Ok(HistoryEntry { url: r.get(0)?, title: r.get(1)?, visited_at: r.get(2)?, visits: r.get(3)?, typed: r.get(4)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Forgets the kept text of every profile's visits.
    pub fn clear_history_text(&self) -> Result<()> {
        self.conn.execute("DELETE FROM history_text", [])?;
        self.checkpoint()
    }

    /// Clears visited pages and the site icons, zoom levels, muted sites and hidden elements kept for them; all reveal where the profile went.
    pub fn clear_history(&self, profile_id: Id) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for table in ["history", "favicons", "site_zoom", "muted_sites", "shield_sites", "zapped_elements"] {
            tx.execute(&format!("DELETE FROM {table} WHERE profile_id = ?1"), [profile_id])?;
        }
        tx.commit()?;
        self.checkpoint()
    }

    pub fn delete_history_since(&self, profile_id: Id, since: i64) -> Result<()> {
        self.conn.execute("DELETE FROM history WHERE profile_id = ?1 AND visited_at >= ?2", params![profile_id, since])?;
        self.checkpoint()
    }

    /// Drops the site icon and the Shield site count kept for `host`.
    pub fn forget_site_records(&self, profile_id: Id, host: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for table in ["favicons", "shield_sites"] {
            tx.execute(&format!("DELETE FROM {table} WHERE profile_id = ?1 AND host = ?2"), params![profile_id, host])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_history_url(&self, profile_id: Id, url: &str) -> Result<()> {
        self.conn.execute("DELETE FROM history WHERE profile_id = ?1 AND url = ?2", params![profile_id, url])?;
        self.checkpoint()
    }

    pub fn local_day_start(&self, at: i64) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT CAST(strftime('%s', ?1 / 1000, 'unixepoch', 'localtime', 'start of day', 'utc') AS INTEGER) * 1000", [at], |r| r.get(0))?)
    }

    /// Moves on with every write to the settings or the profiles, for caches of what Settings shows.
    pub fn settings_changes(&self) -> u64 {
        self.settings_rev.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn settings_changed(&self) {
        self.settings_rev.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// The local calendar date of `at` (ms), as YYYY-MM-DD.
    pub fn local_date(&self, at: i64) -> Result<String> {
        Ok(self.conn.query_row("SELECT strftime('%Y-%m-%d', ?1 / 1000, 'unixepoch', 'localtime')", [at], |r| r.get(0))?)
    }

    pub fn favicons(&self, profile_id: Id) -> Result<Vec<(String, Vec<u8>)>> {
        let mut stmt = self.conn.prepare("SELECT host, png FROM favicons WHERE profile_id = ?1")?;
        let rows = stmt.query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_favicon(&self, profile_id: Id, host: &str, png: &[u8]) -> Result<()> {
        self.conn.execute(
            "INSERT INTO favicons (profile_id, host, png, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (profile_id, host) DO UPDATE SET png = excluded.png, updated_at = excluded.updated_at",
            params![profile_id, host, png, now_ms()],
        )?;
        Ok(())
    }

    /// Hosts whose icon was stored before `before` (Unix time in milliseconds).
    pub fn stale_favicons(&self, profile_id: Id, before: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT host FROM favicons WHERE profile_id = ?1 AND updated_at < ?2")?;
        let rows = stmt.query_map(params![profile_id, before], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_download(&self, profile_id: Id, url: &str, path: &str, state: &str, at: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO downloads (profile_id, url, path, state, started_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![profile_id, url, path, state, at],
        )?;
        Ok(())
    }

    /// Newest first.
    pub fn downloads(&self, profile_id: Id) -> Result<Vec<Download>> {
        let mut stmt = self.conn.prepare("SELECT id, url, path, state, started_at FROM downloads WHERE profile_id = ?1 ORDER BY started_at DESC, id DESC")?;
        let rows =
            stmt.query_map([profile_id], |r| Ok(Download { id: r.get(0)?, url: r.get(1)?, path: r.get(2)?, state: r.get(3)?, started_at: r.get(4)? }))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_download(&self, profile_id: Id, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM downloads WHERE profile_id = ?1 AND id = ?2", params![profile_id, id])?;
        self.checkpoint().ok();
        Ok(())
    }

    pub fn clear_downloads(&self, profile_id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM downloads WHERE profile_id = ?1", [profile_id])?;
        self.checkpoint().ok();
        Ok(())
    }

    /// Rewrites download paths inside the folder `from` to the same place under `to`.
    pub fn move_download_paths(&self, from: &str, to: &str) -> Result<usize> {
        Ok(self
            .conn
            .execute("UPDATE downloads SET path = ?2 || substr(path, length(?1) + 1) WHERE substr(path, 1, length(?1) + 1) = ?1 || '/'", params![from, to])?)
    }

    pub fn site_zoom(&self, profile_id: Id, host: &str) -> Result<Option<f64>> {
        Ok(self.conn.query_row("SELECT zoom FROM site_zoom WHERE profile_id = ?1 AND host = ?2", params![profile_id, host], |r| r.get(0)).optional()?)
    }

    pub fn site_zooms(&self, profile_id: Id) -> Result<Vec<(String, f64)>> {
        let mut stmt = self.conn.prepare("SELECT host, zoom FROM site_zoom WHERE profile_id = ?1 ORDER BY host")?;
        let rows = stmt.query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// `None` forgets the host's zoom.
    pub fn set_site_zoom(&self, profile_id: Id, host: &str, zoom: Option<f64>) -> Result<()> {
        let Some(zoom) = zoom else {
            self.conn.execute("DELETE FROM site_zoom WHERE profile_id = ?1 AND host = ?2", params![profile_id, host])?;
            self.checkpoint().ok();
            return Ok(());
        };
        self.conn.execute(
            "INSERT INTO site_zoom (profile_id, host, zoom) VALUES (?1, ?2, ?3)
             ON CONFLICT (profile_id, host) DO UPDATE SET zoom = excluded.zoom",
            params![profile_id, host, zoom],
        )?;
        Ok(())
    }

    pub fn clear_site_zooms(&self, profile_id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM site_zoom WHERE profile_id = ?1", [profile_id])?;
        self.checkpoint().ok();
        Ok(())
    }

    pub fn muted_sites(&self, profile_id: Id) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT host FROM muted_sites WHERE profile_id = ?1 ORDER BY host")?;
        let rows = stmt.query_map([profile_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_site_muted(&self, profile_id: Id, host: &str, muted: bool) -> Result<()> {
        if muted {
            self.conn.execute("INSERT OR IGNORE INTO muted_sites (profile_id, host) VALUES (?1, ?2)", params![profile_id, host])?;
        } else {
            self.conn.execute("DELETE FROM muted_sites WHERE profile_id = ?1 AND host = ?2", params![profile_id, host])?;
            self.checkpoint().ok();
        }
        Ok(())
    }

    /// Hosts where ad, tracker and cookie-banner blocking is off in this profile.
    pub fn paused_sites(&self, profile_id: Id) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT host FROM paused_sites WHERE profile_id = ?1 ORDER BY host")?;
        let rows = stmt.query_map([profile_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_site_paused(&self, profile_id: Id, host: &str, paused: bool) -> Result<()> {
        if paused {
            self.conn.execute("INSERT OR IGNORE INTO paused_sites (profile_id, host) VALUES (?1, ?2)", params![profile_id, host])?;
        } else {
            self.conn.execute("DELETE FROM paused_sites WHERE profile_id = ?1 AND host = ?2", params![profile_id, host])?;
            self.checkpoint().ok();
        }
        Ok(())
    }

    /// Hidden elements as (host, selector), by host in the order they were hidden.
    pub fn zapped_elements(&self, profile_id: Id) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT host, selector FROM zapped_elements WHERE profile_id = ?1 ORDER BY host, rowid")?;
        let rows = stmt.query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Callers check `selector` with `shield::lists::valid_zap` first.
    pub fn zap_element(&self, profile_id: Id, host: &str, selector: &str) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO zapped_elements (profile_id, host, selector) VALUES (?1, ?2, ?3)", params![profile_id, host, selector])?;
        Ok(())
    }

    /// Shows one hidden element of `host` again, or all of them with `None`.
    pub fn unzap_element(&self, profile_id: Id, host: &str, selector: Option<&str>) -> Result<()> {
        self.conn.execute(
            "DELETE FROM zapped_elements WHERE profile_id = ?1 AND host = ?2 AND (?3 IS NULL OR selector = ?3)",
            params![profile_id, host, selector],
        )?;
        self.checkpoint().ok();
        Ok(())
    }

    /// Hosts with their own style sheet in this profile.
    pub fn site_css(&self, profile_id: Id) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT host, css FROM site_css WHERE profile_id = ?1 ORDER BY host")?;
        let rows = stmt.query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Blank `css` removes the host's style sheet; more than `MAX_SITE_CSS` bytes is refused.
    pub fn set_site_css(&self, profile_id: Id, host: &str, css: &str) -> Result<()> {
        if css.len() > MAX_SITE_CSS {
            return Err(Error::Invalid("site CSS over 64 KiB"));
        }
        if css.trim().is_empty() {
            self.conn.execute("DELETE FROM site_css WHERE profile_id = ?1 AND host = ?2", params![profile_id, host])?;
        } else {
            self.conn.execute(
                "INSERT INTO site_css (profile_id, host, css) VALUES (?1, ?2, ?3)
                 ON CONFLICT (profile_id, host) DO UPDATE SET css = excluded.css",
                params![profile_id, host, css],
            )?;
        }
        self.checkpoint().ok();
        Ok(())
    }

    /// Hosts this profile shows in forced dark mode.
    /// One site's style sheet, empty when it has none.
    pub fn site_css_for(&self, profile_id: Id, host: &str) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT css FROM site_css WHERE profile_id = ?1 AND host = ?2", params![profile_id, host], |r| r.get(0))
            .optional()?
            .unwrap_or_default())
    }

    pub fn site_is_dark(&self, profile_id: Id, host: &str) -> Result<bool> {
        Ok(self.conn.query_row("SELECT 1 FROM site_dark WHERE profile_id = ?1 AND host = ?2", params![profile_id, host], |_| Ok(())).optional()?.is_some())
    }

    pub fn dark_sites(&self, profile_id: Id) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT host FROM site_dark WHERE profile_id = ?1 ORDER BY host")?;
        let rows = stmt.query_map([profile_id], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_site_dark(&self, profile_id: Id, host: &str, dark: bool) -> Result<()> {
        if dark {
            self.conn.execute("INSERT OR IGNORE INTO site_dark (profile_id, host) VALUES (?1, ?2)", params![profile_id, host])?;
        } else {
            self.conn.execute("DELETE FROM site_dark WHERE profile_id = ?1 AND host = ?2", params![profile_id, host])?;
            self.checkpoint().ok();
        }
        Ok(())
    }

    pub fn site_override(&self, profile_id: Id, host: &str) -> Result<Option<ShieldLevel>> {
        let level: Option<String> = self
            .conn
            .query_row("SELECT shield_level FROM site_overrides WHERE profile_id = ?1 AND host = ?2", params![profile_id, host], |r| r.get(0))
            .optional()?;
        level.map(|l| ShieldLevel::parse(&l).ok_or(Error::Invalid("shield level"))).transpose()
    }

    pub fn site_overrides(&self, profile_id: Id) -> Result<Vec<(String, ShieldLevel)>> {
        let mut stmt = self.conn.prepare("SELECT host, shield_level FROM site_overrides WHERE profile_id = ?1")?;
        let rows = stmt.query_map([profile_id], |r| {
            let level: String = r.get(1)?;
            Ok((r.get(0)?, ShieldLevel::parse(&level).ok_or_else(|| invalid(1))?))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_site_override(&self, profile_id: Id, host: &str, level: Option<ShieldLevel>) -> Result<()> {
        match level {
            Some(level) => self.conn.execute(
                "INSERT INTO site_overrides (profile_id, host, shield_level) VALUES (?1, ?2, ?3)
                 ON CONFLICT (profile_id, host) DO UPDATE SET shield_level = excluded.shield_level",
                params![profile_id, host, level.as_str()],
            )?,
            None => self.conn.execute("DELETE FROM site_overrides WHERE profile_id = ?1 AND host = ?2", params![profile_id, host])?,
        };
        Ok(())
    }

    pub fn permission(&self, profile_id: Id, origin: &str, kind: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT decision FROM permissions WHERE profile_id = ?1 AND origin = ?2 AND kind = ?3", params![profile_id, origin, kind], |r| r.get(0))
            .optional()?)
    }

    pub fn set_permission(&self, profile_id: Id, origin: &str, kind: &str, decision: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO permissions (profile_id, origin, kind, decision) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (profile_id, origin, kind) DO UPDATE SET decision = excluded.decision",
            params![profile_id, origin, kind, decision],
        )?;
        Ok(())
    }

    /// The profile's remembered permissions: origin, kind and decision.
    pub fn permissions(&self, profile_id: Id) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare("SELECT origin, kind, decision FROM permissions WHERE profile_id = ?1 ORDER BY origin, kind")?;
        let rows = stmt.query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_permission(&self, profile_id: Id, origin: &str, kind: &str) -> Result<()> {
        self.conn.execute("DELETE FROM permissions WHERE profile_id = ?1 AND origin = ?2 AND kind = ?3", params![profile_id, origin, kind])?;
        self.checkpoint().ok();
        Ok(())
    }

    /// All rows of one profile as JSON, one array per table; blobs are hex strings.
    pub fn export_profile(&self, profile_id: Id) -> Result<String> {
        use rusqlite::types::ValueRef;
        use serde_json::{Map, Value};
        use std::fmt::Write;
        let mut out = Map::new();
        for (table, filter) in PROFILE_TABLES {
            // An FTS5 table's `*` leaves out the rowid, which links the text to its visit.
            let columns = if table == "history_text" { "rowid AS history_id, text" } else { "*" };
            let mut stmt = self.conn.prepare(&format!("SELECT {columns} FROM {table} WHERE {filter}"))?;
            let columns: Vec<String> = stmt.column_names().map(str::to_owned).collect();
            let rows = stmt.query_map([profile_id], |r| {
                let mut row = Map::new();
                for (i, name) in columns.iter().enumerate() {
                    let value = match r.get_ref(i)? {
                        ValueRef::Null => Value::Null,
                        ValueRef::Integer(n) => n.into(),
                        ValueRef::Real(f) => f.into(),
                        ValueRef::Text(t) => String::from_utf8_lossy(t).into(),
                        ValueRef::Blob(b) => b
                            .iter()
                            .fold(String::with_capacity(b.len() * 2), |mut hex, x| {
                                let _ = write!(hex, "{x:02x}");
                                hex
                            })
                            .into(),
                    };
                    row.insert(name.clone(), value);
                }
                Ok(Value::Object(row))
            })?;
            out.insert(table.to_owned(), Value::Array(rows.collect::<rusqlite::Result<_>>()?));
        }
        Ok(serde_json::to_string_pretty(&Value::Object(out)).unwrap_or_default())
    }

    /// Deletes every row of the profile except the profile itself, extensions included.
    pub fn empty_profile(&self, profile_id: Id) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (table, filter) in PROFILE_TABLES.iter().skip(1).rev() {
            tx.execute(&format!("DELETE FROM {table} WHERE {filter}"), [profile_id])?;
        }
        tx.commit()?;
        self.checkpoint().ok();
        Ok(())
    }

    /// Deletes every row of the profile except the profile itself, then gives it one empty space.
    pub fn reset_profile(&self, profile_id: Id, space_name: &str) -> Result<Space> {
        let tx = self.conn.unchecked_transaction()?;
        for (table, filter) in PROFILE_TABLES.iter().skip(1).rev().filter(|(t, _)| !KEPT_ON_RESET.contains(t)) {
            tx.execute(&format!("DELETE FROM {table} WHERE {filter}"), [profile_id])?;
        }
        tx.execute("INSERT INTO spaces (profile_id, name, icon, position) VALUES (?1, ?2, '', 0)", params![profile_id, space_name])?;
        let space = Space {
            id: tx.last_insert_rowid(),
            profile_id,
            name: space_name.to_owned(),
            icon: String::new(),
            position: 0,
            color: String::new(),
            schedule: Schedule::default(),
        };
        tx.commit()?;
        self.checkpoint().ok();
        Ok(space)
    }

    pub fn extensions(&self, profile_id: Id) -> Result<Vec<InstalledExtension>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {EXTENSION_COLUMNS} FROM extensions WHERE profile_id = ?1 ORDER BY id"))?;
        let rows = stmt.query_map([profile_id], extension_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every installed path of every profile, for the launch sweep.
    pub fn extension_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM extensions")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Inserts or replaces the row of `e.install_id`; an update keeps the install id.
    pub fn put_extension(&self, e: &InstalledExtension) -> Result<()> {
        self.conn.execute(
            &format!(
                "INSERT INTO extensions ({EXTENSION_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT (profile_id, install_id) DO UPDATE SET source = ?3, path = ?4, name = ?5, version = ?6, sha256 = ?7,
             archive_sha256 = ?8, permissions = ?9, enabled = ?10, all_sites = ?11, owner_id = ?12, asset = ?13, release_tag = ?14"
            ),
            params![
                e.profile_id,
                e.install_id,
                e.source,
                e.path,
                e.name,
                e.version,
                e.sha256,
                e.archive_sha256,
                e.permissions,
                e.enabled,
                e.all_sites,
                e.owner_id,
                e.asset,
                e.release_tag
            ],
        )?;
        Ok(())
    }

    /// Deletes the install and every host decision and stored item kept for it.
    pub fn delete_extension(&self, profile_id: Id, install_id: &str) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM extension_permissions WHERE profile_id = ?1 AND install_id = ?2", params![profile_id, install_id])?;
        tx.execute("DELETE FROM extension_storage WHERE profile_id = ?1 AND install_id = ?2", params![profile_id, install_id])?;
        tx.execute("DELETE FROM extensions WHERE profile_id = ?1 AND install_id = ?2", params![profile_id, install_id])?;
        tx.commit()?;
        Ok(())
    }

    /// Host decisions for one install: (host, allowed).
    pub fn extension_hosts(&self, profile_id: Id, install_id: &str) -> Result<Vec<(String, bool)>> {
        let mut stmt =
            self.conn.prepare("SELECT host, decision = 'allow' FROM extension_permissions WHERE profile_id = ?1 AND install_id = ?2 ORDER BY host")?;
        let rows = stmt.query_map(params![profile_id, install_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn set_extension_host(&self, profile_id: Id, install_id: &str, host: &str, allow: bool) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO extension_permissions (profile_id, install_id, host, decision) VALUES (?1, ?2, ?3, ?4)",
            params![profile_id, install_id, host, if allow { "allow" } else { "deny" }],
        )?;
        Ok(())
    }

    /// Keeps only the decisions whose host passes `keep`, for an update that no longer asks for the others.
    pub fn retain_extension_hosts(&self, profile_id: Id, install_id: &str, keep: impl Fn(&str) -> bool) -> Result<()> {
        for (host, _) in self.extension_hosts(profile_id, install_id)? {
            if !keep(&host) {
                self.conn.execute(
                    "DELETE FROM extension_permissions WHERE profile_id = ?1 AND install_id = ?2 AND host = ?3",
                    params![profile_id, install_id, host],
                )?;
            }
        }
        Ok(())
    }

    /// `storage.local` of one install: (key, JSON value).
    pub fn extension_storage(&self, profile_id: Id, install_id: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM extension_storage WHERE profile_id = ?1 AND install_id = ?2 ORDER BY key")?;
        let rows = stmt.query_map(params![profile_id, install_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Writes all `changes` or none; `None` removes the key.
    pub fn change_extension_storage(&self, profile_id: Id, install_id: &str, changes: &[(String, Option<String>)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (key, value) in changes {
            match value {
                Some(value) => tx.execute(
                    "INSERT OR REPLACE INTO extension_storage (profile_id, install_id, key, value) VALUES (?1, ?2, ?3, ?4)",
                    params![profile_id, install_id, key, value],
                )?,
                None => {
                    tx.execute("DELETE FROM extension_storage WHERE profile_id = ?1 AND install_id = ?2 AND key = ?3", params![profile_id, install_id, key])?
                }
            };
        }
        tx.commit()?;
        Ok(())
    }

    /// With `secure_delete`, deleted rows are zeroed in the database; this also clears them from the WAL.
    /// Only this connection opens the file, so it is busy only while one of its own statements runs;
    /// deleting callers ignore that rather than report a delete that happened as failed.
    fn checkpoint(&self) -> Result<()> {
        let busy: i64 = self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy != 0 {
            return Err(Error::Invalid("checkpoint busy"));
        }
        Ok(())
    }

    pub fn windows(&self) -> Result<Vec<SavedWindow>> {
        let mut stmt = self.conn.prepare("SELECT id, web_engine, x, y, width, height, space_id, active_tab_id, position FROM windows ORDER BY position")?;
        let rows = stmt.query_map([], |r| {
            Ok(SavedWindow {
                id: r.get(0)?,
                web_engine: r.get::<_, Option<String>>(1)?.and_then(|e| WebEngine::parse(&e)),
                x: r.get(2)?,
                y: r.get(3)?,
                width: r.get(4)?,
                height: r.get(5)?,
                space_id: r.get(6)?,
                active_tab_id: r.get(7)?,
                position: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Adds `window` after the others; its `id` and `position` are assigned here.
    pub fn create_window(&self, window: &SavedWindow) -> Result<SavedWindow> {
        let (id, position) = self.conn.query_row(
            "INSERT INTO windows (web_engine, x, y, width, height, space_id, active_tab_id, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, (SELECT COALESCE(MAX(position) + 1, 0) FROM windows))
             RETURNING id, position",
            params![window.web_engine.map(WebEngine::as_str), window.x, window.y, window.width, window.height, window.space_id, window.active_tab_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(SavedWindow { id, position, ..window.clone() })
    }

    /// Saves frame, space and active tab; the engine is fixed when the window is created.
    pub fn update_window(&self, window: &SavedWindow) -> Result<()> {
        self.conn.execute(
            "UPDATE windows SET x = ?2, y = ?3, width = ?4, height = ?5, space_id = ?6, active_tab_id = ?7 WHERE id = ?1",
            params![window.id, window.x, window.y, window.width, window.height, window.space_id, window.active_tab_id],
        )?;
        Ok(())
    }

    pub fn delete_window(&self, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM windows WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Installed and user-made themes, oldest first.
    pub fn themes(&self) -> Result<Vec<InstalledTheme>> {
        let mut stmt = self.conn.prepare(&format!("SELECT {THEME_COLUMNS} FROM themes ORDER BY id"))?;
        let rows = stmt.query_map([], theme_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn theme(&self, id: Id) -> Result<Option<InstalledTheme>> {
        Ok(self.conn.query_row(&format!("SELECT {THEME_COLUMNS} FROM themes WHERE id = ?1"), [id], theme_row).optional()?)
    }

    /// Inserts `t` when its id is 0, else replaces that row; returns the row's id. CSS over
    /// `MAX_THEME_CSS` bytes is refused.
    pub fn put_theme(&self, t: &InstalledTheme) -> Result<Id> {
        if t.css.len() > MAX_THEME_CSS {
            return Err(Error::Invalid("theme over 256 KiB"));
        }
        let values = params![t.source, t.repo_id, t.owner_id, t.tag, t.name, t.version, t.scheme, t.css, t.sha256, t.page_css, t.installed_at];
        if t.id == 0 {
            self.conn.execute(
                "INSERT INTO themes (source, repo_id, owner_id, tag, name, version, scheme, css, sha256, page_css, installed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                values,
            )?;
            Ok(self.conn.last_insert_rowid())
        } else {
            let changed = self.conn.execute(
                "UPDATE themes SET source = ?1, repo_id = ?2, owner_id = ?3, tag = ?4, name = ?5, version = ?6, scheme = ?7,
                 css = ?8, sha256 = ?9, page_css = ?10, installed_at = ?11 WHERE id = ?12",
                params![t.source, t.repo_id, t.owner_id, t.tag, t.name, t.version, t.scheme, t.css, t.sha256, t.page_css, t.installed_at, t.id],
            )?;
            if changed == 0 {
                return Err(Error::Invalid("theme"));
            }
            Ok(t.id)
        }
    }

    pub fn delete_theme(&self, id: Id) -> Result<()> {
        self.conn.execute("DELETE FROM themes WHERE id = ?1", [id])?;
        Ok(())
    }

    /// App-wide settings that belong to no profile, such as the filter-list update switch.
    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    /// Every setting whose key starts with `prefix`.
    pub fn settings_with_prefix(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM settings WHERE substr(key, 1, length(?1)) = ?1")?;
        let rows = stmt.query_map([prefix], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_setting(&self, key: &str) -> Result<()> {
        self.conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        self.settings_changed();
        Ok(())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value", params![key, value])?;
        self.settings_changed();
        Ok(())
    }
}

fn theme_row(r: &Row) -> rusqlite::Result<InstalledTheme> {
    Ok(InstalledTheme {
        id: r.get(0)?,
        source: r.get(1)?,
        repo_id: r.get(2)?,
        owner_id: r.get(3)?,
        tag: r.get(4)?,
        name: r.get(5)?,
        version: r.get(6)?,
        scheme: r.get(7)?,
        css: r.get(8)?,
        sha256: r.get(9)?,
        page_css: r.get(10)?,
        installed_at: r.get(11)?,
    })
}

fn extension_row(r: &Row) -> rusqlite::Result<InstalledExtension> {
    Ok(InstalledExtension {
        profile_id: r.get(0)?,
        install_id: r.get(1)?,
        source: r.get(2)?,
        path: r.get(3)?,
        name: r.get(4)?,
        version: r.get(5)?,
        sha256: r.get(6)?,
        archive_sha256: r.get(7)?,
        permissions: r.get(8)?,
        enabled: r.get(9)?,
        all_sites: r.get(10)?,
        owner_id: r.get(11)?,
        asset: r.get(12)?,
        release_tag: r.get(13)?,
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn profile_row(r: &Row) -> rusqlite::Result<Profile> {
    let level: String = r.get(3)?;
    Ok(Profile {
        id: r.get(0)?,
        name: r.get(1)?,
        data_store_uuid: r.get(2)?,
        shield_level: ShieldLevel::parse(&level).ok_or_else(|| invalid(3))?,
        ai_enabled: r.get(4)?,
        is_private: r.get(5)?,
        is_dev: r.get(6)?,
        color: r.get(7)?,
    })
}

fn space_row(r: &Row) -> rusqlite::Result<Space> {
    Ok(Space {
        id: r.get(0)?,
        profile_id: r.get(1)?,
        name: r.get(2)?,
        icon: r.get(3)?,
        position: r.get(4)?,
        color: r.get(5)?,
        // Out-of-range values are masked, so one bad row cannot hide a profile's spaces.
        schedule: Schedule {
            days: (r.get::<_, i64>(6)? & 0x7f) as u8,
            start: r.get::<_, i64>(7)?.rem_euclid(24) as u8,
            end: r.get::<_, i64>(8)?.rem_euclid(24) as u8,
        },
    })
}

fn folder_row(r: &Row) -> rusqlite::Result<Folder> {
    Ok(Folder { id: r.get(0)?, space_id: r.get(1)?, parent_id: r.get(2)?, name: r.get(3)?, position: r.get(4)?, icon: r.get(5)? })
}

fn group_row(r: &Row) -> rusqlite::Result<Group> {
    Ok(Group { id: r.get(0)?, space_id: r.get(1)?, name: r.get(2)?, position: r.get(3)?, collapsed: r.get(4)?, color: r.get(5)?, icon: r.get(6)? })
}

fn bookmark_row(r: &Row) -> rusqlite::Result<Bookmark> {
    Ok(Bookmark { id: r.get(0)?, profile_id: r.get(1)?, parent_id: r.get(2)?, url: r.get(3)?, title: r.get(4)?, position: r.get(5)?, created_at: r.get(6)? })
}

fn tab_row(r: &Row) -> rusqlite::Result<Tab> {
    let kind: String = r.get(3)?;
    Ok(Tab {
        id: r.get(0)?,
        space_id: r.get(1)?,
        folder_id: r.get(2)?,
        kind: TabKind::parse(&kind).ok_or_else(|| invalid(3))?,
        url: r.get(4)?,
        title: r.get(5)?,
        position: r.get(6)?,
        last_active: r.get(7)?,
        group_id: r.get(8)?,
        pinned_url: r.get(9)?,
        custom_title: r.get(10)?,
        web_engine: r.get::<_, Option<String>>(11)?.and_then(|e| WebEngine::parse(&e)),
    })
}

fn check_ai_command(name: &str, template: &str) -> Result<()> {
    if name.trim().is_empty() || template.len() > MAX_AI_TEMPLATE {
        return Err(Error::Invalid("ai command"));
    }
    Ok(())
}

fn invalid(column: usize) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(column, "enum".into(), rusqlite::types::Type::Text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_text_is_clipped_at_a_character_boundary() {
        assert_eq!(clip_history_text("short"), "short");
        let long = format!("{}é", "a".repeat(MAX_HISTORY_TEXT - 1));
        assert_eq!(clip_history_text(&long), "a".repeat(MAX_HISTORY_TEXT - 1));
        assert_eq!(clip_history_text(&"b".repeat(MAX_HISTORY_TEXT + 5)).len(), MAX_HISTORY_TEXT);
    }

    #[test]
    fn only_a_file_store_is_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Store::open(dir.path()).unwrap().on_disk());
        assert!(!Store::open_in_memory().unwrap().on_disk());
    }

    #[test]
    fn profile_tables_cover_the_schema() {
        let store = Store::open_in_memory().unwrap();
        let mut stmt = store.conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap();
        let tables: Vec<String> = stmt.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        let fts_shadow = |t: &str| t.starts_with("history_text_");
        let tables: Vec<String> = tables.into_iter().filter(|t| !PROFILE_NEUTRAL.contains(&t.as_str()) && !fts_shadow(t)).collect();
        for table in &tables {
            assert!(PROFILE_TABLES.iter().any(|(t, _)| t == table), "{table} is missing from PROFILE_TABLES");
        }
        assert_eq!(tables.len(), PROFILE_TABLES.len());
    }

    fn store_before(migration: &str) -> Store {
        let at = MIGRATIONS.iter().position(|m| m.contains(migration)).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        for sql in &MIGRATIONS[..at] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", at as i64).unwrap();
        Store { conn, settings_rev: Default::default() }
    }

    #[test]
    fn shield_stats_count_survive_reopening_and_reset_with_the_profile() {
        let mut store = store_before("CREATE TABLE shield_stats");
        store
            .conn
            .execute_batch(
                "INSERT INTO profiles (id, name, data_store_uuid) VALUES (1, 'P', 'u1'), (2, 'Q', 'u2');
             INSERT INTO history (profile_id, url, title, visited_at) VALUES (1, 'https://a.test/', 'A', 1000);",
            )
            .unwrap();
        store.migrate().unwrap();
        let created = store.shield_stats(1).unwrap().created_at;
        assert!(created > 1_700_000_000_000, "imported history must not date the profile back");
        store.count(1, Counter::TrackingParams, 3, None).unwrap();
        store.count(1, Counter::TrackingParams, 2, None).unwrap();
        store.count(1, Counter::LocalBlocked, 1, None).unwrap();
        let stats = store.shield_stats(1).unwrap();
        assert_eq!((stats.tracking_params, stats.https_upgrades, stats.local_blocked), (5, 0, 1));
        assert!(stats.seed >= 0);
        assert_eq!(store.shield_stats(1).unwrap().seed, stats.seed);
        store.count(9, Counter::HttpsUpgrades, 1, None).unwrap();
        assert!(store.shield_stats(9).is_err());
        store.reset_profile(1, "S").unwrap();
        let after = store.shield_stats(1).unwrap();
        assert_eq!((after.created_at, after.tracking_params, after.local_blocked), (created, 0, 0));
        assert!(store.export_profile(2).unwrap().contains("shield_stats"));
        let made = store.create_profile("R", "u3", false).unwrap();
        assert!(store.shield_stats(made.id).unwrap().created_at > 1_700_000_000_000);
    }

    #[test]
    fn space_schedules_are_saved_and_checked() {
        let store = Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "u", false).unwrap();
        let space = store.create_space(profile.id, "Work", "").unwrap();
        assert_eq!(space.schedule, Schedule::default());
        let work = Schedule { days: 0b011_1110, start: 9, end: 17 };
        store.set_space_schedule(space.id, work).unwrap();
        assert_eq!(store.spaces(profile.id).unwrap()[0].schedule, work);
        assert!(store.set_space_schedule(space.id, Schedule { days: 0x80, ..work }).is_err());
        assert!(store.set_space_schedule(space.id, Schedule { end: 24, ..work }).is_err());
        let (weekday, hour) = store.local_time().unwrap();
        assert!(weekday < 7 && hour < 24);
    }

    #[test]
    fn an_emptied_profile_keeps_only_its_row() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "u", true).unwrap();
        let space = store.create_space(p.id, "S", "").unwrap();
        store.create_tab(space.id, TabKind::Ephemeral, None, "https://a.test/", "").unwrap();
        store.set_site_zoom(p.id, "a.test", Some(1.5)).unwrap();
        store.add_history(p.id, "https://a.test/", "A", 1, false).unwrap();
        store.empty_profile(p.id).unwrap();
        assert!(store.profile(p.id).is_ok());
        let left: i64 = PROFILE_TABLES
            .iter()
            .skip(1)
            .map(|(t, f)| store.conn.query_row(&format!("SELECT COUNT(*) FROM {t} WHERE {f}"), [p.id], |r| r.get::<_, i64>(0)).unwrap())
            .sum();
        assert_eq!(left, 0);
    }

    #[test]
    fn themes_are_kept_replaced_capped_and_deleted() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.themes().unwrap().is_empty());
        let mut t = InstalledTheme {
            source: "yours".into(),
            name: "Mine".into(),
            scheme: "light".into(),
            css: ":root {}".into(),
            installed_at: 5,
            ..Default::default()
        };
        t.id = store.put_theme(&t).unwrap();
        assert_eq!(store.theme(t.id).unwrap().as_ref(), Some(&t));
        t.css = ":root { --tarn-accent: #000; }".into();
        t.page_css = true;
        assert_eq!(store.put_theme(&t).unwrap(), t.id, "a set id replaces the row");
        assert_eq!(store.themes().unwrap(), [t.clone()]);
        assert!(store.put_theme(&InstalledTheme { css: "x".repeat(MAX_THEME_CSS + 1), ..t.clone() }).is_err());
        assert!(store.put_theme(&InstalledTheme { id: t.id + 9, ..t.clone() }).is_err(), "an unknown id is not inserted");
        store.delete_theme(t.id).unwrap();
        assert_eq!(store.theme(t.id).unwrap(), None);
    }

    #[test]
    fn settings_are_listed_by_literal_prefix() {
        let store = Store::open_in_memory().unwrap();
        for (key, value) in [("keymap.tarn::NewTab", "alt-cmd-k"), ("keymap_x", "no"), ("keymap%", "no"), ("other", "no")] {
            store.set_setting(key, value).unwrap();
        }
        assert_eq!(store.settings_with_prefix("keymap.").unwrap(), vec![("keymap.tarn::NewTab".to_owned(), "alt-cmd-k".to_owned())]);
    }

    #[test]
    fn web_engine_rejects_unknown_values_and_old_databases_migrate() {
        let mut store = store_before("ADD COLUMN web_engine");
        store
            .conn
            .execute_batch(
                "INSERT INTO profiles (id, name, data_store_uuid) VALUES (1, 'P', 'uuid');
             INSERT INTO spaces (id, profile_id, name, icon, position) VALUES (1, 1, 'S', '', 0);
             INSERT INTO tabs (id, space_id, kind, url, title, position) VALUES (1, 1, 'ephemeral', 'https://a.test/', 'A', 0);",
            )
            .unwrap();
        store.migrate().unwrap();
        assert_eq!(store.tab(1).unwrap().web_engine, None);
        assert!(store.conn.execute("UPDATE tabs SET web_engine = 'gecko' WHERE id = 1", []).is_err());
    }

    #[test]
    fn space_rules_are_validated_replaced_and_go_with_their_space() {
        let mut store = store_before("CREATE TABLE space_rules");
        store
            .conn
            .execute_batch(
                "INSERT INTO profiles (id, name, data_store_uuid) VALUES (1, 'P', 'uuid');
             INSERT INTO spaces (id, profile_id, name, icon, position) VALUES (1, 1, 'A', '', 0), (2, 1, 'B', '', 1);",
            )
            .unwrap();
        store.migrate().unwrap();
        for bad in ["", "*", "*.", "a*.com", "*.*.com", "a..com", ".a.com", "a.com/", "a.com:80", "https://a.com"] {
            assert!(store.set_space_rule(bad, 1).is_err(), "{bad}");
        }
        store.set_space_rule("GitHub.com", 1).unwrap();
        store.set_space_rule("*.example.com", 1).unwrap();
        store.set_space_rule("github.com", 2).unwrap();
        let rules: Vec<(String, Id)> = store.space_rules().unwrap().into_iter().map(|r| (r.host_pattern, r.space_id)).collect();
        assert_eq!(rules, [("*.example.com".to_owned(), 1), ("github.com".to_owned(), 2)]);
        store.delete_space(1).unwrap();
        let rule = store.space_rules().unwrap().remove(0);
        assert_eq!(rule.host_pattern, "github.com");
        store.delete_space_rule(rule.id).unwrap();
        assert!(store.space_rules().unwrap().is_empty());
    }

    #[test]
    fn enabled_extensions_move_from_settings_into_rows_with_every_site_granted() {
        let mut store = store_before("ADD COLUMN install_id");
        store
            .conn
            .execute_batch(
                "INSERT INTO profiles (id, name, data_store_uuid) VALUES (1, 'P', 'u1'), (2, 'Q', 'u2');
             INSERT INTO settings (key, value) VALUES ('extensions.1', '/A/a.appex' || char(10) || '/B/b.appex'),
                 ('extensions.2', ''), ('extensions.9', '/C/c.appex'), ('filter_updates', 'off');",
            )
            .unwrap();
        store.migrate().unwrap();
        let rows = store.extensions(1).unwrap();
        assert_eq!(rows.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(), ["/A/a.appex", "/B/b.appex"]);
        assert!(rows.iter().all(|e| e.enabled && e.all_sites && e.source == "appex" && e.install_id.len() == 32));
        assert_ne!(rows[0].install_id, rows[1].install_id);
        assert!(store.extensions(2).unwrap().is_empty());
        assert_eq!(store.setting("extensions.1").unwrap(), None);
        assert_eq!(store.setting("filter_updates").unwrap().as_deref(), Some("off"));
        assert_eq!(store.setting("extension_all_sites_notice").unwrap().as_deref(), Some("1"));
        assert_eq!(store.extension_paths().unwrap().len(), 2);
    }

    #[test]
    fn extension_rows_and_host_decisions_go_with_remove_and_hosts_alone_with_reset() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        let mut e =
            InstalledExtension { profile_id: p.id, install_id: "i1".into(), source: "file".into(), path: "/x/1".into(), enabled: true, ..Default::default() };
        store.put_extension(&e).unwrap();
        e.path = "/x/2".into();
        store.put_extension(&e).unwrap();
        assert_eq!(store.extensions(p.id).unwrap(), [e.clone()]);
        store.set_extension_host(p.id, "i1", "a.test", true).unwrap();
        store.set_extension_host(p.id, "i1", "b.test", false).unwrap();
        store.retain_extension_hosts(p.id, "i1", |h| h != "b.test").unwrap();
        assert_eq!(store.extension_hosts(p.id, "i1").unwrap(), [("a.test".to_owned(), true)]);
        store.reset_profile(p.id, "S").unwrap();
        assert!(store.extension_hosts(p.id, "i1").unwrap().is_empty());
        assert_eq!(store.extensions(p.id).unwrap().len(), 1);
        store.set_extension_host(p.id, "i1", "a.test", true).unwrap();
        store.change_extension_storage(p.id, "i1", &[("k".into(), Some("1".into())), ("j".into(), Some("[2]".into()))]).unwrap();
        store.change_extension_storage(p.id, "i1", &[("j".into(), None)]).unwrap();
        assert_eq!(store.extension_storage(p.id, "i1").unwrap(), [("k".to_owned(), "1".to_owned())]);
        store.delete_extension(p.id, "i1").unwrap();
        assert!(store.extension_storage(p.id, "i1").unwrap().is_empty());
        assert!(store.extensions(p.id).unwrap().is_empty());
        assert!(store.extension_hosts(p.id, "i1").unwrap().is_empty());
    }

    #[test]
    fn tab_ids_lists_stored_tabs_only() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        let space = store.create_space(p.id, "S", "").unwrap();
        let kept = store.create_tab(space.id, TabKind::Ephemeral, None, "https://a.test/", "A").unwrap();
        let gone = store.create_tab(space.id, TabKind::Pinned, None, "https://b.test/", "B").unwrap();
        store.delete_tab(gone.id).unwrap();
        assert_eq!(store.tab_ids().unwrap(), HashSet::from([kept.id]));
    }

    #[test]
    fn windows_round_trip_lose_deleted_references_and_reject_unknown_engines() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        let space = store.create_space(p.id, "S", "").unwrap();
        let tab = store.create_tab(space.id, TabKind::Ephemeral, None, "https://a.test/", "A").unwrap();
        let blank = SavedWindow { x: 10., y: 20., width: 800., height: 600., ..Default::default() };
        let first = store.create_window(&blank).unwrap();
        let mut second = store.create_window(&SavedWindow { web_engine: Some(WebEngine::Chromium), ..blank.clone() }).unwrap();
        assert_eq!((first.position, second.position), (0, 1));
        second = SavedWindow { x: -5.5, width: 1024., space_id: Some(space.id), active_tab_id: Some(tab.id), ..second };
        store.update_window(&second).unwrap();
        assert_eq!(store.windows().unwrap(), [first.clone(), second.clone()]);
        store.delete_tab(tab.id).unwrap();
        assert_eq!(store.windows().unwrap()[1].active_tab_id, None);
        store.delete_profile(p.id).unwrap();
        assert_eq!(store.windows().unwrap()[1], SavedWindow { space_id: None, active_tab_id: None, ..second });
        store.delete_window(first.id).unwrap();
        assert_eq!(store.windows().unwrap().len(), 1);
        assert!(store.conn.execute("UPDATE windows SET web_engine = 'gecko'", []).is_err());
    }

    #[test]
    fn temporary_spaces_are_unlisted_and_go_without_a_window() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        let space = store.create_space(p.id, "S", "").unwrap();
        let (shown, orphan) = (store.create_temporary_space(p.id, "T").unwrap(), store.create_temporary_space(p.id, "T").unwrap());
        assert_eq!(store.spaces(p.id).unwrap(), std::slice::from_ref(&space));
        assert_eq!(store.every_space(p.id).unwrap().len(), 3);
        assert_eq!(store.temporary_space(shown.id).unwrap().profile_id, p.id);
        assert!(store.temporary_space(space.id).is_err(), "a listed space is not temporary");
        store.create_window(&SavedWindow { space_id: Some(shown.id), ..Default::default() }).unwrap();
        store.delete_orphan_temporary_spaces().unwrap();
        assert!(store.temporary_space(shown.id).is_ok() && store.temporary_space(orphan.id).is_err());
        assert_eq!(store.spaces(p.id).unwrap(), [space]);
    }

    #[test]
    fn history_is_deleted_by_address_or_since_a_time() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "u1", false).unwrap().id;
        let other = store.create_profile("Q", "u2", false).unwrap().id;
        for (url, at) in [("https://a.test/", 10), ("https://b.test/", 20), ("https://a.test/", 30)] {
            store.add_history(p, url, "", at, false).unwrap();
        }
        store.add_history(other, "https://a.test/", "", 40, false).unwrap();
        let urls = |profile| store.history(profile, 10).unwrap().into_iter().map(|h| h.url).collect::<Vec<_>>();
        store.delete_history_since(p, 20).unwrap();
        assert_eq!(urls(p), ["https://a.test/"]);
        store.delete_history_url(p, "https://a.test/").unwrap();
        assert!(urls(p).is_empty());
        assert_eq!(urls(other), ["https://a.test/"], "another profile's history stays");
        let before = store.settings_changes();
        store.add_history(p, "https://b.test/", "", 50, false).unwrap();
        assert_eq!(store.settings_changes(), before, "a visit is not a settings change");
        store.set_setting("k", "v").unwrap();
        assert!(store.settings_changes() > before);
        let day = store.local_day_start(1_000_000_000_000).unwrap();
        assert!(day <= 1_000_000_000_000 && 1_000_000_000_000 - day < 86_400_000);
        assert_eq!(store.local_day_start(day).unwrap(), day);
        let noon = day + 12 * 3_600_000;
        assert_eq!(store.local_date(day).unwrap(), store.local_date(noon).unwrap(), "local midnight and noon share a date");
        assert_ne!(store.local_date(day - 1).unwrap(), store.local_date(day).unwrap());
    }

    #[test]
    fn shield_sites_rank_hosts_and_go_with_history() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "u1", false).unwrap().id;
        store.count(p, Counter::HttpsUpgrades, 1, Some("a.test")).unwrap();
        store.count(p, Counter::LocalBlocked, 1, Some("b.test")).unwrap();
        store.count(p, Counter::TrackingParams, 3, Some("b.test")).unwrap();
        store.count(p, Counter::TrackingParams, 1, Some("")).unwrap();
        assert_eq!(store.shield_sites(p, 5).unwrap(), [("b.test".to_owned(), 4), ("a.test".to_owned(), 1)]);
        assert_eq!(store.shield_sites(p, 1).unwrap().len(), 1);
        assert_eq!((store.shield_site_total(p, "b.test").unwrap(), store.shield_site_total(p, "c.test").unwrap()), (4, 0));
        assert_eq!(store.shield_site_total(p + 1, "b.test").unwrap(), 0, "another profile's sites stay apart");
        for i in 0..MAX_SHIELD_SITES + 5 {
            store.count(p, Counter::HttpsUpgrades, 1, Some(&format!("h{i}.test"))).unwrap();
        }
        assert_eq!(store.shield_sites(p, 1000).unwrap().len() as i64, MAX_SHIELD_SITES);
        assert_eq!(store.shield_sites(p, 1).unwrap()[0], ("b.test".to_owned(), 4), "the busiest stay");
        store.clear_history(p).unwrap();
        assert!(store.shield_sites(p, 5).unwrap().is_empty());
        assert_eq!(store.shield_stats(p).unwrap().tracking_params, 4);
    }

    #[test]
    fn shield_week_buckets_counts_per_day_and_profile() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "u1", false).unwrap().id;
        let other = store.create_profile("Q", "u2", false).unwrap().id;
        let day = |offset: i64| -> String { store.conn.query_row("SELECT date('now', 'localtime', ?1)", [format!("{offset} days")], |r| r.get(0)).unwrap() };
        store
            .conn
            .execute_batch(&format!(
                "INSERT INTO shield_daily VALUES ({p}, '{}', 'tracking_params', 4), ({p}, '{}', 'local_blocked', 2), ({p}, '{}', 'https_upgrades', 9);",
                day(-2),
                day(-6),
                day(-7),
            ))
            .unwrap();
        store.count(p, Counter::TrackingParams, 3, None).unwrap();
        store.count(p, Counter::TrackingParams, 2, None).unwrap();
        store.count(p, Counter::HttpsUpgrades, 1, None).unwrap();
        store.count(p, Counter::ListedStopped, 1, Some("a.test")).unwrap();
        store.count(p, Counter::PopupsBlocked, 2, Some("a.test")).unwrap();
        store.count(p, Counter::ChromiumRequests, 7, Some("b.test")).unwrap();
        store.count(other, Counter::HttpsUpgrades, 50, None).unwrap();
        store.count(9, Counter::HttpsUpgrades, 1, None).unwrap();

        let week = store.shield_week(p).unwrap();
        assert_eq!(week.iter().map(|d| d.day.clone()).collect::<Vec<_>>(), (-6..=0).map(day).collect::<Vec<_>>());
        let today =
            ShieldDay { day: day(0), tracking_params: 5, https_upgrades: 1, local_blocked: 0, listed_stopped: 1, popups_blocked: 2, chromium_requests: 7 };
        assert_eq!(week[6], today);
        assert_eq!(store.shield_sites(p, 5).unwrap(), [("b.test".to_owned(), 7), ("a.test".to_owned(), 3)]);
        assert_eq!(week[4].tracking_params, 4);
        assert_eq!(week[0].local_blocked, 2);
        assert_eq!(week.iter().map(|d| d.https_upgrades).sum::<i64>(), 1, "older buckets are dropped");
        let older: i64 = store.conn.query_row("SELECT COUNT(*) FROM shield_daily WHERE day < ?1", [day(-6)], |r| r.get(0)).unwrap();
        assert_eq!(older, 0);
        assert_eq!(store.shield_stats(p).unwrap().tracking_params, 5);
        assert_eq!(store.shield_week(other).unwrap()[6].https_upgrades, 50);
        store.reset_profile(p, "S").unwrap();
        assert!(store.shield_week(p).unwrap().iter().all(|d| *d == ShieldDay { day: d.day.clone(), ..ShieldDay::default() }));
    }

    #[test]
    fn stored_icons_age_from_the_migration_and_renew_when_set() {
        let mut store = store_before("ADD COLUMN updated_at");
        store
            .conn
            .execute_batch(
                "INSERT INTO profiles (id, name, data_store_uuid) VALUES (1, 'P', 'u1');
             INSERT INTO favicons (profile_id, host, png) VALUES (1, 'old.test', x'00'), (1, 'new.test', x'00');",
            )
            .unwrap();
        let before = now_ms() - 1000;
        store.migrate().unwrap();
        assert!(store.stale_favicons(1, before).unwrap().is_empty());
        let later = now_ms() + 1;
        let mut stale = store.stale_favicons(1, later).unwrap();
        stale.sort();
        assert_eq!(stale, ["new.test", "old.test"]);
        store.conn.execute("UPDATE favicons SET updated_at = 0", []).unwrap();
        store.set_favicon(1, "new.test", b"png").unwrap();
        assert_eq!(store.stale_favicons(1, before).unwrap(), ["old.test"]);
        assert!(store.stale_favicons(2, later).unwrap().is_empty());
    }

    #[test]
    fn snapshots_are_per_profile_and_go_with_the_profile_and_its_reset() {
        let mut store = store_before("CREATE TABLE snapshots");
        store.conn.execute_batch("INSERT INTO profiles (id, name, data_store_uuid) VALUES (1, 'P', 'u1'), (2, 'Q', 'u2');").unwrap();
        store.migrate().unwrap();
        let first = store.create_snapshot(1, "A", 10, b"{}").unwrap();
        let second = store.create_snapshot(1, "B", 20, b"[]").unwrap();
        let other = store.create_snapshot(2, "C", 30, b"x").unwrap();
        assert_eq!(store.snapshots(1).unwrap(), [second.clone(), first.clone()]);
        assert_eq!(store.snapshot_data(1, first.id).unwrap(), b"{}");
        assert!(store.snapshot_data(1, other.id).is_err(), "another profile's snapshot stays out of reach");
        store.rename_snapshot(2, first.id, "Taken").unwrap();
        store.delete_snapshot(2, second.id).unwrap();
        store.rename_snapshot(1, first.id, "Renamed").unwrap();
        assert_eq!(store.snapshots(1).unwrap().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["B", "Renamed"]);
        assert!(store.export_profile(1).unwrap().contains("\"snapshots\""));
        store.reset_profile(1, "S").unwrap();
        assert!(store.snapshots(1).unwrap().is_empty());
        store.delete_profile(2).unwrap();
        assert_eq!(store.conn.query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn feature_hints_count_per_profile_and_go_with_the_profile_and_its_reset() {
        let mut store = store_before("CREATE TABLE feature_hints");
        store.conn.execute_batch("INSERT INTO profiles (id, name, data_store_uuid, created_at) VALUES (1, 'P', 'u1', 5), (2, 'Q', 'u2', 6);").unwrap();
        store.migrate().unwrap();
        assert_eq!(store.hint_counts(1, "Copy Link").unwrap(), (5, None, 0));
        store.count_hint(1, "Copy Link", 10).unwrap();
        store.count_hint(1, "Copy Link", 20).unwrap();
        store.count_hint(1, "Reader", 15).unwrap();
        assert_eq!(store.hint_counts(1, "Copy Link").unwrap(), (5, Some(20), 2));
        assert_eq!(store.hint_counts(2, "Copy Link").unwrap(), (6, None, 0), "counters are per profile");
        assert!(store.export_profile(1).unwrap().contains("\"feature_hints\""));
        store.reset_profile(1, "S").unwrap();
        assert_eq!(store.hint_counts(1, "Copy Link").unwrap().2, 0);
        store.count_hint(2, "Reader", 1).unwrap();
        store.delete_profile(2).unwrap();
        assert_eq!(store.conn.query_row("SELECT COUNT(*) FROM feature_hints", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn notes_are_clean_capped_per_profile_and_go_with_the_profile_and_its_reset() {
        let mut store = store_before("CREATE TABLE notes");
        store.conn.execute_batch("INSERT INTO profiles (id, name, data_store_uuid, created_at) VALUES (1, 'P', 'u1', 5), (2, 'Q', 'u2', 6);").unwrap();
        store.migrate().unwrap();
        let note = store.save_note(1, None, " Reef\n\u{202e}trip\u{7} ", "Day 1\u{200b}\n\tsnorkel\u{feff}").unwrap();
        assert_eq!((note.title.as_str(), note.body.as_str()), ("Reef trip", "Day 1\n\tsnorkel"));
        let edited = store.save_note(1, Some(note.id), "Reef trip", "Day 2").unwrap();
        assert_eq!(edited.id, note.id);
        store.save_note(1, None, "", "Only text").unwrap();
        assert_eq!(store.notes(1).unwrap().iter().map(|n| n.body.as_str()).collect::<Vec<_>>(), ["Only text", "Day 2"]);
        assert!(store.notes(2).unwrap().is_empty(), "notes are per profile");
        assert!(store.save_note(2, Some(note.id), "x", "y").is_err(), "another profile cannot edit it");
        for (title, body) in [("", " \n"), ("\u{202e}", "\u{200b}"), (&"t".repeat(MAX_NOTE_TITLE + 1), "x"), ("t", &"x".repeat(MAX_NOTE + 1))] {
            assert!(store.save_note(1, None, title, body).is_err(), "{title:?}");
        }
        assert!(store.save_note(1, Some(9999), "t", "x").is_err(), "no such note");
        assert!(store.save_note(1, None, &"é".repeat(MAX_NOTE_TITLE), "x").is_ok(), "the title limit counts characters");
        assert!(store.save_note(1, None, &"é".repeat(MAX_NOTE_TITLE + 1), "x").is_err());
        store.delete_note(1, store.notes(1).unwrap()[0].id).unwrap();
        store.delete_note(2, note.id).unwrap();
        assert_eq!(store.notes(1).unwrap().len(), 2, "another profile's id deletes nothing");
        assert!(store.export_profile(1).unwrap().contains("\"notes\""));
        store.reset_profile(1, "S").unwrap();
        assert!(store.notes(1).unwrap().is_empty());
        store.save_note(2, None, "x", "y").unwrap();
        store.delete_profile(2).unwrap();
        assert_eq!(store.conn.query_row("SELECT COUNT(*) FROM notes", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn notes_are_never_written_for_a_private_profile_and_stop_at_the_cap() {
        let store = Store::open_in_memory().unwrap();
        let private = store.create_profile("Private", "u-p", true).unwrap().id;
        assert!(matches!(store.save_note(private, None, "t", "x"), Err(Error::Invalid("note"))));
        assert!(store.notes(private).unwrap().is_empty());
        let saved = store.create_profile("P", "u1", false).unwrap().id;
        let id = store.save_note(saved, None, "t", "x").unwrap().id;
        store.conn.execute("UPDATE profiles SET is_private = 1 WHERE id = ?1", [saved]).unwrap();
        assert!(store.save_note(saved, Some(id), "t", "changed").is_err(), "nor edited once private");
        store.conn.execute("UPDATE profiles SET is_private = 0 WHERE id = ?1", [saved]).unwrap();
        for i in 1..MAX_NOTES {
            store.save_note(saved, None, &i.to_string(), "x").unwrap();
        }
        assert!(store.save_note(saved, None, "one more", "x").is_err());
        assert!(store.save_note(saved, Some(id), "t", "an edit still saves").is_ok());
    }

    #[test]
    fn snippets_are_checked_per_profile_and_go_with_the_profile_and_its_reset() {
        let mut store = store_before("CREATE TABLE snippets");
        store.conn.execute_batch("INSERT INTO profiles (id, name, data_store_uuid, created_at) VALUES (1, 'P', 'u1', 5), (2, 'Q', 'u2', 6);").unwrap();
        store.migrate().unwrap();
        let sig = store.save_snippet(1, "sig", "Best,\nAda").unwrap();
        assert_eq!(store.save_snippet(1, "sig", "Thanks,\nAda").unwrap().id, sig.id, "a name keeps its row");
        store.save_snippet(1, "addr", "1 Reef Road").unwrap();
        assert_eq!(
            store.snippets(1).unwrap().iter().map(|s| (s.name.as_str(), s.body.as_str())).collect::<Vec<_>>(),
            [("addr", "1 Reef Road"), ("sig", "Thanks,\nAda")]
        );
        assert!(store.snippets(2).unwrap().is_empty(), "snippets are per profile");
        for (name, body) in [
            ("", "x"),
            ("Sig", "x"),
            ("a b", "x"),
            (";sig", "x"),
            (&"a".repeat(MAX_SNIPPET_NAME + 1), "x"),
            ("ok", " \n"),
            ("ok", &"x".repeat(MAX_SNIPPET + 1)),
        ] {
            assert!(store.save_snippet(1, name, body).is_err(), "{name:?}");
        }
        store.delete_snippet(2, sig.id).unwrap();
        assert_eq!(store.snippets(1).unwrap().len(), 2, "another profile's id deletes nothing");
        assert!(store.export_profile(1).unwrap().contains("\"snippets\""));
        store.reset_profile(1, "S").unwrap();
        assert!(store.snippets(1).unwrap().is_empty());
        store.save_snippet(2, "x", "y").unwrap();
        store.delete_profile(2).unwrap();
        assert_eq!(store.conn.query_row("SELECT COUNT(*) FROM snippets", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn ai_commands_upgrade_stay_per_profile_and_go_with_the_profile_and_its_reset() {
        let mut store = store_before("CREATE TABLE ai_commands");
        store.conn.execute_batch("INSERT INTO profiles (id, name, data_store_uuid) VALUES (1, 'P', 'u1'), (2, 'Q', 'u2');").unwrap();
        store.migrate().unwrap();
        assert!(!store.profile(1).unwrap().ai_enabled);
        store.set_profile_ai(1, true).unwrap();
        assert!(store.profile(1).unwrap().ai_enabled && !store.profile(2).unwrap().ai_enabled);
        let tldr = store.create_ai_command(1, "TL;DR", "Shorten {page}").unwrap();
        let other = store.create_ai_command(2, "Other", "x").unwrap();
        assert!(store.create_ai_command(1, " ", "x").is_err());
        assert!(store.create_ai_command(1, "Big", &"x".repeat(MAX_AI_TEMPLATE + 1)).is_err());
        store.update_ai_command(2, tldr.id, "Taken", "y").unwrap();
        store.delete_ai_command(2, tldr.id).unwrap();
        assert_eq!(store.ai_commands(1).unwrap(), std::slice::from_ref(&tldr), "another profile cannot change it");
        store.update_ai_command(1, tldr.id, "Short", "Shorten {selection}").unwrap();
        assert_eq!(store.ai_commands(1).unwrap()[0].template, "Shorten {selection}");
        assert!(store.export_profile(1).unwrap().contains("\"ai_commands\""));
        store.reset_profile(1, "S").unwrap();
        assert!(store.ai_commands(1).unwrap().is_empty());
        assert_eq!(store.ai_commands(2).unwrap(), [other]);
        store.delete_profile(2).unwrap();
        assert_eq!(store.conn.query_row("SELECT COUNT(*) FROM ai_commands", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn trailing_dot_overrides_are_merged_into_the_plain_host() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap().id;
        store.set_site_override(p, "a.test.", Some(ShieldLevel::Lockdown)).unwrap();
        store.set_site_override(p, "b.test.", Some(ShieldLevel::Strict)).unwrap();
        store.set_site_override(p, "b.test", Some(ShieldLevel::Standard)).unwrap();
        let merge = MIGRATIONS.iter().find(|m| m.contains("rtrim(host, '.')")).unwrap();
        store.conn.execute_batch(merge).unwrap();
        let mut rows = store.site_overrides(p).unwrap();
        rows.sort();
        assert_eq!(rows, [("a.test".to_owned(), ShieldLevel::Lockdown), ("b.test".to_owned(), ShieldLevel::Standard)]);
    }

    #[test]
    fn zapped_elements_round_trip_and_go_with_history() {
        let mut store = store_before("CREATE TABLE zapped_elements");
        store.conn.execute_batch("INSERT INTO profiles (id, name, data_store_uuid) VALUES (1, 'P', 'u1');").unwrap();
        store.migrate().unwrap();
        for (host, selector) in [("b.test", ".x"), ("a.test", "#top"), ("a.test", ".ad"), ("a.test", "#top")] {
            store.zap_element(1, host, selector).unwrap();
        }
        let row = |h: &str, s: &str| (h.to_owned(), s.to_owned());
        assert_eq!(store.zapped_elements(1).unwrap(), [row("a.test", "#top"), row("a.test", ".ad"), row("b.test", ".x")]);
        store.unzap_element(1, "a.test", Some("#top")).unwrap();
        assert_eq!(store.zapped_elements(1).unwrap(), [row("a.test", ".ad"), row("b.test", ".x")]);
        store.unzap_element(1, "a.test", None).unwrap();
        assert_eq!(store.zapped_elements(1).unwrap(), [row("b.test", ".x")]);
        store.clear_history(1).unwrap();
        assert!(store.zapped_elements(1).unwrap().is_empty());
    }

    #[test]
    fn site_css_is_capped_and_blank_removes_it() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap().id;
        store.set_site_css(p, "a.test", "body { color: red }").unwrap();
        store.set_site_css(p, "a.test", "p { margin: 0 }").unwrap();
        assert!(store.set_site_css(p, "b.test", &"x".repeat(MAX_SITE_CSS + 1)).is_err());
        store.set_site_css(p, "b.test", &"x".repeat(MAX_SITE_CSS)).unwrap();
        assert_eq!(store.site_css(p).unwrap()[0], ("a.test".to_owned(), "p { margin: 0 }".to_owned()));
        store.set_site_css(p, "b.test", " \n").unwrap();
        assert_eq!(store.site_css(p).unwrap().len(), 1);
    }

    #[test]
    fn an_unlisted_new_group_is_placed_once_after_the_listed_ones() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        let space = store.create_space(p.id, "S", "").unwrap().id;
        let (a, b) = (store.create_group(space, "A").unwrap().id, store.create_group(space, "B").unwrap().id);
        let layout = Layout { new_groups: vec![("N".into(), String::new(), String::new())], groups: vec![Slot::Group(b)], ..Default::default() };
        let created = store.apply_layout(space, &layout).unwrap();
        let groups = store.groups(space).unwrap();
        assert_eq!(groups.iter().map(|g| (g.id, g.position)).collect::<Vec<_>>(), [(b, 0), (a, 1), (created[0], 2)]);
    }

    #[test]
    fn a_layout_applies_whole_and_refuses_other_spaces() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        let (space, other) = (store.create_space(p.id, "S", "").unwrap().id, store.create_space(p.id, "O", "").unwrap().id);
        let add = |space: Id, url: &str| store.create_tab(space, TabKind::Ephemeral, None, url, "").unwrap().id;
        let (a, b, c) = (add(space, "https://a.test/"), add(space, "https://b.test/"), add(space, "https://c.test/"));
        let pinned = store.create_tab(space, TabKind::Pinned, None, "https://p.test/", "").unwrap().id;
        let old = store.create_group(space, "Old").unwrap();
        store.update_group(old.id, "Old", true).unwrap();
        let foreign = store.create_group(other, "Foreign").unwrap();
        let layout = Layout {
            new_groups: vec![("New".into(), "blue".into(), "star".into())],
            tabs: vec![(c, Slot::New(0)), (a, Slot::Group(old.id)), (pinned, Slot::Loose), (9999, Slot::Loose)],
            groups: vec![Slot::New(0), Slot::Group(old.id)],
            ..Default::default()
        };
        let created = store.apply_layout(space, &layout).unwrap();
        let tabs = store.tabs(space).unwrap();
        let tab = |id: Id| tabs.iter().find(|t| t.id == id).unwrap().clone();
        assert_eq!((tab(c).group_id, tab(a).group_id, tab(b).group_id), (Some(created[0]), Some(old.id), None));
        assert_eq!((tab(c).position, tab(a).position, tab(b).position), (0, 1, 2), "an unlisted tab keeps its order after the listed ones");
        assert_eq!(tab(pinned).kind, TabKind::Pinned, "a pinned tab is not part of a layout");
        let groups = store.groups(space).unwrap();
        assert_eq!(groups.iter().map(|g| (g.id, g.collapsed)).collect::<Vec<_>>(), [(created[0], false), (old.id, false)], "a group that gets a tab opens");
        assert_eq!((groups[0].color.as_str(), groups[0].icon.as_str()), ("blue", "star"));

        let before = store.tabs(space).unwrap();
        let into_foreign = Layout { tabs: vec![(b, Slot::Group(foreign.id))], ..Default::default() };
        assert!(store.apply_layout(space, &into_foreign).is_err());
        let foreign_tab = add(other, "https://d.test/");
        let with_foreign = Layout {
            new_groups: vec![("X".into(), String::new(), String::new())],
            tabs: vec![(b, Slot::New(0)), (foreign_tab, Slot::Loose)],
            ..Default::default()
        };
        assert!(store.apply_layout(space, &with_foreign).is_err());
        assert_eq!(store.tabs(space).unwrap(), before, "a refused layout changes nothing");
        assert_eq!(store.groups(space).unwrap().len(), 2);

        let drop = Layout {
            tabs: vec![(c, Slot::Loose)],
            drop_if_empty: vec![created[0], old.id, foreign.id],
            collapse: vec![old.id, foreign.id],
            ..Default::default()
        };
        store.apply_layout(space, &drop).unwrap();
        assert_eq!(
            store.groups(space).unwrap().iter().map(|g| (g.id, g.collapsed)).collect::<Vec<_>>(),
            [(old.id, true)],
            "only an empty group of the space is dropped"
        );
        assert!(!store.groups(other).unwrap()[0].collapsed);
        assert_eq!(store.groups(other).unwrap().len(), 1);
        assert_eq!(store.tabs(space).unwrap().len(), 4, "no tab is lost");
    }

    #[test]
    fn a_layout_never_moves_a_tab_from_one_group_into_another() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap();
        let space = store.create_space(p.id, "S", "").unwrap().id;
        let tab = store.create_tab(space, TabKind::Ephemeral, None, "https://a.test/", "").unwrap().id;
        let (first, second) = (store.create_group(space, "A").unwrap().id, store.create_group(space, "B").unwrap().id);
        store.set_tab_group(tab, Some(first)).unwrap();
        let other = store.create_tab(space, TabKind::Ephemeral, None, "https://b.test/", "").unwrap().id;
        store.set_tab_group(other, Some(second)).unwrap();
        store.apply_layout(space, &Layout { tabs: vec![(tab, Slot::Group(second)), (other, Slot::Group(second))], ..Default::default() }).unwrap();
        let tabs = store.tabs(space).unwrap();
        let of = |id: Id| tabs.iter().find(|t| t.id == id).map(|t| (t.group_id, t.position)).unwrap();
        assert_eq!((of(other), of(tab)), ((Some(second), 0), (Some(first), 1)), "a refused tab keeps its group and goes after the placed ones");
        store.apply_layout(space, &Layout { tabs: vec![(tab, Slot::Loose)], ..Default::default() }).unwrap();
        assert_eq!(store.tabs(space).unwrap().iter().find(|t| t.id == tab).unwrap().group_id, None);
    }
}
