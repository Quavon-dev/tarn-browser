use std::cell::{Cell, RefCell};
use std::rc::Rc;

use cef::*;
#[cfg(target_os = "macos")]
use objc2::MainThreadMarker;
#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplication, NSEvent, NSEventType, NSView, NSWindow, NSWindowOrderingMode};
use url::Url;
#[cfg(windows)]
use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, HWND_TOP, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetParent, SetWindowPos, ShowWindow,
};

use crate::handover::{self, Storage, js_string};
use crate::nav_policy::{self, Decision, GateAction, Source};
use crate::tarn_ext::{ExtensionView, FrameId, Gate, PageSpec, World, WorldScripts};
use crate::{Event, Host, Protection};

type Found = Box<dyn Fn(usize, Option<usize>)>;
type Reply = Box<dyn FnOnce(Option<Value>)>;
#[cfg(windows)]
type CreatedHook = Box<dyn FnOnce(Browser)>;

const ADD_SCRIPT: &str = "Page.addScriptToEvaluateOnNewDocument";

#[path = "tarn.rs"]
mod tarn;

struct Tab {
    id: u64,
    /// Whose Shield choices apply (`DataStore::shield_key`); empty on macOS.
    profile: String,
    /// The data store whose request context the tab uses (`DataStore::context_key`); empty on macOS.
    store: String,
    browser: RefCell<Option<Browser>>,
    calls: RefCell<std::collections::HashMap<i32, Reply>>,
    next_call: Cell<i32>,
    devtools: RefCell<Option<Registration>>,
    handover_script: RefCell<Option<String>>,
    sink: RefCell<Rc<dyn Fn(Event)>>,
    title: RefCell<String>,
    history: Cell<(bool, bool)>,
    loading: Cell<bool>,
    found: RefCell<Option<Found>>,
    shell_load: RefCell<Option<Url>>,
    /// Files this view showed through `load_file`; only they get Open File's credit (S6).
    opened_files: RefCell<std::collections::HashSet<Url>>,
    /// The main-frame navigation under way started in the shell, so its redirects may reach a
    /// private address (S10).
    shell_chain: Cell<bool>,
    last_find: RefCell<String>,
    /// Web Inspector may open from the page's menu, as in WebKit tabs (S10).
    inspectable: Cell<bool>,
    #[cfg(target_os = "macos")]
    download_notice: Cell<Option<std::time::Instant>>,
    /// Shared with the pop-ups the tab opens (S23), as WebKit's are.
    recent_popups: Rc<RefCell<std::collections::VecDeque<std::time::Instant>>>,
    /// Downloads this tab started lately; a popup is a tab of its own, and pages open few of them.
    #[cfg(windows)]
    recent_downloads: RefCell<std::collections::VecDeque<std::time::Instant>>,
    recent_prompts: RefCell<std::collections::VecDeque<std::time::Instant>>,
    silenced: Rc<Cell<bool>>,
    /// The URLs a back, forward or reload the shell asked for may go to, and whether the main frame's current
    /// navigation was started by the user (shell, click or history), which its redirects keep.
    history_step: RefCell<Vec<Url>>,
    user_started: Cell<bool>,
    /// The origin of the main-frame navigation last let through, which may ask for credentials
    /// before it commits, and how often credentials were asked for since a page last finished.
    navigating: RefCell<Option<String>>,
    auth_attempts: Cell<u32>,
    /// The shell is loading a page's link (`load_link`): the next main-frame load is not the
    /// user's, whatever URL Chromium reports for it.
    link_load: Cell<bool>,
    /// The profile's download events, which outlive the tab.
    #[cfg(windows)]
    downloads: std::cell::OnceCell<Rc<dyn Fn(Event)>>,
    /// A Lockdown view: no JavaScript and no site styles; only Lockdown sites load in it.
    #[cfg(windows)]
    lockdown_view: Cell<bool>,
    /// A hash of the style script the page last got, so a restyle that changes nothing sends nothing.
    #[cfg(windows)]
    styled: Cell<u64>,
    /// A view where the profile pauses no blocking at creation: it runs the profile's page script.
    #[cfg(windows)]
    filtered: Cell<bool>,
    /// The hash of the page script the view should run (0 for none) and the identifier Chromium
    /// gave the registered one.
    #[cfg(windows)]
    page_hash: Cell<u64>,
    #[cfg(windows)]
    page_script: RefCell<Option<String>>,
    /// The page whose link this view was rebuilt for, judged as the page left until the view shows one.
    #[cfg(windows)]
    opener: RefCell<Option<Url>>,
    /// Finishes the view once CEF made its browser; Windows creates browsers asynchronously.
    #[cfg(windows)]
    created_hook: RefCell<Option<CreatedHook>>,
    /// The renderer process of the main frame, as it reported itself (`RENDERER_PID`).
    renderer: Cell<Option<u32>>,
    /// The back and forward lists of a restored tab beyond Chromium's own, which cannot be given
    /// back to it, and the saved state they came from, kept until the tab shows a page.
    #[cfg(windows)]
    restored: RefCell<crate::view_policy::RestoredHistory>,
    #[cfg(windows)]
    restored_bytes: RefCell<Option<Vec<u8>>>,
    /// A step into `restored` under way: back or forward, the page it loads and the page it leaves,
    /// and whether its navigation started. The lists change once that navigation commits, wherever
    /// it redirected to.
    #[cfg(windows)]
    restored_step: RefCell<Option<(bool, Url, Url)>>,
    #[cfg(windows)]
    restored_started: Cell<bool>,
    /// The restore's own load, which keeps the restored forward list.
    #[cfg(windows)]
    restoring: Cell<bool>,
    tarn: RefCell<tarn::TarnTab>,
}

thread_local! {
    static OPEN: Cell<usize> = const { Cell::new(0) };
    static TABS: RefCell<std::collections::HashMap<u64, Rc<Tab>>> = RefCell::new(std::collections::HashMap::new());
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
    #[cfg(target_os = "macos")]
    static WINDOWS: RefCell<Vec<cef::Window>> = const { RefCell::new(Vec::new()) };
    static VISIBLE: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn any_visible() -> bool {
    VISIBLE.get() > 0
}

/// Whether the owning profile's choices put `url` at `lockdown`; before the first snapshot every
/// URL counts as one, as the request filter cancels everything until then.
fn lockdown(profile: &str, url: &Url) -> bool {
    super::filter::shield_for(profile).is_none_or(|s| (s.site_for)(url).level == shield::ShieldLevel::Lockdown)
}

/// The page a fresh view (about:blank) was opened from stands in for it; without one, about:blank
/// stays and counts as public.
#[cfg(windows)]
fn with_opener(tab: &Tab, page: Option<Url>) -> Option<Url> {
    match (page, tab.opener.borrow().clone()) {
        (Some(p), Some(opener)) if p.as_str() == "about:blank" => Some(opener),
        (page, _) => page,
    }
}

/// Views rebuilt across the Lockdown boundary in the burst window, for all tabs: a page that keeps
/// switching, or a decision the shell does not share, cannot rebuild views forever.
#[cfg(windows)]
const REBUILD_BURST: usize = 4;

#[cfg(windows)]
thread_local! {
    static REBUILDS: RefCell<std::collections::VecDeque<std::time::Instant>> = RefCell::default();
}

/// Handlers carry only this id: CEF asks the client for some handlers on its IO thread, where an
/// `Rc` must not be touched. The tab itself is looked up on the UI thread.
fn tab(id: u64) -> Option<Rc<Tab>> {
    TABS.with_borrow(|t| t.get(&id).cloned())
}

#[cfg(target_os = "macos")]
pub(super) fn notify(id: u64, event: Event) {
    if let Some(tab) = tab(id) {
        emit(&tab, event);
    }
}

fn emit(tab: &Tab, event: Event) {
    let sink = tab.sink.borrow().clone();
    sink(event);
}

fn string(s: Option<&CefString>) -> String {
    s.map(|s| s.to_string()).unwrap_or_default()
}

pub(crate) fn open_count() -> usize {
    OPEN.get()
}

pub(crate) fn close_all() {
    #[cfg(target_os = "macos")]
    for window in WINDOWS.with_borrow_mut(std::mem::take) {
        window.close();
    }
    #[cfg(windows)]
    for tab in TABS.with_borrow(|t| t.values().cloned().collect::<Vec<_>>()) {
        if let Some(host) = tab.browser.borrow().as_ref().and_then(|b| b.host()) {
            host.close_browser(1);
        }
    }
}

/// Sends one DevTools method to the tab's page; `reply` gets the parsed result, or `None`.
fn call(tab: &Tab, host: &BrowserHost, method: &str, params: &str, reply: impl FnOnce(Option<Value>) + 'static) {
    call_in(tab, host, None, method, params, reply);
}

/// As `call`, to the target attached as `session` (`Target.setAutoAttach`) or, without one, the page.
fn call_in(tab: &Tab, host: &BrowserHost, session: Option<&str>, method: &str, params: &str, reply: impl FnOnce(Option<Value>) + 'static) {
    if tab.devtools.borrow().is_none() {
        *tab.devtools.borrow_mut() = host.add_dev_tools_message_observer(Some(&mut TarnDevTools::new(tab.id)));
    }
    if tab.devtools.borrow().is_none() {
        return reply(None);
    }
    let id = tab.next_call.get() + 1;
    tab.next_call.set(id);
    tab.calls.borrow_mut().insert(id, Box::new(reply));
    let session = session.map(|s| format!(",\"sessionId\":{}", js_string(s))).unwrap_or_default();
    let message = format!("{{\"id\":{id},\"method\":{},\"params\":{params}{session}}}", js_string(method));
    if host.send_dev_tools_message(Some(message.as_bytes())) != 1 {
        let failed = tab.calls.borrow_mut().remove(&id);
        if let Some(reply) = failed {
            reply(None);
        }
    }
}

/// The handover script applies to the tab's first page only, whether that page loads or fails.
fn drop_handover(tab: &Tab, browser: Option<&mut Browser>) {
    if let Some(host) = browser.and_then(|b| b.host())
        && let Some(identifier) = tab.handover_script.borrow_mut().take()
    {
        remove_script(tab, &host, &identifier);
    }
}

fn remove_script(tab: &Tab, host: &BrowserHost, identifier: &str) {
    call(tab, host, "Page.removeScriptToEvaluateOnNewDocument", &format!("{{\"identifier\":{}}}", js_string(identifier)), |_| {});
}

/// The profile's page script for a view that runs it: never in a Lockdown view (no JavaScript) or
/// one made for a site where blocking is paused.
#[cfg(windows)]
fn page_script_for(tab: &Tab) -> Option<String> {
    crate::data_store_windows::page_script_of(&tab.profile).filter(|_| tab.filtered.get() && !tab.lockdown_view.get())
}

/// Keeps the page script Chromium registered as `identifier` while it is still the one wanted and
/// none is registered yet; replies to an earlier change can arrive after a later change.
#[cfg(windows)]
fn keep_page_script(tab: &Tab, hash: u64, identifier: Option<String>) {
    let Some(identifier) = identifier else {
        if tab.page_hash.get() == hash && tab.page_script.borrow().is_none() {
            tab.page_hash.set(0);
        }
        return;
    };
    if tab.page_hash.get() == hash && tab.page_script.borrow().is_none() {
        *tab.page_script.borrow_mut() = Some(identifier);
    } else if let Some(host) = tab.browser.borrow().as_ref().and_then(|b| b.host()) {
        remove_script(tab, &host, &identifier);
    }
}

#[cfg(windows)]
fn add_page_script(source: &str) -> String {
    format!("{{\"source\":{}}}", js_string(source))
}

/// Gives every filtered view of the profile with this Shield key its current page script for the
/// next document, replacing the one it had.
#[cfg(windows)]
pub(crate) fn rescript(profile: &str) {
    let tabs: Vec<Rc<Tab>> = TABS.with_borrow(|t| t.values().filter(|t| t.profile == profile).cloned().collect());
    for tab in tabs {
        let script = page_script_for(&tab);
        let hash = script.as_deref().map_or(0, |s| style_hash(s) | 1);
        let Some(host) = tab.browser.borrow().as_ref().and_then(|b| b.host()) else { continue };
        if tab.page_hash.replace(hash) == hash {
            continue;
        }
        if let Some(old) = tab.page_script.take() {
            remove_script(&tab, &host, &old);
        }
        let Some(script) = script else { continue };
        let weak = Rc::downgrade(&tab);
        call(&tab, &host, ADD_SCRIPT, &add_page_script(&script), move |result| {
            if let Some(tab) = weak.upgrade() {
                keep_page_script(&tab, hash, string_at(result, &["identifier"]));
            }
        });
    }
}

/// The value at `path` as a copy of its own: CEF's nested values refer into their parents, which
/// are kept until it is copied.
fn value_at(value: Option<Value>, path: &[&str]) -> Option<Value> {
    let mut chain = vec![value?];
    for key in path {
        let next = chain.last()?.dictionary()?.value(Some(&CefString::from(*key)))?;
        chain.push(next);
    }
    chain.last()?.copy()
}

/// Runs `body` as an async function in an isolated world of the tab's top frame; `done` gets the
/// string it returns, or an empty string.
fn isolated(tab: &Rc<Tab>, host: BrowserHost, body: &str, gesture: bool, done: impl Fn(String) + 'static) {
    let (tab2, expression) = (tab.clone(), js_string(&format!("(async () => {{ {body} }})()")));
    call(tab, &host.clone(), "Page.getFrameTree", "{}", move |tree| {
        let Some(frame) = string_at(tree, &["frameTree", "frame", "id"]) else { return done(String::new()) };
        let params = format!("{{\"frameId\":{},\"worldName\":\"tarn\",\"grantUniveralAccess\":false}}", js_string(&frame));
        let tab3 = tab2.clone();
        call(&tab2, &host.clone(), "Page.createIsolatedWorld", &params, move |world| {
            let Some(context) = value_at(world, &["executionContextId"]).filter(|v| v.get_type() == ValueType::INT).map(|v| v.int()) else {
                return done(String::new());
            };
            let params =
                format!("{{\"expression\":{expression},\"contextId\":{context},\"awaitPromise\":true,\"returnByValue\":true,\"userGesture\":{gesture}}}");
            call(&tab3, &host, "Runtime.evaluate", &params, move |result| done(string_at(result, &["result", "value"]).unwrap_or_default()));
        });
    });
}

#[cfg(windows)]
fn style_hash(script: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    script.hash(&mut hasher);
    hasher.finish()
}

/// Runs a style script built for `page` in the tab's top frame; one that lands in a document of
/// another host does nothing, and one that did not run lets the next restyle send it again.
#[cfg(windows)]
fn send_style(tab: &Rc<Tab>, host: BrowserHost, script: &str, page: Option<&Url>) {
    let guard = page.and_then(Url::host_str).map(|h| {
        format!("if (location.hostname.replace(/\\.$/, '').toLowerCase() !== {}) return ''; ", js_string(&h.trim_end_matches('.').to_ascii_lowercase()))
    });
    let weak = Rc::downgrade(tab);
    isolated(tab, host, &format!("{}{script}\nreturn 'ok';", guard.unwrap_or_default()), false, move |ran| {
        if let Some(tab) = weak.upgrade().filter(|_| ran.is_empty()) {
            tab.styled.set(0);
        }
    });
}

/// Puts the current site styles and hidden elements of the profile with this Shield key into
/// every page of its tabs now; new documents get them from `on_load_start`.
#[cfg(windows)]
pub(crate) fn restyle(profile: &str) {
    let tabs: Vec<Rc<Tab>> = TABS.with_borrow(|t| t.values().filter(|t| t.profile == profile && !t.lockdown_view.get()).cloned().collect());
    for tab in tabs {
        let (host, page) = tab
            .browser
            .borrow()
            .as_ref()
            .map_or((None, None), |b| (b.host(), b.main_frame().and_then(|f| Url::parse(&CefString::from(&f.url()).to_string()).ok())));
        if let Some(host) = host {
            let script =
                crate::data_store_windows::site_styles_of(profile, page.as_ref()).unwrap_or_else(|| crate::page_scripts::REMOVE_SITE_STYLES.to_owned());
            let hash = style_hash(&script);
            if tab.styled.replace(hash) != hash {
                send_style(&tab, host, &script, page.as_ref());
            }
        }
    }
}

/// The bytes of a base64 answer of DevTools.
fn decode(data: Option<String>) -> Option<Vec<u8>> {
    let binary = base64_decode(Some(&CefString::from(data?.as_str())))?;
    let mut bytes = vec![0; binary.size()];
    (binary.data(Some(&mut bytes), 0) == bytes.len() && !bytes.is_empty()).then_some(bytes)
}

fn string_at(value: Option<Value>, path: &[&str]) -> Option<String> {
    value_at(value, path).filter(|v| v.get_type() == ValueType::STRING).map(|v| CefString::from(&v.string()).to_string())
}

type Step = (&'static str, String, Option<String>);

/// Each call names its host, which is recorded again if the clear fails.
fn clear_storage_calls(host: &str, origins: &[String]) -> Vec<Step> {
    let defaults = [format!("https://{host}"), format!("http://{host}")];
    defaults
        .iter()
        .chain(origins)
        .map(|origin| ("Storage.clearDataForOrigin", format!("{{\"origin\":{},\"storageTypes\":\"all\"}}", js_string(origin)), Some(host.to_owned())))
        .collect()
}

/// Clears `host`'s storage through an open Chromium tab of the data store `store` (empty on
/// macOS, where all tabs share the one profile); false when none is open.
pub(crate) fn clear_site_storage(store: &str, host: &str) -> bool {
    let tabs: Vec<Rc<Tab>> = TABS.with_borrow(|t| t.values().filter(|t| t.store == store).cloned().collect());
    let open = |tab: &Rc<Tab>| tab.browser.borrow().clone();
    let origins: Vec<String> = tabs
        .iter()
        .filter_map(open)
        .filter_map(|b| Url::parse(&CefString::from(&b.main_frame()?.url()).to_string()).ok())
        .filter(|u| u.host_str().is_some_and(|h| h.trim_end_matches('.').eq_ignore_ascii_case(host)))
        .filter_map(|u| crate::page::serialized_origin(&u))
        .collect();
    let Some((tab, host_handle)) = tabs.iter().find_map(|t| Some((t.clone(), open(t)?.host()?))) else { return false };
    for (method, params, site) in clear_storage_calls(host, &origins) {
        call(&tab, &host_handle, method, &params, move |result| {
            if let (None, Some(site)) = (result, site) {
                super::data::record(&site);
            }
        });
    }
    true
}

/// Clears the storage of every site an open tab of the data store `store` shows.
#[cfg(windows)]
pub(crate) fn clear_store_storage(store: &str) {
    let hosts: std::collections::BTreeSet<String> = TABS.with_borrow(|t| {
        t.values()
            .filter(|t| t.store == store)
            .filter_map(|t| Url::parse(&CefString::from(&t.browser.borrow().as_ref()?.main_frame()?.url()).to_string()).ok()?.host_str().map(str::to_owned))
            .collect()
    });
    for host in hosts {
        clear_site_storage(store, &host);
    }
}

/// The method of an event, read without parsing the message: DevTools writes it first. `None`
/// for a reply or another layout, which is parsed.
fn event_method(message: &[u8]) -> Option<&str> {
    let rest = message.strip_prefix(b"{\"method\":\"")?;
    std::str::from_utf8(&rest[..rest.iter().position(|b| *b == b'"')?]).ok()
}

wrap_dev_tools_message_observer! {
    struct TarnDevTools {
        id: u64,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_message(&self, _browser: Option<&mut Browser>, message: Option<&[u8]>) -> ::std::os::raw::c_int {
            let Some(tab) = tab(self.id) else { return 0 };
            let Some(message) = message.filter(|m| m.windows(11).any(|w| w == b"\"sessionId\"")) else { return 0 };
            if event_method(message).is_some_and(|m| !tarn::EVENTS.contains(&m)) {
                return 1;
            }
            let Some(parsed) = parse_json_buffer(Some(message), JsonParserOptions::default())
                .filter(|p| string_at(Some(p.clone()), &["sessionId"]).is_some())
            else {
                return 0;
            };
            let id = value_at(Some(parsed.clone()), &["id"]).filter(|v| v.get_type() == ValueType::INT).map(|v| v.int());
            match id {
                Some(id) => {
                    let reply = tab.calls.borrow_mut().remove(&id);
                    if let Some(reply) = reply {
                        reply(value_at(Some(parsed), &["result"]));
                    }
                }
                None => tarn::session_event(&tab, parsed),
            }
            1
        }

        fn on_dev_tools_method_result(&self, _browser: Option<&mut Browser>, message_id: i32, success: i32, result: Option<&[u8]>) {
            let Some(tab) = tab(self.id) else { return };
            let reply = tab.calls.borrow_mut().remove(&message_id);
            if let Some(reply) = reply {
                reply(result.filter(|_| success == 1).and_then(|r| parse_json_buffer(Some(r), JsonParserOptions::default())));
            }
        }

        fn on_dev_tools_event(&self, _browser: Option<&mut Browser>, method: Option<&CefString>, params: Option<&[u8]>) {
            let method = string(method);
            if let Some(tab) = tab(self.id).filter(|_| tarn::EVENTS.contains(&method.as_str())) {
                tarn::devtools_event(&tab, &method, params.and_then(|p| parse_json_buffer(Some(p), JsonParserOptions::default())));
            }
        }
    }
}

wrap_client! {
    struct TarnClient {
        id: u64,
        profile: String,
        store: String,
        lockdown_view: bool,
    }

    impl Client {
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(TarnLoad::new(self.id))
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(TarnDisplay::new(self.id))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(TarnRequest::new(self.id, self.profile.clone(), self.lockdown_view))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(TarnLifeSpan::new(self.id))
        }

        fn find_handler(&self) -> Option<FindHandler> {
            Some(TarnFind::new(self.id))
        }

        fn permission_handler(&self) -> Option<PermissionHandler> {
            Some(TarnPermission::new(self.id))
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(TarnDownload::new(self.id, self.store.clone()))
        }

        fn command_handler(&self) -> Option<CommandHandler> {
            Some(TarnCommand::new())
        }

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(TarnContextMenu::new(self.id, self.profile.clone()))
        }

        fn jsdialog_handler(&self) -> Option<JsdialogHandler> {
            Some(TarnJsDialog::new(self.id))
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(TarnKeys::new())
        }

        fn on_process_message_received(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _source: ProcessId, message: Option<&mut ProcessMessage>) -> i32 {
            let Some(message) = message.filter(|m| CefString::from(&m.name()).to_string() == super::runtime::RENDERER_PID) else { return 0 };
            // Only the main frame's process counts, and a renderer is untrusted: the pid must name one of Tarn's own processes.
            if frame.is_some_and(|f| f.is_main() == 1)
                && let Some(tab) = tab(self.id)
                && let Some(pid) = message.argument_list().and_then(|a| u32::try_from(a.int(0)).ok()).filter(|p| super::runtime::tarn_process(*p))
            {
                tab.renderer.set(Some(pid));
            }
            1
        }
    }
}

wrap_load_handler! {
    struct TarnLoad {
        id: u64,
    }

    impl LoadHandler {
        fn on_loading_state_change(&self, browser: Option<&mut Browser>, is_loading: i32, can_go_back: i32, can_go_forward: i32) {
            let Some(tab) = tab(self.id) else { return };
            #[cfg(windows)]
            let can_go_back = i32::from(can_go_back == 1 && browser.as_deref().is_some_and(native_back));
            let history = (can_go_back == 1, can_go_forward == 1);
            // Chromium tells no load error for a stop or a cancel, so the shell learns the end here.
            emit(&tab, Event::Loading(is_loading == 1));
            // A navigation that ends without a page (a download, a 204, a cancel) leaves no origin that may ask.
            if is_loading == 0 {
                tab.navigating.take();
                // A step that never navigated (a cancelled unload) leaves nothing behind.
                #[cfg(windows)]
                abandon_step(&tab);
            }
            // Chromium settles the back list after the load ends, so the shell hears it here.
            if tab.history.replace(history) != history
                && is_loading == 0
                && let Some(url) = browser.and_then(|b| b.main_frame()).and_then(|f| Url::parse(&CefString::from(&f.url()).to_string()).ok())
            {
                let (can_go_back, can_go_forward) = history_flags(&tab);
                emit(&tab, Event::UrlChanged { url, can_go_back, can_go_forward });
            }
        }

        fn on_load_start(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, _transition_type: TransitionType) {
            let (Some(tab), Some(frame)) = (tab(self.id), frame) else { return };
            if frame.is_main() != 1 {
                // A subframe, often in a renderer of its own, gets its host's list hiding through CEF, which reaches any frame.
                #[cfg(windows)]
                if !tab.lockdown_view.get()
                    && let Some(top) = browser.and_then(|b| b.main_frame()).and_then(|f| Url::parse(&CefString::from(&f.url()).to_string()).ok())
                    && let Ok(url) = Url::parse(&CefString::from(&frame.url()).to_string())
                    && let Some(script) = crate::data_store_windows::frame_hiding_of(&tab.profile, &top, &url)
                {
                    frame.execute_java_script(Some(&CefString::from(script.as_str())), None, 0);
                }
                return;
            }
            tab.loading.set(true);
            tab.silenced.set(false);
            tab.navigating.take();
            #[cfg(windows)]
            if tab.restored_started.take()
                && let Some((back, _, left)) = tab.restored_step.take()
            {
                tab.restored.borrow_mut().stepped(back, left);
            }
            // Windows views take the profile's current style sheets with each page, so a change
            // reaches the next page of every tab.
            #[cfg(windows)]
            if let (false, Some(host)) = (tab.lockdown_view.get(), tab.browser.borrow().as_ref().and_then(|b| b.host())) {
                let page = Url::parse(&CefString::from(&frame.url()).to_string()).ok();
                let script = crate::data_store_windows::site_styles_of(&tab.profile, page.as_ref());
                tab.styled.set(style_hash(script.as_deref().unwrap_or(crate::page_scripts::REMOVE_SITE_STYLES)));
                if let Some(script) = script {
                    send_style(&tab, host, &script, page.as_ref());
                }
            }
            let Ok(url) = Url::parse(&CefString::from(&frame.url()).to_string()) else { return };
            if url.as_str() != "about:blank" {
                drop_handover(&tab, browser);
                #[cfg(windows)]
                tab.opener.take();
            }
            let (can_go_back, can_go_forward) = history_flags(&tab);
            let title = tab.title.borrow().clone();
            emit(&tab, Event::Committed { url, title, can_go_back, can_go_forward });
        }

        fn on_load_end(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, _http_status_code: i32) {
            let (Some(tab), Some(frame)) = (tab(self.id), frame) else { return };
            if frame.is_main() != 1 {
                return;
            }
            tab.loading.set(false);
            tab.history_step.take();
            tab.auth_attempts.set(0);
            super::cookies::changed_in_chromium();
            let Ok(url) = Url::parse(&CefString::from(&frame.url()).to_string()) else { return };
            let (can_go_back, can_go_forward) = history_flags(&tab);
            let title = tab.title.borrow().clone();
            emit(&tab, Event::Finished { url, title, can_go_back, can_go_forward });
        }

        fn on_load_error(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, error_code: Errorcode, error_text: Option<&CefString>, failed_url: Option<&CefString>) {
            let (Some(tab), Some(frame)) = (tab(self.id), frame) else { return };
            let code = sys::cef_errorcode_t::from(error_code);
            if frame.is_main() != 1 {
                return;
            }
            tab.loading.set(false);
            if !string(failed_url).starts_with("about:") {
                drop_handover(&tab, browser);
            }
            if code == sys::cef_errorcode_t::ERR_ABORTED {
                return;
            }
            let url = Url::parse(&string(failed_url)).ok();
            emit(&tab, Event::Failed { url, code: code as isize, message: crate::page::bounded(&string(error_text)) });
        }
    }
}

wrap_display_handler! {
    struct TarnDisplay {
        id: u64,
    }

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let Some(tab) = tab(self.id) else { return };
            let title = crate::page::bounded(&string(title));
            *tab.title.borrow_mut() = title.clone();
            emit(&tab, Event::TitleChanged(title));
        }

        fn on_status_message(&self, _browser: Option<&mut Browser>, value: Option<&CefString>) {
            if let Some(tab) = tab(self.id) {
                let value: String = string(value).chars().take(crate::view_policy::MAX_HOVERED_LINK).collect();
                emit(&tab, Event::LinkHovered((!value.is_empty()).then_some(value)));
            }
        }

        fn on_address_change(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, url: Option<&CefString>) {
            let (Some(tab), Some(frame)) = (tab(self.id), frame) else { return };
            // An address change between load start and end belongs to that document load; any
            // other is same-document (pushState, fragment), which is what `UrlChanged` means.
            if frame.is_main() == 1 && !tab.loading.get() {
                tab.history_step.take();
                if let Ok(url) = Url::parse(&string(url)) {
                    let (can_go_back, can_go_forward) = history_flags(&tab);
                    emit(&tab, Event::UrlChanged { url, can_go_back, can_go_forward });
                }
            }
        }

        #[cfg(windows)]
        fn on_fullscreen_mode_change(&self, _browser: Option<&mut Browser>, fullscreen: i32) {
            if let Some(tab) = tab(self.id) {
                emit(&tab, Event::FullScreen(fullscreen == 1));
            }
        }
    }
}

wrap_request_handler! {
    struct TarnRequest {
        id: u64,
        profile: String,
        lockdown_view: bool,
    }

    impl RequestHandler {
        fn on_before_browse(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, request: Option<&mut Request>, user_gesture: i32, is_redirect: i32) -> i32 {
            let (Some(tab), Some(frame), Some(request)) = (tab(self.id), frame, request) else { return 1 };
            let Ok(url) = Url::parse(&CefString::from(&request.url()).to_string()) else { return 1 };
            if let Some(allowed) = tarn::page_allows(&tab, &url, frame.is_main() == 1) {
                if !allowed && frame.is_main() == 1 && user_gesture == 1 && crate::tarn_ext::opens_as_tab(&url) {
                    emit(&tab, Event::OpenTab { url, from: None, background: false, split: false });
                }
                return i32::from(!allowed);
            }
            if frame.is_main() != 1 {
                let own_file = url.scheme() == "file" && nav_policy::file_subresource_allowed(&url, page_url(browser.as_deref()).as_ref());
                let refused = !(nav_policy::chromium_subframe_allowed(&url) || own_file) || (!self.lockdown_view && lockdown(&self.profile, &url));
                if !refused {
                    tarn::before_browse(&tab, &url, page_url(browser.as_deref()).as_ref(), false);
                }
                return i32::from(refused);
            }
            if super::outdated() {
                emit(&tab, Event::WebEngineOutdated);
                return 1;
            }
            let (shell, user_started, link, step) = if is_redirect == 1 {
                (false, tab.user_started.get(), false, false)
            } else {
                let (shell, step) = (tab.shell_load.take().is_some_and(|u| u == url), tab.history_step.take().contains(&url));
                // CEF reports the shell's own LoadURL as a gesture; a link the shell loads for a page stays the page's.
                let link = tab.link_load.take();
                (shell, !link && (shell || user_gesture == 1 || step), link, step)
            };
            tab.user_started.set(user_started);
            if nav_policy::chromium_store_link(is_redirect == 1, shell, link, user_gesture == 1)
                && let Some(id) = crate::tarn_ext::store_install(&url)
            {
                emit(&tab, Event::StoreInstall(id));
                return 1;
            }
            let opened = tab.opened_files.borrow().contains(&url);
            let source = if nav_policy::file_open(&url, opened && (shell || step)) {
                Source::FileOpen
            } else if shell {
                Source::Typed
            } else {
                Source::Link
            };
            let decision = if source == Source::FileOpen { nav_policy::decide(&url, source) } else { nav_policy::decide_chromium(&url, source) };
            match decision {
                #[cfg(target_os = "macos")]
                Decision::Allow if lockdown(&self.profile, &url) => {
                    if crate::page::within_burst(&tab.recent_popups, crate::page::POPUP_BURST) {
                        emit(&tab, Event::WebEngineRequired(url));
                    }
                    return 1;
                }
                Decision::Allow => {}
                Decision::Prompt | Decision::Block if nav_policy::chromium_external(&url, source) => {
                    // A redirect's prompt names no page: the one being left did not ask for it.
                    let origin = (!shell && is_redirect == 0).then(|| page_url(browser.as_deref())).flatten().as_ref().and_then(crate::page::serialized_origin);
                    if shell || crate::page::within_burst(&tab.recent_prompts, crate::page::PROMPT_BURST) {
                        emit(&tab, Event::ExternalScheme { url, origin });
                    }
                    return 1;
                }
                Decision::Prompt | Decision::Block => {
                    emit(&tab, Event::Blocked { url, message: "Chromium tabs open only web pages.".into(), category: None });
                    return 1;
                }
            }
            // S10: a public page may not send the top frame to a private address; typed loads,
            // their redirects, Back, Forward and Reload are the user's own.
            if shell || is_redirect == 0 {
                tab.shell_chain.set(shell);
            }
            let transition = request.transition_type().get_raw();
            let history = transition & TransitionType::FORWARD_BACK_FLAG.get_raw() != 0 || transition & 0xff == TransitionType::RELOAD.get_raw();
            let own = tab.shell_chain.get() || history;
            let from = browser.as_ref().and_then(|b| b.main_frame()).and_then(|f| Url::parse(&CefString::from(&f.url()).to_string()).ok());
            #[cfg(windows)]
            let from = with_opener(&tab, from);
            // A page without an origin of its own (about:blank) counts as public: only a fresh tab has no URL.
            if !own && from.is_some_and(|f| !nav_policy::local_page(&f)) && shield::rules::local_target(&url) {
                let message = nav_policy::LOCAL_NETWORK_BLOCKED.into();
                emit(&tab, Event::Protected(Protection::LocalNetwork, None));
                emit(&tab, Event::Blocked { url, message, category: None });
                return 1;
            }
            let Some(shield) = super::filter::shield_for(&self.profile) else { return 1 };
            let site = (shield.site_for)(&url);
            match nav_policy::gate(site.blocked, true, shell, false) {
                GateAction::Allow => {}
                GateAction::Warn(category) | GateAction::Notice(category) => {
                    emit(&tab, Event::Blocked { message: crate::page::blocked_message(category, &url), url, category: Some(category) });
                    return 1;
                }
                GateAction::Cancel => return 1,
            }
            // JavaScript cannot be switched off for one navigation in Chromium.
            #[cfg(target_os = "macos")]
            if !site.level.policy().javascript {
                emit(&tab, Event::Blocked { url, message: "This site is set to Lockdown and opens in WebKit.".into(), category: None });
                return 1;
            }
            // The request filter upgrades plain HTTP unless the URL names a port; that one is
            // refused as WebKit refuses it.
            let left_url = browser.and_then(|b| b.main_frame()).and_then(|f| Url::parse(&CefString::from(&f.url()).to_string()).ok());
            #[cfg(windows)]
            let left_url = with_opener(&tab, left_url);
            let left = left_url.clone().or_else(|| Url::parse("about:blank").ok()).map_or(site.level, |u| (shield.site_for)(&u).level);
            if nav_policy::into_private_network(&url, left_url.as_ref(), user_started, left.policy().block_private_network) {
                emit(&tab, Event::Protected(Protection::LocalNetwork, None));
                emit(&tab, Event::Blocked { url, message: nav_policy::LOCAL_NETWORK_BLOCKED.into(), category: None });
                return 1;
            }
            if url.port().is_some() && nav_policy::level_action(site, left, site.filtered, shell, &url) == nav_policy::LevelAction::BlockInsecure {
                emit(&tab, Event::Blocked { url, message: "This site is set to HTTPS only; the insecure link was blocked.".into(), category: None });
                return 1;
            }
            // Windows has no WebKit: a Lockdown site gets a Chromium view of its own without JavaScript,
            // and leaving Lockdown a normal one. The shell rebuilds the view and loads the URL again,
            // as typed only when the user's own, so the new view judges a page's navigation as the page's.
            #[cfg(windows)]
            if matches!(url.scheme(), "http" | "https") && (site.level == shield::ShieldLevel::Lockdown) != self.lockdown_view {
                // The user's own first step is not counted; its redirect hops are, so a redirect loop
                // across the boundary cannot rebuild views forever.
                if (own && is_redirect == 0) || REBUILDS.with(|r| crate::page::within_burst(r, REBUILD_BURST)) {
                    emit(&tab, if own { Event::ShieldLevelChanged(url) } else { Event::RebuildForLink(url) });
                } else {
                    emit(&tab, Event::Blocked { url, message: "This page keeps switching in and out of Lockdown, so Tarn stopped it.".into(), category: None });
                }
                return 1;
            }
            if (shell || user_gesture == 1) && is_redirect == 0 {
                super::cookies::bridge_navigation(&url, self.id);
            }
            // The request filter upgrades it, by the page's rules or the one it leaves; a redirect
            // hop or history step is the same page.
            if is_redirect == 0 && !history && nav_policy::upgrades_to_https(&url, site.level.policy().https_only || left.policy().https_only) {
                emit(&tab, Event::Protected(Protection::HttpsUpgrade, url.host_str().map(str::to_owned)));
            }
            // Only a navigation let through counts: a step into the restored list starts, or a new
            // page ends its forward list, as it ends Chromium's own.
            #[cfg(windows)]
            if is_redirect == 0 && !history {
                let step = tab.restored_step.borrow().as_ref().is_some_and(|(_, target, _)| *target == url);
                tab.restored_started.set(step);
                if !step && !tab.restoring.take() {
                    tab.restored.borrow_mut().new_page();
                }
            }
            *tab.navigating.borrow_mut() = crate::page::serialized_origin(&url);
            tarn::before_browse(&tab, &url, None, true);
            0
        }

        // CEF's IO thread: the tab answers on the UI thread. Only Basic and Digest are asked
        // for, as in WebKit tabs; proxies and integrated schemes are refused.
        fn auth_credentials(&self, _browser: Option<&mut Browser>, origin_url: Option<&CefString>, is_proxy: i32, _host: Option<&CefString>, _port: i32, realm: Option<&CefString>, scheme: Option<&CefString>, callback: Option<&mut AuthCallback>) -> i32 {
            let Some(callback) = callback else { return 0 };
            if is_proxy == 1 || !matches!(string(scheme).to_ascii_lowercase().as_str(), "basic" | "digest") {
                return 0;
            }
            let mut task = AskCredentials::new(self.id, string(origin_url), string(realm), callback.clone());
            i32::from(post_task(ThreadId::UI, Some(&mut task)) == 1)
        }

        // CEF's IO thread: every request is filtered; a main-frame navigation of a site being
        // bridged also waits for its cookies.
        fn resource_request_handler(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, request: Option<&mut Request>, is_navigation: i32, _is_download: i32, _request_initiator: Option<&CefString>, _disable_default_handling: Option<&mut i32>) -> Option<ResourceRequestHandler> {
            let hold = match (frame, request) {
                (Some(frame), Some(request)) if is_navigation == 1 && frame.is_main() == 1 => super::cookies::hold_for(&CefString::from(&request.url()).to_string()),
                _ => None,
            };
            Some(super::filter::TarnFilter::new(hold, self.profile.clone(), self.lockdown_view))
        }

        fn on_open_urlfrom_tab(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, target_url: Option<&CefString>, target_disposition: WindowOpenDisposition, user_gesture: i32) -> i32 {
            if target_disposition == WindowOpenDisposition::CURRENT_TAB {
                return 0;
            }
            // A Shift-click asks for a new window: as in WebKit tabs, the link opens in a peek.
            let tab_disposition = [WindowOpenDisposition::NEW_FOREGROUND_TAB, WindowOpenDisposition::NEW_BACKGROUND_TAB];
            if user_gesture == 1 && (target_disposition == WindowOpenDisposition::NEW_WINDOW || tab_disposition.contains(&target_disposition)) {
                let background = (target_disposition != WindowOpenDisposition::NEW_WINDOW).then_some(target_disposition == WindowOpenDisposition::NEW_BACKGROUND_TAB);
                peek(self.id, &self.profile, target_url, page_url(browser.as_deref()), background, false);
            } else {
                offer_popup(self.id, frame, target_url, user_gesture);
            }
            1
        }

        fn on_certificate_error(&self, _browser: Option<&mut Browser>, _cert_error: Errorcode, _request_url: Option<&CefString>, _ssl_info: Option<&mut Sslinfo>, callback: Option<&mut Callback>) -> i32 {
            if let Some(callback) = callback {
                callback.cancel();
            }
            1
        }

        fn on_render_process_terminated(&self, browser: Option<&mut Browser>, _status: TerminationStatus, _error_code: i32, _error_string: Option<&CefString>) {
            if let Some(tab) = tab(self.id) {
                emit(&tab, Event::ContentProcessTerminated);
            }
        }
    }
}

#[cfg(windows)]
// The DevTools window of the tab `id` shows DevTools' own pages only and sends no request to the
// web; a link it opens is offered in the tab, as a page's pop-up is, and nothing downloads.
wrap_client! {
    struct InspectorClient {
        id: u64,
    }

    impl Client {
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(InspectorRequests::new(self.id))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(InspectorPopups::new(self.id))
        }

        fn download_handler(&self) -> Option<DownloadHandler> {
            Some(NoDownloads::new())
        }
    }
}

#[cfg(windows)]
fn inspector_page(url: &str) -> bool {
    url.starts_with("devtools://") || url == "about:blank"
}

#[cfg(windows)]
/// A web link DevTools opens, offered in the inspected tab; the user's click there opens it.
fn offer_from_inspector(id: u64, target_url: Option<&CefString>, user_gesture: i32) {
    let Some(tab) = tab(id) else { return };
    if let Ok(url) = Url::parse(&string(target_url))
        && user_gesture == 1
        && nav_policy::decide_chromium(&url, Source::Popup) == Decision::Allow
        && crate::page::within_burst(&tab.recent_popups, crate::page::POPUP_BURST)
    {
        emit(&tab, Event::PopupOffered(url));
    }
}

#[cfg(windows)]
wrap_request_handler! {
    struct InspectorRequests {
        id: u64,
    }

    impl RequestHandler {
        fn on_before_browse(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _user_gesture: i32, _is_redirect: i32) -> i32 {
            i32::from(!request.is_some_and(|r| inspector_page(&CefString::from(&r.url()).to_string())))
        }

        fn on_open_urlfrom_tab(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, target_url: Option<&CefString>, _target_disposition: WindowOpenDisposition, user_gesture: i32) -> i32 {
            offer_from_inspector(self.id, target_url, user_gesture);
            1
        }

        fn resource_request_handler(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>, _is_navigation: i32, _is_download: i32, _request_initiator: Option<&CefString>, _disable_default_handling: Option<&mut i32>) -> Option<ResourceRequestHandler> {
            Some(InspectorNoWeb::new())
        }
    }
}

#[cfg(windows)]
wrap_resource_request_handler! {
    struct InspectorNoWeb;

    impl ResourceRequestHandler {
        // CEF's IO thread.
        fn on_before_resource_load(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _callback: Option<&mut Callback>) -> ReturnValue {
            let url = request.map(|r| CefString::from(&r.url()).to_string()).unwrap_or_default();
            let scheme = url.split(':').next().unwrap_or_default().to_ascii_lowercase();
            if matches!(scheme.as_str(), "devtools" | "data" | "blob") { ReturnValue::CONTINUE } else { ReturnValue::CANCEL }
        }
    }
}

#[cfg(windows)]
wrap_life_span_handler! {
    struct InspectorPopups {
        id: u64,
    }

    impl LifeSpanHandler {
        fn on_before_popup(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _popup_id: i32, target_url: Option<&CefString>, _target_frame_name: Option<&CefString>, _target_disposition: WindowOpenDisposition, user_gesture: i32, _popup_features: Option<&PopupFeatures>, _window_info: Option<&mut WindowInfo>, _client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>, _extra_info: Option<&mut Option<DictionaryValue>>, _no_javascript_access: Option<&mut i32>) -> i32 {
            offer_from_inspector(self.id, target_url, user_gesture);
            1
        }

        // Counted with the tabs' browsers, so shutdown waits for a DevTools window too.
        fn on_after_created(&self, _browser: Option<&mut Browser>) {
            OPEN.set(OPEN.get() + 1);
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            OPEN.set(OPEN.get().saturating_sub(1));
        }
    }
}

#[cfg(windows)]
wrap_download_handler! {
    struct NoDownloads;

    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, _url: Option<&CefString>, _request_method: Option<&CefString>) -> i32 {
            0
        }

        // Returning 1 without running the callback cancels.
        fn on_before_download(&self, _browser: Option<&mut Browser>, _download_item: Option<&mut DownloadItem>, _suggested_name: Option<&CefString>, _callback: Option<&mut BeforeDownloadCallback>) -> i32 {
            1
        }
    }
}

wrap_life_span_handler! {
    struct TarnLifeSpan {
        id: u64,
    }

    impl LifeSpanHandler {
        #[allow(clippy::too_many_arguments)]
        // On Windows an allowed pop-up opens as a view of its own the shell adopts into a tab, so
        // it keeps `window.opener` (sign-in and payment windows). On macOS Chromium never opens the
        // window itself: an allowed pop-up is offered by a notice, and a tab opened by that click is
        // the user's load, not the page's.
        fn on_before_popup(&self, _browser: Option<&mut Browser>, frame: Option<&mut Frame>, _popup_id: i32, target_url: Option<&CefString>, _target_frame_name: Option<&CefString>, _target_disposition: WindowOpenDisposition, user_gesture: i32, _popup_features: Option<&PopupFeatures>, window_info: Option<&mut WindowInfo>, client: Option<&mut Option<Client>>, settings: Option<&mut BrowserSettings>, _extra_info: Option<&mut Option<DictionaryValue>>, _no_javascript_access: Option<&mut i32>) -> i32 {
            #[cfg(windows)]
            if let (Some(tab), Ok(url), Some(info), Some(client), Some(settings)) = (tab(self.id), Url::parse(&string(target_url)), window_info, client, settings)
                && !nav_policy::chromium_external(&url, Source::Popup)
            {
                let main_frame = frame.is_some_and(|f| f.is_main() == 1);
                match popup_verdict(&tab, main_frame, &url, user_gesture) {
                    // A Lockdown page must not load before its service workers are bypassed, which a
                    // pop-up Chromium makes itself cannot wait for: it is offered instead.
                    Some(true) if !tab.lockdown_view.get() => {
                        match WebView::for_popup(&tab, url.clone(), info, client, settings) {
                            Some(popup) => {
                                emit(&tab, Event::Popup(popup));
                                return 0;
                            }
                            None => emit(&tab, Event::PopupOffered(url)),
                        }
                    }
                    Some(true) => emit(&tab, Event::PopupOffered(url)),
                    Some(false) => emit(&tab, Event::PopupBlocked(Some(url))),
                    None => {}
                }
                return 1;
            }
            #[cfg(target_os = "macos")]
            let _ = (window_info, client, settings);
            offer_popup(self.id, frame, target_url, user_gesture);
            1
        }

        // Chrome's Inspect opens DevTools in a window of its own, which gets Tarn's client for it.
        #[cfg(windows)]
        fn on_before_dev_tools_popup(&self, _browser: Option<&mut Browser>, _window_info: Option<&mut WindowInfo>, client: Option<&mut Option<Client>>, _settings: Option<&mut BrowserSettings>, _extra_info: Option<&mut Option<DictionaryValue>>, _use_default_window: Option<&mut i32>) {
            if let Some(client) = client {
                *client = Some(InspectorClient::new(self.id));
            }
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            OPEN.set(OPEN.get() + 1);
            if let (Some(tab), Some(browser)) = (tab(self.id), browser) {
                tarn::page_created(&tab, browser);
                *tab.browser.borrow_mut() = Some(browser.clone());
                #[cfg(windows)]
                if let Some(hook) = tab.created_hook.take() {
                    let browser = browser.clone();
                    super::runtime::after(std::time::Duration::ZERO, move || hook(browser));
                }
            }
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            OPEN.set(OPEN.get().saturating_sub(1));
            if let Some(browser) = browser {
                tarn::page_closed(browser);
            }
            if let Some(tab) = TABS.with_borrow_mut(|t| t.remove(&self.id)) {
                tab.browser.take();
                tab.devtools.take();
                for (_, reply) in tab.calls.take() {
                    reply(None);
                }
            }
        }
    }
}

/// Camera and microphone requests become Tarn's own prompt, as in WebKit tabs, and Chromium keeps
/// no grant of its own; screen capture and every other permission Chrome would ask for is
/// dismissed. DISMISS, unlike DENY, writes no content setting; Chromium only counts dismissals per
/// origin.
fn media_kind(camera: bool, microphone: bool) -> Option<crate::PromptKind> {
    match (camera, microphone) {
        (true, true) => Some(crate::PromptKind::CameraAndMicrophone),
        (true, false) => Some(crate::PromptKind::Camera),
        (false, true) => Some(crate::PromptKind::Microphone),
        (false, false) => None,
    }
}

/// Asks the shell about a media request of the tab `id`; `answer` gets whether it was allowed.
fn ask_media(id: u64, browser: Option<&mut Browser>, origin: Option<&CefString>, kind: crate::PromptKind, answer: impl FnOnce(bool) + 'static) {
    let origin = Url::parse(&string(origin)).ok().as_ref().and_then(crate::page::serialized_origin);
    let allowed = tab(id).filter(|t| !t.silenced.get() && crate::page::within_burst(&t.recent_prompts, crate::page::PROMPT_BURST));
    let (Some(tab), Some(origin), Some(page)) = (allowed, origin, page_origin(browser)) else { return answer(false) };
    let top_origin = (page != origin).then_some(page);
    let reply: crate::prompts::Answer = Box::new(move |a| answer(a.is_some()));
    emit(&tab, Event::Prompt(crate::Prompt::answered(origin, top_origin, false, String::new(), kind, reply, tab.silenced.clone())));
}

wrap_permission_handler! {
    struct TarnPermission {
        id: u64,
    }

    impl PermissionHandler {
        fn on_request_media_access_permission(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, requesting_origin: Option<&CefString>, requested_permissions: u32, callback: Option<&mut MediaAccessCallback>) -> i32 {
            let Some(callback) = callback else { return 1 };
            let (audio, video) = (sys::cef_media_access_permission_types_t::CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE as u32, sys::cef_media_access_permission_types_t::CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE as u32);
            let kind = (requested_permissions & !(audio | video) == 0).then(|| media_kind(requested_permissions & video != 0, requested_permissions & audio != 0)).flatten();
            let Some(kind) = kind else {
                callback.cancel();
                return 1;
            };
            let callback = callback.clone();
            ask_media(self.id, browser, requesting_origin, kind, move |allowed| if allowed { callback.cont(requested_permissions) } else { callback.cancel() });
            1
        }

        fn on_show_permission_prompt(&self, _browser: Option<&mut Browser>, _prompt_id: u64, _requesting_origin: Option<&CefString>, _requested_permissions: u32, callback: Option<&mut PermissionPromptCallback>) -> i32 {
            if let Some(callback) = callback {
                callback.cont(PermissionRequestResult::DISMISS);
            }
            1
        }
    }
}

#[cfg(target_os = "macos")]
const DOWNLOAD_NOTICE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(target_os = "macos")]
fn refuse_download(id: u64) {
    if let Some(tab) = tab(id)
        && tab.download_notice.get().is_none_or(|at| at.elapsed() > DOWNLOAD_NOTICE_INTERVAL)
    {
        tab.download_notice.set(Some(std::time::Instant::now()));
        emit(&tab, Event::DownloadFailed { id: None, message: "Downloads in Chromium tabs are not supported yet.".into() });
    }
}

wrap_download_handler! {
    struct TarnDownload {
        id: u64,
        store: String,
    }

    impl DownloadHandler {
        fn can_download(&self, _browser: Option<&mut Browser>, _url: Option<&CefString>, _request_method: Option<&CefString>) -> i32 {
            #[cfg(windows)]
            return 1;
            #[cfg(target_os = "macos")]
            {
                refuse_download(self.id);
                0
            }
        }

        // Returning 1 without running the callback cancels; 0 would hand the download to Chrome.
        fn on_before_download(&self, browser: Option<&mut Browser>, download_item: Option<&mut DownloadItem>, suggested_name: Option<&CefString>, callback: Option<&mut BeforeDownloadCallback>) -> i32 {
            #[cfg(windows)]
            if let (Some(tab), Some(item), Some(callback)) = (tab(self.id), download_item, callback) {
                let sink = tab.downloads.get().cloned().unwrap_or_else(|| tab.sink.borrow().clone());
                if !crate::page::within_burst(&tab.recent_downloads, crate::page::DOWNLOAD_BURST) {
                    sink(Event::DownloadFailed { id: None, message: "Too many downloads from this page; blocked.".into() });
                    return 1;
                }
                let page = browser.and_then(|b| b.main_frame()).and_then(|f| Url::parse(&CefString::from(&f.url()).to_string()).ok());
                let asker = super::downloads_windows::Asker { sink: tab.sink.borrow().clone(), silenced: tab.silenced.clone(), profile: tab.profile.clone(), page: with_opener(&tab, page) };
                super::downloads_windows::begin(sink, &self.store, item, &string(suggested_name), callback, asker);
            }
            #[cfg(target_os = "macos")]
            {
                let _ = (browser, download_item, suggested_name, callback, &self.store);
                refuse_download(self.id);
            }
            1
        }

        fn on_download_updated(&self, _browser: Option<&mut Browser>, download_item: Option<&mut DownloadItem>, callback: Option<&mut DownloadItemCallback>) {
            #[cfg(windows)]
            {
                if let Some(item) = download_item {
                    super::downloads_windows::update(&self.store, item, callback);
                }
            }
            #[cfg(target_os = "macos")]
            {
                let _ = download_item;
                refuse_download(self.id);
                if let Some(callback) = callback {
                    callback.cancel();
                }
            }
        }
    }
}

/// Chrome commands that act on the page only and draw no Chrome UI; every other one is refused.
const PAGE_COMMANDS: [i32; 8] =
    [sys::IDC_BACK, sys::IDC_FORWARD, sys::IDC_RELOAD, sys::IDC_RELOAD_BYPASSING_CACHE, sys::IDC_STOP, sys::IDC_CUT, sys::IDC_COPY, sys::IDC_PASTE];

wrap_task! {
    struct AskCredentials {
        id: u64,
        origin: String,
        realm: String,
        callback: AuthCallback,
    }

    impl Task {
        fn execute(&self) {
            ask_credentials(self.id, &self.origin, &self.realm, &self.callback);
        }
    }
}

/// Only the page's own origin, or the one its main frame is navigating to, may ask for
/// credentials, not embedded third parties or other ports of the same host.
fn ask_credentials(id: u64, origin_url: &str, realm: &str, callback: &AuthCallback) {
    let challenge = Url::parse(origin_url).ok();
    let origin = challenge.as_ref().and_then(crate::page::serialized_origin);
    let (Some(tab), Some(challenge), Some(origin)) = (tab(id), challenge, origin) else { return callback.cancel() };
    let (page, navigating) = (page_url(tab.browser.borrow().as_ref()), tab.navigating.borrow().as_deref().and_then(|o| Url::parse(o).ok()));
    let failures = tab.auth_attempts.get();
    let own = [page, navigating].iter().any(|p| crate::view_policy::may_ask_credentials(&challenge, p.as_ref(), failures));
    if !own || tab.silenced.get() || !crate::page::within_burst(&tab.recent_prompts, crate::page::PROMPT_BURST) {
        return callback.cancel();
    }
    tab.auth_attempts.set(tab.auth_attempts.get() + 1);
    let callback = callback.clone();
    let reply = Box::new(move |answer: Option<(String, Option<(String, String)>)>| match answer {
        Some((_, Some((user, password)))) => callback.cont(Some(&CefString::from(user.as_str())), Some(&CefString::from(password.as_str()))),
        _ => callback.cancel(),
    });
    let insecure = !origin.starts_with("https:");
    let kind = crate::PromptKind::Credentials { realm: crate::page::bounded(realm) };
    emit(&tab, Event::Prompt(crate::Prompt::answered(origin, None, insecure, String::new(), kind, reply, tab.silenced.clone())));
}

/// Drops a step into the restored list that did not navigate, and the marks it set for its load.
#[cfg(windows)]
fn abandon_step(tab: &Tab) {
    if tab.restored_started.get() {
        return;
    }
    if let Some((_, target, _)) = tab.restored_step.take() {
        tab.shell_load.borrow_mut().take_if(|u| *u == target);
    }
}

/// Whether Chromium's own list holds a page to step back to; the blank page a view starts on is none.
#[cfg(windows)]
fn native_back(browser: &Browser) -> bool {
    browser.can_go_back() == 1 && history_urls(browser, |i, current| i < current).iter().any(|u| u.as_str() != "about:blank")
}

/// Whether the tab can step back and forward: in Chromium's own list or, on Windows, a restored one.
fn history_flags(tab: &Tab) -> (bool, bool) {
    let (back, forward) = tab.history.get();
    #[cfg(windows)]
    let (back, forward) = {
        let restored = tab.restored.borrow();
        (back || !restored.back.is_empty(), forward || !restored.forward.is_empty())
    };
    (back, forward)
}

fn page_url(browser: Option<&Browser>) -> Option<Url> {
    Url::parse(&CefString::from(&browser?.main_frame()?.url()).to_string()).ok()
}

/// A peek, or a tab opened by a click (`background` set), is a window the page could open with a
/// synthetic click: it gets the pop-up rules, their rate limit and the S10 check.
fn peek(id: u64, profile: &str, target_url: Option<&CefString>, page: Option<Url>, background: Option<bool>, split: bool) {
    let (Some(tab), Ok(url)) = (tab(id), Url::parse(&string(target_url))) else { return };
    #[cfg(windows)]
    let page = with_opener(&tab, page);
    if nav_policy::decide_chromium(&url, Source::Popup) != Decision::Allow {
        return offer_popup(id, None, target_url, 1);
    }
    let Some(shield) = super::filter::shield_for(profile) else { return };
    let opener = (shield.site_for)(page.as_ref().unwrap_or(&Url::parse("about:blank").expect("about:blank parses")));
    let local = nav_policy::into_private_network(&url, page.as_ref(), false, opener.level.policy().block_private_network);
    if crate::page::within_burst(&tab.recent_popups, crate::page::POPUP_BURST) && nav_policy::popup_allowed(opener, Some((shield.site_for)(&url))) && !local {
        emit(
            &tab,
            match background {
                Some(background) => Event::OpenTab { url, from: page, background, split },
                None => Event::PeekLink(url, page),
            },
        );
    } else {
        if local {
            emit(&tab, Event::Protected(Protection::LocalNetwork, None));
        }
        emit(&tab, Event::PopupBlocked(Some(url)));
    }
}

/// A clicked pop-up the site's rule and the block lists allow is offered; one they refuse is
/// reported as blocked, as in WebKit tabs. Without a click nothing opens, in both engines.
fn offer_popup(id: u64, frame: Option<&mut Frame>, target_url: Option<&CefString>, user_gesture: i32) {
    let Some(tab) = tab(id) else { return };
    let Ok(url) = Url::parse(&string(target_url)) else { return };
    if user_gesture == 1 && nav_policy::chromium_external(&url, Source::Popup) {
        if crate::page::within_burst(&tab.recent_prompts, crate::page::PROMPT_BURST) {
            let origin = page_url(tab.browser.borrow().as_ref()).as_ref().and_then(crate::page::serialized_origin);
            emit(&tab, Event::ExternalScheme { url, origin });
        }
        return;
    }
    match popup_verdict(&tab, frame.is_some_and(|f| f.is_main() == 1), &url, user_gesture) {
        Some(true) => emit(&tab, Event::PopupOffered(url)),
        Some(false) => emit(&tab, Event::PopupBlocked(Some(url))),
        None => {}
    }
}

/// Whether a pop-up to `url` passes the site's rule and the block lists; `None` when nothing
/// opens at all: no click, not a web page, or too many lately.
fn popup_verdict(tab: &Tab, main_frame: bool, url: &Url, user_gesture: i32) -> Option<bool> {
    if user_gesture != 1
        || nav_policy::decide_chromium(url, Source::Popup) != Decision::Allow
        || !crate::page::within_burst(&tab.recent_popups, crate::page::POPUP_BURST)
    {
        return None;
    }
    let page = page_url(tab.browser.borrow().as_ref());
    Some(super::filter::shield_for(&tab.profile).is_some_and(|shield| {
        let mut opener = page.as_ref().map_or(nav_policy::Site::new(shield::ShieldLevel::Standard), |p| (shield.site_for)(p));
        // A site's "Allow" covers its own pages, not the ad frames it embeds.
        if !main_frame && opener.popups == nav_policy::PopupRule::Allow {
            opener.popups = nav_policy::PopupRule::Default;
        }
        nav_policy::popup_allowed(opener, Some((shield.site_for)(url)))
    }))
}

wrap_navigation_entry_visitor! {
    struct HistoryUrls {
        urls: Rc<RefCell<Vec<Option<Url>>>>,
        current: Rc<Cell<Option<usize>>>,
    }

    impl NavigationEntryVisitor {
        fn visit(&self, entry: Option<&mut NavigationEntry>, current: i32, _index: i32, _total: i32) -> i32 {
            if current == 1 {
                self.current.set(Some(self.urls.borrow().len()));
            }
            self.urls.borrow_mut().push(entry.and_then(|e| Url::parse(&CefString::from(&e.url()).to_string()).ok()));
            1
        }
    }
}

/// The URLs of the history entries whose index `side` picks against the current one: where the
/// shell's step may go, since Chromium's back skips entries a page added without a gesture. A step
/// to any other URL, a same-document one included, is not the user's.
fn history_urls(browser: &Browser, side: impl Fn(usize, usize) -> bool) -> Vec<Url> {
    let (urls, current) = (Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(None)));
    if let Some(host) = browser.host() {
        host.navigation_entries(Some(&mut HistoryUrls::new(urls.clone(), current.clone())), 0);
    }
    let Some(current) = current.get() else { return Vec::new() };
    urls.take().into_iter().enumerate().filter(|(i, _)| side(*i, current)).filter_map(|(_, url)| url).collect()
}

fn page_origin(browser: Option<&mut Browser>) -> Option<String> {
    let url = CefString::from(&browser?.main_frame()?.url()).to_string();
    Url::parse(&url).ok().as_ref().and_then(crate::page::serialized_origin)
}

wrap_jsdialog_handler! {
    struct TarnJsDialog {
        id: u64,
    }

    impl JsdialogHandler {
        fn on_jsdialog(&self, browser: Option<&mut Browser>, origin_url: Option<&CefString>, dialog_type: JsdialogType, message_text: Option<&CefString>, default_prompt_text: Option<&CefString>, callback: Option<&mut JsdialogCallback>, suppress_message: Option<&mut i32>) -> i32 {
            let origin = Url::parse(&string(origin_url)).ok().as_ref().and_then(crate::page::serialized_origin);
            let Some(callback) = callback else {
                if let Some(suppress) = suppress_message {
                    *suppress = 1;
                }
                return 0;
            };
            let allowed = tab(self.id).filter(|t| !t.silenced.get() && crate::page::within_burst(&t.recent_prompts, crate::page::PROMPT_BURST));
            let (Some(tab), Some(origin), Some(page)) = (allowed, origin, page_origin(browser)) else {
                callback.cont(0, None);
                return 1;
            };
            let kind = if dialog_type == JsdialogType::CONFIRM {
                crate::PromptKind::Confirm
            } else if dialog_type == JsdialogType::PROMPT {
                crate::PromptKind::Text { default: crate::page::bounded(&string(default_prompt_text)) }
            } else {
                crate::PromptKind::Alert
            };
            let callback = callback.clone();
            let reply = Box::new(move |answer: Option<(String, Option<(String, String)>)>| match answer {
                Some((text, _)) => callback.cont(1, Some(&CefString::from(text.as_str()))),
                None => callback.cont(0, None),
            });
            let top_origin = (page != origin).then_some(page);
            let message = crate::page::bounded(&string(message_text));
            emit(&tab, Event::Prompt(crate::Prompt::answered(origin, top_origin, false, message, kind, reply, tab.silenced.clone())));
            1
        }

        // Leaving a page is never held up by it, as in WebKit tabs.
        fn on_before_unload_dialog(&self, _browser: Option<&mut Browser>, _message_text: Option<&CefString>, _is_reload: i32, callback: Option<&mut JsdialogCallback>) -> i32 {
            if let Some(callback) = callback {
                callback.cont(1, None);
            }
            1
        }
    }
}

wrap_command_handler! {
    struct TarnCommand;

    impl CommandHandler {
        fn on_chrome_command(&self, _browser: Option<&mut Browser>, command_id: i32, _disposition: WindowOpenDisposition) -> i32 {
            if PAGE_COMMANDS.contains(&command_id) {
                return 0;
            }
            // Chrome runs the shortcuts it reserves (close tab, new tab, ...) before any key handler,
            // so a refused command from a key press goes to the shell as that key press.
            #[cfg(target_os = "macos")]
            if let Some(event) = MainThreadMarker::new()
                .filter(|_| super::runtime::in_send_event())
                .and_then(|mtm| NSApplication::sharedApplication(mtm).currentEvent())
                .filter(|e| e.r#type() == NSEventType::KeyDown)
            {
                to_shell(&event);
            }
            1
        }
    }
}

/// Context-menu items kept from Chrome's menu: editing, spelling, copying, the page's own history
/// and media controls, and opening or saving a link, image or media, whose tabs and downloads take
/// Tarn's own paths; nothing that opens Chrome UI or a window.
const KEPT_ITEMS: [i32; 28] = [
    sys::IDC_CONTENT_CONTEXT_OPENLINKNEWTAB,
    sys::IDC_CONTENT_CONTEXT_OPENIMAGENEWTAB,
    sys::IDC_CONTENT_CONTEXT_SAVELINKAS,
    sys::IDC_CONTENT_CONTEXT_SAVEIMAGEAS,
    sys::IDC_CONTENT_CONTEXT_SAVEAVAS,
    sys::IDC_BACK,
    sys::IDC_FORWARD,
    sys::IDC_RELOAD,
    sys::IDC_CONTENT_CONTEXT_UNDO,
    sys::IDC_CONTENT_CONTEXT_REDO,
    sys::IDC_CONTENT_CONTEXT_CUT,
    sys::IDC_CONTENT_CONTEXT_COPY,
    sys::IDC_CONTENT_CONTEXT_PASTE,
    sys::IDC_CONTENT_CONTEXT_PASTE_AND_MATCH_STYLE,
    sys::IDC_CONTENT_CONTEXT_DELETE,
    sys::IDC_CONTENT_CONTEXT_SELECTALL,
    sys::IDC_CONTENT_CONTEXT_COPYLINKLOCATION,
    sys::IDC_CONTENT_CONTEXT_COPYLINKTEXT,
    sys::IDC_CONTENT_CONTEXT_COPYIMAGE,
    sys::IDC_CONTENT_CONTEXT_COPYIMAGELOCATION,
    sys::IDC_CONTENT_CONTEXT_COPYVIDEOFRAME,
    sys::IDC_CONTENT_CONTEXT_COPYAVLOCATION,
    sys::IDC_CONTENT_CONTEXT_LOOP,
    sys::IDC_CONTENT_CONTEXT_CONTROLS,
    sys::IDC_CONTENT_CONTEXT_LOOK_UP,
    sys::IDC_CONTENT_CONTEXT_EMOJI,
    sys::IDC_CONTENT_CONTEXT_SPELLING_SUGGESTION,
    sys::IDC_SPELLCHECK_ADD_TO_DICTIONARY,
];

/// Whether the context-menu command `id` stays; Inspect only where Web Inspector is on.
fn kept_item(id: i32, inspectable: bool) -> bool {
    KEPT_ITEMS.contains(&id)
        || (sys::IDC_SPELLCHECK_SUGGESTION_0..sys::IDC_SPELLCHECK_LANGUAGES_FIRST).contains(&id)
        || (inspectable && id == sys::IDC_CONTENT_CONTEXT_INSPECTELEMENT)
}

/// The context-menu command Tarn adds after Open Link in New Tab.
const OPEN_LINK_IN_SPLIT_VIEW: i32 = sys::cef_menu_id_t::MENU_ID_USER_FIRST as i32;

wrap_context_menu_handler! {
    struct TarnContextMenu {
        id: u64,
        profile: String,
    }

    impl ContextMenuHandler {
        fn on_before_context_menu(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _params: Option<&mut ContextMenuParams>, model: Option<&mut MenuModel>) {
            let Some(model) = model else { return };
            let inspectable = tab(self.id).is_some_and(|t| t.inspectable.get());
            // A separator stays only between two kept items.
            let mut after_item = false;
            for index in (0..model.count()).rev() {
                let keep = if model.type_at(index) == MenuItemType::SEPARATOR { std::mem::replace(&mut after_item, false) } else { kept_item(model.command_id_at(index), inspectable) };
                if keep {
                    after_item |= model.type_at(index) != MenuItemType::SEPARATOR;
                } else {
                    model.remove_at(index);
                }
            }
            if model.count() > 0 && model.type_at(0) == MenuItemType::SEPARATOR {
                model.remove_at(0);
            }
            if let Ok(at) = usize::try_from(model.index_of(sys::IDC_CONTENT_CONTEXT_OPENLINKNEWTAB)) {
                model.insert_item_at(at + 1, OPEN_LINK_IN_SPLIT_VIEW, Some(&CefString::from("Open Link in Split View")));
            }
        }

        fn on_context_menu_command(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, params: Option<&mut ContextMenuParams>, command_id: i32, _event_flags: EventFlags) -> i32 {
            if command_id != OPEN_LINK_IN_SPLIT_VIEW {
                return 0;
            }
            if let Some(params) = params {
                let link = CefString::from(&params.link_url());
                peek(self.id, &self.profile, Some(&link), page_url(browser.as_deref()), Some(false), true);
            }
            1
        }
    }
}

wrap_find_handler! {
    struct TarnFind {
        id: u64,
    }

    impl FindHandler {
        fn on_find_result(&self, browser: Option<&mut Browser>, _identifier: i32, count: i32, _selection_rect: Option<&Rect>, active_match_ordinal: i32, final_update: i32) {
            let Some(tab) = tab(self.id) else { return };
            if final_update == 1 && let Some(found) = tab.found.borrow().as_ref() {
                let count = usize::try_from(count).unwrap_or(0).min(crate::page_scripts::FIND_COUNT_MAX);
                found(count, usize::try_from(active_match_ordinal).ok().filter(|at| (1..=count).contains(at)));
            }
        }
    }
}

wrap_keyboard_handler! {
    struct TarnKeys;

    impl KeyboardHandler {
        // Key presses with the shortcut modifiers are the shell's shortcuts too. Chromium would take
        // some for its own commands, which Tarn refuses, so each goes to the shell window that holds
        // the tab before the page sees it. Editing keys stay the page's alone.
        #[cfg(windows)]
        fn on_pre_key_event(&self, _browser: Option<&mut Browser>, event: Option<&KeyEvent>, os_event: Option<&mut sys::MSG>, _is_keyboard_shortcut: Option<&mut i32>) -> i32 {
            let (Some(event), Some(msg)) = (event, os_event) else { return 0 };
            if event.type_ == KeyEventType::RAWKEYDOWN && forwarded(event.modifiers, event.windows_key_code) {
                // SAFETY: `msg.hwnd` is Chromium's window inside the shell window, on this thread; the key
                // state the shell reads is still this key press's.
                unsafe {
                    let root = GetAncestor(msg.hwnd.0.cast(), GA_ROOT);
                    SendMessageW(root, msg.message, msg.wParam as usize, msg.lParam as isize);
                }
            }
            0
        }

        // The page sees a key press only when the shell has no shortcut for it.
        #[cfg(target_os = "macos")]
        fn on_pre_key_event(&self, _browser: Option<&mut Browser>, event: Option<&KeyEvent>, os_event: *mut u8, _is_keyboard_shortcut: Option<&mut i32>) -> i32 {
            if !event.is_some_and(|e| e.type_ == KeyEventType::RAWKEYDOWN && forwarded(e.modifiers, e.windows_key_code)) {
                return 0;
            }
            // SAFETY: on macOS CEF passes the NSEvent of this key press, or null.
            let Some(native) = (unsafe { os_event.cast::<NSEvent>().as_ref() }) else { return 0 };
            i32::from(to_shell(native))
        }
    }
}

/// Hands a key press in a Chromium window to the shell window it is a child of, where WebKit tabs'
/// key presses arrive by themselves; true when the shell had a shortcut for it.
#[cfg(target_os = "macos")]
fn to_shell(event: &NSEvent) -> bool {
    thread_local! {
        static LAST: Cell<(f64, bool)> = const { Cell::new((-1., false)) };
    }
    let Some(mtm) = MainThreadMarker::new() else { return false };
    // Chrome's reserved commands and the key handler may both pass on one key press; it acts once.
    let (last, handled) = LAST.get();
    if event.timestamp() == last {
        return handled;
    }
    let handled = event.window(mtm).and_then(|w| w.parentWindow()).is_some_and(|shell| shell.performKeyEquivalent(event));
    LAST.set((event.timestamp(), handled));
    handled
}

/// Whether a key press with these CEF modifiers and Windows virtual-key code also goes to the shell:
/// on Windows Ctrl or Alt with a key, but not AltGr (Ctrl and Alt together types characters); on
/// macOS ⌘ or ⌃ with a key. Never a modifier alone, nor the editing and caret keys pages use with
/// the shortcut modifier.
#[cfg(any(windows, target_os = "macos"))]
fn forwarded(modifiers: u32, key: i32) -> bool {
    use sys::cef_event_flags_t as F;
    let has = |flag: F| u64::from(modifiers) & flag.0 as u64 != 0;
    // F6 moves the keyboard out of the page.
    if key == 0x75 {
        return true;
    }
    // Backspace, Enter, Shift, Ctrl, Alt, the caret keys and Delete, the Windows keys.
    const PAGE_KEYS: [i32; 16] = [0x08, 0x0d, 0x10, 0x11, 0x12, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x2e, 0x5b, 0x5c];
    // Select all, copy, paste, cut, undo, redo (Ctrl+Y on Windows); with Shift only paste as text and redo.
    let letters: &[u8] = if has(F::EVENTFLAG_SHIFT_DOWN) {
        b"VZ"
    } else if cfg!(windows) {
        b"ACVXYZ"
    } else {
        b"ACVXZ"
    };
    let editing = u8::try_from(key).is_ok_and(|k| letters.contains(&k)) || PAGE_KEYS.contains(&key);
    #[cfg(windows)]
    return match (has(F::EVENTFLAG_CONTROL_DOWN), has(F::EVENTFLAG_ALT_DOWN)) {
        (true, true) | (false, false) => false,
        (true, false) => !editing,
        (false, true) => key != 0x12,
    };
    #[cfg(target_os = "macos")]
    if has(F::EVENTFLAG_COMMAND_DOWN) { !editing } else { has(F::EVENTFLAG_CONTROL_DOWN) && !PAGE_KEYS.contains(&key) }
}

#[cfg(target_os = "macos")]
wrap_browser_view_delegate! {
    struct TarnBrowserViewDelegate {}

    impl ViewDelegate {}

    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::CHROME
        }

        fn chrome_toolbar_type(&self, _browser_view: Option<&mut BrowserView>) -> ChromeToolbarType {
            ChromeToolbarType::NONE
        }
    }
}

#[cfg(target_os = "macos")]
wrap_window_delegate! {
    struct TarnWindowDelegate {
        browser_view: BrowserView,
    }

    impl ViewDelegate {}

    impl PanelDelegate {}

    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut cef::Window>) {
            let Some(window) = window else { return };
            let mut view = View::from(&self.browser_view);
            window.add_child_view(Some(&mut view));
            WINDOWS.with_borrow_mut(|w| w.push(window.clone()));
        }

        fn on_window_destroyed(&self, window: Option<&mut cef::Window>) {
            let Some(window) = window else { return };
            WINDOWS.with_borrow_mut(|w| w.retain(|other| other.is_same(Some(&mut View::from(&*window))) != 1));
        }

        fn is_frameless(&self, _window: Option<&mut cef::Window>) -> i32 {
            1
        }

        fn can_close(&self, _window: Option<&mut cef::Window>) -> i32 {
            1
        }

        fn can_resize(&self, _window: Option<&mut cef::Window>) -> i32 {
            1
        }

        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::CHROME
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

/// A Chromium tab: a frameless Chrome-style CEF window kept as a child window over the shell
/// window's content rectangle. CEF creates it once the profile's request context is ready;
/// until then the view keeps its frame, visibility and first URL.
pub struct WebView(Rc<Inner>);

#[cfg(target_os = "macos")]
type Created = (BrowserView, cef::Window);
/// A windowed browser in a child window of the shell window.
#[cfg(windows)]
type Created = Browser;

struct Inner {
    created: RefCell<Option<Created>>,
    host: RefCell<Host>,
    tab: Rc<Tab>,
    frame: Cell<Option<(f64, f64, f64, f64)>>,
    /// The page's corner radius in points, and the window, width, height and corner diameter in
    /// pixels of the region last set on it.
    #[cfg(windows)]
    radius: Cell<f64>,
    #[cfg(windows)]
    clip: Cell<Option<(isize, i32, i32, i32)>>,
    visible: Cell<bool>,
    muted: Cell<bool>,
    /// The colour scheme asked for, applied once the browser exists.
    dark: Cell<Option<bool>>,
    first_load: RefCell<Option<Url>>,
    /// The waiting first load is a page's link (`load_link`), not the user's.
    first_link: Cell<bool>,
    handover: RefCell<Option<String>>,
    /// The profile's site style sheets when the view was created (`DataStore::set_site_styles`).
    site_styles: Option<String>,
    gated: Cell<bool>,
    /// CEF could not start; loads fail at once.
    dead: Cell<bool>,
}

impl WebView {
    /// `root` holds Chromium's data; `profile` is the Tarn profile's own UUID. `None` when
    /// Chromium cannot run here or belongs to another profile.
    #[cfg(target_os = "macos")]
    pub fn new(host: &Host, data: &crate::DataStore, root: &std::path::Path, profile: &str, inspectable: bool, sink: Rc<dyn Fn(Event)>) -> Option<Self> {
        if super::runtime::retired() || !crate::chromium_owner::claim(root, profile) || !super::runtime::ensure_started(root) {
            return None;
        }
        super::cookies::attach(data);
        let inner = Self::inner(host, String::new(), String::new(), data.site_styles(), sink);
        inner.tab.inspectable.set(inspectable);
        let weak = Rc::downgrade(&inner);
        super::runtime::with_context(move |mut context| {
            if let Some(inner) = weak.upgrade() {
                WebView(inner).create(&mut context);
            }
        });
        Some(WebView(inner))
    }

    /// A view for the pop-up CEF is about to make for the tab `opener`: fills in the window and
    /// client CEF makes it with. Its events wait in the `Popup` until the shell adopts it.
    #[cfg(windows)]
    fn for_popup(opener: &Tab, url: Url, info: &mut WindowInfo, client: &mut Option<Client>, settings: &mut BrowserSettings) -> Option<crate::Popup> {
        let child = opener.browser.borrow().as_ref()?.host()?.window_handle();
        // SAFETY: a plain query of a window of this thread.
        let root = unsafe { GetAncestor(child.0.cast(), GA_ROOT) };
        if root.is_null() {
            return None;
        }
        let pending: Rc<RefCell<Vec<Event>>> = Rc::default();
        let held = pending.clone();
        let mut inner = Self::inner(&Host { hwnd: root }, opener.profile.clone(), opener.store.clone(), None, Rc::new(move |e| held.borrow_mut().push(e)));
        if let Some(tab) = Rc::get_mut(&mut inner).and_then(|i| Rc::get_mut(&mut i.tab)) {
            tab.recent_popups = opener.recent_popups.clone();
        }
        let tab = &inner.tab;
        tab.lockdown_view.set(opener.lockdown_view.get());
        tab.filtered.set(opener.filtered.get());
        tab.inspectable.set(opener.inspectable.get());
        if let Some(downloads) = opener.downloads.get() {
            let _ = tab.downloads.set(downloads.clone());
        }
        // The pop-up starts blank; S10 and the download checks judge it by the page that opened it.
        *tab.opener.borrow_mut() = with_opener(opener, page_url(opener.browser.borrow().as_ref()));
        TABS.with_borrow_mut(|t| t.insert(tab.id, tab.clone()));
        let weak = Rc::downgrade(&inner);
        *tab.created_hook.borrow_mut() = Some(Box::new(move |browser| match weak.upgrade() {
            Some(inner) => {
                *inner.created.borrow_mut() = Some(browser);
                WebView(inner).placed();
            }
            None => {
                if let Some(host) = browser.host() {
                    host.close_browser(1);
                }
            }
        }));
        let placed = WindowInfo { runtime_style: RuntimeStyle::CHROME, ..Default::default() }.set_as_child(sys::HWND(root.cast()), &Rect::default());
        *info = WindowInfo { style: placed.style & !windows_sys::Win32::UI::WindowsAndMessaging::WS_VISIBLE, ..placed };
        *client = Some(TarnClient::new(tab.id, tab.profile.clone(), tab.store.clone(), tab.lockdown_view.get()));
        // As a view of `create`: no JavaScript in a Lockdown view.
        *settings = BrowserSettings { javascript: if tab.lockdown_view.get() { State::DISABLED } else { State::DEFAULT }, ..settings.clone() };
        Some(crate::Popup { url: Some(url), link: false, view: WebView(inner), pending })
    }

    /// Whether the view applies Shield's rules, as it was made.
    #[cfg(windows)]
    pub(crate) fn filtered(&self) -> bool {
        self.0.tab.filtered.get()
    }

    #[cfg(windows)]
    pub(crate) fn set_inspectable(&self, on: bool) {
        self.0.tab.inspectable.set(on);
    }

    /// A view of the data store `key`, whose Chromium profile lives in `cache` or, without one,
    /// in memory. A view made while CEF cannot run stays blank and reports its first load as
    /// failed.
    #[cfg(windows)]
    pub(crate) fn for_store(host: &Host, data: &crate::DataStore, lockdown: bool, filtered: bool, inspectable: bool, sink: Rc<dyn Fn(Event)>) -> Self {
        let (key, shield, cache) = (data.context_key().to_owned(), data.shield_key().to_owned(), data.cache_dir());
        let inner = Self::inner(host, shield.clone(), key.clone(), None, sink);
        inner.tab.inspectable.set(inspectable);
        inner.tab.lockdown_view.set(lockdown);
        inner.tab.filtered.set(filtered);
        let _ = inner.tab.downloads.set(data.downloads());
        let started = super::runtime::root().is_some_and(|root| super::runtime::ensure_started(&root));
        if started {
            let weak = Rc::downgrade(&inner);
            super::runtime::with_store_context(key, shield, cache, move |mut context| {
                if let Some(inner) = weak.upgrade() {
                    WebView(inner).create(&mut context);
                }
            });
        }
        inner.dead.set(!started);
        WebView(inner)
    }

    fn inner(host: &Host, profile: String, store: String, site_styles: Option<String>, sink: Rc<dyn Fn(Event)>) -> Rc<Inner> {
        let id = NEXT_ID.get();
        NEXT_ID.set(id + 1);
        let tab = Rc::new(Tab {
            id,
            profile,
            store,
            browser: RefCell::new(None),
            calls: RefCell::default(),
            next_call: Cell::new(0),
            devtools: RefCell::new(None),
            handover_script: RefCell::new(None),
            sink: RefCell::new(sink),
            title: RefCell::new(String::new()),
            history: Cell::new((false, false)),
            loading: Cell::new(false),
            found: RefCell::new(None),
            shell_load: RefCell::new(None),
            opened_files: RefCell::default(),
            shell_chain: Cell::new(false),
            last_find: RefCell::new(String::new()),
            inspectable: Cell::new(false),
            #[cfg(target_os = "macos")]
            download_notice: Cell::new(None),
            recent_popups: Rc::default(),
            #[cfg(windows)]
            recent_downloads: RefCell::default(),
            recent_prompts: RefCell::default(),
            silenced: Rc::default(),
            history_step: RefCell::default(),
            user_started: Cell::new(false),
            navigating: RefCell::new(None),
            auth_attempts: Cell::new(0),
            link_load: Cell::new(false),
            #[cfg(windows)]
            downloads: std::cell::OnceCell::new(),
            #[cfg(windows)]
            lockdown_view: Cell::new(false),
            #[cfg(windows)]
            styled: Cell::new(0),
            #[cfg(windows)]
            filtered: Cell::new(false),
            #[cfg(windows)]
            page_hash: Cell::new(0),
            #[cfg(windows)]
            page_script: RefCell::new(None),
            #[cfg(windows)]
            opener: RefCell::new(None),
            #[cfg(windows)]
            created_hook: RefCell::new(None),
            renderer: Cell::new(None),
            #[cfg(windows)]
            restored: RefCell::default(),
            #[cfg(windows)]
            restored_bytes: RefCell::new(None),
            #[cfg(windows)]
            restored_step: RefCell::new(None),
            #[cfg(windows)]
            restored_started: Cell::new(false),
            #[cfg(windows)]
            restoring: Cell::new(false),
            tarn: RefCell::default(),
        });
        Rc::new(Inner {
            created: RefCell::new(None),
            host: RefCell::new(host.clone()),
            tab,
            frame: Cell::new(None),
            #[cfg(windows)]
            radius: Cell::new(0.),
            #[cfg(windows)]
            clip: Cell::new(None),
            visible: Cell::new(false),
            muted: Cell::new(false),
            dark: Cell::new(None),
            first_load: RefCell::new(None),
            first_link: Cell::new(false),
            handover: RefCell::new(None),
            site_styles,
            gated: Cell::new(false),
            dead: Cell::new(false),
        })
    }

    fn create(&self, context: &mut RequestContext) {
        if !super::runtime::running() {
            return;
        }
        let id = self.0.tab.id;
        TABS.with_borrow_mut(|t| t.insert(id, self.0.tab.clone()));
        if let Some(spec) = self.0.tab.tarn.borrow().page.as_ref() {
            tarn::register_scheme(context, spec);
        }
        #[cfg(windows)]
        let lockdown_view = self.0.tab.lockdown_view.get();
        #[cfg(target_os = "macos")]
        let lockdown_view = false;
        let mut client = TarnClient::new(id, self.0.tab.profile.clone(), self.0.tab.store.clone(), lockdown_view);
        let start = CefString::from("about:blank");
        #[cfg(target_os = "macos")]
        let created = {
            let mut delegate = TarnBrowserViewDelegate::new();
            browser_view_create(Some(&mut client), Some(&start), Some(&BrowserSettings::default()), None, Some(context), Some(&mut delegate)).and_then(
                |browser_view| {
                    let mut window_delegate = TarnWindowDelegate::new(browser_view.clone());
                    window_create_top_level(Some(&mut window_delegate)).map(|window| (browser_view, window))
                },
            )
        };
        // Created hidden at zero size; `placed` sizes and shows it once CEF made it, which for a
        // new profile waits until its request context is initialised.
        #[cfg(windows)]
        {
            let parent = sys::HWND(self.0.host.borrow().hwnd.cast());
            let info = WindowInfo { runtime_style: RuntimeStyle::CHROME, ..Default::default() }.set_as_child(parent, &Rect::default());
            let info = WindowInfo { style: info.style & !windows_sys::Win32::UI::WindowsAndMessaging::WS_VISIBLE, ..info };
            let weak = Rc::downgrade(&self.0);
            *self.0.tab.created_hook.borrow_mut() = Some(Box::new(move |browser| {
                match weak.upgrade() {
                    Some(inner) => {
                        *inner.created.borrow_mut() = Some(browser);
                        WebView(inner).placed();
                    }
                    // The view was dropped while CEF was making its browser.
                    None => {
                        if let Some(host) = browser.host() {
                            host.close_browser(1);
                        }
                    }
                }
            }));
            let javascript = if self.0.tab.lockdown_view.get() { State::DISABLED } else { State::DEFAULT };
            let settings = BrowserSettings { javascript, ..Default::default() };
            if browser_host_create_browser(Some(&info), Some(&mut client), Some(&start), Some(&settings), None, Some(context)) != 1 {
                self.0.tab.created_hook.take();
                TABS.with_borrow_mut(|t| t.remove(&id));
            }
        }
        #[cfg(target_os = "macos")]
        {
            let Some(created) = created else {
                TABS.with_borrow_mut(|t| t.remove(&id));
                return;
            };
            *self.0.created.borrow_mut() = Some(created);
            if let Some(child) = self.ns_window() {
                child.setHasShadow(false);
            }
            self.placed();
        }
    }

    /// Gives a new browser the frame, visibility and mute state asked for before it existed, and
    /// loads the first URL.
    fn placed(&self) {
        if let Some((x, y, w, h)) = self.0.frame.get() {
            self.set_frame(x, y, w, h);
        }
        // Chrome style shows a new child window whatever its style says.
        if self.0.visible.get() || cfg!(windows) {
            self.show(self.0.visible.get());
        }
        if self.0.muted.get() {
            self.set_muted(true);
        }
        if let Some(dark) = self.0.dark.get() {
            self.set_dark(dark);
        }
        self.load_first();
    }

    /// Clears storage of sites cleared before this tab existed and installs the site style
    /// sheets and a handed-over storage script, then loads the first URL once Chromium has
    /// confirmed all of them.
    fn load_first(&self) {
        let host = self.browser_host();
        let mut steps: Vec<Step> = Vec::new();
        let mut handover_step = None;
        #[cfg(windows)]
        let mut bypass_step = None;
        #[cfg(windows)]
        let mut page_step = None;
        if let Some(host) = &host {
            let tarn = tarn::first(&self.0.tab, host);
            steps.extend(super::data::take_pending_sites().iter().flat_map(|h| clear_storage_calls(h, &[])));
            let handover = self.0.handover.borrow_mut().take();
            // A service worker the site registered before it was set to Lockdown would run its
            // script on the site's pages; a Lockdown view loads past them.
            #[cfg(windows)]
            if self.0.tab.lockdown_view.get() {
                steps.push(("Network.enable", "{}".into(), None));
                bypass_step = Some(steps.len());
                steps.push(("Network.setBypassServiceWorker", "{\"bypass\":true}".into(), None));
            }
            #[cfg(windows)]
            let page_world = !self.0.tab.lockdown_view.get();
            #[cfg(target_os = "macos")]
            let page_world = false;
            if self.0.site_styles.is_some() || handover.is_some() || page_world || tarn {
                // The page agent runs new-document scripts only once the Page domain is enabled.
                steps.push(("Page.enable", "{}".into(), None));
            }
            for (source, is_handover) in [(self.0.site_styles.clone(), false), (handover, true)] {
                let Some(source) = source else { continue };
                if is_handover {
                    handover_step = Some(steps.len());
                }
                steps.push((ADD_SCRIPT, format!("{{\"source\":{},\"worldName\":\"tarn\"}}", js_string(&source)), None));
            }
            // Page-world scripts, as WebKit's: they must change what the page itself sees. Both are
            // compiled in and hold no bridge to the shell (S24).
            #[cfg(windows)]
            if page_world {
                steps.push((ADD_SCRIPT, add_page_script(crate::page_scripts::ALWAYS_VISIBLE), None));
                if let Some(script) = page_script_for(&self.0.tab) {
                    let hash = style_hash(&script) | 1;
                    self.0.tab.page_hash.set(hash);
                    page_step = Some((steps.len(), hash));
                    steps.push((ADD_SCRIPT, add_page_script(&script), None));
                }
            }
        }
        let Some(host) = host.filter(|_| !steps.is_empty()) else {
            // Without DevTools a Lockdown view cannot keep service workers out, so it loads nothing.
            #[cfg(windows)]
            if self.0.tab.lockdown_view.get() {
                let url = self.0.first_load.borrow_mut().take();
                return emit(&self.0.tab, Event::Failed { url, code: 0, message: "Tarn could not open this Lockdown page.".into() });
            }
            if let Some(url) = self.0.first_load.borrow_mut().take() {
                self.start(&url, self.0.first_link.get());
            }
            return;
        };
        self.0.gated.set(true);
        let left = Rc::new(Cell::new(steps.len()));
        let refused = Rc::new(Cell::new(false));
        for (i, (method, params, site)) in steps.into_iter().enumerate() {
            let (weak, left, refused) = (Rc::downgrade(&self.0), left.clone(), refused.clone());
            call(&self.0.tab, &host, method, &params, move |result| {
                // A Lockdown page must not load if its service workers could still run.
                #[cfg(windows)]
                if bypass_step == Some(i) && result.is_none() {
                    refused.set(true);
                }
                if let (None, Some(site)) = (&result, &site) {
                    super::data::record(site);
                }
                let Some(inner) = weak.upgrade() else { return };
                let identifier = string_at(result, &["identifier"]);
                if handover_step == Some(i) {
                    *inner.tab.handover_script.borrow_mut() = identifier.clone();
                }
                #[cfg(windows)]
                if let Some((_, hash)) = page_step.filter(|(step, _)| *step == i) {
                    keep_page_script(&inner.tab, hash, identifier);
                }
                left.set(left.get() - 1);
                if left.get() == 0 {
                    inner.gated.set(false);
                    let (url, link) = (inner.first_load.borrow_mut().take(), inner.first_link.get());
                    if refused.get() {
                        let message = "Tarn could not keep this site's service workers out, so the Lockdown page did not load.".into();
                        emit(&inner.tab, Event::Failed { url, code: 0, message });
                    } else if let Some(url) = url {
                        WebView(inner).start(&url, link);
                    }
                }
            });
        }
    }

    /// Writes `storage` into the first document this view loads, if it has that origin; call
    /// before the first load. Only for a switch the user started, in the same profile.
    pub fn hand_over(&self, storage: Storage) {
        *self.0.handover.borrow_mut() = Some(handover::write_script(&storage));
    }

    /// The storage of the top-level origin, read in an isolated world; `None` for a non-web
    /// page, empty storage or more than `handover::MAX_BYTES`.
    pub fn read_storage(&self, done: impl FnOnce(Option<Storage>) + 'static) {
        let Some(origin) = self.url().as_ref().and_then(crate::page::serialized_origin) else { return done(None) };
        let script = format!("return {};", handover::read_script(&origin));
        let done = RefCell::new(Some(done));
        self.run_isolated(&script, move |json| {
            if let Some(done) = done.take() {
                done(Storage::new(&origin, json));
            }
        });
    }

    pub fn find_text(&self, query: &str, done: impl Fn(Vec<String>) + 'static) {
        self.run_isolated(&crate::page_scripts::find_text_script(query), move |found| done(crate::page_scripts::find_snippets(&found)));
    }

    pub fn icon_links(&self, origin: &Url, done: impl Fn(String) + 'static) {
        match crate::page_scripts::icon_links_script(origin) {
            Some(script) => self.run_isolated(&script, done),
            None => done(String::new()),
        }
    }

    /// The page's article for `reader::Article::parse`, or "1" with `probe` when it has one; empty
    /// when it has none. Read in the engine's isolated world.
    pub fn article(&self, origin: &Url, probe: bool, done: impl Fn(String) + 'static) {
        self.run_isolated(&crate::reader::script(&origin.origin().ascii_serialization(), probe), done);
    }

    pub fn fetch_icon(&self, origin: &Url, urls: &[Url], max_bytes: usize, done: impl Fn(String) + 'static) {
        match crate::page_scripts::fetch_icon_script(origin, urls, max_bytes) {
            Some(script) => self.run_isolated(&script, done),
            None => done(String::new()),
        }
    }

    pub fn page_text(&self, origin: &Url, max_chars: usize, done: impl Fn(String) + 'static) {
        match crate::page_scripts::page_text_script(origin, max_chars) {
            Some(script) => self.run_isolated(&script, done),
            None => done(String::new()),
        }
    }

    pub fn pick_element(&self, done: impl Fn(String) + 'static) {
        self.run_isolated_as_gesture(crate::page_scripts::PICKER, true, done);
    }

    pub fn json_text(&self, origin: &Url, max_bytes: usize, done: impl Fn(String) + 'static) {
        match crate::page_scripts::json_text_script(origin, max_bytes) {
            Some(script) => self.run_isolated(&script, done),
            None => done(String::new()),
        }
    }

    pub fn picture_in_picture(&self, done: impl Fn(String) + 'static) {
        self.run_isolated_as_gesture(crate::page_scripts::PICTURE_IN_PICTURE, true, done);
    }

    /// The visible page as PNG, scaled to `width` points when given.
    pub fn snapshot_png(&self, width: Option<f64>, done: impl Fn(Option<Vec<u8>>) + 'static) {
        let Some(host) = self.browser_host() else { return done(None) };
        let tab = self.0.tab.clone();
        call(&self.0.tab, &host.clone(), "Page.getLayoutMetrics", "{}", move |metrics| {
            let number = |key| {
                value_at(metrics.clone(), &["cssVisualViewport", key]).map(|v| if v.get_type() == ValueType::INT { f64::from(v.int()) } else { v.double() })
            };
            let size = |key| number(key).filter(|v| *v > 0.);
            let (x, y) = (number("pageX").unwrap_or(0.), number("pageY").unwrap_or(0.));
            let clip = match (size("clientWidth"), size("clientHeight"), width) {
                (Some(w), Some(h), Some(target)) => {
                    format!(",\"clip\":{{\"x\":{x},\"y\":{y},\"width\":{w},\"height\":{h},\"scale\":{}}}", (target / w).min(1.))
                }
                _ => String::new(),
            };
            call(&tab, &host, "Page.captureScreenshot", &format!("{{\"format\":\"png\"{clip}}}"), move |shot| done(decode(string_at(shot, &["data"]))));
        });
    }

    /// The page printed to PDF.
    pub fn pdf(&self, done: impl Fn(Option<Vec<u8>>) + 'static) {
        let Some(host) = self.browser_host() else { return done(None) };
        call(&self.0.tab, &host, "Page.printToPDF", "{\"printBackground\":true}", move |pdf| done(decode(string_at(pdf, &["data"]))));
    }

    /// Runs `body` as an async function in an isolated world of the top frame; `done` gets the
    /// string it returns, or an empty string.
    fn run_isolated(&self, body: &str, done: impl Fn(String) + 'static) {
        self.run_isolated_as_gesture(body, false, done);
    }

    /// `gesture` marks the run as the user's action, which a page's own script then shares, so it
    /// is only for what the user just asked for (picture in picture, picking an element).
    fn run_isolated_as_gesture(&self, body: &str, gesture: bool, done: impl Fn(String) + 'static) {
        let Some(host) = self.browser_host() else { return done(String::new()) };
        isolated(&self.0.tab, host, body, gesture, done);
    }

    #[cfg(target_os = "macos")]
    fn window(&self) -> Option<cef::Window> {
        self.0.created.borrow().as_ref().map(|(_, w)| w.clone())
    }

    #[cfg(target_os = "macos")]
    fn browser(&self) -> Option<Browser> {
        self.0.created.borrow().as_ref()?.0.browser()
    }

    #[cfg(windows)]
    fn browser(&self) -> Option<Browser> {
        self.0.created.borrow().clone()
    }

    /// The browser's own child window of the shell window.
    #[cfg(windows)]
    fn hwnd(&self) -> Option<windows_sys::Win32::Foundation::HWND> {
        let handle = self.browser_host()?.window_handle();
        (!handle.0.is_null()).then_some(handle.0.cast())
    }

    fn browser_host(&self) -> Option<BrowserHost> {
        self.browser()?.host()
    }

    #[cfg(target_os = "macos")]
    fn ns_window(&self) -> Option<Retained<NSWindow>> {
        let handle = self.window()?.window_handle();
        if handle.is_null() {
            return None;
        }
        // SAFETY: CEF returns the live NSView of its Views window on the main thread.
        unsafe { &*handle.cast::<NSView>() }.window()
    }

    #[cfg(target_os = "macos")]
    fn attach(&self) {
        let (Some(child), Some(parent)) = (self.ns_window(), self.0.host.borrow().view.window()) else { return };
        // SAFETY: both windows are live and belong to this process.
        unsafe { parent.addChildWindow_ordered(&child, NSWindowOrderingMode::Above) };
    }

    #[cfg(target_os = "macos")]
    fn detach(&self) {
        let (Some(child), Some(parent)) = (self.ns_window(), self.0.host.borrow().view.window()) else { return };
        parent.removeChildWindow(&child);
    }

    #[cfg(target_os = "macos")]
    pub fn rehost(self, host: &Host, sink: Rc<dyn Fn(Event)>) -> Self {
        self.detach();
        *self.0.host.borrow_mut() = host.clone();
        *self.0.tab.sink.borrow_mut() = sink;
        if self.0.visible.get() {
            self.attach();
        }
        self
    }

    #[cfg(windows)]
    pub fn rehost(self, host: &Host, sink: Rc<dyn Fn(Event)>) -> Self {
        *self.0.host.borrow_mut() = host.clone();
        *self.0.tab.sink.borrow_mut() = sink;
        if let Some(child) = self.hwnd() {
            // SAFETY: both windows belong to this thread.
            unsafe { SetParent(child, host.hwnd) };
        }
        if let Some((x, y, w, h)) = self.0.frame.get() {
            self.set_frame(x, y, w, h);
        }
        self
    }

    /// Loads a URL that the caller has already passed through `nav_policy`; never a `file:` URL.
    pub fn load(&self, url: &Url) {
        if url.scheme() == "file" {
            return emit(&self.0.tab, Event::Blocked { url: url.clone(), message: nav_policy::FILE_NOT_OPENED.to_owned(), category: None });
        }
        self.start(url, false);
    }

    /// Shows a file the user picked with Open File (`nav_policy::Source::FileOpen`).
    pub fn load_file(&self, url: &Url) {
        if url.scheme() == "file" {
            self.0.tab.opened_files.borrow_mut().insert(url.clone());
            self.start(url, false);
        }
    }

    /// Makes this view one of an install's own pages (`tarn_ext::PageKind`): it loads only that
    /// origin, served by the profile's handler, with the bootstrap in its main world. Call it
    /// right after creating the view, before its first load.
    pub fn set_extension_page(&self, spec: PageSpec) {
        self.0.tab.tarn.borrow_mut().page = Some(spec);
    }

    /// Sends `json` to an extension page view's own page.
    pub fn post_to_page(&self, json: &str) {
        let world = self.0.tab.tarn.borrow().page.as_ref().map(|s| s.world.clone());
        if let Some(world) = world {
            tarn::post(&self.0.tab, &world, Some(FrameId::MAIN), json);
        }
    }

    /// Loads `url` as a page's link: without the shell's mark, so S10 judges it and every redirect
    /// hop as the page's, and a tab with no page yet counts as public.
    pub fn load_link(&self, url: &Url) {
        self.start(url, true);
    }

    /// As `load_link`, from `opener`, the page of the view this one replaced.
    #[cfg(windows)]
    pub(crate) fn load_link_from(&self, url: &Url, opener: Option<&Url>) {
        *self.0.tab.opener.borrow_mut() = opener.cloned();
        self.start(url, true);
    }

    fn start(&self, url: &Url, link: bool) {
        if self.0.dead.get() {
            emit(&self.0.tab, Event::Failed { url: Some(url.clone()), code: 0, message: "Chromium could not start.".into() });
            return;
        }
        let Some(frame) = self.browser().and_then(|b| b.main_frame()).filter(|_| !self.0.gated.get()) else {
            *self.0.first_load.borrow_mut() = Some(url.clone());
            self.0.first_link.set(link);
            return;
        };
        *self.0.tab.shell_load.borrow_mut() = (!link).then(|| url.clone());
        self.0.tab.link_load.set(link);
        frame.load_url(Some(&CefString::from(url.as_str())));
    }

    pub fn set_frame(&self, x: f64, y: f64, width: f64, height: f64) {
        self.0.frame.set(Some((x, y, width, height)));
        let host = self.0.host.borrow();
        #[cfg(windows)]
        if let Some(child) = self.hwnd() {
            let (x, y, width, height) = host.pixels(x, y, width, height);
            let diameter = (self.0.radius.get() * 2. * host.scale()).round() as i32;
            // SAFETY: a child window of this thread; the window owns the region once it is set,
            // and no region removes the clip.
            unsafe {
                SetWindowPos(child, std::ptr::null_mut(), x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
                let clip = (child as isize, width, height, diameter);
                if self.0.clip.replace(Some(clip)) != Some(clip) {
                    let region = if diameter > 0 { CreateRoundRectRgn(0, 0, width + 1, height + 1, diameter, diameter) } else { std::ptr::null_mut() };
                    if SetWindowRgn(child, region, 1) == 0 && !region.is_null() {
                        DeleteObject(region);
                        self.0.clip.set(None);
                    }
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            let (Some(child), Some(parent)) = (self.ns_window(), host.view.window()) else { return };
            let rect = host.view.convertRect_toView(host.rect(x, y, width, height), None);
            child.setFrame_display(parent.convertRectToScreen(rect), true);
        }
    }

    /// Rounds the page's window to the card's corners.
    #[cfg(windows)]
    pub(crate) fn set_corner_radius(&self, radius: f64) {
        self.0.radius.set(radius);
        if let Some((x, y, w, h)) = self.0.frame.get() {
            self.set_frame(x, y, w, h);
        }
    }

    /// Chrome's own two-finger swipe back and forward.
    #[cfg(windows)]
    pub(crate) fn set_gestures(&self, on: bool) {
        super::runtime::set_swipe_navigation(on);
    }

    pub fn set_visible(&self, visible: bool) {
        if self.0.visible.replace(visible) != visible {
            VISIBLE.set(if visible { VISIBLE.get() + 1 } else { VISIBLE.get().saturating_sub(1) });
            self.show(visible);
            if !visible {
                super::cookies::changed_in_chromium();
                // A page put away (another tab, an overlay) leaves full screen, as it would in Chrome.
                #[cfg(windows)]
                if let Some(host) = self.browser_host().filter(|h| h.is_fullscreen() == 1) {
                    host.exit_fullscreen(1);
                }
            }
        }
    }

    #[cfg(windows)]
    fn show(&self, visible: bool) {
        let (Some(child), Some(host)) = (self.hwnd(), self.browser_host()) else { return };
        host.was_hidden(i32::from(!visible));
        // SAFETY: a child window of this thread; a shown page goes above the other tabs' pages.
        unsafe {
            ShowWindow(child, if visible { SW_SHOWNA } else { SW_HIDE });
            if visible {
                SetWindowPos(child, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn show(&self, visible: bool) {
        let Some(window) = self.window() else { return };
        if visible {
            window.show();
            self.attach();
        } else {
            self.detach();
            window.hide();
        }
    }

    /// The page's opacity, for fades; a Windows child window is layered only while it fades.
    pub fn set_alpha(&self, alpha: f64) {
        #[cfg(target_os = "macos")]
        if let Some(child) = self.ns_window() {
            child.setAlphaValue(alpha);
        }
        #[cfg(windows)]
        if let Some(child) = self.hwnd() {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GWL_EXSTYLE, GetWindowLongPtrW, LWA_ALPHA, SetLayeredWindowAttributes, SetWindowLongPtrW, WS_EX_LAYERED,
            };
            let alpha = (alpha.clamp(0., 1.) * 255.).round() as u8;
            let layered = WS_EX_LAYERED as isize;
            // SAFETY: a child window of this thread.
            unsafe {
                let style = GetWindowLongPtrW(child, GWL_EXSTYLE);
                if alpha == u8::MAX {
                    if style & layered != 0 {
                        SetWindowLongPtrW(child, GWL_EXSTYLE, style & !layered);
                    }
                } else {
                    if style & layered == 0 {
                        SetWindowLongPtrW(child, GWL_EXSTYLE, style | layered);
                    }
                    SetLayeredWindowAttributes(child, 0, alpha, LWA_ALPHA);
                }
            }
        }
    }

    pub fn raise(&self) {
        #[cfg(target_os = "macos")]
        if self.0.visible.get() {
            self.detach();
            self.attach();
        }
        #[cfg(windows)]
        if let Some(child) = self.hwnd() {
            // SAFETY: a child window of this thread.
            unsafe { SetWindowPos(child, HWND_TOP, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
        }
    }

    pub fn focus(&self) {
        if let Some(host) = self.browser_host() {
            host.set_focus(1);
        }
    }

    pub fn url(&self) -> Option<Url> {
        let frame = self.browser()?.main_frame()?;
        Url::parse(&CefString::from(&frame.url()).to_string()).ok()
    }

    pub fn title(&self) -> String {
        self.0.tab.title.borrow().clone()
    }

    pub fn find(&self, text: &str, backwards: bool, found: impl Fn(usize, Option<usize>) + 'static) {
        let Some(host) = self.browser_host() else { return found(0, None) };
        *self.0.tab.found.borrow_mut() = Some(Box::new(found));
        let next = self.0.tab.last_find.replace(text.to_owned()) == text;
        host.find(Some(&CefString::from(text)), i32::from(!backwards), 0, i32::from(next));
    }

    /// The colour scheme pages see (`prefers-color-scheme`): the shell's theme, not the system's.
    pub fn set_dark(&self, dark: bool) {
        self.0.dark.set(Some(dark));
        if let Some(host) = self.browser_host() {
            let scheme = if dark { "dark" } else { "light" };
            let params = format!("{{\"features\":[{{\"name\":\"prefers-color-scheme\",\"value\":\"{scheme}\"}}]}}");
            call(&self.0.tab, &host, "Emulation.setEmulatedMedia", &params, |_| {});
        }
    }

    /// Ends the find and clears its highlight, keeping the page's selection; the next find starts afresh.
    pub fn clear_find(&self) {
        self.0.tab.last_find.replace(String::new());
        if let Some(host) = self.browser_host() {
            host.stop_finding(0);
        }
    }

    pub fn is_muted(&self) -> bool {
        self.0.muted.get()
    }

    /// The process that renders the top document.
    pub fn process_id(&self) -> Option<i32> {
        self.0.tab.renderer.get().filter(|p| super::runtime::tarn_process(*p)).and_then(|p| i32::try_from(p).ok())
    }

    /// What sleeping would lose; nothing to read in a Lockdown view, where no script of Tarn's runs on the page.
    pub fn page_keep(&self, done: impl Fn(crate::page_scripts::PageKeep) + 'static) {
        if self.lockdown_view() {
            return done(Default::default());
        }
        self.run_isolated(crate::page_scripts::PAGE_KEEP, move |answer| done(crate::page_scripts::page_keep(&answer)));
    }

    pub fn scroll_to(&self, offset: (f64, f64)) {
        if !self.lockdown_view() {
            self.run_isolated(&crate::page_scripts::scroll_script(offset), |_| {});
        }
    }

    #[cfg(windows)]
    fn lockdown_view(&self) -> bool {
        self.0.tab.lockdown_view.get()
    }

    /// macOS opens Lockdown sites in WebKit, never in Chromium.
    #[cfg(target_os = "macos")]
    fn lockdown_view(&self) -> bool {
        false
    }

    /// Whether the top document is playing video or audio now. The page answers for itself, so
    /// at worst it keeps its own tab awake.
    pub fn media_playing(&self, done: impl Fn(bool) + 'static) {
        let Some(host) = self.browser_host() else { return done(false) };
        let script = "[...document.querySelectorAll('video,audio')].some(m => !m.paused && !m.ended)";
        let params = format!("{{\"expression\":{},\"returnByValue\":true}}", js_string(script));
        call(&self.0.tab, &host, "Runtime.evaluate", &params, move |result| {
            done(value_at(result, &["result", "value"]).is_some_and(|v| v.get_type() == ValueType::BOOL && v.bool() != 0));
        });
    }

    pub fn set_muted(&self, muted: bool) {
        self.0.muted.set(muted);
        if let Some(host) = self.browser_host() {
            host.set_audio_muted(i32::from(muted));
        }
    }

    pub fn print(&self) {
        if let Some(host) = self.browser_host() {
            host.print();
        }
    }

    pub fn back(&self) {
        if let Some(b) = self.browser() {
            #[cfg(windows)]
            if !native_back(&b) {
                return self.restored_step(true);
            }
            *self.0.tab.history_step.borrow_mut() = if b.can_go_back() == 1 { history_urls(&b, |i, current| i < current) } else { Vec::new() };
            b.go_back();
        }
    }

    pub fn forward(&self) {
        if let Some(b) = self.browser() {
            #[cfg(windows)]
            if b.can_go_forward() != 1 {
                return self.restored_step(false);
            }
            *self.0.tab.history_step.borrow_mut() = if b.can_go_forward() == 1 { history_urls(&b, |i, current| i > current) } else { Vec::new() };
            b.go_forward();
        }
    }

    /// Steps back (`true`) or forward into the restored list past Chromium's own: the page there
    /// replaces the current entry, as the user's own load, and the current page joins the other side.
    #[cfg(windows)]
    fn restored_step(&self, back: bool) {
        let tab = &self.0.tab;
        let (Some(left), Some(target)) = (self.url(), tab.restored.borrow().next(back).cloned()) else { return };
        // Not while a page loads, whose end would cancel the step, nor from the blank page a view starts on.
        if tab.restored_step.borrow().is_some() || tab.loading.get() || left.as_str() == "about:blank" || self.browser_host().is_none() {
            return;
        }
        *tab.restored_step.borrow_mut() = Some((back, target.clone(), left));
        *tab.shell_load.borrow_mut() = Some(target.clone());
        let weak = Rc::downgrade(tab);
        // The step is the user's, as Back is, so it tells the next site nothing of the page left.
        let script = format!(
            "const m = document.createElement('meta'); m.name = 'referrer'; m.content = 'no-referrer'; (document.head || document.documentElement).append(m); location.replace({}); return 'ok';",
            js_string(target.as_str())
        );
        self.run_isolated(&script, move |done| {
            if let Some(tab) = weak.upgrade().filter(|_| done != "ok") {
                abandon_step(&tab);
            }
        });
    }

    /// The back list to save while the tab sleeps, is closed or Tarn quits: Chromium's own and the
    /// restored one around it.
    #[cfg(windows)]
    pub(crate) fn saved_history(&self) -> Option<Vec<u8>> {
        let shown = self.browser().zip(self.url().filter(|u| u.as_str() != "about:blank"));
        // Before the restored page commits there is nothing newer to save than what was restored.
        let Some((browser, current)) = shown else { return self.0.tab.restored_bytes.borrow().clone() };
        let restored = self.0.tab.restored.borrow();
        let back = restored.back.iter().cloned().chain(history_urls(&browser, |i, c| i < c)).collect();
        let forward = history_urls(&browser, |i, c| i > c).into_iter().chain(restored.forward.iter().cloned()).collect();
        Some(crate::view_policy::SavedHistory::new(back, current, forward)?.to_bytes())
    }

    /// Loads the saved page and keeps its back and forward lists for `back` and `forward`.
    #[cfg(windows)]
    pub(crate) fn restore_history(&self, bytes: &[u8]) {
        let Some(saved) = crate::view_policy::SavedHistory::from_bytes(bytes) else { return };
        let tab = &self.0.tab;
        *tab.restored.borrow_mut() = crate::view_policy::RestoredHistory { back: saved.back, forward: saved.forward };
        *tab.restored_bytes.borrow_mut() = Some(bytes.to_vec());
        tab.restoring.set(true);
        self.load(&saved.current);
    }

    pub fn reload(&self) {
        if let Some(b) = self.browser() {
            *self.0.tab.history_step.borrow_mut() = history_urls(&b, |i, current| i == current);
            b.reload();
        }
    }

    /// Reloads a page whose renderer ended; not the user's navigation, as a page can end its own renderer.
    pub fn recover(&self) {
        match self.url().filter(|u| self.0.tab.opened_files.borrow().contains(u)) {
            Some(url) => self.load_file(&url),
            None => {
                if let Some(b) = self.browser() {
                    b.reload();
                }
            }
        }
    }

    pub fn reload_from_origin(&self) {
        if let Some(b) = self.browser() {
            *self.0.tab.history_step.borrow_mut() = history_urls(&b, |i, current| i == current);
            b.reload_ignore_cache();
        }
    }

    pub fn stop(&self) {
        if let Some(b) = self.browser() {
            b.stop_load();
        }
    }

    /// An Edit menu command on the page's focused frame, for a menu that does not reach the page.
    pub fn edit(&self, command: EditCommand) {
        let Some(frame) = self.browser().and_then(|b| b.focused_frame()) else { return };
        match command {
            EditCommand::Undo => frame.undo(),
            EditCommand::Redo => frame.redo(),
            EditCommand::Cut => frame.cut(),
            EditCommand::Copy => frame.copy(),
            EditCommand::Paste => frame.paste(),
            EditCommand::SelectAll => frame.select_all(),
        }
    }

    pub fn zoom(&self) -> f64 {
        self.browser_host().map_or(1.0, |h| 1.2f64.powf(h.zoom_level()))
    }

    pub fn set_zoom(&self, zoom: f64) {
        if let Some(host) = self.browser_host() {
            host.set_zoom_level(zoom.max(crate::MIN_ZOOM).ln() / 1.2f64.ln());
        }
    }
}

impl Drop for WebView {
    fn drop(&mut self) {
        if Rc::strong_count(&self.0) > 1 {
            return;
        }
        if self.0.visible.get() {
            VISIBLE.set(VISIBLE.get().saturating_sub(1));
        }
        if !super::runtime::running() {
            return;
        }
        // Chromium closes the page later; until then it must not stay on screen over the next tab.
        self.show(false);
        self.0.tab.browser.take();
        #[cfg(target_os = "macos")]
        {
            if let Some((_, window)) = self.0.created.borrow_mut().take() {
                window.close();
            }
        }
        #[cfg(windows)]
        if let Some(host) = self.0.created.borrow_mut().take().and_then(|b| b.host()) {
            host.close_browser(1);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod mac_tests {
    #[test]
    fn command_shortcuts_reach_the_shell_and_editing_keys_stay_with_the_page() {
        let (ctrl, alt, shift, cmd) = (4, 8, 2, 128);
        assert!(super::forwarded(cmd, i32::from(b'T')));
        assert!(super::forwarded(cmd | shift, i32::from(b'C')), "Copy Link");
        assert!(super::forwarded(cmd | ctrl, i32::from(b'F')), "full screen");
        assert!(super::forwarded(ctrl, 0x09), "Ctrl+Tab");
        assert!(super::forwarded(ctrl, i32::from(b'1')), "a space");
        assert!(super::forwarded(cmd, i32::from(b'Y')), "history");
        for key in *b"ACVXZ" {
            assert!(!super::forwarded(cmd, i32::from(key)));
        }
        assert!(!super::forwarded(cmd | shift, i32::from(b'Z')), "redo");
        assert!(!super::forwarded(cmd, 0x25), "Cmd+Left");
        assert!(!super::forwarded(cmd, 0x08), "Cmd+Backspace");
        assert!(!super::forwarded(ctrl, 0x27), "Ctrl+Right");
        assert!(!super::forwarded(alt, i32::from(b'L')), "Option types characters");
        assert!(!super::forwarded(shift, i32::from(b'L')));
    }

    #[test]
    fn the_page_menu_keeps_safe_items_and_inspect_only_where_web_inspector_is_on() {
        use cef::sys;
        assert!(super::kept_item(sys::IDC_CONTENT_CONTEXT_COPYIMAGE, false));
        assert!(super::kept_item(sys::IDC_RELOAD, false));
        assert!(!super::kept_item(sys::IDC_CONTENT_CONTEXT_INSPECTELEMENT, false));
        assert!(super::kept_item(sys::IDC_CONTENT_CONTEXT_INSPECTELEMENT, true));
        assert!(!super::kept_item(sys::IDC_CONTENT_CONTEXT_SAVELINKAS, true));
        assert!(!super::kept_item(sys::IDC_CONTENT_CONTEXT_OPENLINKNEWTAB, true));
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn shortcuts_reach_the_shell_and_editing_keys_stay_with_the_page() {
        let (ctrl, alt, shift) = (4, 8, 2);
        assert!(super::forwarded(ctrl, i32::from(b'L')));
        assert!(super::forwarded(ctrl | shift, i32::from(b'T')));
        assert!(super::forwarded(alt, 0x25));
        assert!(!super::forwarded(ctrl | alt, i32::from(b'Q')), "AltGr types characters");
        assert!(!super::forwarded(ctrl, 0x11), "Ctrl alone");
        assert!(!super::forwarded(alt, 0x12), "Alt alone");
        assert!(!super::forwarded(ctrl, 0x08), "Ctrl+Backspace");
        assert!(!super::forwarded(ctrl, 0x25), "Ctrl+Left");
        for key in *b"ACVXYZ" {
            assert!(!super::forwarded(ctrl, i32::from(key)));
        }
        for key in *b"VZ" {
            assert!(!super::forwarded(ctrl | shift, i32::from(key)));
        }
        assert!(super::forwarded(ctrl | shift, i32::from(b'C')), "Copy Link");
        assert!(super::forwarded(ctrl | shift, i32::from(b'A')), "Library");
        assert!(!super::forwarded(shift, i32::from(b'L')));
        assert!(!super::forwarded(0, i32::from(b'L')));
    }

    #[test]
    fn page_scripts_skip_lockdown_and_paused_views() {
        let host = crate::Host { hwnd: std::ptr::null_mut() };
        let inner = super::WebView::inner(&host, "s:ps".into(), "e:ps".into(), None, std::rc::Rc::new(|_| {}));
        let tab = &inner.tab;
        crate::data_store_windows::set_page_script_for_test("s:ps", Some("yt"));
        tab.filtered.set(true);
        assert_eq!(super::page_script_for(tab).as_deref(), Some("yt"));
        tab.filtered.set(false);
        assert_eq!(super::page_script_for(tab), None, "a paused site gets no page script");
        tab.filtered.set(true);
        tab.lockdown_view.set(true);
        assert_eq!(super::page_script_for(tab), None, "a Lockdown view runs no script");
        crate::data_store_windows::set_page_script_for_test("s:ps", None);
    }

    #[test]
    fn late_page_script_replies_never_leave_two_registered() {
        let host = crate::Host { hwnd: std::ptr::null_mut() };
        let inner = super::WebView::inner(&host, "s:t".into(), "e:t".into(), None, std::rc::Rc::new(|_| {}));
        let tab = &inner.tab;
        tab.page_hash.set(2);
        super::keep_page_script(tab, 1, Some("a".into()));
        assert_eq!(*tab.page_script.borrow(), None, "a script replaced before its reply is dropped");
        super::keep_page_script(tab, 2, Some("b".into()));
        assert_eq!(tab.page_script.borrow().as_deref(), Some("b"));
        super::keep_page_script(tab, 2, Some("b2".into()));
        assert_eq!(tab.page_script.borrow().as_deref(), Some("b"), "the same script registered twice runs once");
        tab.page_script.take();
        super::keep_page_script(tab, 2, None);
        assert_eq!(tab.page_hash.get(), 0, "a refused script lets the next change register it");
    }
}

impl ExtensionView for WebView {
    fn set_extension_gate(&self, gate: Gate) {
        tarn::set_gate(&self.0.tab, gate);
    }

    fn add_extension_scripts(&self, scripts: Rc<WorldScripts>) {
        tarn::add(&self.0.tab, scripts);
    }

    fn remove_extension_scripts(&self, world: &World) {
        tarn::remove(&self.0.tab, world);
    }

    fn post_extension_message(&self, world: &World, frame: Option<FrameId>, json: &str) {
        tarn::post(&self.0.tab, world, frame, json);
    }
}
