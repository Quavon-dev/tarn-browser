use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};
use std::time::Instant;

use block2::{DynBlock, RcBlock};
use engine::downloads::{flagged_source, folder_for, sanitize_filename, sorted_dir, unique_path};
use engine::nav_policy::Site;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSData, NSError, NSObject, NSProgressReporting, NSString, NSTimer, NSURL, NSURLResponse};
use objc2_web_kit::{WKDownload, WKDownloadDelegate, WKWebViewConfiguration, WKWebpagePreferences};
use serde_json::{Value, json};
use url::Url;

use crate::component::{Inner, Props};
use crate::webkit::WebView;

const PROGRESS_SECONDS: f64 = 0.5;

type DestinationHandler = DynBlock<dyn Fn(*mut NSURL)>;

thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    static TRANSFERS: RefCell<Vec<Transfer>> = const { RefCell::new(Vec::new()) };
    static ORIGINS: RefCell<Vec<(Retained<WKDownload>, Weak<Inner>)>> = const { RefCell::new(Vec::new()) };
    static VIEWS: RefCell<Vec<Weak<Inner>>> = const { RefCell::new(Vec::new()) };
    static UNSENT: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    static DELEGATE: RefCell<Option<Retained<Downloads>>> = const { RefCell::new(None) };
    static TIMER: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Saving,
    Pausing,
    Paused,
    Flagged,
    Ended,
}

pub struct Transfer {
    pub id: String,
    pub name: String,
    pub path: String,
    pub url: Option<Url>,
    pub download: Option<Retained<WKDownload>>,
    pub resume: Option<Retained<NSData>>,
    pub hold: Option<RcBlock<dyn Fn(*mut NSURL)>>,
    pub state: State,
    pub sent: u64,
    origin: Weak<Inner>,
    props: Props,
    resumer: Option<Retained<WebView>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnDownloads"]
    struct Downloads;

    unsafe impl NSObjectProtocol for Downloads {}

    unsafe impl WKDownloadDelegate for Downloads {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn destination(&self, download: &WKDownload, _response: &NSURLResponse, suggested: &NSString, handler: &DestinationHandler) {
            destination(download, &suggested.to_string(), handler);
        }

        #[unsafe(method(downloadDidFinish:))]
        fn finished(&self, download: &WKDownload) {
            finished(download);
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn failed(&self, download: &WKDownload, error: &NSError, _resume: Option<&NSData>) {
            failed(download, error.localizedDescription().to_string());
        }
    }
);

fn delegate(mtm: MainThreadMarker) -> Retained<Downloads> {
    DELEGATE.with(|d| {
        d.borrow_mut()
            .get_or_insert_with(|| {
                let this = Downloads::alloc(mtm).set_ivars(());
                // SAFETY: NSObject's designated initializer on a freshly allocated instance.
                unsafe { msg_send![super(this), init] }
            })
            .clone()
    })
}

pub fn next_id() -> String {
    NEXT_ID.with(|n| n.replace(n.get() + 1)).to_string()
}

fn same(download: &Option<Retained<WKDownload>>, other: &WKDownload) -> bool {
    download.as_ref().is_some_and(|d| std::ptr::eq(Retained::as_ptr(d), other))
}

fn file_url(path: &str) -> Retained<NSURL> {
    NSURL::fileURLWithPath(&NSString::from_str(path))
}

fn progress(download: &WKDownload) -> (u64, Option<u64>) {
    let progress = download.progress();
    let (done, total) = (progress.completedUnitCount(), progress.totalUnitCount());
    (u64::try_from(done).unwrap_or(0), u64::try_from(total).ok().filter(|t| *t > 0))
}

fn downloads_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| Path::new(&home).join("Documents/Downloads"))
}

fn download_url(download: &WKDownload) -> String {
    // SAFETY: WebKit hands over a live download on the main thread.
    let request = unsafe { download.originalRequest() };
    let raw = request.and_then(|r| r.URL()).and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default();
    engine::view_policy::without_credentials(&raw)
}

fn flagged(props: &Props, urls: [Option<Url>; 3]) -> Option<Url> {
    flagged_source(urls, |u| Site::new(props.site(u)))
}

fn live(view: &Weak<Inner>) -> Option<Rc<Inner>> {
    view.upgrade().filter(|v| !v.closed.get())
}

fn emit(origin: &Weak<Inner>, data: Value) {
    let target = live(origin).or_else(|| VIEWS.with(|v| v.borrow().iter().find_map(live)));
    match target {
        Some(view) => view.emit("download", data),
        None => UNSENT.with(|u| u.borrow_mut().push(data)),
    }
}

fn with<T>(id: &str, f: impl FnOnce(&mut Transfer) -> T) -> Option<T> {
    TRANSFERS.with(|t| t.borrow_mut().iter_mut().find(|t| t.id == id).map(f))
}

fn remove(id: &str) {
    TRANSFERS.with(|t| t.borrow_mut().retain(|t| t.id != id));
}

fn take(download: &WKDownload) -> Option<Transfer> {
    TRANSFERS.with(|t| {
        let mut transfers = t.borrow_mut();
        let at = transfers.iter().position(|t| same(&t.download, download))?;
        Some(transfers.remove(at))
    })
}

pub(crate) fn register(view: &Rc<Inner>) {
    VIEWS.with(|v| {
        let mut views = v.borrow_mut();
        views.retain(|v| live(v).is_some());
        views.push(Rc::downgrade(view));
    });
}

pub(crate) fn flush(view: &Rc<Inner>) {
    for data in UNSENT.with(|u| u.take()) {
        view.emit("download", data);
    }
}

pub(crate) fn adopt(download: &WKDownload, origin: Weak<Inner>, mtm: MainThreadMarker) {
    ORIGINS.with(|o| o.borrow_mut().push((download.retain(), origin)));
    let delegate = delegate(mtm);
    // SAFETY: main thread; the delegate lives for the app's run, so the download's weak reference stays valid.
    unsafe { download.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
}

fn origin_of(download: &WKDownload) -> Weak<Inner> {
    ORIGINS.with(|o| {
        let mut origins = o.borrow_mut();
        let at = origins.iter().position(|(d, _)| std::ptr::eq(Retained::as_ptr(d), download));
        at.map(|at| origins.remove(at).1).unwrap_or_default()
    })
}

fn refuse(download: &WKDownload, origin: Weak<Inner>, name: &str, message: &str) {
    let id = next_id();
    TRANSFERS.with(|t| {
        t.borrow_mut().push(Transfer {
            id: id.clone(),
            name: name.to_owned(),
            path: String::new(),
            url: None,
            download: Some(download.retain()),
            resume: None,
            hold: None,
            state: State::Ended,
            sent: 0,
            origin: origin.clone(),
            props: Props::default(),
            resumer: None,
        })
    });
    emit(&origin, json!({ "id": id, "state": "failed", "name": name, "message": message }));
}

fn destination(download: &WKDownload, suggested: &str, handler: &DestinationHandler) {
    let origin = origin_of(download);
    let name = sanitize_filename(suggested);
    let Some(view) = live(&origin) else { return handler.call((std::ptr::null_mut(),)) };
    if !view.admit_download(Instant::now()) {
        refuse(download, origin, &name, "Too many downloads at once.");
        return handler.call((std::ptr::null_mut(),));
    }
    let Some(path) =
        downloads_dir().filter(|dir| std::fs::create_dir_all(dir).is_ok()).map(|dir| unique_path(&dir, &name, |p| p.symlink_metadata().is_ok() || saving(p)))
    else {
        refuse(download, origin, &name, "The Downloads folder could not be created.");
        return handler.call((std::ptr::null_mut(),));
    };
    let shown = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
    let id = next_id();
    // SAFETY: WebKit hands over a live download on the main thread.
    let request = unsafe { download.originalRequest() };
    let url = request.and_then(|r| r.URL()).and_then(|u| u.absoluteString()).and_then(|s| Url::parse(&s.to_string()).ok());
    let page = view.web().and_then(|w| w.current_url());
    let props = view.props.borrow().clone();
    let source = flagged(&props, [url.clone(), None, page]);
    let path_text = path.to_string_lossy().into_owned();
    emit(&origin, json!({ "id": id, "state": "started", "name": shown, "pausable": true }));
    let hold = source.as_ref().map(|_| handler.copy());
    TRANSFERS.with(|t| {
        t.borrow_mut().push(Transfer {
            id: id.clone(),
            name: shown.clone(),
            path: path_text.clone(),
            url,
            download: Some(download.retain()),
            resume: None,
            hold,
            state: if source.is_some() { State::Flagged } else { State::Saving },
            sent: 0,
            origin: origin.clone(),
            props,
            resumer: None,
        })
    });
    match source {
        Some(source) => emit(&origin, json!({ "id": id, "state": "flagged", "name": shown, "host": source.host_str().unwrap_or_default() })),
        None => {
            handler.call((Retained::as_ptr(&file_url(&path_text)).cast_mut(),));
            watch_progress();
        }
    }
}

fn saving(path: &Path) -> bool {
    TRANSFERS.with_borrow(|t| t.iter().any(|t| (t.state != State::Ended || t.download.is_some()) && Path::new(&t.path) == path))
}

fn finished(download: &WKDownload) {
    let url = download_url(download);
    let Some(t) = take(download).filter(|t| t.state != State::Ended) else { return };
    let path = if t.props.sort_downloads { sort(Path::new(&t.path), &t.name) } else { PathBuf::from(&t.path) };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(t.name);
    emit(&t.origin, json!({ "id": t.id, "state": "finished", "name": name, "path": path.to_string_lossy(), "url": url, "quarantined": false }));
}

fn failed(download: &WKDownload, message: String) {
    let pausing = TRANSFERS.with(|t| t.borrow().iter().any(|t| same(&t.download, download) && t.state == State::Pausing));
    if pausing {
        return;
    }
    match take(download) {
        Some(t) if t.state == State::Ended => {}
        Some(t) => emit(&t.origin, json!({ "id": t.id, "state": "failed", "name": t.name, "message": message })),
        None => emit(&origin_of(download), json!({ "id": next_id(), "state": "failed", "name": "", "message": message })),
    }
}

fn watch_progress() {
    if TIMER.replace(true) {
        return;
    }
    let block = RcBlock::new(move |timer: NonNull<NSTimer>| {
        let saving = report_progress();
        TIMER.set(saving);
        if !saving {
            // SAFETY: NSTimer passes itself, live on the main run loop, for the duration of the call.
            unsafe { timer.as_ref().invalidate() };
        }
    });
    // SAFETY: main thread; the timer runs on the main run loop, where the block's captures live.
    unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(PROGRESS_SECONDS, true, &block) };
}

fn report_progress() -> bool {
    let mut events = Vec::new();
    let mut saving = false;
    TRANSFERS.with(|t| {
        for t in t.borrow_mut().iter_mut().filter(|t| t.state == State::Saving) {
            let Some(download) = &t.download else { continue };
            saving = true;
            let (done, total) = progress(download);
            if done != t.sent {
                t.sent = done;
                events.push((t.origin.clone(), json!({ "id": t.id, "state": "progress", "done": done, "total": total })));
            }
        }
    });
    for (origin, event) in events {
        emit(&origin, event);
    }
    saving
}

pub(crate) fn command(name: &str, args: &Value, mtm: MainThreadMarker) -> bool {
    let id = args.get("id").and_then(Value::as_str).unwrap_or_default().to_owned();
    match name {
        "downloadPause" => pause(&id),
        "downloadResume" => resume(&id, mtm),
        "downloadCancel" => cancel(&id),
        "downloadKeep" => keep(&id, args.get("keep").and_then(Value::as_bool).unwrap_or(false), mtm),
        _ => return false,
    }
    true
}

fn pause(id: &str) {
    let Some(download) = with(id, |t| (t.state == State::Saving).then(|| t.download.clone()).flatten()).flatten() else { return };
    let bytes = progress(&download).0;
    with(id, |t| t.state = State::Pausing);
    let id = id.to_owned();
    let done = RcBlock::new(move |data: *mut NSData| {
        // SAFETY: WebKit passes the resume data or null for the duration of the call.
        let data = unsafe { Retained::retain(data) };
        let paused = data.is_some();
        if let Some(path) = with(&id, |t| (t.state == State::Ended).then(|| t.path.clone())).flatten() {
            remove(&id);
            if !path.is_empty() {
                std::fs::remove_file(path).ok();
            }
            return;
        }
        let Some((origin, name)) = with(&id, |t| {
            t.resume = data;
            t.download = None;
            t.resumer = None;
            t.state = if paused { State::Paused } else { State::Ended };
            (t.origin.clone(), t.name.clone())
        }) else {
            return;
        };
        if paused {
            emit(&origin, json!({ "id": id, "state": "paused", "done": bytes }));
        } else {
            remove(&id);
            emit(&origin, json!({ "id": id, "state": "failed", "name": name, "message": "This download cannot be resumed." }));
        }
    });
    // SAFETY: main thread; WebKit copies the block and calls it once with the resume data.
    unsafe { download.cancel(Some(&done)) };
}

fn resume(id: &str, mtm: MainThreadMarker) {
    let Some((data, url, origin, props)) =
        with(id, |t| (t.state == State::Paused).then(|| t.resume.clone().map(|d| (d, t.url.clone(), t.origin.clone(), t.props.clone()))).flatten()).flatten()
    else {
        return;
    };
    let page = live(&origin).and_then(|v| v.web()).and_then(|w| w.current_url());
    if let Some(source) = flagged(&props, [url, None, page]) {
        let name = with(id, |t| {
            t.state = State::Flagged;
            t.name.clone()
        })
        .unwrap_or_default();
        return emit(&origin, json!({ "id": id, "state": "flagged", "name": name, "host": source.host_str().unwrap_or_default() }));
    }
    resume_now(id, &data, mtm);
}

fn resumer(profile: &str, mtm: MainThreadMarker) -> Retained<WebView> {
    let store = crate::stores::live_store(profile, mtm);
    // SAFETY: plain allocations and setters on the main thread; the web view copies the configuration.
    let configuration = unsafe {
        let preferences = WKWebpagePreferences::new(mtm);
        preferences.setAllowsContentJavaScript(false);
        let configuration = WKWebViewConfiguration::new(mtm);
        configuration.setWebsiteDataStore(&store);
        configuration.setDefaultWebpagePreferences(Some(&preferences));
        configuration
    };
    WebView::init_with(WebView::alloc(mtm), CGRect::default(), &configuration)
}

fn resume_now(id: &str, data: &NSData, mtm: MainThreadMarker) {
    let Some((origin, profile)) = with(id, |t| (t.origin.clone(), t.props.profile.clone())) else { return };
    let shown = live(&origin).and_then(|v| v.web()).filter(|_| live(&origin).is_some_and(|v| v.profile() == profile));
    let web = shown.unwrap_or_else(|| resumer(&profile, mtm));
    with(id, |t| t.resumer = Some(web.clone()));
    let id = id.to_owned();
    let done = RcBlock::new(move |download: NonNull<WKDownload>| {
        // SAFETY: WebKit passes the resumed download, live on the main thread, for the duration of the call.
        let download = unsafe { download.as_ref() };
        let delegate = delegate(mtm);
        // SAFETY: main thread; the delegate lives for the app's run.
        unsafe { download.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
        let Some(origin) = with(&id, |t| {
            t.download = Some(download.retain());
            t.resume = None;
            t.state = State::Saving;
            t.origin.clone()
        }) else {
            return;
        };
        emit(&origin, json!({ "id": id, "state": "resumed" }));
        watch_progress();
    });
    web.resume_download(data, &done);
}

fn cancel(id: &str) {
    let Some((download, path, hold)) = with(id, |t| {
        t.state = State::Ended;
        t.resume = None;
        t.resumer = None;
        (t.download.clone(), t.path.clone(), t.hold.take())
    }) else {
        return;
    };
    if let Some(hold) = hold {
        hold.call((std::ptr::null_mut(),));
    }
    match download {
        // SAFETY: main thread; cancelling without a handler discards the resume data.
        Some(download) => unsafe { download.cancel(None) },
        None => remove(id),
    }
    if !path.is_empty() {
        std::fs::remove_file(path).ok();
    }
}

fn keep(id: &str, keep: bool, mtm: MainThreadMarker) {
    if !keep {
        return cancel(id);
    }
    let Some((hold, resume, path)) = with(id, |t| (t.state == State::Flagged).then(|| (t.hold.take(), t.resume.clone(), t.path.clone()))).flatten() else {
        return;
    };
    match (hold, resume) {
        (Some(hold), _) => {
            with(id, |t| t.state = State::Saving);
            hold.call((Retained::as_ptr(&file_url(&path)).cast_mut(),));
            watch_progress();
        }
        (None, Some(data)) => {
            with(id, |t| t.state = State::Paused);
            resume_now(id, &data, mtm);
        }
        (None, None) => {}
    }
}

fn sort(path: &Path, name: &str) -> PathBuf {
    let (Some(folder), Some(dir)) = (folder_for(name), path.parent()) else { return path.to_path_buf() };
    let Some(target) = sorted_dir(dir, folder) else { return path.to_path_buf() };
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let moved = unique_path(&target, &file, |p| p.symlink_metadata().is_ok());
    match std::fs::rename(path, &moved) {
        Ok(()) => moved,
        Err(_) => path.to_path_buf(),
    }
}
