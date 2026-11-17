//! WebKit's side of Tarn extensions (`tarn_ext`): each install's content scripts in a content world
//! of its own, let through per frame navigation, a message handler that only that world sees, and
//! the install's own pages served from `tarn-extension://`.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::rc::{Rc, Weak};

use block2::{DynBlock, RcBlock};
use objc2::rc::{Retained, Weak as ObjcWeak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSPopover, NSPopoverBehavior, NSPopoverDelegate, NSViewController};
use objc2_foundation::{NSData, NSDictionary, NSError, NSHTTPURLResponse, NSNotification, NSObject, NSPoint, NSRect, NSRectEdge, NSSize, NSString, NSURL};
use objc2_web_kit::{
    WKContentWorld, WKFrameInfo, WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationType, WKScriptMessage,
    WKScriptMessageHandler, WKUIDelegate, WKURLSchemeHandler, WKURLSchemeTask, WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebView,
    WKWebViewConfiguration, WKWindowFeatures,
};
use shield::ShieldLevel;
use url::Url;

use crate::delegates::{Delegate, to_url};
use crate::handover::js_string;
use crate::tarn_ext::{
    Decisions, FrameId, FrameNav, Gate, HELLO, Injection, MAX_DECISIONS, PageSpec, RECEIVE, Resources, RunAt, SCHEME, World, WorldScripts, content_injections,
    frame_key, gate_injection, opens_as_tab, page_loads,
};
use crate::webview::Host;
use crate::{DataStore, Event};

const HANDLER: &str = "tarnExt";
const SETUP: &str = "const tarnPost = window.webkit && window.webkit.messageHandlers.tarnExt; if (!tarnPost) return; \
    const tarnDoc = Array.from(crypto.getRandomValues(new Uint8Array(16)), b => b.toString(16).padStart(2, '0')).join(''); \
    globalThis.__tarnExtDoc = tarnDoc; globalThis.__tarnExtPost = s => tarnPost.postMessage(tarnDoc + String(s)); tarnPost.postMessage(tarnDoc + '\\u0001');";
const PAGE_SETUP: &str = "const tarnPost = window.webkit && window.webkit.messageHandlers.tarnExt; if (!tarnPost) return; globalThis.__tarnExtPost = s => tarnPost.postMessage(String(s)); tarnPost.postMessage('\\u0001');";
const POPUP_SIZE: (f64, f64) = (360.0, 480.0);
const MAX_POPUP: (f64, f64) = (800.0, 600.0);

/// The worlds of one profile's content controllers: WebKit can only remove every user script of a
/// controller at once, so the others are put back around the worlds' scripts.
pub(crate) struct Worlds {
    controllers: Vec<Retained<WKUserContentController>>,
    handler: Retained<Handler>,
    worlds: RefCell<BTreeMap<World, Installed>>,
    ours: RefCell<Vec<Retained<WKUserScript>>>,
    held: RefCell<HashMap<usize, VecDeque<String>>>,
}

/// One world's scripts: its content scripts are made once, its gate again for each decision.
struct Installed {
    scripts: Rc<WorldScripts>,
    decisions: Decisions,
    content: Vec<Retained<WKUserScript>>,
}

fn user_script(mtm: MainThreadMarker, injection: &Injection, world: &WKContentWorld) -> Retained<WKUserScript> {
    let time = if injection.run_at == RunAt::DocumentStart { WKUserScriptInjectionTime::AtDocumentStart } else { WKUserScriptInjectionTime::AtDocumentEnd };
    // SAFETY: main-thread creation of a fresh script in the install's own world.
    unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(mtm),
            &NSString::from_str(&injection.source),
            time,
            !injection.all_frames,
            world,
        )
    }
}

thread_local! {
    /// The worlds of each content controller, by its address, so a view finds its profile's.
    static CONTROLLERS: RefCell<HashMap<usize, Weak<Worlds>>> = RefCell::new(HashMap::new());
    /// The views that run Tarn extensions, by their WKWebView's address.
    static VIEWS: RefCell<HashMap<usize, Rc<ViewState>>> = RefCell::new(HashMap::new());
}

fn address<T>(object: &T) -> usize {
    std::ptr::from_ref(object) as usize
}

fn content_world(mtm: MainThreadMarker, world: &World) -> Retained<WKContentWorld> {
    // SAFETY: main-thread call; WebKit returns the same world for the same name.
    unsafe { WKContentWorld::worldWithName(&NSString::from_str(world.name()), mtm) }
}

impl Worlds {
    /// For a profile's controllers of every level but Lockdown, where no extension runs.
    pub(crate) fn new(mtm: MainThreadMarker, controllers: Vec<Retained<WKUserContentController>>) -> Rc<Self> {
        let worlds =
            Rc::new(Worlds { controllers, handler: Handler::new(mtm), worlds: RefCell::default(), ours: RefCell::default(), held: RefCell::default() });
        CONTROLLERS.with_borrow_mut(|c| {
            for controller in &worlds.controllers {
                c.insert(address(&**controller), Rc::downgrade(&worlds));
            }
        });
        worlds
    }

    pub(crate) fn add(&self, scripts: Rc<WorldScripts>) {
        let world = scripts.world.clone();
        let mtm = self.handler.mtm();
        let content = content_world(mtm, &world);
        let fresh = {
            let mut worlds = self.worlds.borrow_mut();
            if worlds.get(&world).is_some_and(|w| *w.scripts == *scripts) {
                return;
            }
            let made = content_injections(&scripts).iter().map(|i| user_script(mtm, i, &content)).collect();
            worlds.insert(world.clone(), Installed { scripts, decisions: Decisions::default(), content: made }).is_none()
        };
        if fresh {
            for controller in &self.controllers {
                // SAFETY: main-thread call; the name is new in this world (checked above), as WebKit requires.
                unsafe {
                    controller.addScriptMessageHandler_contentWorld_name(ProtocolObject::from_ref(&*self.handler), &content, &NSString::from_str(HANDLER))
                };
            }
        }
        self.install();
    }

    pub(crate) fn remove(&self, world: &World) {
        if self.worlds.borrow_mut().remove(world).is_none() {
            return;
        }
        let content = content_world(self.handler.mtm(), world);
        for controller in &self.controllers {
            // SAFETY: main-thread call removing the handler added in `add`.
            unsafe { controller.removeScriptMessageHandlerForName_contentWorld(&NSString::from_str(HANDLER), &content) };
        }
        self.install();
    }

    /// The documents of a view without a gate read the same scripts as the profile's tabs, so no
    /// tab may record an answer for them while that view lives.
    fn hold(&self, view: usize, frame: &FrameNav) {
        let Some(key) = frame_key(frame) else { return };
        let mut held = self.held.borrow_mut();
        let keys = held.entry(view).or_default();
        if keys.contains(&key) {
            return;
        }
        if keys.len() >= MAX_DECISIONS {
            keys.pop_front();
        }
        keys.push_back(key);
    }

    fn release(&self, view: usize) {
        self.held.borrow_mut().remove(&view);
    }

    fn decide(&self, gate: &Gate, frame: &FrameNav) {
        let held = frame_key(frame).is_some_and(|k| self.held.borrow().values().any(|keys| keys.contains(&k)));
        let mut changed = false;
        for (world, w) in self.worlds.borrow_mut().iter_mut() {
            let run: Vec<usize> = if held { Vec::new() } else { gate(world, frame).into_iter().filter(|ix| *ix < w.scripts.scripts.len()).collect() };
            changed |= w.decisions.record(frame, run);
        }
        if changed {
            self.install();
        }
    }

    /// Puts the worlds' current scripts into every controller, after the controller's own. WebKit
    /// can only remove all of a controller's scripts, so only the gates are made again.
    pub(crate) fn install(&self) {
        let mtm = self.handler.mtm();
        let old = std::mem::take(&mut *self.ours.borrow_mut());
        let mut ours = Vec::new();
        for (world, w) in self.worlds.borrow().iter() {
            ours.push(user_script(mtm, &gate_injection(&w.scripts, &w.decisions, SETUP), &content_world(mtm, world)));
            ours.extend(w.content.iter().cloned());
        }
        for controller in &self.controllers {
            // SAFETY: main thread (the controllers were created there and are not Send).
            unsafe {
                let others: Vec<_> = controller.userScripts().iter().filter(|s| !old.iter().any(|o| std::ptr::eq(&**o, &**s))).collect();
                controller.removeAllUserScripts();
                for script in others.iter().chain(&ours) {
                    controller.addUserScript(script);
                }
            }
        }
        *self.ours.borrow_mut() = ours;
    }

    /// The controller's other scripts were rebuilt: the worlds' scripts go back after them.
    pub(crate) fn reinstalled(&self) {
        self.ours.borrow_mut().clear();
        self.install();
    }
}

impl Drop for Worlds {
    fn drop(&mut self) {
        CONTROLLERS.with_borrow_mut(|c| {
            for controller in &self.controllers {
                c.remove(&address(&**controller));
            }
        });
    }
}

/// What a tab's view knows for Tarn extensions: its gate and the frames that reported in.
struct ViewState {
    delegate: ObjcWeak<Delegate>,
    gate: RefCell<Option<Gate>>,
    frames: RefCell<Frames<Retained<WKFrameInfo>>>,
}

const MAX_FRAMES: usize = 256;

/// The subframe documents that reported in, each numbered on its own and holding the random value
/// its setup drew, which a reply must match, so that a reply never reaches the frame's next
/// document; the oldest go first past `MAX_FRAMES`.
struct Frames<T> {
    serial: u64,
    docs: BTreeMap<u64, (World, FrameId, T, String)>,
}

impl<T> Default for Frames<T> {
    fn default() -> Self {
        Frames { serial: 0, docs: BTreeMap::new() }
    }
}

impl<T: Clone> Frames<T> {
    /// The id of the document that drew `doc` and sent a message from `frame` (WebKit's own
    /// number) in `world`; the setup's hello starts a new document of that world, and a message
    /// from an older document of the frame is dropped.
    fn document(&mut self, world: &World, frame: FrameId, info: T, doc: &str, hello: bool) -> Option<FrameId> {
        let current = self.docs.iter().find(|(_, (w, f, _, _))| w == world && *f == frame).map(|(serial, (_, _, _, known))| (*serial, known == doc));
        match current {
            Some((serial, same)) if !hello => return same.then_some(FrameId(serial | 1 << 63)),
            Some((serial, _)) => {
                self.docs.remove(&serial);
            }
            None => {}
        }
        if self.docs.len() >= MAX_FRAMES {
            self.docs.pop_first();
        }
        self.serial += 1;
        self.docs.insert(self.serial, (world.clone(), frame, info, doc.to_owned()));
        Some(FrameId(self.serial | 1 << 63))
    }

    fn reach(&self, world: &World, id: Option<FrameId>) -> Vec<(T, String)> {
        self.docs
            .iter()
            .filter(|(serial, (w, _, _, _))| w == world && id.is_none_or(|id| id.0 & !(1 << 63) == **serial))
            .map(|(_, (_, _, info, doc))| (info.clone(), doc.clone()))
            .collect()
    }
}

fn state(view: &WKWebView) -> Option<Rc<ViewState>> {
    VIEWS.with_borrow(|v| v.get(&address(view)).cloned())
}

fn worlds_of(view: &WKWebView) -> Option<Rc<Worlds>> {
    // SAFETY: main-thread reads of a live web view's configuration.
    let controller = unsafe { view.configuration().userContentController() };
    CONTROLLERS.with_borrow(|c| c.get(&address(&*controller))?.upgrade())
}

/// Starts or replaces the view's gate; its events go out through `delegate`.
pub(crate) fn set_gate(view: &WKWebView, delegate: &Delegate, gate: Gate) {
    let state = VIEWS.with_borrow_mut(|v| {
        v.entry(address(view))
            .or_insert_with(|| Rc::new(ViewState { delegate: ObjcWeak::new(delegate), gate: RefCell::new(None), frames: RefCell::default() }))
            .clone()
    });
    *state.gate.borrow_mut() = Some(gate);
    if let Some(worlds) = worlds_of(view) {
        worlds.release(address(view));
    }
}

/// The main frame committed a new document: its subframes' documents are gone.
pub(crate) fn committed(view: &WKWebView) {
    if let Some(state) = state(view) {
        *state.frames.borrow_mut() = Frames::default();
    }
}

pub(crate) fn forget_view(view: &WKWebView) {
    VIEWS.with_borrow_mut(|v| v.remove(&address(view)));
    if let Some(worlds) = worlds_of(view) {
        worlds.release(address(view));
    }
}

pub(crate) fn add_scripts(view: &WKWebView, scripts: Rc<WorldScripts>) {
    if let Some(worlds) = worlds_of(view) {
        worlds.add(scripts);
    }
}

pub(crate) fn remove_scripts(view: &WKWebView, world: &World) {
    if let Some(worlds) = worlds_of(view) {
        worlds.remove(world);
    }
}

/// A frame of `view` is about to load `url`: the gate decides there and then, before WebKit is
/// answered, so the decision is in the world's scripts when the document starts. A view of the
/// profile without a gate shares its worlds' scripts, so its documents get an empty decision that
/// holds until the view gets a gate or goes away. An http URL is decided for its https form too,
/// which Shield's `make-https` rule or HSTS may load without another navigation action.
pub(crate) fn gate_frame(view: &WKWebView, url: &Url, main_frame: bool) {
    let Some(worlds) = worlds_of(view) else { return };
    let gate = state(view).and_then(|s| s.gate.borrow().clone());
    let held = gate.is_none();
    let gate = gate.unwrap_or_else(|| Rc::new(|_: &World, _: &FrameNav| Vec::new()));
    // SAFETY: main-thread read of the live web view.
    let page = if main_frame { None } else { to_url(unsafe { view.URL() }) };
    let mut https = url.clone();
    let upgraded = (url.scheme() == "http" && https.set_scheme("https").is_ok()).then_some(&https);
    for url in std::iter::once(url).chain(upgraded) {
        let Some(top) = (if main_frame { Some(url) } else { page.as_ref() }) else { return };
        let frame = FrameNav { url, top, main_frame };
        if held {
            worlds.hold(address(view), &frame);
        }
        worlds.decide(&gate, &frame);
    }
}

pub(crate) fn post(view: &WKWebView, world: &World, frame: Option<FrameId>, json: &str) {
    let mtm = view.mtm();
    let content = content_world(mtm, world);
    let main = (frame.is_none_or(|f| f == FrameId::MAIN)).then_some((None, None));
    let subframes = match frame {
        Some(FrameId::MAIN) => Vec::new(),
        id => state(view).map(|s| s.frames.borrow().reach(world, id)).unwrap_or_default(),
    };
    for (target, doc) in main.into_iter().chain(subframes.into_iter().map(|(info, doc)| (Some(info), Some(doc)))) {
        let script = NSString::from_str(&crate::tarn_ext::reply(doc.as_deref(), json));
        // SAFETY: main-thread call on a live web view; the script only calls the shim's receiver in the install's world.
        unsafe { view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(&script, target.as_deref(), &content, None) };
    }
}

/// The engine's own number for a frame, never `MAIN` for a subframe. WebKit has no public frame
/// identifier; its private `_handle.frameID` is used where it exists.
fn frame_id(frame: &WKFrameInfo) -> Option<FrameId> {
    // SAFETY: main-thread read of a live frame info.
    if unsafe { frame.isMainFrame() } {
        return Some(FrameId::MAIN);
    }
    if !frame.respondsToSelector(sel!(_handle)) {
        return None;
    }
    // SAFETY: the selector exists (checked above) and returns a retained `_WKFrameHandle` or nil.
    let handle: Option<Retained<NSObject>> = unsafe { msg_send![frame, _handle] };
    let handle = handle.filter(|h| h.respondsToSelector(sel!(frameID)))?;
    // SAFETY: the selector exists on the handle (checked above) and returns an unsigned 64-bit id.
    let id: u64 = unsafe { msg_send![&*handle, frameID] };
    Some(FrameId(id | 1 << 63))
}

fn body_text(message: &WKScriptMessage) -> Option<String> {
    // SAFETY: main-thread read of a live message; the body is only used after a class check.
    let body = unsafe { message.body() };
    body.downcast_ref::<NSString>().map(|s| s.to_string())
}

fn received(message: &WKScriptMessage) {
    // SAFETY: main-thread reads of a live message.
    let (view, content, frame) = unsafe { (message.webView(), message.world(), message.frameInfo()) };
    let (Some(view), Some(text)) = (view, body_text(message)) else { return };
    // SAFETY: main-thread read of the world's name; the handler is only added to Tarn's worlds.
    let Some(world) = unsafe { content.name() }.and_then(|n| World::parse(&n.to_string())) else { return };
    let (Some(state), Some(id)) = (state(&view), frame_id(&frame)) else { return };
    let origin = crate::delegates::origin_of(&frame).and_then(|o| Url::parse(&o).ok());
    let Some((doc, payload)) = crate::tarn_ext::split_document(&text) else { return };
    let hello = payload.strip_prefix(HELLO).is_some_and(str::is_empty);
    let id = if id == FrameId::MAIN { Some(id) } else { state.frames.borrow_mut().document(&world, id, frame, doc, hello) };
    let (Some(id), false) = (id, hello) else { return };
    if let Some(delegate) = state.delegate.load() {
        delegate.emit(Event::ExtensionMessage { world, frame: id, origin, json: payload.to_owned() });
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnExtensionHandler"]
    struct Handler;

    unsafe impl NSObjectProtocol for Handler {}

    unsafe impl WKScriptMessageHandler for Handler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            received(message);
        }
    }
);

impl Handler {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Handler::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's designated initialiser on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) struct PageState {
    world: World,
    origin: String,
    resources: Resources,
    sink: RefCell<Rc<dyn Fn(Event)>>,
    closed: Rc<dyn Fn()>,
}

fn respond(task: &ProtocolObject<dyn WKURLSchemeTask>, url: &NSURL, resource: crate::tarn_ext::Resource) {
    let mut keys = vec![NSString::from_str("Content-Type"), NSString::from_str("Cache-Control"), NSString::from_str("X-Content-Type-Options")];
    let mime = if resource.mime.starts_with("text/") || resource.mime == "application/json" {
        format!("{}; charset=utf-8", resource.mime)
    } else {
        resource.mime.to_owned()
    };
    let mut values = vec![NSString::from_str(&mime), NSString::from_str("no-store"), NSString::from_str("nosniff")];
    if let Some(csp) = resource.csp {
        keys.push(NSString::from_str("Content-Security-Policy"));
        values.push(NSString::from_str(csp));
    }
    let keys: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<&NSString> = values.iter().map(|v| &**v).collect();
    let headers = NSDictionary::from_slices(&keys, &values);
    // SAFETY: main-thread calls answering a task WebKit started and has not stopped; each step is called once, in order.
    unsafe {
        let response = NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
            NSHTTPURLResponse::alloc(),
            url,
            isize::from(resource.status as i16),
            Some(&NSString::from_str("HTTP/1.1")),
            Some(&headers),
        );
        let Some(response) = response else { return task.didFailWithError(&NSError::new(-1, &NSString::from_str("tarn-extension"))) };
        task.didReceiveResponse(&response);
        task.didReceiveData(&NSData::with_bytes(&resource.body));
        task.didFinish();
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnExtensionPageDelegate"]
    #[ivars = PageState]
    pub(crate) struct PageDelegate;

    unsafe impl NSObjectProtocol for PageDelegate {}

    unsafe impl WKScriptMessageHandler for PageDelegate {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: main-thread reads of a live message's frame.
            let (main, protocol, host) = unsafe {
                let frame = message.frameInfo();
                let origin = frame.securityOrigin();
                (frame.isMainFrame(), origin.protocol().to_string(), origin.host().to_string())
            };
            let state = self.ivars();
            if !main || protocol != SCHEME || host != state.origin {
                return;
            }
            let Some(text) = body_text(message).filter(|t| !t.starts_with(HELLO)) else { return };
            let sink = state.sink.borrow().clone();
            sink(Event::ExtensionMessage { world: state.world.clone(), frame: FrameId::MAIN, origin: None, json: text });
        }
    }

    unsafe impl WKURLSchemeHandler for PageDelegate {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start_task(&self, _web_view: &WKWebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            // SAFETY: main-thread read of the task's request.
            let Some(ns_url) = (unsafe { task.request() }).URL() else { return };
            let url = to_url(Some(ns_url.clone()));
            let state = self.ivars();
            let resource = match url.filter(|u| u.host_str() == Some(state.origin.as_str())) {
                Some(url) => (state.resources)(&url),
                None => crate::tarn_ext::Resource { status: 404, mime: "text/plain", csp: None, body: Vec::new() },
            };
            respond(task, &ns_url, resource);
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop_task(&self, _web_view: &WKWebView, _task: &ProtocolObject<dyn WKURLSchemeTask>) {}
    }

    unsafe impl WKNavigationDelegate for PageDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_policy(&self, _web_view: &WKWebView, action: &WKNavigationAction, handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>) {
            // SAFETY: main-thread reads of the live action.
            let (url, main, kind) = unsafe { (to_url(action.request().URL()), action.targetFrame().is_none_or(|f| f.isMainFrame()), action.navigationType()) };
            if url.as_ref().is_some_and(|u| page_loads(u, &self.ivars().origin, main)) {
                return handler.call((WKNavigationActionPolicy::Allow,));
            }
            if let Some(url) = url.filter(|u| main && kind == WKNavigationType::LinkActivated && opens_as_tab(u)) {
                let sink = self.ivars().sink.borrow().clone();
                sink(Event::OpenTab { url, from: None, background: false });
            }
            handler.call((WKNavigationActionPolicy::Cancel,));
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
            fit_popover(web_view);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, _web_view: &WKWebView) {
            (self.ivars().closed)();
        }
    }

    unsafe impl WKUIDelegate for PageDelegate {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            _web_view: &WKWebView,
            _configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            // SAFETY: main-thread read of the live action.
            if let Some(url) = to_url(unsafe { action.request() }.URL()).filter(opens_as_tab) {
                let sink = self.ivars().sink.borrow().clone();
                sink(Event::OpenTab { url, from: None, background: false });
            }
            None
        }
    }

    unsafe impl NSPopoverDelegate for PageDelegate {
        #[unsafe(method(popoverDidClose:))]
        fn popover_did_close(&self, _notification: &NSNotification) {
            (self.ivars().closed)();
        }
    }
);

fn fit_popover(view: &WKWebView) {
    let Some(popover) = POPOVERS.with_borrow(|p| p.get(&address(view)).cloned()) else { return };
    let script = NSString::from_str("[document.documentElement.scrollWidth, document.documentElement.scrollHeight].join(' ')");
    let done = RcBlock::new(move |result: *mut AnyObject, _error: *mut NSError| {
        // SAFETY: WebKit passes a retained result or null; it is read only after a class check.
        let text = unsafe { result.as_ref() }.and_then(|r| r.downcast_ref::<NSString>()).map(|s| s.to_string()).unwrap_or_default();
        let mut parts = text.split(' ').filter_map(|n| n.parse::<f64>().ok());
        if let (Some(width), Some(height)) = (parts.next(), parts.next()) {
            popover.setContentSize(NSSize::new(width.clamp(40.0, MAX_POPUP.0), height.clamp(24.0, MAX_POPUP.1)));
        }
    });
    // SAFETY: main-thread call on a live web view; the extension's own page answers in its own world.
    unsafe { view.evaluateJavaScript_completionHandler(&script, Some(&done)) };
}

thread_local! {
    static POPOVERS: RefCell<HashMap<usize, Retained<NSPopover>>> = RefCell::new(HashMap::new());
}

/// One of an install's own pages: its background page, kept hidden, or its popup. It loads only
/// the install's `tarn-extension://` origin, with the bootstrap in its main world and the profile's
/// built-in rules (private network, HTTPS); it never gets the profile's page scripts.
pub struct ExtensionPage {
    view: Retained<WKWebView>,
    delegate: Retained<PageDelegate>,
    builtin: crate::data_store::Builtin,
}

impl ExtensionPage {
    /// `closed` runs when its popup closes or its process ends. `None` off the main thread.
    /// `host` is where a popup shows later; macOS needs none to make the page.
    pub fn new(_host: &Host, data: &DataStore, level: ShieldLevel, spec: PageSpec, sink: Rc<dyn Fn(Event)>, closed: Rc<dyn Fn()>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let PageSpec { world, origin, bootstrap, resources, inspectable } = spec;
        let state = PageState { world, origin, resources, sink: RefCell::new(sink), closed };
        let this = PageDelegate::alloc(mtm).set_ivars(state);
        // SAFETY: NSObject's designated initialiser on a freshly allocated instance.
        let delegate: Retained<PageDelegate> = unsafe { msg_send![super(this), init] };
        // SAFETY: main-thread configuration of fresh objects. The handler is in the page's main
        // world, which is the extension's own page; it answers only the main frame of its origin.
        unsafe {
            let controller = WKUserContentController::new(mtm);
            let page = WKContentWorld::pageWorld(mtm);
            controller.addScriptMessageHandler_contentWorld_name(ProtocolObject::from_ref(&*delegate), &page, &NSString::from_str(HANDLER));
            let source = format!("(() => {{ {PAGE_SETUP}\n{bootstrap}}})();");
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                WKUserScript::alloc(mtm),
                &NSString::from_str(&source),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
                &page,
            );
            controller.addUserScript(&script);
            let config = WKWebViewConfiguration::new(mtm);
            config.setWebsiteDataStore(&data.store);
            config.setUserContentController(&controller);
            config.setURLSchemeHandler_forURLScheme(Some(ProtocolObject::from_ref(&*delegate)), &NSString::from_str(SCHEME));
            config.preferences().setJavaScriptCanOpenWindowsAutomatically(false);
            let view =
                WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::new(NSPoint::ZERO, NSSize::new(POPUP_SIZE.0, POPUP_SIZE.1)), &config);
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            view.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            view.setInspectable(inspectable);
            let page = ExtensionPage { view, delegate, builtin: data.builtin(level) };
            let list = data.builtin_list(level);
            let weak = ObjcWeak::from_retained(&page.view);
            page.builtin.then(move |result| {
                let Some(view) = weak.load() else { return };
                match (result, list.borrow().clone()) {
                    (Ok(()), Some(list)) => view.configuration().userContentController().addContentRuleList(&list),
                    _ => view.stopLoading(),
                }
            });
            Some(page)
        }
    }

    /// Loads one of the install's pages once the built-in rules are attached; nothing else loads.
    pub fn load(&self, url: &Url) {
        if url.scheme() != SCHEME || url.host_str() != Some(self.delegate.ivars().origin.as_str()) {
            return;
        }
        let Some(ns_url) = NSURL::URLWithString(&NSString::from_str(url.as_str())) else { return };
        let view = ObjcWeak::from_retained(&self.view);
        self.builtin.then(move |result| {
            if let (Ok(()), Some(view)) = (result, view.load()) {
                // SAFETY: main-thread load of the extension's own page into a live view.
                unsafe { view.loadRequest(&objc2_foundation::NSURLRequest::requestWithURL(&ns_url)) };
            }
        });
    }

    pub fn post(&self, json: &str) {
        let script = NSString::from_str(&format!("globalThis.{RECEIVE} && globalThis.{RECEIVE}({})", js_string(json)));
        // SAFETY: main-thread call on a live web view; the page is the extension's own.
        unsafe { self.view.evaluateJavaScript_completionHandler(&script, None) };
    }

    pub fn set_sink(&self, sink: Rc<dyn Fn(Event)>) {
        *self.delegate.ivars().sink.borrow_mut() = sink;
    }

    /// Shows the page in a popover pointing at `x, y` (shell points, origin top-left) of `host`.
    pub fn show_popup(&self, host: &Host, x: f64, y: f64) {
        let mtm = self.view.mtm();
        let controller = NSViewController::new(mtm);
        controller.setView(&self.view);
        let popover = NSPopover::new(mtm);
        popover.setBehavior(NSPopoverBehavior::Transient);
        popover.setContentViewController(Some(&controller));
        popover.setContentSize(NSSize::new(POPUP_SIZE.0, POPUP_SIZE.1));
        popover.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate)));
        let edge = if host.view.isFlipped() { NSRectEdge::MaxY } else { NSRectEdge::MinY };
        popover.showRelativeToRect_ofView_preferredEdge(host.rect(x, y, 1.0, 1.0), &host.view, edge);
        POPOVERS.with_borrow_mut(|p| p.insert(address(&*self.view), popover));
    }

    pub fn close_popup(&self) {
        if let Some(popover) = POPOVERS.with_borrow_mut(|p| p.remove(&address(&*self.view))) {
            // SAFETY: main-thread call on a live popover.
            unsafe { popover.performClose(None) };
        }
    }
}

impl Drop for ExtensionPage {
    fn drop(&mut self) {
        if let Some(popover) = POPOVERS.with_borrow_mut(|p| p.remove(&address(&*self.view))) {
            popover.setDelegate(None);
            popover.close();
        }
        // SAFETY: main-thread teardown; the delegates are cleared before their owner drops them.
        unsafe {
            self.view.stopLoading();
            self.view.setNavigationDelegate(None);
            self.view.setUIDelegate(None);
            self.view.configuration().userContentController().removeAllScriptMessageHandlers();
        }
        self.view.removeFromSuperview();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_document_of_a_frame_gets_its_own_id_and_the_table_stays_bounded() {
        let (world, other) = (World::for_install("ab12").unwrap(), World::for_install("cd34").unwrap());
        let mut frames = Frames::default();
        let first = frames.document(&world, FrameId(7), 1, "a", true).unwrap();
        assert_eq!(frames.document(&world, FrameId(7), 1, "a", false), Some(first));
        assert_eq!(frames.reach(&world, Some(first)), [(1, "a".to_owned())]);
        assert!(frames.reach(&other, Some(first)).is_empty(), "an id answers only in its world");
        let theirs = frames.document(&other, FrameId(7), 1, "b", true).unwrap();
        assert_eq!(frames.document(&world, FrameId(7), 1, "a", false), Some(first), "another install's hello in the same document keeps this one's id");
        assert_eq!(frames.reach(&other, Some(theirs)), [(1, "b".to_owned())]);
        let next = frames.document(&world, FrameId(7), 2, "c", true).unwrap();
        assert!(next != first && next != FrameId::MAIN && frames.reach(&world, Some(first)).is_empty(), "a reply never reaches the next document");
        assert_eq!(frames.reach(&world, None), [(2, "c".to_owned())]);
        assert_eq!(frames.document(&world, FrameId(7), 1, "a", false), None, "a late message of the older document is dropped");
        assert_eq!(frames.reach(&world, None), [(2, "c".to_owned())]);
        for n in 0..2 * MAX_FRAMES as u64 {
            frames.document(&world, FrameId(n + 100), 0, "d", true);
        }
        assert_eq!(frames.docs.len(), MAX_FRAMES);
        assert!(frames.reach(&world, Some(next)).is_empty(), "the frame's entry was evicted");
        let again = frames.document(&world, FrameId(7), 2, "c", false).unwrap();
        assert_eq!(frames.reach(&world, Some(again)), [(2, "c".to_owned())], "an evicted document comes back with the value it drew");
    }

    #[test]
    fn every_message_of_a_document_carries_the_value_its_replies_must_match() {
        assert!(SETUP.contains("globalThis.__tarnExtDoc = tarnDoc") && SETUP.contains("tarnPost.postMessage(tarnDoc + String(s))"));
        assert!(SETUP.contains("tarnPost.postMessage(tarnDoc + '\\u0001')"));
        assert!(!PAGE_SETUP.contains("tarnDoc"), "an extension page's messages carry none");
    }
}
