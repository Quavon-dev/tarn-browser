use std::path::Path;

use serde_json::Value;

use crate::{Bookmark, Cookie, Error, MAX_DEPTH, Parsed, Result, SameSite, Visit, collect, crypto, open, title, web_url};

/// Seconds between 1601-01-01, Chromium's epoch, and the Unix epoch.
const EPOCH_DELTA: i64 = 11_644_473_600;
/// From this cookie database version on, every encrypted value starts with its host's hash.
const HASHED_VERSION: i64 = 24;
const MAX_BOOKMARKS_FILE: u64 = 64 * 1024 * 1024;

fn unix_seconds(chromium_micros: i64) -> i64 {
    chromium_micros / 1_000_000 - EPOCH_DELTA
}

pub(crate) fn cookies(path: &Path, key: &[u8; 16]) -> Result<Parsed<Cookie>> {
    let conn = open(path)?;
    let version: i64 = conn.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'version'", [], |r| r.get(0))?;
    let mut stmt = conn.prepare(
        "SELECT host_key, name, value, encrypted_value, path, expires_utc, is_secure, is_httponly, samesite,
                has_expires AND is_persistent, COALESCE(top_frame_site_key, '') != '' FROM cookies LIMIT 20000",
    )?;
    collect(&mut stmt.query([])?, |r| {
        let host: String = r.get(0)?;
        let plain: String = r.get(2)?;
        let encrypted: Vec<u8> = r.get(3)?;
        let value = if !plain.is_empty() || encrypted.is_empty() {
            Some(plain)
        } else {
            crypto::decrypt(key, &encrypted, &host, version >= HASHED_VERSION).map(|v| v.to_string())
        };
        let Some(value) = value else { return Ok(None) };
        Ok(Some(Cookie {
            host,
            name: r.get(1)?,
            value,
            path: r.get(4)?,
            expires: r.get::<_, bool>(9)?.then(|| r.get(5).map(unix_seconds)).transpose()?,
            secure: r.get(6)?,
            http_only: r.get(7)?,
            same_site: match r.get::<_, i64>(8)? {
                0 => SameSite::None,
                1 => SameSite::Lax,
                2 => SameSite::Strict,
                _ => SameSite::Unspecified,
            },
            partitioned: r.get(10)?,
        }))
    })
}

pub(crate) fn history(path: &Path) -> Result<Parsed<Visit>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare(
        "SELECT u.url, substr(COALESCE(u.title, ''), 1, 1024), v.visit_time FROM visits v JOIN urls u ON u.id = v.url
         WHERE u.hidden = 0 AND length(u.url) <= 8192 ORDER BY v.visit_time DESC LIMIT 500000",
    )?;
    collect(&mut stmt.query([])?, |r| {
        let url: String = r.get(0)?;
        if !web_url(&url) {
            return Ok(None);
        }
        let micros: i64 = r.get(2)?;
        Ok(Some(Visit { url, title: title(r.get(1)?), visited_at: micros / 1000 - EPOCH_DELTA * 1000 }))
    })
}

pub(crate) fn bookmarks(path: &Path) -> Result<Parsed<Bookmark>> {
    std::fs::File::open(path)?;
    if std::fs::metadata(path)?.len() > MAX_BOOKMARKS_FILE {
        return Err(Error::Format("bookmarks file too large"));
    }
    let json: Value = serde_json::from_slice(&std::fs::read(path)?).map_err(|_| Error::Format("bookmarks"))?;
    let roots = json.get("roots").and_then(Value::as_object).ok_or(Error::Format("bookmarks"))?;
    let mut out = Parsed::default();
    for key in ["bookmark_bar", "other", "synced"] {
        if let Some(Bookmark::Folder { title, children }) = roots.get(key).and_then(|n| node(n, 0, &mut out.skipped))
            && !children.is_empty()
        {
            out.items.push(Bookmark::Folder { title, children });
        }
    }
    Ok(out)
}

fn node(n: &Value, depth: usize, skipped: &mut usize) -> Option<Bookmark> {
    let title = title(n.get("name").and_then(Value::as_str).unwrap_or_default().to_owned());
    match n.get("type").and_then(Value::as_str) {
        Some("url") => match n.get("url").and_then(Value::as_str).filter(|u| web_url(u)) {
            Some(url) => Some(Bookmark::Url { title, url: url.to_owned() }),
            None => {
                *skipped += 1;
                None
            }
        },
        Some("folder") if depth < MAX_DEPTH => {
            let children = n.get("children").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
            Some(Bookmark::Folder { title, children: children.iter().filter_map(|c| node(c, depth + 1, skipped)).collect() })
        }
        _ => {
            *skipped += 1;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{derive, encrypt, hashed};
    use rusqlite::{Connection, params};

    fn cookie_db(path: &Path, version: i64, rows: &[(&str, &str, &[u8], i64, i64)]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE cookies (creation_utc INTEGER, host_key TEXT, top_frame_site_key TEXT, name TEXT, value TEXT,
                 encrypted_value BLOB, path TEXT, expires_utc INTEGER, is_secure INTEGER, is_httponly INTEGER,
                 has_expires INTEGER, is_persistent INTEGER, samesite INTEGER);",
        )
        .unwrap();
        conn.execute("INSERT INTO meta VALUES ('version', ?1)", [version.to_string()]).unwrap();
        for (host, name, enc, samesite, flags) in rows {
            conn.execute(
                "INSERT INTO cookies VALUES (0, ?1, ?6, ?2, '', ?3, '/', ?4, ?5 & 1, ?5 >> 1 & 1, 1, 1, ?7)",
                params![host, name, enc, (EPOCH_DELTA + 2_000_000_000) * 1_000_000, flags, if *flags & 4 != 0 { "https://top.test" } else { "" }, samesite],
            )
            .unwrap();
        }
    }

    #[test]
    fn reads_and_decrypts_cookies_with_their_attributes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cookies");
        let key = derive(b"synthetic test password");
        let good = encrypt(&key, &hashed(".a.test", "v1"));
        let other_host = encrypt(&key, &hashed(".evil.test", "v2"));
        let bad_padding = &good[..good.len() - 1];
        cookie_db(
            &path,
            24,
            &[
                (".a.test", "dom", &good, 1, 0b011),
                ("a.test", "stolen", &other_host, 1, 0),
                ("a.test", "broken", bad_padding, 1, 0),
                ("a.test", "v20", b"v20xxxxxxxxxxxxxxxxx", 1, 0),
                ("b.test", "part", b"", 2, 0b100),
            ],
        );
        let parsed = cookies(&path, &key).unwrap();
        assert_eq!(parsed.skipped, 3, "a hash mismatch, bad padding and a v20 value are skipped");
        let [dom, part] = parsed.items.as_slice() else { panic!("{:?}", parsed.items) };
        assert_eq!((dom.host.as_str(), dom.name.as_str(), dom.value.as_str()), (".a.test", "dom", "v1"));
        assert!(dom.secure && dom.http_only && !dom.partitioned);
        assert_eq!((dom.same_site, dom.expires), (SameSite::Lax, Some(2_000_000_000)));
        assert!(part.partitioned && !part.secure);
        assert_eq!(part.same_site, SameSite::Strict);
    }

    #[test]
    fn values_without_the_host_hash_before_version_24() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Cookies");
        let key = derive(b"synthetic test password");
        cookie_db(&path, 23, &[("a.test", "old", &encrypt(&key, b"plain"), -1, 0)]);
        let parsed = cookies(&path, &key).unwrap();
        assert_eq!((parsed.items[0].value.as_str(), parsed.items[0].same_site), ("plain", SameSite::Unspecified));
    }

    #[test]
    fn reads_visits_and_the_bookmark_tree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("History");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT, hidden INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE visits (id INTEGER PRIMARY KEY, url INTEGER, visit_time INTEGER);
             INSERT INTO urls VALUES (1, 'https://a.test/', 'A', 0), (2, 'chrome://settings', 'S', 0), (3, 'https://frame.test/', 'F', 1);
             INSERT INTO visits (url, visit_time) VALUES (1, 13000000000000000), (1, 13000000001000000), (2, 13000000000000000), (3, 13000000000000000);",
        )
        .unwrap();
        let visits = history(&path).unwrap();
        assert_eq!(visits.skipped, 1);
        assert_eq!(visits.items.len(), 2);
        assert_eq!(visits.items[1], Visit { url: "https://a.test/".into(), title: "A".into(), visited_at: 1_355_526_400_000 }, "newest first");

        let path = dir.path().join("Bookmarks");
        std::fs::write(
            &path,
            r#"{"roots":{"bookmark_bar":{"type":"folder","name":"Bookmarks Bar","children":[
                {"type":"url","name":"A","url":"https://a.test/"},
                {"type":"folder","name":"F","children":[{"type":"url","name":"B","url":"https://b.test/"},{"type":"url","name":"J","url":"javascript:x"}]}]},
              "other":{"type":"folder","name":"Other Bookmarks","children":[]},
              "synced":{"type":"folder","name":"Mobile Bookmarks","children":[]}}}"#,
        )
        .unwrap();
        let marks = bookmarks(&path).unwrap();
        assert_eq!(marks.skipped, 1);
        let url = |t: &str, u: &str| Bookmark::Url { title: t.into(), url: u.into() };
        assert_eq!(
            marks.items,
            [Bookmark::Folder {
                title: "Bookmarks Bar".into(),
                children: vec![url("A", "https://a.test/"), Bookmark::Folder { title: "F".into(), children: vec![url("B", "https://b.test/")] }],
            }]
        );
    }

    #[test]
    fn a_view_in_place_of_a_table_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("History");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT, hidden INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO urls VALUES (1, 'https://a.test/', 'A', 0);
                 CREATE VIEW visits AS SELECT 1 AS id, 1 AS url, 13000000000000000 AS visit_time;",
            )
            .unwrap();
        assert!(matches!(history(&path), Err(Error::Database(_))), "a view, which could recurse without end, is never read");
    }

    #[test]
    fn one_huge_title_repeated_by_many_visits_stays_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("History");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE urls (id INTEGER PRIMARY KEY, url TEXT, title TEXT, hidden INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE visits (id INTEGER PRIMARY KEY, url INTEGER, visit_time INTEGER);
             INSERT INTO urls (id, url, title) VALUES (1, 'https://a.test/', ''), (2, 'https://b.test/', 'B');
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
             INSERT INTO visits (url, visit_time) SELECT 1, 13000000000000000 + i FROM n;",
        )
        .unwrap();
        conn.execute("UPDATE urls SET title = ?1 WHERE id = 1", ["x".repeat(4 * 1024 * 1024)]).unwrap();
        drop(conn);
        match history(&path) {
            Ok(visits) => assert!(visits.items.iter().all(|v| v.title.len() <= 1024 * 4)),
            Err(e) => assert!(matches!(e, Error::Database(_)), "{e}"),
        }
    }

    #[test]
    fn a_deeply_nested_tree_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Bookmarks");
        let mut tree = r#"{"type":"url","name":"x","url":"https://x.test/"}"#.to_owned();
        for _ in 0..100 {
            tree = format!(r#"{{"type":"folder","name":"f","children":[{tree}]}}"#);
        }
        std::fs::write(&path, format!(r#"{{"roots":{{"bookmark_bar":{tree}}}}}"#)).unwrap();
        assert_eq!(bookmarks(&path).unwrap_err(), Error::Format("bookmarks"), "serde_json's recursion limit refuses it");
    }

    fn real_source() -> crate::Source {
        crate::sources(&std::env::home_dir().unwrap()).into_iter().find(|s| s.browser != crate::Browser::Firefox).expect("a Chromium browser")
    }

    /// Reads a real profile's history and bookmarks: `cargo test -p import -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the local browser profile"]
    fn reads_a_real_profile() {
        let source = real_source();
        let (visits, marks) = (source.history().unwrap(), source.bookmarks().unwrap());
        println!(
            "{:?}: {} visits ({} skipped), {} bookmark roots ({} skipped)",
            source.browser,
            visits.items.len(),
            visits.skipped,
            marks.items.len(),
            marks.skipped
        );
        assert!(!visits.items.is_empty());
    }

    /// Decrypts a real profile's cookies; macOS asks for the keychain item once.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "reads the local browser profile and the keychain"]
    fn decrypts_a_real_profile() {
        let source = real_source();
        let cookies = source.cookies().unwrap();
        println!("{:?}: {} cookies, {} skipped", source.browser, cookies.items.len(), cookies.skipped);
        assert!(cookies.skipped * 10 < cookies.items.len(), "the domain hash matches host_key");
    }
}
