//! A profile's website data as each platform's engine keeps it (`docs/shared-core.md` §1).

use std::rc::Rc;

use engine::Event;
use engine::cookies::Cookie;

pub trait WebsiteData: Sized {
    /// Compiled content-blocking rules the engine applies to this data.
    type RuleLists;

    /// `None` when the platform cannot open it now. `own_store` names the profile's own extension
    /// storage; `downloads` receives this profile's download events.
    fn persistent(uuid: &str, own_store: &str, downloads: Rc<dyn Fn(Event)>) -> Option<Self>;
    /// A store for the private space and Lockdown: website data must never reach disk.
    fn ephemeral(downloads: Rc<dyn Fn(Event)>) -> Option<Self>;
    /// Empties a profile's persistent store without opening it.
    fn wipe_persistent(uuid: &str);
    /// `done` gets how many cookies were written and how many were refused.
    fn import_cookies(&self, cookies: Vec<Cookie>, now: i64, done: Box<dyn FnOnce(usize, usize)>);
}

#[cfg(any(target_os = "macos", windows))]
impl WebsiteData for engine::DataStore {
    type RuleLists = engine::FilterLists;

    fn persistent(uuid: &str, own_store: &str, downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        engine::DataStore::persistent(uuid, own_store, downloads)
    }

    fn ephemeral(downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        engine::DataStore::ephemeral(downloads)
    }

    fn wipe_persistent(uuid: &str) {
        engine::DataStore::wipe_persistent(uuid);
    }

    fn import_cookies(&self, cookies: Vec<Cookie>, now: i64, done: Box<dyn FnOnce(usize, usize)>) {
        engine::DataStore::import_cookies(self, cookies, now, done);
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        pub static WIPED: RefCell<Vec<String>> = RefCell::default();
    }

    /// Website data that lives in memory and accepts every cookie.
    pub struct Memory {
        pub persistent: Option<String>,
        pub cookies: RefCell<Vec<Cookie>>,
    }

    impl WebsiteData for Memory {
        type RuleLists = ();

        fn persistent(uuid: &str, _: &str, _: Rc<dyn Fn(Event)>) -> Option<Self> {
            Some(Memory { persistent: Some(uuid.to_owned()), cookies: RefCell::default() })
        }

        fn ephemeral(_: Rc<dyn Fn(Event)>) -> Option<Self> {
            Some(Memory { persistent: None, cookies: RefCell::default() })
        }

        fn wipe_persistent(uuid: &str) {
            WIPED.with(|w| w.borrow_mut().push(uuid.to_owned()));
        }

        fn import_cookies(&self, cookies: Vec<Cookie>, _: i64, done: Box<dyn FnOnce(usize, usize)>) {
            let n = cookies.len();
            self.cookies.borrow_mut().extend(cookies);
            done(n, 0);
        }
    }
}
