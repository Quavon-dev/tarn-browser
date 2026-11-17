use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use browser_core::site::{Sets, SiteRules};
use engine::downloads::{flagged_source, folder_for, sanitize_filename, sorted_dir, unique_path};
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender};
use serde_json::json;
use shield::ShieldLevel;
use shield::domains::DomainSet;
use url::Url;
use webkit6::glib;
use webkit6::{CookiePersistentStorage, Download, NetworkSession, TLSErrorsPolicy, WebsiteData, WebsiteDataTypes, gio};

use crate::browser::profile_data::Wipe;
use crate::data::{self, StoreKey};

pub const ORIGIN_XATTR: &str = "user.xdg.origin.url";

static ROUTE: Mutex<Option<UnboundedSender<(String, serde_json::Value)>>> = Mutex::new(None);

const PROGRESS_EVERY: Duration = Duration::from_millis(500);

thread_local! {
    static SESSIONS: RefCell<HashMap<StoreKey, NetworkSession>> = RefCell::default();
    pub(super) static DOWNLOADS: RefCell<HashMap<String, Tracked>> = RefCell::default();
    static NEXT_DOWNLOAD: Cell<u64> = const { Cell::new(0) };
}

pub(super) struct Tracked {
    download: Download,
    reported: Rc<Cell<bool>>,
    held: Option<Held>,
}

struct Held {
    suggested: String,
    tab: Rc<str>,
    sort: bool,
}

pub fn cancel(id: &str) {
    if let Some(tracked) = DOWNLOADS.with_borrow_mut(|d| d.remove(id)) {
        tracked.download.cancel();
    }
}

pub fn keep(id: &str, keep: bool) {
    let held = DOWNLOADS.with_borrow_mut(|d| {
        let tracked = d.get_mut(id)?;
        if !keep {
            tracked.reported.set(true);
            return d.remove(id).map(|t| (t.download, None));
        }
        tracked.held.take().map(|h| (tracked.download.clone(), Some(h)))
    });
    match held {
        Some((download, Some(held))) => place(&download, id, &held.tab, &held.suggested, held.sort),
        Some((download, None)) => download.cancel(),
        None => {}
    }
}

fn flagged(malware: Option<Arc<DomainSet>>, urls: [Option<Url>; 3]) -> Option<Url> {
    let sets = RefCell::new(Sets { malware: Some(malware?), ..Sets::default() });
    flagged_source(urls, |u| browser_core::site::site(ShieldLevel::Standard, &SiteRules::default(), &sets, u))
}

pub fn release(key: &StoreKey) {
    SESSIONS.with_borrow_mut(|sessions| sessions.remove(key));
}

pub fn acquire(key: &StoreKey) -> Option<NetworkSession> {
    if let Some(session) = SESSIONS.with_borrow(|sessions| sessions.get(key).cloned()) {
        return Some(session);
    }
    let session = open(key)?;
    SESSIONS.with_borrow_mut(|sessions| sessions.insert(key.clone(), session.clone()));
    Some(session)
}

pub fn clear(key: &StoreKey) {
    clear_then(key, || {});
}

pub fn wipe(wipe: Wipe) {
    let Wipe { key, remove } = wipe;
    let released = key.clone();
    clear_then(&key, move || {
        if let Some(uuid) = remove {
            release(&released);
            data::wipe(&data::root(), &uuid);
        }
    });
}

fn clear_then(key: &StoreKey, done: impl FnOnce() + Send + 'static) {
    let open = SESSIONS.with_borrow(|sessions| sessions.get(key).cloned());
    match (open.and_then(|s| s.website_data_manager()), key) {
        (Some(manager), _) => manager.clear(WebsiteDataTypes::all(), glib::TimeSpan::from_seconds(0), None::<&gio::Cancellable>, move |result| {
            if let Err(e) = result {
                eprintln!("tarn: website data was not cleared: {e}");
            }
            done();
        }),
        (None, StoreKey::Persistent(uuid)) => {
            data::wipe(&data::root(), uuid);
            done();
        }
        (None, StoreKey::Ephemeral(_) | StoreKey::Burner(_)) => done(),
    }
}

/// Removes every kind of stored data of `host` and the sites below it from the profile's open
/// store, then reports how many records went.
pub fn clear_site(key: &StoreKey, host: &str, done: impl FnOnce(usize) + 'static) {
    let Some(manager) = SESSIONS.with_borrow(|sessions| sessions.get(key).cloned()).and_then(|s| s.website_data_manager()) else { return done(0) };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let remover = manager.clone();
    manager.fetch(WebsiteDataTypes::all(), None::<&gio::Cancellable>, move |records| {
        let records: Vec<WebsiteData> =
            records.unwrap_or_default().into_iter().filter(|r| r.name().is_some_and(|n| engine::view_policy::site_matches(&host, &n))).collect();
        if records.is_empty() {
            return done(0);
        }
        let (sender, removed) = futures_channel::oneshot::channel();
        let refs: Vec<&WebsiteData> = records.iter().collect();
        remover.remove(WebsiteDataTypes::all(), &refs, None::<&gio::Cancellable>, move |result| {
            if let Err(e) = result {
                eprintln!("tarn: site data was not cleared: {e}");
            }
            sender.send(()).ok();
        });
        let count = records.len();
        glib::MainContext::default().spawn_local(async move {
            removed.await.ok();
            done(count);
        });
    });
}

fn open(key: &StoreKey) -> Option<NetworkSession> {
    let session = match key {
        StoreKey::Ephemeral(_) | StoreKey::Burner(_) => NetworkSession::new_ephemeral(),
        StoreKey::Persistent(uuid) => {
            let root = data::store_dir(&data::root(), uuid)?;
            let (data, cache) = (root.join("data"), root.join("cache"));
            if let Err(e) = create_private_dir(&data) {
                eprintln!("tarn: cannot create {}: {e}", data.display());
                return None;
            }
            let session = NetworkSession::new(data.to_str(), cache.to_str());
            if let Some(cookies) = session.cookie_manager() {
                cookies.set_persistent_storage(&data.join("cookies.sqlite").to_string_lossy(), CookiePersistentStorage::Sqlite);
            }
            session
        }
    };
    session.set_tls_errors_policy(TLSErrorsPolicy::Fail);
    session.set_persistent_credential_storage_enabled(false);
    session.connect_download_started(|_, download| super::adopt_download(download));
    Some(session)
}

pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}

pub fn downloads_dir() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads).unwrap_or_else(|| glib::home_dir().join("Downloads"))
}

pub fn destination(dir: &Path, suggested: &str, sort: bool) -> PathBuf {
    let name = sanitize_filename(suggested);
    let dir = folder_for(&name).filter(|_| sort).and_then(|folder| sorted_dir(dir, folder)).unwrap_or_else(|| dir.to_owned());
    unique_path(&dir, &name, |p| p.exists())
}

pub fn mark_origin(path: &Path, url: &str) -> bool {
    xattr::set(path, ORIGIN_XATTR, url.as_bytes()).is_ok()
}

pub trait DownloadSink {
    fn tab(&self) -> String;
    fn admit_download(&self, now: Instant) -> bool;
    fn sort_downloads(&self) -> bool;
    fn page(&self) -> Option<Url>;
    fn malware(&self) -> Option<Arc<DomainSet>>;
}

fn place(download: &Download, id: &str, tab: &str, suggested: &str, sort: bool) {
    let dir = downloads_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        report(tab, json!({ "id": id, "state": "failed", "name": suggested, "message": e.to_string() }));
        if let Some(tracked) = DOWNLOADS.with_borrow_mut(|d| d.remove(id)) {
            tracked.reported.set(true);
        }
        download.cancel();
        return;
    }
    let path = destination(&dir, suggested, sort);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    download.set_destination(&path.to_string_lossy());
    report(tab, json!({ "id": id, "state": "started", "name": name, "path": path, "pausable": false }));
}

pub fn route() -> UnboundedReceiver<(String, serde_json::Value)> {
    let (sender, receiver) = futures_channel::mpsc::unbounded();
    *ROUTE.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
    receiver
}

fn report(tab: &str, data: serde_json::Value) {
    if let Some(route) = ROUTE.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        route.unbounded_send((tab.to_owned(), data)).ok();
    }
}

fn next_id() -> String {
    NEXT_DOWNLOAD.with(|n| {
        n.set(n.get() + 1);
        n.get().to_string()
    })
}

pub fn report_saved(tab: &str, url: &str, path: &Path, saved: Result<(), String>) {
    let (id, name) = (next_id(), path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    report(
        tab,
        match saved {
            Ok(()) => json!({ "id": id, "state": "finished", "name": name, "path": path, "url": url }),
            Err(message) => json!({ "id": id, "state": "failed", "name": name, "message": message }),
        },
    );
}

pub fn track(download: &Download, sink: &dyn DownloadSink) {
    if !sink.admit_download(Instant::now()) {
        download.cancel();
        return;
    }
    download.set_allow_overwrite(false);
    let id = next_id();
    let reported = Rc::new(Cell::new(false));
    DOWNLOADS.with_borrow_mut(|d| d.insert(id.clone(), Tracked { download: download.clone(), reported: reported.clone(), held: None }));
    let tab: Rc<str> = sink.tab().into();
    let sort = sink.sort_downloads();
    let (page, malware) = (sink.page(), sink.malware());
    let (named, named_id) = (tab.clone(), id.clone());
    download.connect_decide_destination(move |download, suggested| {
        let id = &named_id;
        let uri = |u: Option<glib::GString>| u.and_then(|u| Url::parse(&u).ok());
        let urls = [uri(download.request().and_then(|r| r.uri())), uri(download.response().and_then(|r| r.uri())), page.clone()];
        let Some(from) = flagged(malware.clone(), urls) else {
            place(download, id, &named, suggested, sort);
            return true;
        };
        let name = sanitize_filename(suggested);
        let held = Held { suggested: suggested.to_owned(), tab: named.clone(), sort };
        DOWNLOADS.with_borrow_mut(|d| d.get_mut(id).map(|t| t.held = Some(held)));
        report(&named, json!({ "id": id, "state": "started", "name": name, "pausable": false }));
        report(&named, json!({ "id": id, "state": "flagged", "name": name, "host": from.host_str().unwrap_or_default() }));
        true
    });
    let (progress, progress_id) = (tab.clone(), id.clone());
    let last = Cell::new(None::<Instant>);
    download.connect_received_data(move |download, _| {
        let now = Instant::now();
        if last.get().is_some_and(|t| now.duration_since(t) < PROGRESS_EVERY) {
            return;
        }
        last.set(Some(now));
        let total = download.response().map(|r| r.content_length()).filter(|n| *n > 0);
        report(&progress, json!({ "id": progress_id, "state": "progress", "done": download.received_data_length(), "total": total }));
    });
    let (finished, finished_id) = (tab.clone(), id.clone());
    download.connect_finished(move |download| {
        if DOWNLOADS.with_borrow_mut(|d| d.remove(&finished_id)).is_none() {
            return;
        }
        let Some(path) = download.destination().map(|d| PathBuf::from(d.as_str())) else { return };
        let url = download.request().and_then(|r| r.uri()).map(|u| super::policy::without_credentials(&u)).unwrap_or_default();
        let quarantined = mark_origin(&path, &url);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        report(&finished, json!({ "id": finished_id, "state": "finished", "name": name, "path": path, "url": url, "quarantined": quarantined }));
    });
    download.connect_failed(move |download, error| {
        DOWNLOADS.with_borrow_mut(|d| d.remove(&id));
        if reported.replace(true) {
            return;
        }
        let name = download.destination().and_then(|d| Path::new(d.as_str()).file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
        report(&tab, json!({ "id": id, "state": "failed", "name": name, "message": error.message() }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_get_safe_unique_names_and_their_origin() {
        let dir = tempfile::tempdir().unwrap();
        let first = destination(dir.path(), "../../etc/passwd", true);
        assert_eq!(first, dir.path().join("passwd"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(destination(dir.path(), "passwd", false), dir.path().join("passwd (2)"));
        assert_eq!(destination(dir.path(), "\u{202e}gpj.exe", false), dir.path().join("gpj.exe"));
        assert_eq!(destination(dir.path(), "photo.JPG", false), dir.path().join("photo.JPG"));
        assert_eq!(destination(dir.path(), "photo.JPG", true), dir.path().join("Images/photo.JPG"));
        std::os::unix::fs::symlink(dir.path().join("elsewhere"), dir.path().join("Archives")).unwrap();
        assert_eq!(destination(dir.path(), "a.zip", true), dir.path().join("a.zip"), "a link is never followed");
        if mark_origin(&first, "https://a.test/f") {
            assert_eq!(xattr::get(&first, ORIGIN_XATTR).unwrap().as_deref(), Some(&b"https://a.test/f"[..]));
        }
    }

    #[test]
    fn data_dirs_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("a/b");
        create_private_dir(&d).unwrap();
        assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
    }
}

#[cfg(test)]
mod display_tests {
    use std::time::Duration;

    use nsh_platform_linux::gtk4 as gtk;
    use webkit6::WebView;
    use webkit6::prelude::*;

    use super::*;

    fn spin_until(done: impl Fn() -> bool, secs: u64) -> bool {
        let context = glib::MainContext::default();
        let end = Instant::now() + Duration::from_secs(secs);
        while !done() && Instant::now() < end {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(10));
        }
        done()
    }

    fn store_marker(session: &NetworkSession, marker: &str) -> bool {
        let web = WebView::builder().network_session(session).build();
        let page = format!(
            "<script>document.cookie='m={marker}; max-age=99999'; localStorage.setItem('m', '{marker}');
             const r = indexedDB.open('db'); r.onupgradeneeded = () => r.result.createObjectStore('s');
             r.onsuccess = () => {{ const t = r.result.transaction('s', 'readwrite'); t.objectStore('s').put('{marker}', 'k');
             t.oncomplete = () => document.title = 'done'; }};</script>"
        );
        web.load_html(&page, Some("http://marker.test/"));
        spin_until(|| web.title().is_some_and(|t| t == "done"), 30)
    }

    fn files_containing(dir: &Path, marker: &[u8], found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                files_containing(&path, marker, found);
            } else if std::fs::read(&path).is_ok_and(|b| b.windows(marker.len()).any(|w| w == marker)) {
                found.push(path);
            }
        }
    }

    #[test]
    #[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
    fn the_ephemeral_store_writes_no_website_data_to_disk() {
        gtk::init().expect("a display");
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
        let private = StoreKey::Ephemeral("display-test".into());
        let control = StoreKey::Persistent("display-control".into());
        let burner = StoreKey::parse("burner:0123456789abcdef0123456789abcdef").unwrap();
        let (eph, per, burn) = (acquire(&private).unwrap(), acquire(&control).unwrap(), acquire(&burner).unwrap());
        assert_ne!(eph, burn, "a burner has a store of its own");
        assert!(store_marker(&eph, "zqEPHEMERALzq"), "the private page stored its data");
        assert!(store_marker(&burn, "zqBURNERzq"), "the burner page stored its data");
        assert!(store_marker(&per, "zqPERSISTzq"), "the control page stored its data");
        release(&burner);
        drop((eph, per, burn));
        SESSIONS.with_borrow_mut(|sessions| sessions.clear());
        spin_until(|| false, 5);
        let (mut leaked, mut kept) = (Vec::new(), Vec::new());
        files_containing(&home, b"zqEPHEMERALzq", &mut leaked);
        files_containing(&home, b"zqBURNERzq", &mut leaked);
        files_containing(&home, b"zqPERSISTzq", &mut kept);
        assert!(!kept.is_empty(), "the scan finds the persistent control's data under {}", home.display());
        assert!(leaked.is_empty(), "private data reached disk: {leaked:?}");
        eprintln!("control data found in {kept:?}; no private data under {}", home.display());
    }
}
