use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{Bookmark, Cookie, MAX_DEPTH, Parsed, Result, SameSite, Visit, collect, open, title, web_url};

/// Firefox's top-level bookmark folders by GUID, with the names it shows for them.
const ROOTS: [(&str, &str); 4] =
    [("toolbar_____", "Bookmarks Toolbar"), ("menu________", "Bookmarks Menu"), ("unfiled_____", "Other Bookmarks"), ("mobile______", "Mobile Bookmarks")];
const TYPE_URL: i64 = 1;
const TYPE_FOLDER: i64 = 2;

/// Container and partitioned cookies carry origin attributes; they belong to no plain profile
/// and are skipped.
pub(crate) fn cookies(path: &Path) -> Result<Parsed<Cookie>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare("SELECT host, name, value, path, expiry, isSecure, isHttpOnly, sameSite, originAttributes FROM moz_cookies LIMIT 20000")?;
    collect(&mut stmt.query([])?, |r| {
        if !r.get::<_, String>(8)?.is_empty() {
            return Ok(None);
        }
        let expiry: i64 = r.get(4)?;
        Ok(Some(Cookie {
            host: r.get(0)?,
            name: r.get(1)?,
            value: r.get(2)?,
            path: r.get(3)?,
            // Newer Firefox versions store milliseconds; no cookie expires before 1973 in seconds.
            expires: Some(if expiry > 100_000_000_000 { expiry / 1000 } else { expiry }),
            secure: r.get(5)?,
            http_only: r.get(6)?,
            // Firefox stores a cookie set without SameSite as 0, the value of an explicit None.
            same_site: match r.get::<_, i64>(7)? {
                1 => SameSite::Lax,
                2 => SameSite::Strict,
                _ => SameSite::Unspecified,
            },
            partitioned: false,
        }))
    })
}

pub(crate) fn history(path: &Path) -> Result<Parsed<Visit>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare(
        "SELECT p.url, substr(COALESCE(p.title, ''), 1, 1024), v.visit_date FROM moz_historyvisits v JOIN moz_places p ON p.id = v.place_id
         WHERE p.hidden = 0 AND length(p.url) <= 8192 ORDER BY v.visit_date DESC LIMIT 500000",
    )?;
    collect(&mut stmt.query([])?, |r| {
        let url: String = r.get(0)?;
        if !web_url(&url) {
            return Ok(None);
        }
        Ok(Some(Visit { url, title: title(r.get(1)?), visited_at: r.get::<_, i64>(2)? / 1000 }))
    })
}

struct Row {
    id: i64,
    kind: i64,
    title: String,
    url: Option<String>,
}

pub(crate) fn bookmarks(path: &Path) -> Result<Parsed<Bookmark>> {
    let conn = open(path)?;
    let mut stmt = conn.prepare(
        "SELECT b.id, b.type, b.parent, substr(COALESCE(b.title, ''), 1, 1024), CASE WHEN length(p.url) <= 8192 THEN p.url END, b.guid
         FROM moz_bookmarks b LEFT JOIN moz_places p ON p.id = b.fk ORDER BY b.parent, b.position LIMIT 500000",
    )?;
    let rows = collect(&mut stmt.query([])?, |r| {
        let guid: Option<String> = r.get(5)?;
        Ok(Some((r.get::<_, i64>(2)?, guid, Row { id: r.get(0)?, kind: r.get(1)?, title: title(r.get(3)?), url: r.get(4)? })))
    })?;
    let mut children: HashMap<i64, Vec<Row>> = HashMap::new();
    let mut roots = HashMap::new();
    for (parent, guid, row) in rows.items {
        if let Some((_, name)) = ROOTS.iter().find(|(g, _)| guid.as_deref() == Some(*g)) {
            roots.insert(*name, row.id);
        }
        children.entry(parent).or_default().push(row);
    }
    let mut out = Parsed { items: Vec::new(), skipped: rows.skipped };
    let mut seen = HashSet::new();
    for (_, name) in ROOTS {
        let Some(&id) = roots.get(name) else { continue };
        let items = folder(&children, id, 0, &mut seen, &mut out.skipped).unwrap_or_default();
        if !items.is_empty() {
            out.items.push(Bookmark::Folder { title: name.into(), children: items });
        }
    }
    Ok(out)
}

/// Expands each folder id once: a crafted database with repeated ids or cycles stays linear.
/// `None` for a folder already expanded, which is skipped rather than kept empty.
fn folder(children: &HashMap<i64, Vec<Row>>, id: i64, depth: usize, seen: &mut HashSet<i64>, skipped: &mut usize) -> Option<Vec<Bookmark>> {
    if !seen.insert(id) {
        *skipped += 1;
        return None;
    }
    let mut out = Vec::new();
    for row in children.get(&id).map(Vec::as_slice).unwrap_or_default() {
        match (row.kind, &row.url) {
            (TYPE_URL, Some(url)) if web_url(url) => out.push(Bookmark::Url { title: row.title.clone(), url: url.clone() }),
            (TYPE_FOLDER, _) if depth < MAX_DEPTH => {
                out.extend(folder(children, row.id, depth + 1, seen, skipped).map(|items| Bookmark::Folder { title: row.title.clone(), children: items }))
            }
            (TYPE_URL | TYPE_FOLDER, _) => *skipped += 1,
            _ => {}
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn reads_cookies_and_skips_container_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cookies.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY, originAttributes TEXT NOT NULL DEFAULT '', name TEXT, value TEXT,
                     host TEXT, path TEXT, expiry INTEGER, isSecure INTEGER, isHttpOnly INTEGER, sameSite INTEGER);
                 INSERT INTO moz_cookies (originAttributes, name, value, host, path, expiry, isSecure, isHttpOnly, sameSite) VALUES
                     ('', 'sid', 'v', '.a.test', '/', 2000000000, 1, 1, 2),
                     ('', 'ms', 'v', 'b.test', '/x', 2000000000000, 0, 0, 0),
                     ('^userContextId=1', 'box', 'v', 'a.test', '/', 2000000000, 0, 0, 1),
                     ('', NULL, 'v', 'a.test', '/', 2000000000, 0, 0, 1);",
            )
            .unwrap();
        let parsed = cookies(&path).unwrap();
        assert_eq!(parsed.skipped, 2, "a container cookie and a row with a NULL name are skipped, not fatal");
        let [sid, ms] = parsed.items.as_slice() else { panic!() };
        assert_eq!((sid.host.as_str(), sid.value.as_str(), sid.same_site, sid.expires), (".a.test", "v", SameSite::Strict, Some(2_000_000_000)));
        assert!(sid.secure && sid.http_only);
        assert_eq!((ms.path.as_str(), ms.same_site, ms.expires), ("/x", SameSite::Unspecified, Some(2_000_000_000)));
    }

    #[test]
    fn reads_visits_and_the_bookmark_tree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("places.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url TEXT, title TEXT, hidden INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE moz_historyvisits (id INTEGER PRIMARY KEY, place_id INTEGER, visit_date INTEGER);
                 CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, parent INTEGER, position INTEGER, title TEXT, guid TEXT);
                 INSERT INTO moz_places VALUES (1, 'https://a.test/', 'A', 0), (2, 'place:sort=8', NULL, 0), (3, 'https://b.test/', NULL, 0), (4, 'https://frame.test/', NULL, 1);
                 INSERT INTO moz_historyvisits (place_id, visit_date) VALUES (1, 1700000000000000), (2, 1700000000000000), (4, 1700000000000000);
                 INSERT INTO moz_bookmarks VALUES
                     (1, 2, NULL, 0, 0, '', 'root________'),
                     (2, 2, NULL, 1, 0, 'menu', 'menu________'),
                     (3, 2, NULL, 1, 1, 'toolbar', 'toolbar_____'),
                     (4, 1, 3, 3, 1, 'B', 'b'),
                     (5, 2, NULL, 3, 0, 'F', 'f'),
                     (6, 1, 1, 5, 0, 'A', 'a'),
                     (7, 1, 2, 5, 1, 'Recent', 'r'),
                     (8, 3, NULL, 3, 2, '', 's');",
            )
            .unwrap();
        let visits = history(&path).unwrap();
        assert_eq!((visits.items.len(), visits.skipped), (1, 1));
        assert_eq!(visits.items[0], Visit { url: "https://a.test/".into(), title: "A".into(), visited_at: 1_700_000_000_000 });
        let marks = bookmarks(&path).unwrap();
        assert_eq!(marks.skipped, 1, "the place: query is skipped, the separator ignored");
        let url = |t: &str, u: &str| Bookmark::Url { title: t.into(), url: u.into() };
        assert_eq!(
            marks.items,
            [Bookmark::Folder {
                title: "Bookmarks Toolbar".into(),
                children: vec![Bookmark::Folder { title: "F".into(), children: vec![url("A", "https://a.test/")] }, url("B", "https://b.test/")],
            }]
        );
    }

    #[test]
    fn a_folder_cycle_ends_at_the_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("places.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE moz_places (id INTEGER PRIMARY KEY, url TEXT, title TEXT);
                 CREATE TABLE moz_bookmarks (id INTEGER PRIMARY KEY, type INTEGER, fk INTEGER, parent INTEGER, position INTEGER, title TEXT, guid TEXT);
                 INSERT INTO moz_bookmarks VALUES (2, 2, NULL, 3, 0, 'menu', 'menu________'), (3, 2, NULL, 2, 0, 'loop', 'l');",
            )
            .unwrap();
        assert_eq!(bookmarks(&path).unwrap().skipped, 1);
    }

    #[test]
    fn repeated_folder_ids_are_expanded_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("places.sqlite");
        Connection::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE moz_places (id INTEGER, url TEXT, title TEXT);
                 CREATE TABLE moz_bookmarks (id INTEGER, type INTEGER, fk INTEGER, parent INTEGER, position INTEGER, title TEXT, guid TEXT);
                 INSERT INTO moz_bookmarks VALUES (2, 2, NULL, 0, 0, 'menu', 'menu________'), (2, 2, NULL, 2, 0, 'a', 'a'), (2, 2, NULL, 2, 1, 'b', 'b');",
            )
            .unwrap();
        assert_eq!(bookmarks(&path).unwrap().skipped, 2, "finishes instead of recursing 2^64 times");
    }
}
