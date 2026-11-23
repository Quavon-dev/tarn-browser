use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncError {
    TooLarge,
    Truncated,
    BadMagic,
    UnsupportedVersion,
    WeakKdf,
    CostlyKdf,
    /// Wrong passphrase or tampered file; the two cannot be told apart.
    Decrypt,
    /// The file belongs to another sync set or passphrase (different salt or KDF parameters).
    KeyMismatch,
    Malformed,
    TooMany,
    TooLong,
    BadId,
    /// A timestamp is negative or too far in the future; it would win every later merge.
    BadTime,
    Rng,
}

/// 32 lowercase hex characters (UUID v4, simple form).
pub type SyncId = String;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Meta {
    /// Milliseconds since the Unix epoch.
    pub updated_at: i64,
    pub device: SyncId,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Space {
    pub id: SyncId,
    pub name: String,
    pub icon: String,
    pub position: i64,
    pub meta: Meta,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Folder {
    pub id: SyncId,
    pub space: SyncId,
    pub parent: Option<SyncId>,
    pub name: String,
    pub position: i64,
    pub meta: Meta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TabKind {
    Tile,
    Pinned,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Tab {
    pub id: SyncId,
    pub space: SyncId,
    pub folder: Option<SyncId>,
    pub kind: TabKind,
    pub url: String,
    pub title: String,
    pub position: i64,
    pub meta: Meta,
}

/// Keys: `shield_level`, `site_override:<host>`; values: Shield level names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Setting {
    pub key: String,
    pub value: String,
    pub meta: Meta,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub device: SyncId,
    /// Incremented by the writing device on every write. Rejecting a lower value than last seen
    /// from this device (rollback) and checking `device` against the file name is the caller's job.
    pub seq: u64,
    pub spaces: Vec<Space>,
    pub folders: Vec<Folder>,
    pub tabs: Vec<Tab>,
    pub settings: Vec<Setting>,
}
