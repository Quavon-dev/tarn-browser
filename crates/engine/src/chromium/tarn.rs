//! Chromium's side of Tarn extensions (`tarn_ext`), over the in-process DevTools channel only
//! (S22): each install's scripts are new-document scripts in an isolated world named after it, its
//! messages arrive through a binding that exists only in that world, and replies are evaluated in
//! the world's context of the frame they go to. Frames in other processes are targets of their
//! own, attached over the same channel (`Target.setAutoAttach`), where the same happens.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::{Mutex, PoisonError};

use cef::*;
use url::Url;

use super::{ADD_SCRIPT, Tab, call, call_in, emit, string_at, value_at};
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
    sessions: Sessions,
    contexts_seen: u64,
    worlds_made: u64,
}

/// Where a DevTools message goes or came from: an attached frame's session, or the page itself.
type Target = Option<String>;

/// The sessions of the frames in other processes, with the target each was attached from: those
/// announced, whose setup waits for the event's dispatch to end, and those set up.
#[derive(Default)]
struct Sessions {
    announced: BTreeMap<String, Target>,
    live: BTreeMap<String, Target>,
}

impl Sessions {
    fn announce(&mut self, session: String, parent: Target) {
        self.announced.insert(session, parent);
    }

    /// Sets up an announced session, kept when `keep`; false when it ended before.
    fn start(&mut self, session: &str, keep: bool) -> bool {
        let Some(parent) = self.announced.remove(session) else { return false };
        if keep {
            self.live.insert(session.to_owned(), parent);
        }
        true
    }

    /// Ends the session and those attached through it, announced or set up; returns them.
    fn end(&mut self, session: String) -> Vec<Target> {
        let mut gone: Vec<Target> = vec![Some(session)];
        while let Some(next) =
            self.live.iter().chain(&self.announced).find(|(s, parent)| gone.contains(parent) && !gone.contains(&Some((*s).clone()))).map(|(s, _)| s.clone())
        {
            gone.push(Some(next));
        }
        self.live.retain(|s, _| !gone.contains(&Some(s.clone())));
        self.announced.retain(|s, _| !gone.contains(&Some(s.clone())));
        gone
    }
}

struct WorldState {
    scripts: Rc<WorldScripts>,
    decisions: Decisions,
    registered: Vec<(Target, String)>,
    /// Per target, the replies its last registration still owes and whether newer decisions wait
    /// for them.
    waiting: BTreeMap<Target, (usize, bool)>,
    generation: u64,
    contexts: HashMap<(Target, i32), Context>,
}

impl WorldState {
    /// One reply to `target`'s registration arrived; `Some(true)` when the target must be
    /// registered again, `None` when it is not waiting (it went away).
    fn added(&mut self, target: &Target, identifier: Option<String>) -> Option<bool> {
        let (owed, stale) = self.waiting.get_mut(target)?;
        *owed = owed.saturating_sub(1);
        self.registered.extend(identifier.map(|i| (target.clone(), i)));
        Some(*owed == 0 && std::mem::take(stale))
    }
}

/// A context of the world as DevTools reported it, never as its scripts say; `serial` numbers it
/// across the tab's targets, whose context ids overlap.
struct Context {
    frame: String,
    origin: Option<Url>,
    ended: bool,
    serial: u64,
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
                tarn.worlds_made += 1;
                let generation = tarn.worlds_made;
                let state = WorldState {
                    scripts,
                    decisions: Decisions::default(),
                    registered: Vec::new(),
                    waiting: BTreeMap::new(),
                    generation,
                    contexts: HashMap::new(),
                };
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
    for (target, identifier) in state.registered {
        remove_script(tab, &host, target.as_deref(), &identifier);
    }
    let last = tab.tarn.borrow().worlds.is_empty() && tab.tarn.borrow().page.is_none();
    for target in targets(tab) {
        call_in(tab, &host, target.as_deref(), "Runtime.removeBinding", &format!("{{\"name\":{}}}", js_string(&binding(world))), |_| {});
        if last {
            call_in(tab, &host, target.as_deref(), "Target.setAutoAttach", r#"{"autoAttach":false,"waitForDebuggerOnStart":false}"#, |_| {});
            call_in(tab, &host, target.as_deref(), "Runtime.disable", "{}", |_| {});
        }
    }
}

fn remove_script(tab: &Tab, host: &BrowserHost, target: Option<&str>, identifier: &str) {
    call_in(tab, host, target, "Page.removeScriptToEvaluateOnNewDocument", &format!("{{\"identifier\":{}}}", js_string(identifier)), |_| {});
}

/// The page and every attached frame.
fn targets(tab: &Tab) -> Vec<Target> {
    std::iter::once(None).chain(tab.tarn.borrow().sessions.live.keys().cloned().map(Some)).collect()
}

/// The Runtime domain must be on for a binding to reach contexts made after it, as every new
/// document's are (without it no `Runtime.bindingCalled` arrives from a new document); it is
/// only on in tabs of a profile with Tarn extensions.
fn bind(tab: &Rc<Tab>, host: &BrowserHost, world: &World) {
    enable(tab, host);
    for target in targets(tab) {
        bind_in(tab, host, target.as_deref(), world);
    }
}

fn bind_in(tab: &Tab, host: &BrowserHost, target: Option<&str>, world: &World) {
    let params = format!("{{\"name\":{},\"executionContextName\":{}}}", js_string(&binding(world)), js_string(world.name()));
    call_in(tab, host, target, "Runtime.addBinding", &params, |_| {});
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

/// Turns on the domains whose events say which frame and origin each context belongs to, has
/// frames in other processes attached before their first document, and learns the main frame's id.
fn enable(tab: &Rc<Tab>, host: &BrowserHost) {
    enable_in(tab, host, None);
    let weak = Rc::downgrade(tab);
    call(tab, host, "Page.getFrameTree", "{}", move |tree| {
        if let (Some(tab), Some(id)) = (weak.upgrade(), string_at(tree, &["frameTree", "frame", "id"])) {
            tab.tarn.borrow_mut().main_frame = Some(id);
        }
    });
}

fn enable_in(tab: &Tab, host: &BrowserHost, target: Option<&str>) {
    call_in(tab, host, target, "Page.enable", "{}", |_| {});
    call_in(tab, host, target, "Runtime.enable", "{}", |_| {});
    let attach = r#"{"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true,"filter":[{"type":"iframe"}]}"#;
    call_in(tab, host, target, "Target.setAutoAttach", attach, |_| {});
}

/// Replaces the world's new-document scripts in every target with ones carrying its current
/// decisions.
fn register(tab: &Rc<Tab>, host: &BrowserHost, world: &World) {
    for target in targets(tab) {
        register_in(tab, host, target, world);
    }
}

/// Replaces the world's new-document scripts in one target. Chromium runs them in the order they
/// were added, the gate first, so every decision brings all of them again; the old ones are removed
/// first, by the identifiers their replies gave, and while any reply is owed the swap waits for it,
/// so no document ever gets two generations (scripts twice, an older gate deciding).
fn register_in(tab: &Rc<Tab>, host: &BrowserHost, target: Target, world: &World) {
    let (generation, old, list) = {
        let mut tarn = tab.tarn.borrow_mut();
        let Some(state) = tarn.worlds.get_mut(world) else { return };
        if let Some((owed, stale)) = state.waiting.get_mut(&target)
            && *owed > 0
        {
            *stale = true;
            return;
        }
        let list: Vec<Injection> =
            injections(&state.scripts, &state.decisions, &setup(world)).into_iter().filter(|i| i.run_at == RunAt::DocumentStart).collect();
        let (old, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut state.registered).into_iter().partition(|(t, _)| *t == target);
        state.registered = kept;
        state.waiting.insert(target.clone(), (list.len(), false));
        (state.generation, old, list)
    };
    for (_, identifier) in old {
        remove_script(tab, host, target.as_deref(), &identifier);
    }
    for injection in list {
        let params = format!("{{\"source\":{},\"worldName\":{}}}", js_string(&injection.source), js_string(world.name()));
        let (weak, world, target) = (Rc::downgrade(tab), world.clone(), target.clone());
        call_in(tab, host, target.clone().as_deref(), ADD_SCRIPT, &params, move |result| {
            let Some(tab) = weak.upgrade() else { return };
            let identifier = string_at(result, &["identifier"]);
            let done = tab.tarn.borrow_mut().worlds.get_mut(&world).filter(|s| s.generation == generation).and_then(|s| s.added(&target, identifier.clone()));
            let Some(host) = host_of(&tab) else { return };
            match (done, identifier) {
                (Some(true), _) => register_in(&tab, &host, target, &world),
                (None, Some(identifier)) => remove_script(&tab, &host, target.as_deref(), &identifier),
                _ => {}
            }
        });
    }
}

/// A frame in another process was attached, paused before its first document: it gets the
/// worlds' bindings and scripts, and attaches its own such frames, before it goes on. Called
/// after the event's dispatch: a resume sent from inside it is lost and the frame never loads.
fn attach(tab: &Rc<Tab>, session: &str, params: Option<Value>) {
    let frame = string_at(params, &["targetInfo", "type"]).as_deref() == Some("iframe");
    if !tab.tarn.borrow_mut().sessions.start(session, frame) {
        return;
    }
    let Some(host) = host_of(tab) else { return };
    if frame {
        let worlds: Vec<World> = tab.tarn.borrow().worlds.keys().cloned().collect();
        enable_in(tab, &host, Some(session));
        for world in &worlds {
            bind_in(tab, &host, Some(session), world);
            register_in(tab, &host, Some(session.to_owned()), world);
        }
    }
    call_in(tab, &host, Some(session), "Runtime.runIfWaitingForDebugger", "{}", |_| {});
}

/// A frame's session ended, and with it the sessions attached through it.
fn detach(tab: &Tab, params: Option<Value>) {
    let Some(session) = string_at(params, &["sessionId"]) else { return };
    let mut tarn = tab.tarn.borrow_mut();
    let gone = tarn.sessions.end(session);
    for state in tarn.worlds.values_mut() {
        state.registered.retain(|(t, _)| !gone.contains(t));
        state.waiting.retain(|t, _| !gone.contains(t));
        state.contexts.retain(|(t, _), _| !gone.contains(t));
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

fn frame_of(main: bool, serial: u64) -> FrameId {
    if main { FrameId::MAIN } else { FrameId(serial | 1 << 62) }
}

fn int_at(params: Option<Value>, path: &[&str]) -> Option<i32> {
    value_at(params, path).filter(|v| v.get_type() == ValueType::INT).map(|v| v.int())
}

/// The only DevTools events read; others are dropped before their parameters are parsed.
pub(super) const EVENTS: [&str; 7] = [
    "Page.frameNavigated",
    "Runtime.bindingCalled",
    "Runtime.executionContextCreated",
    "Runtime.executionContextDestroyed",
    "Runtime.executionContextsCleared",
    "Target.attachedToTarget",
    "Target.detachedFromTarget",
];

/// The DevTools events of the page that say where each context is, and the bindings' calls.
pub(super) fn devtools_event(tab: &Rc<Tab>, method: &str, params: Option<Value>) {
    match method {
        "Page.frameNavigated" if string_at(params.clone(), &["frame", "parentId"]).is_none() => {
            if let Some(id) = string_at(params, &["frame", "id"]) {
                tab.tarn.borrow_mut().main_frame = Some(id);
            }
        }
        _ => target_event(tab, None, method, params),
    }
}

/// An event of an attached frame, with the session it came from.
pub(super) fn session_event(tab: &Rc<Tab>, message: Value) {
    let (Some(session), Some(method)) = (string_at(Some(message.clone()), &["sessionId"]), string_at(Some(message.clone()), &["method"])) else { return };
    target_event(tab, Some(session), &method, value_at(Some(message), &["params"]));
}

fn target_event(tab: &Rc<Tab>, target: Target, method: &str, params: Option<Value>) {
    match method {
        "Runtime.bindingCalled" => binding_called(tab, target, params),
        "Runtime.executionContextCreated" => context_created(tab, target, params),
        "Runtime.executionContextDestroyed" => {
            if let Some(context) = int_at(params, &["executionContextId"]) {
                forget_contexts(tab, &target, Some(context));
            }
        }
        "Runtime.executionContextsCleared" => forget_contexts(tab, &target, None),
        "Target.attachedToTarget" => {
            let Some(session) = string_at(params.clone(), &["sessionId"]) else { return };
            tab.tarn.borrow_mut().sessions.announce(session.clone(), target);
            let weak = Rc::downgrade(tab);
            super::super::runtime::after(std::time::Duration::ZERO, move || {
                if let Some(tab) = weak.upgrade() {
                    attach(&tab, &session, params);
                }
            });
        }
        "Target.detachedFromTarget" => detach(tab, params),
        _ => {}
    }
}

fn context_created(tab: &Tab, target: Target, params: Option<Value>) {
    let Some(id) = int_at(params.clone(), &["context", "id"]) else { return };
    let (Some(name), Some(frame)) = (string_at(params.clone(), &["context", "name"]), string_at(params.clone(), &["context", "auxData", "frameId"])) else {
        return;
    };
    let default = value_at(params.clone(), &["context", "auxData", "isDefault"]).filter(|v| v.get_type() == ValueType::BOOL).is_some_and(|v| v.bool() == 1);
    let isolated = string_at(params.clone(), &["context", "auxData", "type"]).as_deref() == Some("isolated");
    let origin = string_at(params, &["context", "origin"]).and_then(|o| Url::parse(&o).ok()).filter(|u| matches!(u.scheme(), "http" | "https"));
    let mut tarn = tab.tarn.borrow_mut();
    tarn.contexts_seen += 1;
    let serial = tarn.contexts_seen;
    if default && tarn.page.is_some() {
        if target.is_none() {
            tarn.page_contexts.insert(id, frame);
        }
    } else if !default
        && isolated
        && let Some(state) = tarn.worlds.iter_mut().find(|(w, _)| w.name() == name).map(|(_, s)| s)
    {
        state.contexts.insert((target, id), Context { frame, origin, ended: false, serial });
    }
}

fn forget_contexts(tab: &Tab, target: &Target, context: Option<i32>) {
    let mut tarn = tab.tarn.borrow_mut();
    let gone = |t: &Target, c: &i32| t == target && context.is_none_or(|x| x == *c);
    tarn.page_contexts.retain(|c, _| !gone(&None, c));
    for state in tarn.worlds.values_mut() {
        state.contexts.retain(|(t, c), _| !gone(t, c));
    }
}

/// `Runtime.bindingCalled`: the world is the binding's, the frame and origin those DevTools gave
/// the context it was called in; a context DevTools did not report sends nothing.
fn binding_called(tab: &Rc<Tab>, target: Target, params: Option<Value>) {
    let (Some(name), Some(payload)) = (string_at(params.clone(), &["name"]), string_at(params.clone(), &["payload"])) else { return };
    let Some(context) = int_at(params, &["executionContextId"]) else { return };
    let hello = payload.strip_prefix(HELLO);
    let page_world = tab.tarn.borrow().page.as_ref().filter(|s| binding(&s.world) == name).map(|s| s.world.clone());
    if let Some(world) = page_world {
        if target.is_some() {
            return;
        }
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
        let Some(known) = state.contexts.get_mut(&(target.clone(), context)) else { return };
        let main = target.is_none() && main_frame.as_ref() == Some(&known.frame);
        match hello {
            Some("end") if !known.ended => {
                known.ended = true;
                evaluate = content_injections(&state.scripts).into_iter().filter(|i| i.run_at != RunAt::DocumentStart).map(|i| i.source).collect();
                None
            }
            Some(_) => None,
            None => payload.starts_with('{').then(|| (frame_of(main, known.serial), known.origin.clone())),
        }
    };
    if let Some((frame, origin)) = message {
        emit(tab, Event::ExtensionMessage { world, frame, origin, json: payload });
    }
    if let Some(host) = host_of(tab) {
        for source in evaluate {
            run_in(tab, &host, target.as_deref(), context, &source);
        }
    }
}

fn run_in(tab: &Tab, host: &BrowserHost, target: Option<&str>, context: i32, source: &str) {
    let params = format!("{{\"expression\":{},\"contextId\":{context},\"returnByValue\":true,\"silent\":true}}", js_string(source));
    call_in(tab, host, target, "Runtime.evaluate", &params, |_| {});
}

pub(super) fn post(tab: &Rc<Tab>, world: &World, frame: Option<FrameId>, json: &str) {
    let Some(host) = host_of(tab) else { return };
    let source = format!("globalThis.{RECEIVE} && globalThis.{RECEIVE}({})", js_string(json));
    let contexts: Vec<(Target, i32)> = {
        let tarn = tab.tarn.borrow();
        if tarn.page.as_ref().is_some_and(|s| s.world == *world) {
            tarn.page_contexts.iter().filter(|(_, f)| tarn.main_frame.as_ref() == Some(*f)).map(|(c, _)| (None, *c)).collect()
        } else {
            let Some(state) = tarn.worlds.get(world) else { return };
            let main = |target: &Target, known: &Context| target.is_none() && tarn.main_frame.as_ref() == Some(&known.frame);
            state
                .contexts
                .iter()
                .filter(|((t, _), known)| frame.is_none_or(|f| f == frame_of(main(t, known), known.serial)))
                .map(|(key, _)| key.clone())
                .collect()
        }
    };
    for (target, context) in contexts {
        run_in(tab, &host, target.as_deref(), context, &source);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registration_waits_for_every_reply_before_the_next() {
        let scripts = Rc::new(WorldScripts { world: World::for_install("ab12").unwrap(), bootstrap: String::new(), scripts: Vec::new() });
        let mut state =
            WorldState { scripts, decisions: Decisions::default(), registered: Vec::new(), waiting: BTreeMap::new(), generation: 1, contexts: HashMap::new() };
        let page: Target = None;
        assert_eq!(state.added(&page, Some("1".into())), None, "a target that is not waiting keeps nothing");
        state.waiting.insert(page.clone(), (2, true));
        assert_eq!(state.added(&page, Some("1".into())), Some(false));
        assert_eq!(state.added(&page, None), Some(true), "the last reply, even a failed one, lets newer decisions go out");
        assert_eq!(state.registered, [(None, "1".to_owned())]);
        assert_eq!(state.waiting[&page], (0, false));
    }

    #[test]
    fn a_session_that_ends_before_its_setup_is_never_kept() {
        let mut sessions = Sessions::default();
        sessions.announce("a".into(), None);
        assert_eq!(sessions.end("a".into()), [Some("a".to_owned())]);
        assert!(!sessions.start("a", true), "its setup finds it gone");
        assert!(sessions.live.is_empty() && sessions.announced.is_empty());
    }

    #[test]
    fn ending_a_session_ends_those_attached_through_it() {
        let mut sessions = Sessions::default();
        sessions.announce("a".into(), None);
        assert!(sessions.start("a", true));
        sessions.announce("b".into(), Some("a".into()));
        assert!(sessions.start("b", true));
        sessions.announce("c".into(), Some("b".into()));
        sessions.announce("d".into(), None);
        let mut gone = sessions.end("a".into());
        gone.sort();
        assert_eq!(gone, [Some("a".to_owned()), Some("b".to_owned()), Some("c".to_owned())]);
        assert!(!sessions.start("c", true), "a child announced under it goes too");
        assert!(sessions.start("d", false) && sessions.live.is_empty(), "another frame's target is set up, not kept");
    }
}
