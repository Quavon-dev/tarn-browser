use std::path::{Path, PathBuf};
use std::rc::Rc;

use browser_core::WebsiteData;
use engine::Event;
use engine::cookies::Cookie;
use nsh_core::prelude::{Owner, run_on_main_thread, spawn};

pub const PRIVATE_PREFIX: &str = "private:";
pub const BURNER_PREFIX: &str = "burner:";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum StoreKey {
    Persistent(String),
    Ephemeral(String),
    Burner(String),
}

impl StoreKey {
    pub fn parse(prop: &str) -> Option<StoreKey> {
        if let Some(id) = prop.strip_prefix(BURNER_PREFIX) {
            return (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| StoreKey::Burner(id.to_ascii_lowercase()));
        }
        match prop.strip_prefix(PRIVATE_PREFIX) {
            Some(session) => valid_id(session).then(|| StoreKey::Ephemeral(session.to_owned())),
            None => valid_id(prop).then(|| StoreKey::Persistent(prop.to_owned())),
        }
    }

    pub fn prop(&self) -> String {
        match self {
            StoreKey::Persistent(uuid) => uuid.clone(),
            StoreKey::Ephemeral(session) => format!("{PRIVATE_PREFIX}{session}"),
            StoreKey::Burner(id) => format!("{BURNER_PREFIX}{id}"),
        }
    }

    pub fn burner(&self) -> StoreKey {
        match self {
            StoreKey::Ephemeral(session) => StoreKey::Burner(session.replace('-', "")),
            other => other.clone(),
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn root() -> PathBuf {
    browser_core::data_dir(None).unwrap_or_else(|| PathBuf::from(".tarn"))
}

pub fn store_dir(root: &Path, uuid: &str) -> Option<PathBuf> {
    valid_id(uuid).then(|| root.join("webkit").join(uuid))
}

pub fn wipe(root: &Path, uuid: &str) {
    let Some(dir) = store_dir(root, uuid) else { return };
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("tarn: could not empty {}: {e}", dir.display()),
    }
}

pub fn wipe_retired(root: &Path, disk: &store::Store) {
    for uuid in browser_core::profiles::retired_stores(disk) {
        wipe(root, &uuid);
        if store_dir(root, &uuid).is_none_or(|dir| !dir.exists()) {
            browser_core::profiles::set_retired(disk, &uuid, false);
        }
    }
}

pub struct Data {
    pub key: StoreKey,
}

impl WebsiteData for Data {
    type RuleLists = ();

    fn persistent(uuid: &str, _own_store: &str, _downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        valid_id(uuid).then(|| Data { key: StoreKey::Persistent(uuid.to_owned()) })
    }

    fn ephemeral(_downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        Some(Data { key: StoreKey::Ephemeral(uuid::Uuid::new_v4().to_string()) })
    }

    fn wipe_persistent(uuid: &str) {
        wipe(&root(), uuid);
    }

    fn import_cookies(&self, cookies: Vec<Cookie>, now: i64, done: Box<dyn FnOnce(usize, usize)>) {
        let (key, total) = (self.key.clone(), cookies.len());
        Owner::detached().run(|| {
            drop(spawn(async move {
                let answer = match run_on_main_thread(move || crate::webview::session::import_cookies(&key, cookies, now)).await {
                    Ok(answer) => answer.await.ok(),
                    Err(_) => None,
                };
                let (written, refused) = answer.unwrap_or((0, total));
                done(written, refused);
            }))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_keys_round_trip_and_refuse_paths() {
        let p = StoreKey::parse("0b6e-uuid").unwrap();
        assert_eq!(p, StoreKey::Persistent("0b6e-uuid".into()));
        assert_eq!(StoreKey::parse(&p.prop()), Some(p));
        let e = StoreKey::parse("private:abc").unwrap();
        assert_eq!(e, StoreKey::Ephemeral("abc".into()));
        assert_eq!(e.prop(), "private:abc");
        let b = StoreKey::parse("burner:0123456789ABCDEF0123456789abcdef").unwrap();
        assert_eq!(b.prop(), "burner:0123456789abcdef0123456789abcdef");
        let session = StoreKey::Ephemeral("01234567-89ab-cdef-0123-456789abcdef".into());
        assert_eq!(StoreKey::parse(&session.burner().prop()), Some(session.burner()));
        for bad in ["", "../x", "a/b", "private:", "private:../x", &"a".repeat(65), "burner:abc", "burner:../0123456789abcdef0123456789ab"] {
            assert_eq!(StoreKey::parse(bad), None, "{bad}");
        }
        assert_eq!(store_dir(Path::new("/d"), "u-1"), Some(PathBuf::from("/d/webkit/u-1")));
        assert_eq!(store_dir(Path::new("/d"), ".."), None);
    }

    #[test]
    fn ephemeral_data_names_a_fresh_session_each_time() {
        let a = Data::ephemeral(Rc::new(|_| {})).unwrap();
        let b = Data::ephemeral(Rc::new(|_| {})).unwrap();
        assert!(matches!(a.key, StoreKey::Ephemeral(_)));
        assert_ne!(a.key, b.key);
        assert!(Data::persistent("../x", "", Rc::new(|_| {})).is_none());
    }

    #[test]
    fn wiping_removes_only_that_store() {
        let dir = tempfile::tempdir().unwrap();
        let keep = store_dir(dir.path(), "keep").unwrap();
        let gone = store_dir(dir.path(), "gone").unwrap();
        for d in [&keep, &gone] {
            std::fs::create_dir_all(d.join("data")).unwrap();
        }
        wipe(dir.path(), "gone");
        wipe(dir.path(), "../keep");
        assert!(keep.exists() && !gone.exists());
    }

    #[test]
    fn retired_stores_are_wiped_and_unlisted() {
        let dir = tempfile::tempdir().unwrap();
        let disk = store::Store::open_in_memory().unwrap();
        let (keep, gone) = (store_dir(dir.path(), "keep").unwrap(), store_dir(dir.path(), "gone").unwrap());
        for d in [&keep, &gone] {
            std::fs::create_dir_all(d.join("data")).unwrap();
        }
        browser_core::profiles::set_retired(&disk, "gone", true);
        wipe_retired(dir.path(), &disk);
        assert!(keep.exists() && !gone.exists());
        assert!(browser_core::profiles::retired_stores(&disk).is_empty());
    }
}
