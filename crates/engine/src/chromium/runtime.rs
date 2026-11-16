use std::cell::{Cell, RefCell};
use std::ffi::c_void;
#[cfg(target_os = "macos")]
use std::ffi::{CStr, CString};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use cef::*;
#[cfg(target_os = "macos")]
use objc2::ffi;
#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, Imp, Sel};
#[cfg(target_os = "macos")]
use objc2::sel;

#[cfg(target_os = "macos")]
const FRAMEWORK: &str = "Chromium Embedded Framework.framework/Chromium Embedded Framework";
const SWITCHES: &[&str] = &[
    "disable-search-engine-choice-screen",
    "disable-background-networking",
    "disable-component-update",
    "disable-sync",
    "disable-breakpad",
    "disable-domain-reliability",
    "disable-client-side-phishing-detection",
    "disable-default-apps",
    "no-first-run",
    "no-default-browser-check",
    "no-pings",
    "metrics-recording-only",
];
/// Switches on Tarn's own argv that would turn off a sandbox, site isolation or web security, run
/// another program in front of a child, or open a debugging channel; Chromium passes some of the
/// browser's switches on to every child process. Removed even
/// though `command_line_args_disabled` already drops argv, should a CEF update stop doing so.
const REFUSED: &[&str] = &[
    "no-sandbox",
    "disable-gpu-sandbox",
    "single-process",
    "disable-web-security",
    "ignore-certificate-errors",
    "disable-site-isolation-trials",
    "renderer-cmd-prefix",
    "utility-cmd-prefix",
    "gpu-launcher",
    "browser-subprocess-path",
    "remote-debugging-port",
    "remote-debugging-pipe",
];

/// Chromium preferences Tarn holds fixed, by dotted path. CEF's preference API refuses them at run
/// time, so they are written before Chromium starts. Permissions that ask default to "block", so Chrome
/// never asks or counts dismissals, and device choosers (USB, HID, serial, Bluetooth) never open.
const LOCKED: &[(&str, i64)] = &[
    ("profile.cookie_controls_mode", 1),
    ("net.network_prediction_options", 2),
    ("profile.default_content_setting_values.geolocation", 2),
    ("profile.default_content_setting_values.notifications", 2),
    ("profile.default_content_setting_values.media_stream_camera", 2),
    ("profile.default_content_setting_values.media_stream_mic", 2),
    ("profile.default_content_setting_values.usb_guard", 2),
    ("profile.default_content_setting_values.hid_guard", 2),
    ("profile.default_content_setting_values.serial_guard", 2),
    ("profile.default_content_setting_values.bluetooth_guard", 2),
    ("profile.default_content_setting_values.bluetooth_scanning", 2),
    ("profile.default_content_setting_values.midi_sysex", 2),
    ("profile.default_content_setting_values.automatic_downloads", 2),
    ("profile.default_content_setting_values.file_system_write_guard", 2),
    ("profile.default_content_setting_values.window_placement", 2),
    ("profile.default_content_setting_values.local_fonts", 2),
    ("profile.default_content_setting_values.idle_detection", 2),
    ("profile.default_content_setting_values.clipboard", 2),
    ("profile.default_content_setting_values.sensors", 2),
    ("profile.default_content_setting_values.midi", 2),
    ("profile.default_content_setting_values.storage_access", 2),
    ("profile.default_content_setting_values.ar", 2),
    ("profile.default_content_setting_values.vr", 2),
    ("profile.default_content_setting_values.local_network_access", 2),
];

/// `preferences` with every `LOCKED` value set and `ssl.error_override_allowed` off; unreadable
/// preferences start fresh, as Chromium itself would.
fn lock_preferences(preferences: &str) -> String {
    let mut prefs = serde_json::from_str::<serde_json::Value>(preferences).ok().filter(serde_json::Value::is_object).unwrap_or_else(|| serde_json::json!({}));
    let values = LOCKED.iter().map(|(path, v)| (*path, serde_json::Value::from(*v))).chain([("ssl.error_override_allowed", false.into())]);
    for (path, value) in values {
        let (parents, leaf) = path.rsplit_once('.').expect("a dotted path");
        let mut node = &mut prefs;
        for key in parents.split('.') {
            let object = node.as_object_mut().expect("an object");
            node = object.entry(key).or_insert_with(|| serde_json::json!({}));
            if !node.is_object() {
                *node = serde_json::json!({});
            }
        }
        node.as_object_mut().expect("an object").insert(leaf.into(), value);
    }
    prefs.to_string()
}

fn write_preferences(profile: &Path) -> bool {
    let file = profile.join("Preferences");
    let current = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return false,
    };
    let staged = profile.join("Preferences.tarn");
    let (mut dir, mut options) = (std::fs::DirBuilder::new(), std::fs::OpenOptions::new());
    #[cfg(unix)]
    {
        dir.mode(0o700);
        options.mode(0o600);
    }
    dir.recursive(true).create(profile).is_ok()
        && options
            .write(true)
            .create(true)
            .truncate(true)
            .open(&staged)
            .and_then(|mut f| std::io::Write::write_all(&mut f, lock_preferences(&current).as_bytes()).and_then(|()| f.sync_all()))
            .and_then(|()| std::fs::rename(&staged, &file))
            .is_ok()
}

thread_local! {
    static STATE: Cell<State> = const { Cell::new(State::Off) };
    #[cfg(target_os = "macos")]
    static HANDLING_SEND_EVENT: Cell<bool> = const { Cell::new(false) };
    static PUMPING: Cell<bool> = const { Cell::new(false) };
    static READY: Cell<bool> = const { Cell::new(false) };
    #[cfg(target_os = "macos")]
    static RETIRED: Cell<bool> = const { Cell::new(false) };
    static ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static BEATS: Cell<u64> = const { Cell::new(0) };
    static WAITING: RefCell<Vec<Box<dyn FnOnce()>>> = const { RefCell::new(Vec::new()) };
    #[cfg(target_os = "macos")]
    static WORKER_FILTER: RefCell<Option<RequestContext>> = const { RefCell::new(None) };
    /// One request context per data store, by `DataStore::context_key`.
    #[cfg(windows)]
    static CONTEXTS: RefCell<std::collections::HashMap<String, RequestContext>> = RefCell::new(std::collections::HashMap::new());
    /// The swipe navigation last given to Chromium (`set_swipe_navigation`).
    #[cfg(windows)]
    static SWIPE: Cell<Option<bool>> = const { Cell::new(None) };
}

#[cfg(target_os = "macos")]
static NSAPP_SEND_EVENT: OnceLock<Imp> = OnceLock::new();
const HEARTBEAT_HZ: u64 = 30;
const IDLE_HEARTBEAT_HZ: u64 = 2;
/// How often a shown Chromium page's cookies are passed to WebKit between loads and tab switches,
/// which pass them at once.
const COOKIE_SYNC_SECS: u64 = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Off,
    Running,
    Closing,
    Failed,
    Stopped,
}

wrap_app! {
    struct TarnApp {
        root: String,
    }

    impl App {
        fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
            let Some(line) = command_line else { return };
            if process_type.is_some_and(|p| !p.to_string().is_empty()) {
                return;
            }
            // A Windows build started without CEF's sandbox launcher (`cargo run`) gets `no-sandbox`
            // from CEF itself (`Settings::no_sandbox`); without it its children cannot start.
            for switch in REFUSED.iter().filter(|s| !(**s == "no-sandbox" && unsandboxed())) {
                line.remove_switch(Some(&CefString::from(*switch)));
            }
            for switch in SWITCHES {
                line.append_switch(Some(&CefString::from(*switch)));
            }
            // Chrome moves the disk cache of a profile under `~/Library/Application Support` to
            // `~/Library/Caches`, outside the folder a wipe deletes. With it, each profile's
            // cache is `<root>/<profile folder name>`, which on Windows is the profile folder.
            line.append_switch_with_value(Some(&CefString::from("disk-cache-dir")), Some(&CefString::from(self.root.as_str())));
            // With the back-forward cache a navigation moves the page into a new view, which Chrome
            // paints in its theme's new-tab colour: dark grey behind every page without a background
            // of its own. Without it a navigation keeps its view, and a page left is not kept in memory.
            #[cfg(windows)]
            {
                let name = CefString::from("disable-features");
                let old = CefString::from(&line.switch_value(Some(&name))).to_string();
                let value = if old.is_empty() { "BackForwardCache".to_owned() } else { format!("{old},BackForwardCache") };
                line.append_switch_with_value(Some(&name), Some(&CefString::from(value.as_str())));
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(TarnProcessHandler::new())
        }
    }
}

wrap_browser_process_handler! {
    struct TarnProcessHandler;

    impl BrowserProcessHandler {
        // Views wait for Chromium's cookie jar to be emptied; a jar that cannot be emptied
        // keeps Chromium from showing pages. On Windows Chromium's own jar is the profile's.
        fn on_context_initialized(&self) {
            #[cfg(windows)]
            {
                READY.set(true);
                for then in WAITING.with_borrow_mut(std::mem::take) {
                    later(then);
                }
            }
            #[cfg(target_os = "macos")]
            super::cookies::empty_chromium(|emptied| {
                if !emptied {
                    eprintln!("tarn: Chromium's cookie store could not be emptied; Chromium tabs stay blank");
                    return;
                }
                READY.set(true);
                later(super::data::clear_pending_cookies);
                for then in WAITING.with_borrow_mut(std::mem::take) {
                    later(then);
                }
            });
        }

        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            schedule_pump(delay_ms);
        }

    }
}

/// Loads and starts CEF on first use. Returns false when it cannot run: no framework in the
/// bundle (an unbundled build), or an earlier start failed.
pub(crate) fn ensure_started(root: &Path) -> bool {
    match STATE.get() {
        State::Running => return true,
        State::Closing | State::Failed | State::Stopped => return false,
        State::Off => {}
    }
    let started = start(root);
    STATE.set(if started { State::Running } else { State::Failed });
    if started {
        ROOT.set(Some(root.to_path_buf()));
        later(|| heartbeat(std::ptr::null_mut()));
    }
    started
}

pub(crate) fn running() -> bool {
    STATE.get() == State::Running
}

/// Chooses Chrome's two-finger swipe back and forward for every browser at once. Chrome keeps it
/// in one Local State preference that CEF refuses to change while running, so the choice is
/// written into Local State once Chromium has shut down and holds from the next launch.
#[cfg(windows)]
pub fn set_swipe_navigation(on: bool) {
    SWIPE.set(Some(on));
}

/// Writes the swipe choice into Chromium's Local State under `root`; Chrome's other settings stay.
#[cfg(windows)]
fn write_swipe(root: &Path, on: bool) {
    let file = root.join("Local State");
    let Ok(mut state) = std::fs::read_to_string(&file).map(|s| serde_json::from_str::<serde_json::Value>(&s).unwrap_or_default()) else { return };
    let Some(a11y) = state
        .as_object_mut()
        .and_then(|s| s.entry("settings").or_insert_with(|| serde_json::json!({})).as_object_mut())
        .and_then(|s| s.entry("a11y").or_insert_with(|| serde_json::json!({})).as_object_mut())
    else {
        return;
    };
    if a11y.insert("overscroll_history_navigation".to_owned(), on.into()) == Some(on.into()) {
        return;
    }
    // Chromium resets an unreadable Local State and with it the key its saved cookies are sealed with.
    let staged = root.join("Local State.tarn");
    let written = std::fs::File::create(&staged).and_then(|mut f| std::io::Write::write_all(&mut f, state.to_string().as_bytes()).and_then(|()| f.sync_all()));
    if written.and_then(|()| std::fs::rename(&staged, &file)).is_err() {
        eprintln!("tarn: Chromium's swipe setting could not be saved");
    }
}

/// True while CEF has not touched its folder in this process.
#[cfg(target_os = "macos")]
pub(crate) fn idle() -> bool {
    matches!(STATE.get(), State::Off | State::Failed)
}

/// After a wipe, a live CEF can still write the old profile back; it gets no new views until
/// the next launch, which deletes the folder before CEF starts.
#[cfg(target_os = "macos")]
pub(crate) fn retire() {
    RETIRED.set(true);
}

#[cfg(target_os = "macos")]
pub(crate) fn retired() -> bool {
    RETIRED.get()
}

pub(crate) fn root() -> Option<PathBuf> {
    ROOT.with_borrow(Clone::clone)
}

#[cfg(windows)]
pub(crate) fn set_root(root: &Path) {
    ROOT.set(Some(root.to_path_buf()));
}

#[cfg(target_os = "macos")]
fn contents() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.parent()?.to_path_buf())
}

/// The sandbox information CEF's bootstrap launcher hands `RunWinMain` (crates/app-dll); null
/// in a build run without the launcher, whose Chromium children then run unsandboxed.
#[cfg(windows)]
static SANDBOX_INFO: std::sync::atomic::AtomicPtr<c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// Keeps the sandbox information of CEF's launcher for `subprocess` and `start`; call before both.
///
/// # Safety
/// `info` must be null or the `sandbox_info` pointer CEF's launcher passed to `RunWinMain`, valid
/// for the rest of the process.
#[cfg(windows)]
pub unsafe fn set_sandbox_info(info: *mut c_void) {
    SANDBOX_INFO.store(info, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(windows)]
fn sandbox_info() -> *mut u8 {
    SANDBOX_INFO.load(std::sync::atomic::Ordering::Relaxed).cast()
}

fn unsandboxed() -> bool {
    #[cfg(windows)]
    return sandbox_info().is_null();
    #[cfg(not(windows))]
    false
}

/// CEF's files sit next to `tarn.exe`, which runs Chromium's child processes too (`subprocess`).
/// A release `tarn.exe` is CEF's bootstrap launcher, which sandboxes the children and loads Tarn
/// from `tarn.dll`.
#[cfg(windows)]
fn start(root: &Path) -> bool {
    // A release build only runs Chromium in its sandbox, so a plain tarn.exe shipped by mistake fails closed.
    if unsandboxed() && option_env!("TARN_RELEASE_BUILD").is_some_and(|v| !v.is_empty()) {
        crate::alert("Chromium cannot start without its sandbox.", "This copy of Tarn was not started through its tarn.exe launcher. Reinstall Tarn.");
        return false;
    }
    let Some(root) = std::fs::create_dir_all(root).and_then(|()| std::path::absolute(root)).ok() else { return false };
    super::pump_windows::init(|| pump(std::ptr::null_mut()));
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let settings = Settings {
        no_sandbox: unsandboxed().into(),
        command_line_args_disabled: 1,
        external_message_pump: 1,
        root_cache_path: CefString::from(root.to_string_lossy().as_ref()),
        persist_session_cookies: 0,
        log_severity: LogSeverity::ERROR,
        ..Default::default()
    };
    let mut app = TarnApp::new(root.to_string_lossy().into_owned());
    initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), sandbox_info()) == 1
}

/// Runs a Chromium child process when CEF started this executable as one; call first in `main`.
/// `None` in the browser process, where CEF needs this call before `initialize` as well.
#[cfg(windows)]
pub fn subprocess() -> Option<i32> {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let code = execute_process(Some(args.as_main_args()), Some(&mut ChildApp::new()), sandbox_info());
    (code >= 0).then_some(code)
}

/// The message a renderer sends with its process id when it starts a main-frame document; the
/// macOS helper (crates/chromium-helper) sends the same.
pub(super) const RENDERER_PID: &str = "tarn.renderer-pid";

#[cfg(windows)]
wrap_app! {
    struct ChildApp;

    impl App {
        fn render_process_handler(&self) -> Option<RenderProcessHandler> {
            Some(ChildRenderer::new())
        }
    }
}

#[cfg(windows)]
wrap_render_process_handler! {
    struct ChildRenderer;

    impl RenderProcessHandler {
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(ChildLoad::new())
        }
    }
}

#[cfg(windows)]
wrap_load_handler! {
    struct ChildLoad;

    impl LoadHandler {
        fn on_load_start(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _transition_type: TransitionType) {
            let Some(frame) = frame.filter(|f| f.is_main() == 1) else { return };
            let Some(mut message) = process_message_create(Some(&CefString::from(RENDERER_PID))) else { return };
            let Ok(pid) = i32::try_from(std::process::id()) else { return };
            if message.argument_list().is_some_and(|a| a.set_int(0, pid) == 1) {
                frame.send_process_message(ProcessId::BROWSER, Some(&mut message));
            }
        }
    }
}

/// Whether `pid` is another running process of this Tarn executable (a Chromium child).
#[cfg(windows)]
pub(super) fn tarn_process(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
    static OWN: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    if pid == std::process::id() {
        return false;
    }
    let Some(own) = OWN.get_or_init(|| std::env::current_exe().and_then(std::fs::canonicalize).ok()) else { return false };
    // SAFETY: a handle opened here is closed below; null means no access.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    let mut path = [0u16; 32_768];
    let mut len = path.len() as u32;
    // SAFETY: `process` is open with query access; `path` holds `len` UTF-16 units.
    let read = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &raw mut len) } != 0;
    // SAFETY: closes the handle opened above.
    unsafe { CloseHandle(process) };
    read && std::fs::canonicalize(String::from_utf16_lossy(&path[..len as usize])).is_ok_and(|p| &p == own)
}

/// Whether `pid` is another running process of this bundle (a Chromium helper).
#[cfg(target_os = "macos")]
pub(super) fn tarn_process(pid: u32) -> bool {
    unsafe extern "C" {
        fn proc_pidpath(pid: std::ffi::c_int, buffer: *mut c_void, size: u32) -> std::ffi::c_int;
    }
    static OWN: OnceLock<Option<PathBuf>> = OnceLock::new();
    if pid == std::process::id() {
        return false;
    }
    let (Some(own), Ok(pid)) = (OWN.get_or_init(|| contents().and_then(|c| c.join("Frameworks").canonicalize().ok())), i32::try_from(pid)) else {
        return false;
    };
    // PROC_PIDPATHINFO_MAXSIZE.
    let mut path = [0u8; 4096];
    // SAFETY: `path` is writable for the `size` bytes passed; the call writes at most that many.
    let len = unsafe { proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
    let path = usize::try_from(len).ok().filter(|n| *n > 0).and_then(|n| std::str::from_utf8(&path[..n]).ok());
    path.and_then(|p| Path::new(p).canonicalize().ok()).is_some_and(|p| p.starts_with(own))
}

/// Prepares the browser process before GPUI starts: GPUI must draw without DirectComposition,
/// whose surface would cover Chromium's child windows, and its thread gets the window hooks.
#[cfg(windows)]
pub fn prepare_browser_process() {
    // SAFETY: called at the top of `main`, before any other thread exists.
    unsafe { std::env::set_var("GPUI_DISABLE_DIRECT_COMPOSITION", "1") };
    super::pump_windows::install_hooks();
}

#[cfg(target_os = "macos")]
fn start(root: &Path) -> bool {
    let Some(contents) = contents() else { return false };
    let framework = contents.join("Frameworks").join(FRAMEWORK);
    let Some(path) = framework.to_str().and_then(|p| std::ffi::CString::new(p).ok()).filter(|_| framework.exists()) else {
        return false;
    };
    // SAFETY: `path` is a NUL-terminated path to the bundled framework binary.
    if load_library(Some(unsafe { &*path.as_ptr() })) != 1 {
        return false;
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    if !adopt_cef_protocols() {
        return false;
    }
    // CEF compares `cache_path` with the canonical `root_cache_path` and falls back to memory
    // when a symlinked prefix (`/var` → `/private/var`) makes them differ.
    let Some(root) = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(root)
        .and_then(|()| root.canonicalize())
        .ok()
        .filter(|root| write_preferences(&root.join("Default")))
    else {
        return false;
    };
    let args = args::Args::new();
    let settings = Settings {
        no_sandbox: 0,
        command_line_args_disabled: 1,
        external_message_pump: 1,
        root_cache_path: CefString::from(root.to_string_lossy().as_ref()),
        cache_path: CefString::from(root.join("Default").to_string_lossy().as_ref()),
        persist_session_cookies: 0,
        log_severity: LogSeverity::ERROR,
        keychain_service_name: CefString::from("Tarn Safe Storage"),
        keychain_account_name: CefString::from("Tarn"),
        ..Default::default()
    };
    let mut app = TarnApp::new(root.to_string_lossy().into_owned());
    if initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
        return false;
    }
    true
}

/// Runs `job` from the main queue, outside CEF's message loop: a browser created inside a CEF
/// task or context callback never starts navigating (and one created from a context callback
/// crashed in `add_child_view`).
fn later(job: impl FnOnce() + 'static) {
    after(Duration::ZERO, job);
}

#[cfg(windows)]
pub(crate) use super::pump_windows::after;

/// Runs `job` on the main queue after `delay`; `job` need not be `Send`, so the caller must be on
/// the main thread too.
#[cfg(target_os = "macos")]
pub(crate) fn after(delay: Duration, job: impl FnOnce() + 'static) {
    assert!(objc2::MainThreadMarker::new().is_some(), "runtime::after called off the main thread");
    extern "C" fn run(context: *mut c_void) {
        // SAFETY: `context` is the box leaked below, passed to this function exactly once.
        let job = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce()>>()) };
        job();
    }
    let job: Box<Box<dyn FnOnce()>> = Box::new(Box::new(job));
    let delay = i64::try_from(delay.as_nanos()).unwrap_or(i64::MAX);
    // SAFETY: the main queue is static; `run` takes back ownership of the leaked box.
    unsafe { dispatch_after_f(dispatch_time(0, delay), &raw const _dispatch_main_q, Box::into_raw(job).cast(), run) };
}

/// Runs `then` with Chromium's request context once CEF is ready. There is one persistent
/// context: separate request contexts per profile never finished initialising in CEF 156 Chrome
/// style (observed before the pump heartbeat ran), so `crate::chromium_owner`
/// gives Chromium to one Tarn profile.
#[cfg(target_os = "macos")]
pub(crate) fn with_context(then: impl FnOnce(RequestContext) + 'static) {
    let run = move || {
        if running()
            && let Some(mut context) = request_context_get_global_context()
        {
            WORKER_FILTER.with_borrow_mut(|f| {
                if f.is_none() {
                    *f = super::filter::worker_filter(&mut context);
                }
            });
            then(context);
        }
    };
    if READY.get() {
        later(run);
    } else {
        WAITING.with_borrow_mut(|w| w.push(Box::new(run)));
    }
}

/// Runs `then` with the request context of the data store `key` once CEF is ready, creating it
/// on first use: in `cache` (below the root) or, without one, in memory. Each Tarn profile has its
/// own Chromium profile on Windows; its locked preferences are written before Chromium opens it.
/// Service workers have no tab, so their requests follow the Shield `shield` of the profile that
/// opened the context first.
#[cfg(windows)]
pub(crate) fn with_store_context(key: String, shield: String, cache: Option<PathBuf>, then: impl FnOnce(RequestContext) + 'static) {
    let run = move || {
        if !running() {
            return;
        }
        let existing = CONTEXTS.with_borrow(|c| c.get(&key).cloned());
        let context = existing.or_else(|| {
            if cache.as_deref().is_some_and(|dir| !write_preferences(dir)) {
                return None;
            }
            let settings = RequestContextSettings {
                cache_path: CefString::from(cache.as_deref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default().as_str()),
                persist_session_cookies: 0,
                ..Default::default()
            };
            let context = request_context_create_context(Some(&settings), Some(&mut super::filter::TarnContextFilter::new(shield.clone())))?;
            CONTEXTS.with_borrow_mut(|c| c.insert(key.clone(), context.clone()));
            Some(context)
        });
        if let Some(context) = context {
            then(context);
        }
    };
    if READY.get() {
        later(run);
    } else {
        WAITING.with_borrow_mut(|w| w.push(Box::new(run)));
    }
}

/// The live request context of the data store `key`, if CEF opened one.
#[cfg(windows)]
pub(crate) fn store_context(key: &str) -> Option<RequestContext> {
    CONTEXTS.with_borrow(|c| c.get(key).cloned())
}

/// Closes every Chromium window and shuts CEF down; call once when the app quits.
pub fn shutdown() {
    if STATE.get() != State::Running {
        #[cfg(windows)]
        if let (State::Off, Some(on), Some(root)) = (STATE.get(), SWIPE.get(), root()) {
            write_swipe(&root, on);
        }
        return;
    }
    let emptied = if READY.get() { super::cookies::finish() } else { std::rc::Rc::new(Cell::new(true)) };
    let until = Instant::now() + Duration::from_secs(2);
    while !emptied.get() && Instant::now() < until {
        do_message_loop_work();
        std::thread::sleep(Duration::from_millis(5));
    }
    STATE.set(State::Closing);
    super::view::close_all();
    let until = Instant::now() + Duration::from_secs(2);
    while super::view::open_count() > 0 && Instant::now() < until {
        do_message_loop_work();
        std::thread::sleep(Duration::from_millis(5));
    }
    STATE.set(State::Stopped);
    #[cfg(target_os = "macos")]
    WORKER_FILTER.take();
    #[cfg(windows)]
    CONTEXTS.with_borrow_mut(std::collections::HashMap::clear);
    cef::shutdown();
    #[cfg(windows)]
    if let (Some(on), Some(root)) = (SWIPE.get(), root()) {
        write_swipe(&root, on);
    }
}

/// Whether AppKit is dispatching an event right now, so `NSApp.currentEvent` is the one being handled.
#[cfg(target_os = "macos")]
pub(super) fn in_send_event() -> bool {
    HANDLING_SEND_EVENT.get()
}

#[cfg(target_os = "macos")]
extern "C-unwind" fn is_handling_send_event(_: &AnyObject, _: Sel) -> Bool {
    Bool::new(HANDLING_SEND_EVENT.get())
}

#[cfg(target_os = "macos")]
extern "C-unwind" fn set_handling_send_event(_: &AnyObject, _: Sel, value: Bool) {
    HANDLING_SEND_EVENT.set(value.as_bool());
}

#[cfg(target_os = "macos")]
type SendEvent = unsafe extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject);

#[cfg(target_os = "macos")]
extern "C-unwind" fn send_event(this: &AnyObject, cmd: Sel, event: *mut AnyObject) {
    let was = HANDLING_SEND_EVENT.replace(true);
    if let Some(imp) = NSAPP_SEND_EVENT.get() {
        // SAFETY: the IMP is NSApplication's own sendEvent:, called as the `super` of the method
        // this module adds to GPUI's subclass, with the receiver and arguments it was given.
        unsafe { std::mem::transmute::<Imp, SendEvent>(*imp)(this, cmd, event) };
    }
    HANDLING_SEND_EVENT.set(was);
}

/// CEF needs the application object to report whether it is inside `sendEvent:`. GPUI owns that
/// object, so the methods and protocols are added to its class at run time.
#[cfg(target_os = "macos")]
fn adopt_cef_protocols() -> bool {
    let (Some(nsapp), Some(gpui)) = (AnyClass::get(c"NSApplication"), AnyClass::get(c"GPUIApplication")) else { return false };
    let Some(super_send) = nsapp.instance_method(sel!(sendEvent:)) else { return false };
    NSAPP_SEND_EVENT.get_or_init(|| super_send.implementation());
    let class = gpui as *const AnyClass as *mut AnyClass;
    let bool_encoding = <Bool as objc2::Encode>::ENCODING.to_string();
    let (Ok(is_handling), Ok(set_handling)) = (CString::new(format!("{bool_encoding}@:")), CString::new(format!("v@:{bool_encoding}"))) else {
        return false;
    };
    // SAFETY: `class` is a registered class; each IMP has the signature its type encoding states.
    let added = unsafe {
        [
            ffi::class_addMethod(
                class,
                sel!(isHandlingSendEvent),
                std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel) -> Bool, Imp>(is_handling_send_event),
                is_handling.as_ptr(),
            ),
            ffi::class_addMethod(
                class,
                sel!(setHandlingSendEvent:),
                std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel, Bool), Imp>(set_handling_send_event),
                set_handling.as_ptr(),
            ),
            ffi::class_addMethod(
                class,
                sel!(sendEvent:),
                std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject), Imp>(send_event),
                c"v@:@".as_ptr(),
            ),
        ]
    };
    let protocols: [&CStr; 2] = [c"CrAppProtocol", c"CrAppControlProtocol"];
    for name in protocols {
        let Some(protocol) = AnyProtocol::get(name) else { return false };
        // SAFETY: both are live runtime objects.
        unsafe { ffi::class_addProtocol(class, protocol) };
    }
    added.iter().all(|b| b.as_bool())
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_after_f(when: u64, queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
    fn dispatch_time(when: u64, delta: i64) -> u64;
}

extern "C" fn pump(_: *mut c_void) {
    if STATE.get() != State::Running {
        return;
    }
    if PUMPING.replace(true) {
        schedule_pump(1000 / 30);
        return;
    }
    do_message_loop_work();
    PUMPING.set(false);
}

extern "C" fn heartbeat(context: *mut c_void) {
    if STATE.get() != State::Running {
        return;
    }
    pump(context);
    // With no page on screen CEF's own pump requests carry the work; the beat only backs them up.
    let visible = super::view::any_visible();
    let beat = BEATS.get().wrapping_add(1);
    BEATS.set(beat);
    if visible && beat.is_multiple_of(COOKIE_SYNC_SECS * HEARTBEAT_HZ) {
        super::cookies::changed_in_chromium();
    }
    schedule(1_000_000_000 / if visible { HEARTBEAT_HZ } else { IDLE_HEARTBEAT_HZ } as i64, heartbeat);
}

/// CEF asks for this from any of its threads.
#[cfg(target_os = "macos")]
fn schedule_pump(delay_ms: i64) {
    schedule(delay_ms.clamp(0, 1000 / 30) * 1_000_000, pump);
}

#[cfg(windows)]
fn schedule_pump(delay_ms: i64) {
    super::pump_windows::wake(delay_ms.clamp(0, 1000 / 30));
}

/// Calls `work` with a null context on the main thread in `delay_ns`.
#[cfg(target_os = "macos")]
fn schedule(delay_ns: i64, work: extern "C" fn(*mut c_void)) {
    // SAFETY: the main queue is a static object that lives for the process; `work` ignores its
    // context pointer.
    unsafe { dispatch_after_f(dispatch_time(0, delay_ns), &raw const _dispatch_main_q, std::ptr::null_mut(), work) };
}

#[cfg(windows)]
fn schedule(delay_ns: i64, work: extern "C" fn(*mut c_void)) {
    after(Duration::from_nanos(u64::try_from(delay_ns).unwrap_or(0)), move || work(std::ptr::null_mut()));
}

#[cfg(test)]
mod tests {
    use super::{LOCKED, REFUSED, SWITCHES, lock_preferences, write_preferences};

    #[cfg(windows)]
    #[test]
    fn a_reported_renderer_must_be_another_process_of_this_executable() {
        assert!(!super::tarn_process(std::process::id()), "not this process");
        assert!(!super::tarn_process(0));
        let mut other = std::process::Command::new("cmd").args(["/c", "ping -n 5 127.0.0.1 >nul"]).spawn().unwrap();
        assert!(!super::tarn_process(other.id()), "another program");
        other.kill().ok();
        other.wait().ok();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_reported_renderer_must_be_a_helper_of_this_bundle() {
        assert!(!super::tarn_process(std::process::id()), "not this process");
        assert!(!super::tarn_process(1), "not a process outside the bundle");
        let mut other = std::process::Command::new("sleep").arg("5").spawn().unwrap();
        assert!(!super::tarn_process(other.id()), "another program");
        other.kill().ok();
        other.wait().ok();
    }

    #[test]
    fn tarn_never_adds_a_switch_it_refuses() {
        assert!(SWITCHES.iter().all(|switch| !REFUSED.contains(switch)));
        assert!(REFUSED.iter().all(|switch| !switch.starts_with('-') && *switch == switch.to_ascii_lowercase()));
    }

    fn parsed(s: &str) -> serde_json::Value {
        serde_json::from_str(&lock_preferences(s)).unwrap()
    }

    #[test]
    fn locked_preferences_are_set_and_others_kept() {
        let all = parsed("");
        for (path, value) in LOCKED {
            let got = path.split('.').fold(&all, |node, key| &node[key]);
            assert_eq!(got, value, "{path}");
        }
        assert_eq!(all["ssl"]["error_override_allowed"], false);
        assert_eq!(parsed(r#"{"profile":{"default_content_setting_values":7}}"#)["profile"]["default_content_setting_values"]["usb_guard"], 2);
        assert_eq!(parsed("")["profile"]["cookie_controls_mode"], 1);
        let kept = parsed(r#"{"profile":{"cookie_controls_mode":2,"name":"x"},"other":true}"#);
        assert_eq!(kept["profile"]["cookie_controls_mode"], 1);
        assert_eq!((kept["profile"]["name"].as_str(), kept["other"].as_bool()), (Some("x"), Some(true)));
        for broken in ["not json", "[]", r#"{"profile":3}"#, "{\"profile\":{"] {
            assert_eq!(parsed(broken)["profile"]["cookie_controls_mode"], 1, "{broken}");
        }
    }

    #[test]
    fn preferences_are_written_privately_and_replaced_whole() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("Default");
        assert!(write_preferences(&profile));
        let file = profile.join("Preferences");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(&profile).unwrap().permissions().mode() & 0o777, 0o700);
        }
        std::fs::write(&file, "{\"profile\":{").unwrap();
        assert!(write_preferences(&profile));
        assert!(std::fs::read_to_string(&file).unwrap().contains("\"cookie_controls_mode\":1"));
        assert!(!profile.join("Preferences.tarn").exists());
    }
}
