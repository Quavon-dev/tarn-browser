//! Brings another browser's data into a profile (browser-import plan): history and bookmarks
//! through the store, cookies into the profile's WebKit jar, which the cookie bridge carries to
//! Chromium on each site's next load; on Windows straight into its Chromium jar. Only a normal
//! persistent profile is a target.

use engine::cookies::{Cookie, SameSite};
pub use import::{Bookmark, Browser, Error, Outcome, Parsed, Source, Visit, available, sources};
use shield::ShieldLevel;
use store::{Id, Store};

use crate::{Core, WebsiteData, now};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kinds {
    pub cookies: bool,
    pub history: bool,
    pub bookmarks: bool,
}

/// What was read from a source; `None` for a kind that was not asked for.
#[derive(Debug, Default)]
pub struct Read {
    pub cookies: Option<import::Result<Parsed<import::Cookie>>>,
    pub history: Option<import::Result<Parsed<Visit>>>,
    pub bookmarks: Option<import::Result<Parsed<Bookmark>>>,
}

/// Reads `kinds` from `source`. Blocks on disk and on the keychain prompt, so it runs off the
/// main thread.
pub fn read(source: &Source, kinds: Kinds) -> Read {
    Read {
        cookies: kinds.cookies.then(|| source.cookies()),
        history: kinds.history.then(|| source.history()),
        bookmarks: kinds.bookmarks.then(|| source.bookmarks()),
    }
}

/// Per kind, what was imported or why nothing was.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub cookies: Option<Result<Outcome, Error>>,
    pub history: Option<Result<Outcome, Error>>,
    pub bookmarks: Option<Result<Outcome, Error>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Private and Lockdown profiles keep website data in memory only.
    NotPersistent,
    /// The profile has no open runtime to write cookies into.
    NotOpen,
}

/// A source cookie as the bridge writes it. WebKit reads a cookie without `SameSite` as `Lax`.
pub fn engine_cookie(c: &mut import::Cookie) -> Cookie {
    Cookie {
        name: std::mem::take(&mut c.name),
        value: std::mem::take(&mut c.value),
        domain: c.host.trim_start_matches('.').to_ascii_lowercase(),
        host_only: !c.host.starts_with('.'),
        path: std::mem::take(&mut c.path),
        secure: c.secure,
        http_only: c.http_only,
        same_site: match c.same_site {
            import::SameSite::None => SameSite::None,
            import::SameSite::Strict => SameSite::Strict,
            import::SameSite::Lax | import::SameSite::Unspecified => SameSite::Lax,
        },
        expires: c.expires,
        partitioned: c.partitioned,
    }
}

fn store_error(e: store::Error) -> Error {
    match e {
        store::Error::Sql(e) => Error::Database(e.to_string()),
        other => Error::Database(other.to_string()),
    }
}

/// Adds the visits the profile does not have yet.
pub fn import_history(store: &Store, profile_id: Id, parsed: &Parsed<Visit>) -> Result<Outcome, Error> {
    let added = store.add_history_bulk(profile_id, parsed.items.iter().map(|v| (v.url.as_str(), v.title.as_str(), v.visited_at))).map_err(store_error)?;
    Ok(Outcome { imported: added, skipped: parsed.skipped + parsed.items.len() - added, failed: 0 })
}

/// Adds the bookmark tree under one top-level "Imported from <browser>" folder. A folder of the
/// same name at the same place is reused and a URL its folder already holds is skipped, so a
/// second import adds nothing twice.
pub fn import_bookmarks(store: &Store, profile_id: Id, browser: &str, parsed: &Parsed<Bookmark>) -> Result<Outcome, Error> {
    let title = format!("Imported from {browser}");
    let mut out = Outcome { skipped: parsed.skipped, ..Outcome::default() };
    if parsed.items.is_empty() {
        return Ok(out);
    }
    store
        .batch(|s| {
            let mut existing = s.bookmarks(profile_id)?;
            let root = folder(s, profile_id, &mut existing, None, &title)?;
            add(s, profile_id, &mut existing, root, &parsed.items, &mut out)
        })
        .map_err(store_error)?;
    Ok(out)
}

/// Each existing folder is reused once, so same-named siblings keep matching their own folder.
fn folder(s: &Store, profile_id: Id, existing: &mut Vec<store::Bookmark>, parent: Option<Id>, title: &str) -> Result<Id, store::Error> {
    match existing.iter().position(|b| b.parent_id == parent && b.url.is_none() && b.title == title) {
        Some(ix) => Ok(existing.remove(ix).id),
        None => Ok(s.add_bookmark(profile_id, parent, None, title, now())?.id),
    }
}

fn add(s: &Store, profile_id: Id, existing: &mut Vec<store::Bookmark>, parent: Id, nodes: &[Bookmark], out: &mut Outcome) -> Result<(), store::Error> {
    for node in nodes {
        match node {
            Bookmark::Url { url, .. } if existing.iter().any(|b| b.parent_id == Some(parent) && b.url.as_deref() == Some(url)) => out.skipped += 1,
            Bookmark::Url { title, url } => {
                s.add_bookmark(profile_id, Some(parent), Some(url), title, now())?;
                out.imported += 1;
            }
            Bookmark::Folder { title, children } => {
                let id = folder(s, profile_id, existing, Some(parent), title)?;
                add(s, profile_id, existing, id, children, out)?;
            }
        }
    }
    Ok(())
}

impl<D: WebsiteData> Core<D> {
    /// Whether `profile_id` may receive imported data: a normal profile, not private, not
    /// Lockdown, and not using the store of a Lockdown profile, which empties it whenever it opens.
    pub fn import_target(&self, private: bool, profile_id: Id) -> Result<(), Refused> {
        let mut profile = self.store(private).profile(profile_id).map_err(|_| Refused::NotOpen)?;
        if private || profile.is_private || profile.shield_level == ShieldLevel::Lockdown {
            return Err(Refused::NotPersistent);
        }
        crate::share_store(&self.disk, &mut profile);
        let lockdown_owns = self
            .disk
            .profiles()
            .unwrap_or_default()
            .into_iter()
            .any(|p| p.data_store_uuid == profile.data_store_uuid && p.shield_level == ShieldLevel::Lockdown);
        if lockdown_owns {
            return Err(Refused::NotPersistent);
        }
        Ok(())
    }

    /// Writes what `read` holds into the persistent profile `profile_id`; `done` gets the report
    /// once the engine has answered for every cookie.
    pub fn import(&self, profile_id: Id, browser: import::Browser, read: Read, done: impl FnOnce(Report) + 'static) -> Result<(), Refused> {
        self.import_target(false, profile_id)?;
        if read.cookies.as_ref().is_some_and(Result::is_ok) && !self.profiles.borrow().contains_key(&(false, profile_id)) {
            return Err(Refused::NotOpen);
        }
        let mut report = Report {
            history: read.history.map(|r| r.and_then(|p| import_history(&self.disk, profile_id, &p))),
            bookmarks: read.bookmarks.map(|r| r.and_then(|p| import_bookmarks(&self.disk, profile_id, browser.name(), &p))),
            cookies: None,
        };
        match read.cookies {
            Some(Ok(mut parsed)) => {
                let cookies = parsed.items.iter_mut().map(engine_cookie).collect();
                let skipped = parsed.skipped;
                // The runtime's borrow ends with this statement; WebKit answers later.
                self.profiles.borrow().get(&(false, profile_id)).ok_or(Refused::NotOpen)?.data.import_cookies(
                    cookies,
                    now() / 1000,
                    Box::new(move |imported, refused| {
                        report.cookies = Some(Ok(Outcome { imported, skipped, failed: refused }));
                        done(report);
                    }),
                );
            }
            other => {
                report.cookies = other.map(|r| r.map(|_| Outcome::default()));
                done(report);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(title: &str, url: &str) -> Bookmark {
        Bookmark::Url { title: title.into(), url: url.into() }
    }

    #[test]
    fn source_cookies_map_to_the_bridge_fields() {
        let mut c = import::Cookie {
            host: ".A.test".into(),
            name: "sid".into(),
            value: "v".into(),
            path: "/".into(),
            secure: true,
            http_only: true,
            same_site: import::SameSite::Unspecified,
            expires: Some(5),
            partitioned: false,
        };
        let e = engine_cookie(&mut c);
        assert_eq!((e.domain.as_str(), e.host_only, e.value.as_str(), e.same_site), ("a.test", false, "v", SameSite::Lax));
        assert!(c.value.is_empty(), "the source copy of the value is gone");
        c.host = "a.test".into();
        assert!(engine_cookie(&mut c).host_only);
    }

    #[test]
    fn a_second_import_adds_nothing_twice() {
        let store = Store::open_in_memory().unwrap();
        let p = store.create_profile("P", "uuid", false).unwrap().id;
        let visits = Parsed { items: vec![Visit { url: "https://a.test/".into(), title: "A".into(), visited_at: 1 }], skipped: 2 };
        assert_eq!(import_history(&store, p, &visits).unwrap(), Outcome { imported: 1, skipped: 2, failed: 0 });
        assert_eq!(import_history(&store, p, &visits).unwrap(), Outcome { imported: 0, skipped: 3, failed: 0 });

        let folder = |t: &str, children| Bookmark::Folder { title: t.into(), children };
        let tree = Parsed {
            items: vec![folder(
                "Bookmarks Bar",
                vec![url("A", "https://a.test/"), folder("F", vec![url("B", "https://b.test/")]), folder("F", vec![url("C", "https://c.test/")])],
            )],
            skipped: 0,
        };
        assert_eq!(import_bookmarks(&store, p, "Arc", &tree).unwrap().imported, 3);
        assert_eq!(import_bookmarks(&store, p, "Arc", &tree).unwrap(), Outcome { imported: 0, skipped: 3, failed: 0 }, "same-named folders keep their own");
        let marks = store.bookmarks(p).unwrap();
        assert_eq!(marks.len(), 7, "Imported from Arc, Bookmarks Bar, A, F, B, F and C, once each");
        let root = marks.iter().find(|b| b.parent_id.is_none()).unwrap();
        assert_eq!(root.title, "Imported from Arc");
        assert_eq!(import_bookmarks(&store, p, "Firefox", &tree).unwrap().imported, 3, "another browser gets its own folder");
        assert_eq!(import_bookmarks(&store, p, "Dia", &Parsed::default()).unwrap(), Outcome::default());
        assert!(store.bookmarks(p).unwrap().iter().all(|b| b.title != "Imported from Dia"), "an empty source adds no folder");
    }

    #[test]
    fn only_a_normal_persistent_profile_is_a_target() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::TestCore::open(dir.path(), false);
        let normal = core.disk.profiles().unwrap()[0].id;
        assert_eq!(core.import_target(false, normal), Ok(()));
        let lockdown = core.disk.create_profile("L", "uuid-l", false).unwrap().id;
        core.disk.set_profile_shield_level(lockdown, ShieldLevel::Lockdown).unwrap();
        assert_eq!(core.import_target(false, lockdown), Err(Refused::NotPersistent));
        let sharing = core.disk.create_profile("S", "uuid-s", false).unwrap().id;
        core.disk.set_setting(&crate::shared_key(sharing), "uuid-l").unwrap();
        assert_eq!(core.import_target(false, sharing), Err(Refused::NotPersistent), "its store is the Lockdown profile's");
        let owner = core.disk.create_profile("O", "uuid-o", false).unwrap().id;
        let locked_sharer = core.disk.create_profile("B", "uuid-b", false).unwrap().id;
        core.disk.set_setting(&crate::shared_key(locked_sharer), "uuid-o").unwrap();
        core.disk.set_profile_shield_level(locked_sharer, ShieldLevel::Lockdown).unwrap();
        assert_eq!(core.import_target(false, owner), Ok(()), "a Lockdown profile sharing this store leaves it alone");
        assert_eq!(core.import_target(false, locked_sharer), Err(Refused::NotPersistent));
        let private = core.store(true).create_profile("P", "uuid-p", true).unwrap().id;
        assert_eq!(core.import_target(true, private), Err(Refused::NotPersistent));
        let read = Read { history: Some(Ok(Parsed::default())), ..Read::default() };
        assert_eq!(core.import(lockdown, import::Browser::Arc, read, |_| panic!("nothing is written")), Err(Refused::NotPersistent));
        let read = Read { cookies: Some(Ok(Parsed::default())), ..Read::default() };
        assert_eq!(core.import(normal, import::Browser::Arc, read, |_| panic!()), Err(Refused::NotOpen), "cookies need the profile's runtime");
    }

    #[test]
    fn history_and_bookmarks_import_without_a_runtime() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::TestCore::open(dir.path(), false);
        let p = core.disk.profiles().unwrap()[0].id;
        let read = Read {
            history: Some(Ok(Parsed { items: vec![Visit { url: "https://a.test/".into(), title: String::new(), visited_at: 1 }], skipped: 0 })),
            bookmarks: Some(Ok(Parsed { items: vec![url("A", "https://a.test/")], skipped: 0 })),
            cookies: Some(Err(Error::KeychainDenied)),
        };
        let report = std::rc::Rc::new(std::cell::RefCell::new(None));
        let slot = report.clone();
        core.import(p, import::Browser::Chrome, read, move |r| *slot.borrow_mut() = Some(r)).unwrap();
        let report = report.take().unwrap();
        assert_eq!(report.cookies, Some(Err(Error::KeychainDenied)));
        assert_eq!(report.history, Some(Ok(Outcome { imported: 1, skipped: 0, failed: 0 })));
        assert_eq!(report.bookmarks.unwrap().unwrap().imported, 1);
    }
}
