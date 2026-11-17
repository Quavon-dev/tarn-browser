//! Reads another browser's cookies, history and bookmarks into plain structs (browser-import
//! plan). Source databases are untrusted: they are opened read-only and immutable, without a
//! copy, and decrypted values stay in memory.

#![forbid(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::config::DbConfig;
use rusqlite::limits::Limit;
use rusqlite::{Connection, OpenFlags};
use zeroize::Zeroize;

mod chromium;
mod crypto;
pub mod detect;
mod firefox;
#[cfg(target_os = "macos")]
mod keychain;
#[cfg(target_os = "linux")]
mod keyring;

pub use detect::{available, sources};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Browser {
    Chrome,
    Edge,
    Brave,
    Arc,
    Dia,
    Chromium,
    Firefox,
}

impl Browser {
    pub const ALL: [Browser; 7] = [Browser::Chrome, Browser::Edge, Browser::Brave, Browser::Arc, Browser::Dia, Browser::Chromium, Browser::Firefox];

    pub fn name(self) -> &'static str {
        match self {
            Browser::Chrome => "Chrome",
            Browser::Edge => "Edge",
            Browser::Brave => "Brave",
            Browser::Arc => "Arc",
            Browser::Dia => "Dia",
            Browser::Chromium => "Chromium",
            Browser::Firefox => "Firefox",
        }
    }

    /// Where the browser keeps its profiles, relative to the home folder.
    #[cfg(target_os = "macos")]
    fn profiles_dirs(self) -> &'static [&'static str] {
        match self {
            Browser::Chrome => &["Library/Application Support/Google/Chrome"],
            Browser::Edge => &["Library/Application Support/Microsoft Edge"],
            Browser::Brave => &["Library/Application Support/BraveSoftware/Brave-Browser"],
            Browser::Arc => &["Library/Application Support/Arc/User Data"],
            Browser::Dia => &["Library/Application Support/Dia/User Data"],
            Browser::Chromium => &[],
            Browser::Firefox => &["Library/Application Support/Firefox/Profiles"],
        }
    }

    /// Where the browser keeps its profiles, relative to the home folder.
    #[cfg(windows)]
    fn profiles_dirs(self) -> &'static [&'static str] {
        match self {
            Browser::Chrome => &["AppData/Local/Google/Chrome/User Data"],
            Browser::Edge => &["AppData/Local/Microsoft/Edge/User Data"],
            Browser::Brave => &["AppData/Local/BraveSoftware/Brave-Browser/User Data"],
            // Arc for Windows is a packaged app: Windows keeps its data in the package's folder.
            Browser::Arc => &["AppData/Local/Packages/TheBrowserCompany.Arc_*/LocalCache/Local/Arc/User Data"],
            Browser::Dia | Browser::Chromium => &[],
            Browser::Firefox => &["AppData/Roaming/Mozilla/Firefox/Profiles"],
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    fn profiles_dirs(self) -> &'static [&'static str] {
        match self {
            Browser::Chrome => &[".config/google-chrome", ".var/app/com.google.Chrome/config/google-chrome"],
            Browser::Edge => &[".config/microsoft-edge", ".var/app/com.microsoft.Edge/config/microsoft-edge"],
            Browser::Brave => &[".config/BraveSoftware/Brave-Browser", ".var/app/com.brave.Browser/config/BraveSoftware/Brave-Browser"],
            Browser::Arc | Browser::Dia => &[],
            Browser::Chromium => &[".config/chromium", ".var/app/org.chromium.Chromium/config/chromium", "snap/chromium/common/chromium"],
            Browser::Firefox => {
                &[".mozilla/firefox", ".config/mozilla/firefox", "snap/firefox/common/.mozilla/firefox", ".var/app/org.mozilla.firefox/.mozilla/firefox"]
            }
        }
    }

    /// The keychain item holding the Safe Storage password: service and account.
    fn keychain_item(self) -> Option<(&'static str, &'static str)> {
        match self {
            Browser::Chrome => Some(("Chrome Safe Storage", "Chrome")),
            Browser::Edge if cfg!(target_os = "linux") => Some(("Chromium Safe Storage", "Chromium")),
            Browser::Edge => Some(("Microsoft Edge Safe Storage", "Microsoft Edge")),
            Browser::Brave => Some(("Brave Safe Storage", "Brave")),
            Browser::Arc => Some(("Arc Safe Storage", "Arc")),
            Browser::Dia => Some(("Dia Safe Storage", "Dia")),
            Browser::Chromium => Some(("Chromium Safe Storage", "Chromium")),
            Browser::Firefox => None,
        }
    }
}

/// One profile of an installed browser. `label` is display-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub browser: Browser,
    pub label: String,
    pub dir: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SameSite {
    Unspecified,
    None,
    Lax,
    Strict,
}

/// A source cookie. Its value is left out of `Debug` and zeroized on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct Cookie {
    /// As the source stores it: a leading dot marks a domain cookie.
    pub host: String,
    pub name: String,
    pub value: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    /// Unix seconds; `None` for a session cookie.
    pub expires: Option<i64>,
    pub partitioned: bool,
}

impl fmt::Debug for Cookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cookie")
            .field("host", &self.host)
            .field("name", &self.name)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("same_site", &self.same_site)
            .field("expires", &self.expires)
            .field("partitioned", &self.partitioned)
            .finish_non_exhaustive()
    }
}

impl Drop for Cookie {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    pub url: String,
    pub title: String,
    /// Unix milliseconds.
    pub visited_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bookmark {
    Url { title: String, url: String },
    Folder { title: String, children: Vec<Bookmark> },
}

/// What a parser read, and how many rows it left out as unreadable or unsupported.
#[derive(Debug)]
pub struct Parsed<T> {
    pub items: Vec<T>,
    pub skipped: usize,
}

impl<T> Default for Parsed<T> {
    fn default() -> Self {
        Parsed { items: Vec::new(), skipped: 0 }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Carries a reason, never a value or a key.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    NotFound,
    /// The system refused to let Tarn read the file (Full Disk Access on macOS).
    DiskAccess,
    /// The user denied access to the browser's keychain item, or cancelled the prompt.
    KeychainDenied,
    Keychain(i32),
    /// The browser encrypts the data in a way Tarn cannot read on this system.
    Unsupported,
    Database(String),
    Format(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => f.write_str("not found"),
            Error::DiskAccess => f.write_str(if cfg!(windows) {
                "Windows did not allow reading the browser's files"
            } else if cfg!(target_os = "macos") {
                "macOS did not allow reading the browser's files"
            } else {
                "the system did not allow reading the browser's files"
            }),
            Error::KeychainDenied => f.write_str(if cfg!(target_os = "linux") { "keyring access was denied" } else { "keychain access was denied" }),
            Error::Keychain(code) => write!(f, "keychain error {code}"),
            Error::Unsupported => f.write_str("not readable on this system"),
            Error::Database(e) => write!(f, "database: {e}"),
            Error::Format(what) => write!(f, "unexpected format: {what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Database(e.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => Error::NotFound,
            std::io::ErrorKind::PermissionDenied => Error::DiskAccess,
            _ => Error::Database(e.kind().to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl Source {
    /// Reads and decrypts the cookies; a Chromium source asks the keychain, which macOS may
    /// confirm with the user, so this blocks and belongs off the main thread.
    pub fn cookies(&self) -> Result<Parsed<Cookie>> {
        match self.browser.keychain_item() {
            None => firefox::cookies(&self.dir.join("cookies.sqlite")),
            Some(item) => {
                let path = [self.dir.join("Network/Cookies"), self.dir.join("Cookies")].into_iter().find(|p| p.exists()).ok_or(Error::NotFound)?;
                std::fs::File::open(&path)?;
                chromium::cookies(&path, &keys(item)?)
            }
        }
    }

    pub fn history(&self) -> Result<Parsed<Visit>> {
        match self.browser {
            Browser::Firefox => firefox::history(&self.dir.join("places.sqlite")),
            _ => chromium::history(&self.dir.join("History")),
        }
    }

    /// The source's top-level folders (bookmark bar, menu, other), each with what it holds.
    pub fn bookmarks(&self) -> Result<Parsed<Bookmark>> {
        match self.browser {
            Browser::Firefox => firefox::bookmarks(&self.dir.join("places.sqlite")),
            _ => chromium::bookmarks(&self.dir.join("Bookmarks")),
        }
    }
}

#[cfg(target_os = "macos")]
fn keys((service, account): (&str, &str)) -> Result<crypto::Keys> {
    Ok(crypto::Keys { v10: crypto::derive(&keychain::password(service, account)?, crypto::MAC_ROUNDS), v11: None })
}

#[cfg(target_os = "linux")]
fn keys((label, _): (&str, &str)) -> Result<crypto::Keys> {
    let v11 = keyring::password(label).map(|secret| crypto::derive(&secret, crypto::LINUX_ROUNDS));
    Ok(crypto::Keys { v10: crypto::derive(crypto::LINUX_V10_PASSWORD, crypto::LINUX_ROUNDS), v11 })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn keys(_: (&str, &str)) -> Result<crypto::Keys> {
    Err(Error::Unsupported)
}

/// Bookmark trees deeper than this are cut off; a crafted source cannot recurse without end.
const MAX_DEPTH: usize = 64;

/// Rows read per source and kind; a crafted database cannot exhaust memory. History keeps the newest.
const MAX_ROWS: usize = 500_000;
const MAX_URL: usize = 8 * 1024;

/// Collects `read` over every row; a row it cannot read, a NULL where a value belongs or `None`
/// from `read`, counts as skipped instead of ending the import.
fn collect<T>(rows: &mut rusqlite::Rows<'_>, mut read: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Option<T>>) -> Result<Parsed<T>> {
    let mut out = Parsed::default();
    while let Some(r) = rows.next()? {
        if out.items.len() == MAX_ROWS {
            out.skipped += 1;
            continue;
        }
        match read(r) {
            Ok(Some(item)) => out.items.push(item),
            _ => out.skipped += 1,
        }
    }
    Ok(out)
}

/// Titles are cut to the length Tarn keeps for a page title.
fn title(mut s: String) -> String {
    if let Some((cut, _)) = s.char_indices().nth(1024) {
        s.truncate(cut);
        s.shrink_to_fit();
    }
    s
}

/// Only pages a browser tab can open again are imported.
fn web_url(url: &str) -> bool {
    if url.len() > MAX_URL {
        return false;
    }
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    matches!(scheme.as_deref(), Some("http" | "https"))
}

/// Opens a source database read-only and immutable, so no journal or copy is written and a
/// running browser's lock is not taken, with attaching, views and schema-defined functions off,
/// so a crafted schema cannot turn a queried table into an endless recursive view.
fn open(path: &Path) -> Result<Connection> {
    std::fs::File::open(path)?;
    let uri = format!("file:{}?mode=ro&immutable=1", encode(path));
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(uri, flags)?;
    conn.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, 1024 * 1024)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_VIEW, false)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    Ok(conn)
}

/// Percent-encodes a path for an SQLite URI, keeping `/` and unreserved characters.
fn encode(path: &Path) -> String {
    let mut out = String::new();
    for &b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_paths_escape_what_sqlite_would_parse() {
        assert_eq!(encode(Path::new("/a b/c?d#e%f")), "/a%20b/c%3Fd%23e%25f");
    }

    #[test]
    fn long_titles_are_cut_on_a_character_boundary() {
        assert_eq!(title("é".repeat(2000)).chars().count(), 1024);
        assert_eq!(title("short".into()), "short");
    }

    #[test]
    fn only_web_urls_are_kept() {
        assert!(web_url("https://a.test/") && web_url("HTTP://a.test"));
        assert!(!web_url("chrome://settings") && !web_url("javascript:alert(1)") && !web_url("place:sort=8"));
    }

    #[test]
    fn a_cookie_debug_leaves_the_value_out() {
        let c = Cookie {
            host: ".a.test".into(),
            name: "sid".into(),
            value: "secret".into(),
            path: "/".into(),
            secure: true,
            http_only: true,
            same_site: SameSite::Lax,
            expires: None,
            partitioned: false,
        };
        assert!(!format!("{c:?}").contains("secret"));
    }
}
