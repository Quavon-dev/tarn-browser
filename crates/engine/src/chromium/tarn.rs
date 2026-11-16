//! Chromium's side of Tarn extensions (`tarn_ext`), over the in-process DevTools channel only
//! (S22): each install's scripts are new-document scripts in an isolated world named after it, its
//! messages arrive through a binding that exists only in that world, and replies are evaluated in
//! the world's context of the frame they go to.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::{Mutex, PoisonError};

use cef::*;
use url::Url;

use super::{ADD_SCRIPT, Tab, call, emit, string_at, value_at};
use crate::Event;
use crate::handover::js_string;
use crate::tarn_ext::{
    Decisions, FrameId, FrameNav, Gate, HELLO, Injection, PageSpec, RECEIVE, RunAt, SCHEME, World, WorldScripts, content_injections, injections,
};

/// The browsers of extension page views, by CEF's browser id, with the origin each may load.
static PAGE_BROWSERS: Mutex<BTreeMap<i32, String>> = Mutex::new(BTreeMap::new());

#[derive(Default)]
pub(super) struct TarnTab {
    gate: Option<Gate>,
    worlds: BTreeMap<World, WorldState>,
    /// The install whose own page this view shows; its bootstrap runs in the main world.
    pub(super) page: Option<PageSpec>,
    /// The main world's contexts, by the DevTools frame they belong to.
    page_contexts: HashMap<i32, String>,
    /// DevTools' id of the tab's main frame.
    main_frame: Option<String>,
}

struct WorldState {
    scripts: Rc<WorldScripts>,
    decisions: Decisions,
    registered: Vec<String>,
    generation: u64,
    contexts: HashMap<i32, Context>,
}

/// A context of the world as DevTools reported it, never as its scripts say.
struct Context {
    frame: String,
    origin: Option<Url>,
    ended: bool,
}

fn binding(world: &World) -> String {
    format!("__tarnExt_{}", world.install_id())
}

/// Takes the binding out of the world's global before any content script runs and asks for the
/// end-of-document scripts.
fn setup(world: &World) -> String {
    format!(
        "const tarnBind = globalThis.{b}; try {{ delete globalThis.{b}; }} catch (e) {{}} if (typeof tarnBind !== 'function') return; \
         globalThis.__tarnExtPost = s => tarnBind(String(s)); \
         document.addEventListener('DOMContentLoaded', () => tarnBind('\\u0001end'), {{ once: true }});",
        b = binding(world)
    )
}

/// The page view's main world, in its top document of its own origin only.
fn page_setup(spec: &PageSpec) -> String {
    format!(
        "(() => {{ const tarnBind = globalThis.{b}; try {{ delete globalThis.{b}; }} catch (e) {{}} \
         if (typeof tarnBind !== 'function' || window !== window.top || location.origin !== {origin}) return; \
         globalThis.__tarnExtPost = s => tarnBind(String(s));\n{boot}}})();",
        b = binding(&spec.world),
        origin = js_string(&format!("{SCHEME}://{}", spec.origin)),
        boot = spec.bootstrap,
    )
}

fn host_of(tab: &Tab) -> Option<BrowserHost> {
    tab.browser.borrow().as_ref()?.host()
}

pub(super) fn set_gate(tab: &Tab, gate: Gate) {
    tab.tarn.borrow_mut().gate = Some(gate);
}

pub(super) fn add(tab: &Rc<Tab>, scripts: Rc<WorldScripts>) {
    let world = scripts.world.clone();
    {
        let mut tarn = tab.tarn.borrow_mut();
        match tarn.worlds.get_mut(&world) {
            Some(state) if *state.scripts == *scripts => return,
            Some(state) => {
                state.scripts = scripts;
                state.decisions = Decisions::default();
            }
            None => {
                let state = WorldState { scripts, decisions: Decisions::default(), registered: Vec::new(), generation: 0, contexts: HashMap::new() };
                tarn.worlds.insert(world.clone(), state);
            }
        }
    }
    if let Some(host) = host_of(tab) {
        bind(tab, &host, &world);
        register(tab, &host, &world);
    }
}

pub(super) fn remove(tab: &Rc<Tab>, world: &World) {
    let Some(state) = tab.tarn.borrow_mut().worlds.remove(world) else { return };
    let Some(host) = host_of(tab) else { return };
    for identifier in state.registered {
        super::remove_script(tab, &host, &identifier);
    }
    call(tab, &host, "Runtime.removeBinding", &format!("{{\"name\":{}}}", js_string(&binding(world))), |_| {});
}

/// The Runtime domain must be on for a binding to reach contexts made after it, as every new
/// document's are; it is only on in tabs of a profile with Tarn extensions.
fn bind(tab: &Rc<Tab>, host: &BrowserHost, world: &World) {
    enable(tab, host);
    let params = format!("{{\"name\":{},\"executionContextName\":{}}}", js_string(&binding(world)), js_string(world.name()));
    call(tab, host, "Runtime.addBinding", &params, |_| {});
}

/// Everything for the tab's first page, sent before the steps its first load waits for.
pub(super) fn first(tab: &Rc<Tab>, host: &BrowserHost) -> bool {
    let worlds: Vec<World> = tab.tarn.borrow().worlds.keys().cloned().collect();
    for world in &worlds {
        bind(tab, host, world);
        register(tab, host, world);
    }
    let page = tab.tarn.borrow().page.clone();
    if let Some(spec) = &page {
        enable(tab, host);
        call(tab, host, "Runtime.addBinding", &format!("{{\"name\":{}}}", js_string(&binding(&spec.world))), |_| {});
        call(tab, host, ADD_SCRIPT, &format!("{{\"source\":{}}}", js_string(&page_setup(spec))), |_| {});
    }
    !worlds.is_empty() || page.is_some()
}

/// Turns on the domains whose events say which frame and origin each context belongs to, and
/// learns the main frame's id.
fn enable(tab: &Rc<Tab>, host: &BrowserHost) {
    call(tab, host, "Page.enable", "{}", |_| {});
    call(tab, host, "Runtime.enable", "{}", |_| {});
    let weak = Rc::downgrade(tab);
    call(tab, host, "Page.getFrameTree", "{}", move |tree| {
        if let (Some(tab), Some(id)) = (weak.upgrade(), string_at(tree, &["frameTree", "frame", "id"])) {
            tab.tarn.borrow_mut().main_frame = Some(id);
        }
    });
}

/// Replaces the world's new-document scripts with ones carrying its current decisions; replies to
/// an earlier generation remove what they registered.
fn register(tab: &Rc<Tab>, host: &BrowserHost, world: &World) {
    let (old, generation, list) = {
        let mut tarn = tab.tarn.borrow_mut();
        let Some(state) = tarn.worlds.get_mut(world) else { return };
        state.generation += 1;
        let list: Vec<Injection> =
            injections(&state.scripts, &state.decisions, &setup(world)).into_iter().filter(|i| i.run_at == RunAt::DocumentStart).collect();
        (std::mem::take(&mut state.registered), state.generation, list)
    };
    for identifier in old {
        super::remove_script(tab, host, &identifier);
    }
    for injection in list {
        let params = format!("{{\"source\":{},\"worldName\":{}}}", js_string(&injection.source), js_string(world.name()));
        let (weak, world) = (Rc::downgrade(tab), world.clone());
        call(tab, host, ADD_SCRIPT, &params, move |result| {
            let (Some(tab), Some(identifier)) = (weak.upgrade(), string_at(result, &["identifier"])) else { return };
            let current =
                tab.tarn.borrow_mut().worlds.get_mut(&world).filter(|s| s.generation == generation).map(|s| s.registered.push(identifier.clone())).is_some();
            if !current && let Some(host) = host_of(&tab) {
                super::remove_script(&tab, &host, &identifier);
            }
        });
    }
}

/// A frame is about to navigate: the gate decides now, and the world's scripts follow before
/// Chromium is answered. A document whose decision has not arrived runs nothing.
pub(super) fn before_browse(tab: &Rc<Tab>, url: &Url, top: Option<&Url>, main_frame: bool) {
    let Some(gate) = tab.tarn.borrow().gate.clone() else { return };
    let Some(top) = (if main_frame { Some(url) } else { top }) else { return };
    let frame = FrameNav { url, top, main_frame };
    let changed: Vec<World> = {
        let mut tarn = tab.tarn.borrow_mut();
        tarn.worlds
            .iter_mut()
            .filter_map(|(world, state)| {
                let run: Vec<usize> = gate(world, &frame).into_iter().filter(|ix| *ix < state.scripts.scripts.len()).collect();
                state.decisions.record(&frame, run).then(|| world.clone())
            })
            .collect()
    };
    if let Some(host) = host_of(tab) {
        for world in changed {
            register(tab, &host, &world);
        }
    }
}

/// Whether an extension page view may load `url`: its own origin only, or a blank frame in it.
pub(super) fn page_allows(tab: &Tab, url: &Url, main_frame: bool) -> Option<bool> {
    let tarn = tab.tarn.borrow();
    let spec = tarn.page.as_ref()?;
    Some(crate::tarn_ext::page_loads(url, &spec.origin, main_frame))
}

fn frame_of(main: bool, context: i32) -> FrameId {
    if main { FrameId::MAIN } else { FrameId(u64::from(context.unsigned_abs()) | 1 << 62) }
}

fn int_at(params: Option<Value>, path: &[&str]) -> Option<i32> {
    value_at(params, path).filter(|v| v.get_type() == ValueType::INT).map(|v| v.int())
}

/// The DevTools events that say where each context is, and the bindings' calls.
pub(super) fn devtools_event(tab: &Rc<Tab>, method: &str, params: Option<Value>) {
    match method {
        "Runtime.bindingCalled" => binding_called(tab, params),
        "Runtime.executionContextCreated" => context_created(tab, params),
        "Runtime.executionContextDestroyed" => {
            if let Some(context) = int_at(params, &["executionContextId"]) {
                forget_contexts(tab, Some(context));
            }
        }
        "Runtime.executionContextsCleared" => forget_contexts(tab, None),
        "Page.frameNavigated" if string_at(params.clone(), &["frame", "parentId"]).is_none() => {
            if let Some(id) = string_at(params, &["frame", "id"]) {
                tab.tarn.borrow_mut().main_frame = Some(id);
            }
        }
        _ => {}
    }
}

fn context_created(tab: &Tab, params: Option<Value>) {
    let Some(id) = int_at(params.clone(), &["context", "id"]) else { return };
    let (Some(name), Some(frame)) = (string_at(params.clone(), &["context", "name"]), string_at(params.clone(), &["context", "auxData", "frameId"])) else {
        return;
    };
    let default = value_at(params.clone(), &["context", "auxData", "isDefault"]).filter(|v| v.get_type() == ValueType::BOOL).is_some_and(|v| v.bool() == 1);
    let origin = string_at(params, &["context", "origin"]).and_then(|o| Url::parse(&o).ok()).filter(|u| matches!(u.scheme(), "http" | "https"));
    let mut tarn = tab.tarn.borrow_mut();
    if default && tarn.page.is_some() {
        tarn.page_contexts.insert(id, frame);
    } else if let Some(state) = tarn.worlds.iter_mut().find(|(w, _)| w.name() == name).map(|(_, s)| s) {
        state.contexts.insert(id, Context { frame, origin, ended: false });
    }
}

fn forget_contexts(tab: &Tab, context: Option<i32>) {
    let mut tarn = tab.tarn.borrow_mut();
    let gone = |c: &i32| context.is_none_or(|x| x == *c);
    tarn.page_contexts.retain(|c, _| !gone(c));
    for state in tarn.worlds.values_mut() {
        state.contexts.retain(|c, _| !gone(c));
    }
}

/// `Runtime.bindingCalled`: the world is the binding's, the frame and origin those DevTools gave
/// the context it was called in; a context DevTools did not report sends nothing.
fn binding_called(tab: &Rc<Tab>, params: Option<Value>) {
    let (Some(name), Some(payload)) = (string_at(params.clone(), &["name"]), string_at(params.clone(), &["payload"])) else { return };
    let Some(context) = int_at(params, &["executionContextId"]) else { return };
    let hello = payload.strip_prefix(HELLO);
    let page_world = tab.tarn.borrow().page.as_ref().filter(|s| binding(&s.world) == name).map(|s| s.world.clone());
    if let Some(world) = page_world {
        let main = {
            let tarn = tab.tarn.borrow();
            tarn.page_contexts.get(&context).is_some_and(|f| tarn.main_frame.as_ref() == Some(f))
        };
        if main && hello.is_none() && payload.starts_with('{') {
            emit(tab, Event::ExtensionMessage { world, frame: FrameId::MAIN, origin: None, json: payload });
        }
        return;
    }
    let Some(world) = tab.tarn.borrow().worlds.keys().find(|w| binding(w) == name).cloned() else { return };
    let mut evaluate = Vec::new();
    let message = {
        let mut tarn = tab.tarn.borrow_mut();
        let main_frame = tarn.main_frame.clone();
        let Some(state) = tarn.worlds.get_mut(&world) else { return };
        let Some(known) = state.contexts.get_mut(&context) else { return };
        let main = main_frame.as_ref() == Some(&known.frame);
        match hello {
            Some("end") if !known.ended => {
                known.ended = true;
                evaluate = content_injections(&state.scripts).into_iter().filter(|i| i.run_at != RunAt::DocumentStart).map(|i| i.source).collect();
                None
            }
            Some(_) => None,
            None => payload.starts_with('{').then(|| (frame_of(main, context), if main { None } else { known.origin.clone() })),
        }
    };
    if let Some((frame, origin)) = message {
        emit(tab, Event::ExtensionMessage { world, frame, origin, json: payload });
    }
    if let Some(host) = host_of(tab) {
        for source in evaluate {
            run_in(tab, &host, context, &source);
        }
    }
}

fn run_in(tab: &Tab, host: &BrowserHost, context: i32, source: &str) {
    let params = format!("{{\"expression\":{},\"contextId\":{context},\"returnByValue\":true,\"silent\":true}}", js_string(source));
    call(tab, host, "Runtime.evaluate", &params, |_| {});
}

pub(super) fn post(tab: &Rc<Tab>, world: &World, frame: Option<FrameId>, json: &str) {
    let Some(host) = host_of(tab) else { return };
    let source = format!("globalThis.{RECEIVE} && globalThis.{RECEIVE}({})", js_string(json));
    let contexts: Vec<i32> = {
        let tarn = tab.tarn.borrow();
        if tarn.page.as_ref().is_some_and(|s| s.world == *world) {
            tarn.page_contexts.iter().filter(|(_, f)| tarn.main_frame.as_ref() == Some(*f)).map(|(c, _)| *c).collect()
        } else {
            let Some(state) = tarn.worlds.get(world) else { return };
            let main = |known: &Context| tarn.main_frame.as_ref() == Some(&known.frame);
            state.contexts.iter().filter(|(c, known)| frame.is_none_or(|f| f == frame_of(main(known), **c))).map(|(c, _)| *c).collect()
        }
    };
    for context in contexts {
        run_in(tab, &host, context, &source);
    }
}

/// The browser of an extension page view was made: it alone may load its origin.
pub(super) fn page_created(tab: &Tab, browser: &Browser) {
    if let Some(spec) = tab.tarn.borrow().page.as_ref() {
        PAGE_BROWSERS.lock().unwrap_or_else(PoisonError::into_inner).insert(browser.identifier(), spec.origin.clone());
    }
}

pub(super) fn page_closed(browser: &Browser) {
    PAGE_BROWSERS.lock().unwrap_or_else(PoisonError::into_inner).remove(&browser.identifier());
}

/// Serves an extension page view's own origin from its profile's handler, off the UI thread, to
/// that view's browser only: the request context is the profile's, shared with its web tabs.
pub(super) fn register_scheme(context: &mut RequestContext, spec: &PageSpec) {
    let mut factory = TarnSchemeFactory::new(spec.origin.clone(), spec.resources.clone());
    context.register_scheme_handler_factory(Some(&CefString::from(SCHEME)), Some(&CefString::from(spec.origin.as_str())), Some(&mut factory));
}

wrap_scheme_handler_factory! {
    struct TarnSchemeFactory {
        origin: String,
        resources: crate::tarn_ext::Resources,
    }

    impl SchemeHandlerFactory {
        fn create(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _scheme_name: Option<&CefString>, request: Option<&mut Request>) -> Option<ResourceHandler> {
            let viewer = browser.and_then(|b| PAGE_BROWSERS.lock().unwrap_or_else(PoisonError::into_inner).get(&b.identifier()).cloned());
            let url = request.and_then(|r| Url::parse(&CefString::from(&r.url()).to_string()).ok());
            let resource = match url.filter(|u| u.scheme() == SCHEME && u.host_str() == Some(self.origin.as_str()) && viewer.as_deref() == Some(self.origin.as_str())) {
                Some(url) => (self.resources)(&url),
                None => crate::tarn_ext::Resource { status: 404, mime: "text/plain", csp: None, body: Vec::new() },
            };
            Some(TarnResource::new(resource, std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0))))
        }
    }
}

wrap_resource_handler! {
    struct TarnResource {
        resource: crate::tarn_ext::Resource,
        sent: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl ResourceHandler {
        fn open(&self, _request: Option<&mut Request>, handle_request: Option<&mut ::std::os::raw::c_int>, _callback: Option<&mut Callback>) -> ::std::os::raw::c_int {
            if let Some(handle) = handle_request {
                *handle = 1;
            }
            1
        }

        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect_url: Option<&mut CefString>) {
            if let Some(response) = response {
                response.set_status(i32::from(self.resource.status));
                response.set_mime_type(Some(&CefString::from(self.resource.mime)));
                if self.resource.mime.starts_with("text/") || self.resource.mime == "application/json" {
                    response.set_charset(Some(&CefString::from("utf-8")));
                }
                let mut headers = vec![("Cache-Control", "no-store"), ("X-Content-Type-Options", "nosniff")];
                headers.extend(self.resource.csp.map(|csp| ("Content-Security-Policy", csp)));
                for (name, value) in headers {
                    response.set_header_by_name(Some(&CefString::from(name)), Some(&CefString::from(value)), 1);
                }
            }
            if let Some(length) = response_length {
                *length = i64::try_from(self.resource.body.len()).unwrap_or(i64::MAX);
            }
        }

        fn read(&self, data_out: *mut u8, bytes_to_read: ::std::os::raw::c_int, bytes_read: Option<&mut ::std::os::raw::c_int>, _callback: Option<&mut ResourceReadCallback>) -> ::std::os::raw::c_int {
            use std::sync::atomic::Ordering;
            let start = self.sent.load(Ordering::Relaxed);
            let rest = self.resource.body.get(start..).unwrap_or_default();
            let n = rest.len().min(usize::try_from(bytes_to_read).unwrap_or(0));
            if n == 0 || data_out.is_null() {
                if let Some(read) = bytes_read {
                    *read = 0;
                }
                return 0;
            }
            // SAFETY: CEF hands a buffer of `bytes_to_read` writable bytes; `n` is at most that and
            // at most what is left of the body, and the two never overlap.
            unsafe { std::ptr::copy_nonoverlapping(rest.as_ptr(), data_out, n) };
            self.sent.store(start + n, Ordering::Relaxed);
            if let Some(read) = bytes_read {
                *read = i32::try_from(n).unwrap_or(0);
            }
            1
        }
    }
}
