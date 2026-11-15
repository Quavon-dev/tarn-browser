CREATE TABLE profiles (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    data_store_uuid TEXT NOT NULL UNIQUE,
    shield_level TEXT NOT NULL DEFAULT 'standard' CHECK (shield_level IN ('standard','strict','lockdown')),
    ai_enabled INTEGER NOT NULL DEFAULT 0,
    is_private INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE spaces (
    id INTEGER PRIMARY KEY,
    profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    icon TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL
);
CREATE TABLE folders (
    id INTEGER PRIMARY KEY,
    space_id INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    parent_id INTEGER REFERENCES folders(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    position INTEGER NOT NULL
);
CREATE TABLE tabs (
    id INTEGER PRIMARY KEY,
    space_id INTEGER NOT NULL REFERENCES spaces(id) ON DELETE CASCADE,
    folder_id INTEGER REFERENCES folders(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('tile','pinned','ephemeral')),
    url TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL,
    last_active INTEGER NOT NULL DEFAULT 0,
    interaction_state BLOB,
    CHECK (folder_id IS NULL OR kind = 'pinned')
);
CREATE TABLE history (
    id INTEGER PRIMARY KEY,
    profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    visited_at INTEGER NOT NULL
);
CREATE INDEX history_profile_visited ON history(profile_id, visited_at DESC);
CREATE TABLE site_overrides (
    profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    host TEXT NOT NULL,
    shield_level TEXT NOT NULL CHECK (shield_level IN ('standard','strict','lockdown')),
    PRIMARY KEY (profile_id, host)
);
CREATE TABLE permissions (
    profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    origin TEXT NOT NULL,
    kind TEXT NOT NULL,
    decision TEXT NOT NULL,
    PRIMARY KEY (profile_id, origin, kind)
);
CREATE TABLE downloads (
    id INTEGER PRIMARY KEY,
    profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    path TEXT NOT NULL,
    state TEXT NOT NULL,
    started_at INTEGER NOT NULL
);
CREATE TABLE favicons (
    profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    host TEXT NOT NULL,
    png BLOB NOT NULL,
    PRIMARY KEY (profile_id, host)
);
CREATE TABLE extensions (
    id INTEGER PRIMARY KEY,
    profile_id INTEGER NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 0
);
