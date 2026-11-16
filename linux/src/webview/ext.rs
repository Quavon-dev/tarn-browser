use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use engine::tarn_ext::{FrameId, MAX_MESSAGE_BYTES, RECEIVE, Resources, RunAt, SCHEME, World, WorldScripts, page_loads};
use futures_channel::mpsc::UnboundedSender;
use nsh_platform_linux::gtk4 as gtk;
use serde_json::json;
use shield::ShieldLevel;
use url::Url;
use webkit6::prelude::*;
use webkit6::{
    NavigationPolicyDecision, PolicyDecision, PolicyDecisionType, ResponsePolicyDecision, URISchemeRequest, URISchemeResponse, UserContentInjectedFrames,
    UserContentManager, UserScript, UserScriptInjectionTime, UserStyleLevel, UserStyleSheet, WebContext, WebView, gio, glib, soup,
};

use super::{State, VIEWS, filters};
use crate::data::StoreKey;

const PAGE_HANDLER: &str = "tarnExt";

/// What the views on the GTK thread tell the shell, whose thread runs the extension runtime.
pub enum Event {
    Frame(Box<Frame>),
    PageMessage { page: u64, json: String },
    PageFailed { page: u64 },
}

/// A frame's document waits for the gate: which scripts of `worlds` may run at `url`.
pub struct Frame {
    pub request: u64,
    pub tab: String,
    pub store: String,
    pub worlds: Vec<World>,
    pub url: Url,
    pub top: Url,
    pub main: bool,
}

#[derive(Default)]
struct Store {
    worlds: Vec<Rc<WorldScripts>>,
    rules: Vec<(String, String)>,
}

thread_local! {
    static SINK: RefCell<Option<UnboundedSender<Event>>> = const { RefCell::new(None) };
    static STORES: RefCell<HashMap<String, Store>> = RefCell::default();
    static PAGES: RefCell<HashMap<u64, Option<Rc<PageView>>>> = RefCell::default();
    static SHOWN: RefCell<HashMap<u64, glib::WeakRef<gtk::Box>>> = RefCell::default();
    static NEXT_REQUEST: Cell<u64> = const { Cell::new(0) };
    static SCHEME_READY: Cell<bool> = const { Cell::new(false) };
}

pub(super) enum Added {
    Script(UserScript),
    Sheet(UserStyleSheet),
}

#[derive(Default)]
pub(super) struct Ext {
    pub(super) store: RefCell<Option<String>>,
    pub(super) worlds: RefCell<BTreeMap<World, Rc<WorldScripts>>>,
    pub(super) handlers: RefCell<Vec<World>>,
    signals: RefCell<Vec<(World, glib::SignalHandlerId)>>,
    main_worlds: RefCell<Vec<World>>,
    tokens: RefCell<HashMap<World, String>>,
    responded: Cell<bool>,
    item: RefCell<Option<webkit6::BackForwardListItem>>,
    kept: RefCell<Vec<(webkit6::BackForwardListItem, HashMap<World, String>)>>,
    pub(super) allowed: RefCell<BTreeMap<(World, usize), Vec<String>>>,
    pub(super) added: RefCell<Vec<Added>>,
    held: RefCell<HashMap<u64, (PolicyDecision, Url, bool)>>,
    pub(super) filters: RefCell<Vec<String>>,
}

/// Runs `work` on the GTK thread, which owns every web view; the shell calls from its own thread.
fn on_gtk(work: impl FnOnce() + Send + 'static) {
    drop(glib::MainContext::default().spawn(async move { work() }));
}

pub fn connect(sink: UnboundedSender<Event>) {
    on_gtk(move || SINK.with_borrow_mut(|s| *s = Some(sink)));
}

fn tell(event: Event) -> bool {
    SINK.with_borrow(|s| s.as_ref().is_some_and(|s| s.unbounded_send(event).is_ok()))
}

/// The worlds of every view of the persistent store `store`, current and future.
pub fn set_scripts(store: String, worlds: Vec<WorldScripts>) {
    on_gtk(move || {
        STORES.with_borrow_mut(|s| s.entry(store.clone()).or_default().worlds = worlds.into_iter().map(Rc::new).collect());
        refresh(&store);
    });
}

/// The installs' network rules for the views of `store`, as `(filter id, content rule list)`.
pub fn set_rules(store: String, rules: Vec<(String, String)>) {
    on_gtk(move || {
        STORES.with_borrow_mut(|s| s.entry(store.clone()).or_default().rules = rules);
        refresh(&store);
    });
}

/// The store whose extensions a view runs: a persistent store's tab below Lockdown.
pub fn store_of(key: &StoreKey, tab: &str, level: ShieldLevel) -> Option<String> {
    match key {
        StoreKey::Persistent(id) if tab.starts_with("false-") && level != ShieldLevel::Lockdown => Some(id.clone()),
        _ => None,
    }
}

/// WebKit user-content patterns that match exactly `url`'s document, query included, with or
/// without a fragment (WebKit's patterns name no port); `None` where a `*` in the address would
/// widen the match.
fn patterns(url: &Url) -> Option<Vec<String>> {
    let host = url.host_str()?;
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let base = format!("{}://{host}{}{query}", url.scheme(), url.path());
    if !matches!(url.scheme(), "http" | "https") || base.contains('*') {
        return None;
    }
    Some(vec![format!("{base}#*"), base])
}

/// Adds or takes out one document's patterns for a script; an entry left empty goes, since an
/// empty allow-list would match every page. True when the entry changed.
fn allow(allowed: &mut BTreeMap<(World, usize), Vec<String>>, key: (World, usize), list: &[String], want: bool) -> bool {
    let entry = allowed.entry(key.clone()).or_default();
    let has = list.iter().all(|p| entry.contains(p));
    if want && !has {
        entry.extend(list.iter().cloned());
    } else if !want {
        entry.retain(|p| !list.contains(p));
    }
    if entry.is_empty() {
        allowed.remove(&key);
    }
    want != has
}

const TOKEN: usize = 32;

/// WebKitGTK names no frame for a script message, so the main frame's document draws a token,
/// says it first, and starts every message with it; a subframe never learns it.
fn transport(world: &World) -> String {
    let name = serde_json::to_string(world.name()).unwrap_or_default();
    format!(
        "(function () {{ const h = window.top === window && window.webkit && window.webkit.messageHandlers ? window.webkit.messageHandlers[{name}] : null; \
         let token = ''; if (h) {{ const b = new Uint8Array({half}); crypto.getRandomValues(b); token = Array.from(b, x => x.toString(16).padStart(2, '0')).join(''); h.postMessage(token); }} \
         globalThis.__tarnExtPost = s => {{ if (!h) throw new Error('unavailable'); h.postMessage(token + String(s)); }}; }})();\n",
        half = TOKEN / 2
    )
}

fn untoken(state: &State, world: &World, message: String) -> Option<String> {
    let mut tokens = state.ext.tokens.borrow_mut();
    if let Some(token) = tokens.get(world) {
        return message.strip_prefix(token.as_str()).map(str::to_owned);
    }
    if message.len() == TOKEN && message.bytes().all(|b| b.is_ascii_hexdigit()) && state.ext.main_worlds.borrow().contains(world) {
        tokens.insert(world.clone(), message);
    }
    None
}

fn time(run_at: RunAt) -> UserScriptInjectionTime {
    match run_at {
        RunAt::DocumentStart => UserScriptInjectionTime::Start,
        RunAt::DocumentEnd | RunAt::DocumentIdle => UserScriptInjectionTime::End,
    }
}

fn frames(all: bool) -> UserContentInjectedFrames {
    if all { UserContentInjectedFrames::AllFrames } else { UserContentInjectedFrames::TopFrame }
}

fn sync(state: &State) {
    let Some(content) = state.content.borrow().clone() else { return };
    for added in state.ext.added.borrow_mut().drain(..) {
        match added {
            Added::Script(s) => content.remove_script(&s),
            Added::Sheet(s) => content.remove_style_sheet(&s),
        }
    }
    let worlds = state.ext.worlds.borrow();
    let allowed = state.ext.allowed.borrow();
    let mut added = Vec::new();
    for (world, scripts) in worlds.iter() {
        let mine: Vec<(&Rc<WorldScripts>, usize, &Vec<String>)> =
            allowed.range((world.clone(), 0)..=(world.clone(), usize::MAX)).map(|((_, ix), list)| (scripts, *ix, list)).collect();
        if mine.is_empty() {
            continue;
        }
        let mut every: Vec<&str> = mine.iter().flat_map(|(_, _, l)| l.iter().map(String::as_str)).collect();
        every.sort_unstable();
        every.dedup();
        let any_frame = mine.iter().any(|(s, ix, _)| s.scripts.get(*ix).is_some_and(|c| c.all_frames));
        let boot = format!("{}{}", transport(world), scripts.bootstrap);
        added.push(Added::Script(UserScript::for_world(&boot, frames(any_frame), UserScriptInjectionTime::Start, world.name(), &every, &[])));
        for (scripts, ix, list) in mine {
            let Some(script) = scripts.scripts.get(ix) else { continue };
            let list: Vec<&str> = list.iter().map(String::as_str).collect();
            for css in &script.css {
                added.push(Added::Sheet(UserStyleSheet::for_world(css, frames(script.all_frames), UserStyleLevel::User, world.name(), &list, &[])));
            }
            for js in &script.js {
                added.push(Added::Script(UserScript::for_world(js, frames(script.all_frames), time(script.run_at), world.name(), &list, &[])));
            }
        }
    }
    for a in &added {
        match a {
            Added::Script(s) => content.add_script(s),
            Added::Sheet(s) => content.add_style_sheet(s),
        }
    }
    *state.ext.added.borrow_mut() = added;
}

/// Records the gate's answer for one document: each script of a world may run where it said so.
fn record(state: &State, url: &Url, main: bool, runs: &[(World, Vec<usize>)]) -> bool {
    let Some(list) = patterns(url) else { return false };
    let worlds = state.ext.worlds.borrow();
    let mut allowed = state.ext.allowed.borrow_mut();
    let mut changed = false;
    if main {
        *state.ext.main_worlds.borrow_mut() = runs.iter().filter(|(_, r)| !r.is_empty()).map(|(w, _)| w.clone()).collect();
    }
    for (world, scripts) in worlds.iter() {
        let run = runs.iter().find(|(w, _)| w == world).map(|(_, r)| r.as_slice()).unwrap_or_default();
        for (ix, _) in scripts.scripts.iter().enumerate().filter(|(_, s)| main || s.all_frames) {
            changed |= allow(&mut allowed, (world.clone(), ix), &list, run.contains(&ix));
        }
    }
    changed
}

pub(super) fn started(state: &State) {
    state.ext.responded.set(false);
    for (_, (decision, _, _)) in state.ext.held.borrow_mut().drain() {
        decision.ignore();
    }
    state.ext.main_worlds.borrow_mut().clear();
    let had = !state.ext.allowed.borrow().is_empty();
    state.ext.allowed.borrow_mut().clear();
    if had {
        sync(state);
    }
}

pub(super) fn committed(state: &State, web: &WebView) {
    let list = web.back_forward_list();
    let item = list.as_ref().and_then(|l| l.current_item());
    let outgoing = state.ext.tokens.take();
    let mut kept = state.ext.kept.borrow_mut();
    if let Some(left) = state.ext.item.replace(item.clone())
        && !outgoing.is_empty()
    {
        kept.retain(|(i, _)| *i != left);
        kept.push((left, outgoing));
    }
    let listed: Vec<webkit6::BackForwardListItem> = list.map(|l| l.back_list().into_iter().chain(l.forward_list()).collect()).unwrap_or_default();
    let restored = item.filter(|_| !state.ext.responded.replace(false));
    if let Some(ix) = restored.and_then(|item| kept.iter().position(|(i, _)| *i == item)) {
        *state.ext.tokens.borrow_mut() = kept.remove(ix).1;
    }
    kept.retain(|(i, _)| listed.contains(i));
}

/// Holds a frame's document, which the view would show, until the shell's gate has said which
/// scripts may run in it; false when the view runs no extension, the address takes none, or the
/// shell cannot be asked or a subframe has no committed top page, when no script may run at the
/// address. A subframe is gated against the page committed in the main frame, which is still the
/// one it belongs to while a main-frame load is pending.
pub(super) fn hold(state: &State, response: &ResponsePolicyDecision) -> bool {
    if response.is_main_frame_main_resource() {
        state.ext.responded.set(true);
    }
    let Some(store) = state.ext.store.borrow().clone() else { return false };
    let worlds: Vec<World> = state.ext.worlds.borrow().keys().cloned().collect();
    let url = response.response().and_then(|r| r.uri()).and_then(|u| Url::parse(&u).ok());
    let Some(url) = url.filter(|u| !worlds.is_empty() && patterns(u).is_some()) else { return false };
    let main = response.is_main_frame_main_resource();
    let top = if main { Some(url.clone()) } else { state.page.borrow().clone() };
    let request = NEXT_REQUEST.replace(NEXT_REQUEST.get() + 1) + 1;
    let tab = state.tab.borrow().clone();
    let asked = top.is_some_and(|top| tell(Event::Frame(Box::new(Frame { request, tab, store, worlds, url: url.clone(), top, main }))));
    if !asked {
        if record(state, &url, main, &[]) {
            sync(state);
        }
        return false;
    }
    state.ext.held.borrow_mut().insert(request, (response.clone().upcast(), url, main));
    true
}

/// The gate's answer to a held document: what it allowed may run there, then the document loads.
pub fn answer(request: u64, runs: Vec<(World, Vec<usize>)>) {
    on_gtk(move || {
        for state in live_views() {
            let Some((decision, url, main)) = state.ext.held.borrow_mut().remove(&request) else { continue };
            if record(&state, &url, main, &runs) {
                sync(&state);
            }
            return decision.use_();
        }
    });
}

fn register(state: &Rc<State>, content: &UserContentManager, world: &World) {
    if state.ext.handlers.borrow().contains(world) {
        return;
    }
    content.register_script_message_handler(world.name(), Some(world.name()));
    let weak = Rc::downgrade(state);
    let (name, from) = (world.name().to_owned(), world.clone());
    let signal = content.connect_script_message_received(Some(world.name()), move |_, value| {
        let Some(state) = weak.upgrade() else { return };
        if !value.is_string() {
            return;
        }
        let Some(mut json) = untoken(&state, &from, value.to_str().to_string()) else { return };
        if json.len() > MAX_MESSAGE_BYTES {
            let cut = (MAX_MESSAGE_BYTES + 1..=json.len()).find(|i| json.is_char_boundary(*i)).unwrap_or(json.len());
            json.truncate(cut);
        }
        state.emit("extensionMessage", json!({ "world": name, "frame": FrameId::MAIN.0, "json": json }));
    });
    state.ext.handlers.borrow_mut().push(world.clone());
    state.ext.signals.borrow_mut().push((world.clone(), signal));
}

fn remove_world(state: &State, content: &UserContentManager, world: &World) {
    state.ext.worlds.borrow_mut().remove(world);
    state.ext.allowed.borrow_mut().retain(|(w, _), _| w != world);
    if state.ext.handlers.borrow().contains(world) {
        content.unregister_script_message_handler(world.name(), Some(world.name()));
    }
    state.ext.handlers.borrow_mut().retain(|w| w != world);
    let (gone, kept): (Vec<_>, Vec<_>) = state.ext.signals.take().into_iter().partition(|(w, _)| w == world);
    *state.ext.signals.borrow_mut() = kept;
    gone.into_iter().for_each(|(_, id)| content.disconnect(id));
    state.ext.tokens.borrow_mut().remove(world);
}

pub(super) fn attach(state: &Rc<State>) {
    let Some(content) = state.content.borrow().clone() else { return };
    let store = StoreKey::parse(&state.profile.borrow()).and_then(|key| store_of(&key, &state.tab.borrow(), state.level.get()));
    let (worlds, rules) = store
        .as_ref()
        .and_then(|id| STORES.with_borrow(|s| s.get(id).map(|s| (s.worlds.clone(), s.rules.clone()))))
        .filter(|(w, _)| !w.is_empty())
        .unwrap_or_default();
    *state.ext.store.borrow_mut() = store.filter(|_| !worlds.is_empty());
    let keep: Vec<World> = worlds.iter().map(|w| w.world.clone()).collect();
    let gone: Vec<World> = state.ext.worlds.borrow().keys().filter(|w| !keep.contains(w)).cloned().collect();
    gone.iter().for_each(|w| remove_world(state, &content, w));
    for scripts in worlds {
        register(state, &content, &scripts.world);
        state.ext.worlds.borrow_mut().insert(scripts.world.clone(), scripts);
    }
    let wanted: Vec<String> = rules.iter().map(|(id, _)| id.clone()).collect();
    let old: Vec<String> = state.ext.filters.borrow().iter().filter(|id| !wanted.contains(id)).cloned().collect();
    old.iter().for_each(|id| content.remove_filter_by_id(id));
    state.ext.filters.borrow_mut().retain(|id| wanted.contains(id));
    for (id, source) in rules {
        if state.ext.filters.borrow().contains(&id) {
            continue;
        }
        state.ext.filters.borrow_mut().push(id.clone());
        let weak = Rc::downgrade(state);
        let wanted = id.clone();
        filters::compile_one(id, source, move |filter| {
            let Some(state) = weak.upgrade() else { return };
            if state.ext.filters.borrow().contains(&wanted)
                && let Some(content) = state.content.borrow().as_ref()
            {
                content.add_filter(&filter);
            }
        });
    }
    sync(state);
}

pub(super) fn detach(state: &State) {
    for (_, (decision, _, _)) in state.ext.held.borrow_mut().drain() {
        decision.ignore();
    }
    state.ext.handlers.borrow_mut().clear();
    state.ext.signals.borrow_mut().clear();
    state.ext.tokens.borrow_mut().clear();
    state.ext.main_worlds.borrow_mut().clear();
    state.ext.added.borrow_mut().clear();
    state.ext.filters.borrow_mut().clear();
    state.ext.allowed.borrow_mut().clear();
}

fn live_views() -> Vec<Rc<State>> {
    VIEWS.with_borrow(|views| views.iter().filter_map(std::rc::Weak::upgrade).collect())
}

fn refresh(store: &str) {
    for state in live_views() {
        let key = StoreKey::parse(&state.profile.borrow());
        if matches!(key, Some(StoreKey::Persistent(id)) if id == store) {
            attach(&state);
        }
    }
}

fn receive(json: &str) -> String {
    format!("if (typeof globalThis.{RECEIVE} === 'function') globalThis.{RECEIVE}({});", serde_json::to_string(json).unwrap_or_default())
}

/// Calls `__tarnExtReceive(json)` in `world` of the main frame of the views of `tab` (every
/// view for `None`); a subframe or a world the view does not run is dropped.
pub fn post(tab: Option<String>, world: World, frame: Option<FrameId>, json: String) {
    on_gtk(move || {
        if frame.is_some_and(|f| f != FrameId::MAIN) {
            return;
        }
        let script = receive(&json);
        for state in live_views().into_iter().filter(|s| tab.as_ref().is_none_or(|t| *s.tab.borrow() == *t)) {
            if let Some(web) = state.web().filter(|_| state.ext.worlds.borrow().contains_key(&world)) {
                web.evaluate_javascript(&script, Some(world.name()), None, None::<&gio::Cancellable>, |_| {});
            }
        }
    });
}

/// A shell-owned view of an extension's own page (`tarn-extension://<origin>/`): hidden for the
/// background, shown for the popup. The shim runs in its main world, which only the
/// extension's own documents reach.
pub struct PageView {
    web: WebView,
    origin: String,
    resources: Resources,
}

impl Drop for PageView {
    fn drop(&mut self) {
        self.web.stop_loading();
        if let Some(parent) = self.web.parent().and_then(|p| p.downcast::<gtk::Box>().ok()) {
            parent.remove(&self.web);
        }
        self.web.try_close();
    }
}

/// Calls `__tarnExtReceive(json)` in the main world of page `page`.
pub fn post_page(page: u64, json: String) {
    on_gtk(move || {
        let Some(view) = PAGES.with_borrow(|p| p.get(&page).cloned().flatten()) else { return };
        view.web.evaluate_javascript(&receive(&json), None, None, None::<&gio::Cancellable>, |_| {});
    });
}

pub fn close_page(page: u64) {
    on_gtk(move || {
        SHOWN.with_borrow_mut(|s| s.remove(&page));
        drop(PAGES.with_borrow_mut(|p| p.remove(&page)));
    });
}

fn place(page: u64, view: &PageView) {
    let Some(container) = SHOWN.with_borrow_mut(|s| s.remove(&page)).and_then(|c| c.upgrade()) else { return };
    if view.web.parent().is_none() {
        view.web.set_hexpand(true);
        view.web.set_vexpand(true);
        container.append(&view.web);
    }
}

/// Shows page `page` in `container` now, or once it is made.
pub fn show_page(page: u64, container: &gtk::Box) {
    SHOWN.with_borrow_mut(|s| s.insert(page, container.downgrade()));
    if let Some(view) = PAGES.with_borrow(|p| p.get(&page).cloned().flatten()) {
        place(page, &view);
    }
}

fn loads(url: &str, origin: &str, main_frame: bool) -> bool {
    Url::parse(url).is_ok_and(|u| page_loads(&u, origin, main_frame))
}

fn respond(request: &URISchemeRequest, status: u16, mime: &str, csp: Option<&str>, body: Vec<u8>) {
    let length = body.len() as i64;
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(body));
    let response = URISchemeResponse::new(&stream, length);
    response.set_status(u32::from(status), None);
    response.set_content_type(mime);
    let headers = soup::MessageHeaders::new(soup::MessageHeadersType::Response);
    headers.append("Content-Type", mime);
    if let Some(csp) = csp {
        headers.append("Content-Security-Policy", csp);
    }
    headers.append("X-Content-Type-Options", "nosniff");
    response.set_http_headers(headers);
    request.finish_with_response(&response);
}

fn serve(request: &URISchemeRequest) {
    let url = request.uri().and_then(|u| Url::parse(&u).ok());
    let viewer = request.web_view();
    let page = PAGES.with_borrow(|pages| {
        pages.values().flatten().find(|p| Some(&p.web) == viewer.as_ref() && url.as_ref().and_then(Url::host_str) == Some(p.origin.as_str())).cloned()
    });
    match (page, url) {
        (Some(page), Some(url)) => {
            let r = (page.resources)(&url);
            respond(request, r.status, r.mime, r.csp, r.body);
        }
        _ => respond(request, 403, "text/plain", Some(browser_core::tarn_ext::resources::PAGE_CSP), Vec::new()),
    }
}

fn register_scheme() {
    if SCHEME_READY.replace(true) {
        return;
    }
    let Some(context) = WebContext::default() else { return };
    if let Some(security) = context.security_manager() {
        security.register_uri_scheme_as_secure(SCHEME);
        security.register_uri_scheme_as_display_isolated(SCHEME);
    }
    context.register_uri_scheme(SCHEME, serve);
}

fn page_transport() -> String {
    format!(
        "(function () {{ const h = window.top === window && window.webkit && window.webkit.messageHandlers ? window.webkit.messageHandlers.{PAGE_HANDLER} : null; \
         globalThis.__tarnExtPost = s => {{ if (!h) throw new Error('unavailable'); h.postMessage(String(s)); }}; }})();\n"
    )
}

pub struct PageSpec {
    pub key: StoreKey,
    pub level: ShieldLevel,
    pub lists: Vec<String>,
    pub origin: String,
    pub url: Url,
    pub bootstrap: String,
    pub resources: Resources,
}

/// Makes page `page` from `spec`; its messages go to the shell as `Event::PageMessage`.
pub fn open_page(page: u64, spec: PageSpec) {
    on_gtk(move || make_page(page, spec));
}

/// The page could not be made: it goes, and the shell hears so.
fn failed(id: u64) {
    SHOWN.with_borrow_mut(|s| s.remove(&id));
    drop(PAGES.with_borrow_mut(|p| p.remove(&id)));
    tell(Event::PageFailed { page: id });
}

fn make_page(id: u64, spec: PageSpec) {
    register_scheme();
    PAGES.with_borrow_mut(|p| p.insert(id, None));
    let PageSpec { key, level, lists, origin, url, bootstrap, resources } = spec;
    filters::with_filters(level, lists, move |result| {
        if !PAGES.with_borrow(|p| p.contains_key(&id)) {
            return;
        }
        let Ok(rules) = result else { return failed(id) };
        let Some(session) = super::session::acquire(&key) else { return failed(id) };
        let content = UserContentManager::new();
        rules.iter().for_each(|f| content.add_filter(f));
        let source = format!("{}{bootstrap}", page_transport());
        let own_pages = format!("{SCHEME}://{origin}/*");
        content.add_script(&UserScript::new(&source, UserContentInjectedFrames::TopFrame, UserScriptInjectionTime::Start, &[own_pages.as_str()], &[]));
        content.register_script_message_handler(PAGE_HANDLER, None);
        let web = WebView::builder().network_session(&session).user_content_manager(&content).build();
        if let Some(settings) = WebViewExt::settings(&web) {
            settings.set_enable_javascript(true);
            settings.set_javascript_can_open_windows_automatically(false);
            settings.set_enable_developer_extras(false);
        }
        let page = Rc::new(PageView { web: web.clone(), origin: origin.clone(), resources });
        PAGES.with_borrow_mut(|pages| pages.insert(id, Some(page.clone())));
        let base = origin.clone();
        web.connect_decide_policy(move |_, decision, kind| {
            let uri = decision
                .downcast_ref::<NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
                .and_then(|a| a.request())
                .and_then(|r| r.uri())
                .map(|u| u.to_string());
            match kind {
                PolicyDecisionType::NavigationAction if uri.as_deref().is_some_and(|u| loads(u, &base, false)) => false,
                PolicyDecisionType::NavigationAction | PolicyDecisionType::NewWindowAction => {
                    decision.ignore();
                    true
                }
                _ => false,
            }
        });
        let base = origin.clone();
        web.connect_load_changed(move |web, event| {
            if event == webkit6::LoadEvent::Committed && !web.uri().is_some_and(|u| loads(&u, &base, true)) {
                web.stop_loading();
                on_gtk(move || failed(id));
            }
        });
        let viewer = web.downgrade();
        let base = origin.clone();
        content.connect_script_message_received(Some(PAGE_HANDLER), move |_, value| {
            let on_own_page = viewer.upgrade().and_then(|w| w.uri()).is_some_and(|u| loads(&u, &base, true));
            if on_own_page && value.is_string() {
                tell(Event::PageMessage { page: id, json: value.to_str().to_string() });
            }
        });
        web.load_uri(url.as_str());
        place(id, &page);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_is_allowed_by_its_exact_address_until_the_gate_says_no() {
        let url = |s: &str| Url::parse(s).unwrap();
        let ok = patterns(&url("https://a.test:8443/p?ok=1#top")).unwrap();
        assert_eq!(ok, ["https://a.test/p?ok=1#*", "https://a.test/p?ok=1"]);
        assert!(patterns(&url("https://a.test/*x")).is_none() && patterns(&url("file:///p")).is_none());
        let private = patterns(&url("https://a.test/p?private=1")).unwrap();
        let world = World::for_install("ab12").unwrap();
        let mut allowed = BTreeMap::new();
        assert!(allow(&mut allowed, (world.clone(), 0), &ok, true));
        assert!(!allow(&mut allowed, (world.clone(), 0), &private, false), "another query was never allowed");
        assert_eq!(allowed[&(world.clone(), 0)], ok);
        assert!(allow(&mut allowed, (world.clone(), 0), &ok, false));
        assert!(allowed.is_empty(), "no empty allow-list is left, which would match every page");
    }
}
