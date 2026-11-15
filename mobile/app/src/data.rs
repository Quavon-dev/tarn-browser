use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use browser_core::WebsiteData;
use engine::Event;
use engine::cookies::Cookie;

#[cfg(target_os = "ios")]
pub type Data = tarn_mobile_ios::Data;
#[cfg(target_os = "android")]
pub type Data = tarn_mobile_android::Data;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub type Data = Memory;

/// What the shell needs from a platform beyond `WebsiteData`.
pub trait Platform: WebsiteData {
    /// The `profile` prop of `tarn.webview`: the canonical store UUID for a persistent store,
    /// `private:<32 hex digits>` for an ephemeral one, new for every ephemeral store.
    fn store_id(&self) -> String;

    /// Where the shell keeps its own data; `None` uses NSH's data directory.
    fn data_dir() -> Option<PathBuf> {
        None
    }

    /// Whether this process is the private space and shows nothing else.
    fn private_process() -> bool {
        false
    }

    /// Opens the private space in another process: `Some(started)`, or `None` when the shell
    /// opens it in place.
    fn open_private_space() -> Option<bool> {
        None
    }

    fn page_world_scripts() -> bool {
        false
    }

    /// Closes this process's private space: `Some(closed)`, or `None` when the shell closes it in
    /// place.
    fn close_private_space() -> Option<bool> {
        None
    }
}

#[cfg(target_os = "ios")]
impl Platform for tarn_mobile_ios::Data {
    fn store_id(&self) -> String {
        tarn_mobile_ios::Data::store_id(self)
    }
}

#[cfg(target_os = "android")]
impl Platform for tarn_mobile_android::Data {
    fn store_id(&self) -> String {
        tarn_mobile_android::Data::store_id(self)
    }

    fn data_dir() -> Option<PathBuf> {
        tarn_mobile_android::host().map(|h| h.data_dir.clone())
    }

    fn private_process() -> bool {
        matches!(tarn_mobile_android::space(), tarn_mobile_android::Space::Private { .. })
    }

    fn page_world_scripts() -> bool {
        true
    }

    fn open_private_space() -> Option<bool> {
        Some(tarn_mobile_android::open_private_space())
    }

    fn close_private_space() -> Option<bool> {
        Self::private_process().then(tarn_mobile_android::close_private_space)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Memory(String);

static MEMORY_STORES: AtomicU64 = AtomicU64::new(0);

impl Platform for Memory {
    fn store_id(&self) -> String {
        self.0.clone()
    }
}

impl WebsiteData for Memory {
    type RuleLists = ();

    fn persistent(uuid: &str, _: &str, _: Rc<dyn Fn(Event)>) -> Option<Self> {
        Some(Memory(uuid.to_ascii_lowercase()))
    }

    fn ephemeral(_: Rc<dyn Fn(Event)>) -> Option<Self> {
        Some(Memory(format!("private:{:032x}", MEMORY_STORES.fetch_add(1, Ordering::Relaxed))))
    }

    fn wipe_persistent(_: &str) {}

    fn import_cookies(&self, cookies: Vec<Cookie>, _: i64, done: Box<dyn FnOnce(usize, usize)>) {
        done(0, cookies.len());
    }
}
