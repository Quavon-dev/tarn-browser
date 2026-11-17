use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::{DynBlock, RcBlock};
use engine::tarn_ext::{Decisions, FrameId, FrameNav, RunAt, World, WorldScripts, injections};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_foundation::{NSError, NSObject, NSString};
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKContentWorld, WKFrameInfo, WKNavigationAction, WKNavigationActionPolicy, WKScriptMessage,
    WKScriptMessageHandler, WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebpagePreferences,
};
use serde_json::{Value, json};
use shield::ShieldLevel;
use url::Url;

use crate::component::{Built, Inner};
use crate::data::{StoreKind, store_kind};
use crate::ext_scripts::{HANDLER, SETUP, TOKEN};
use crate::tarn_ext;
use crate::webkit::WebView;

thread_local! {
    static NEXT_FRAME_REQUEST: Cell<u64> = const { Cell::new(1) };
    static LISTS: RefCell<HashMap<String, Option<Retained<WKContentRuleList>>>> = RefCell::new(HashMap::new());
    static COMPILING: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

#[derive(Default)]
pub(crate) struct ExtView {
    worlds: Vec<Rc<WorldScripts>>,
    approved: HashMap<World, Decisions>,
    handlers: Vec<(World, Retained<ExtHandler>)>,
    frames: HashMap<(World, String), (u64, Retained<WKFrameInfo>, String)>,
    main: HashMap<World, Retained<WKFrameInfo>>,
    next_frame: u64,
    rules: Vec<(String, String)>,
    lists: Vec<Retained<WKContentRuleList>>,
    pending: HashMap<u64, Held>,
}

pub(crate) type ActionHandler = DynBlock<dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>)>;

pub(crate) struct Held {
    handler: RcBlock<dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>)>,
    preferences: Retained<WKWebpagePreferences>,
    url: Url,
    top: Url,
    main: bool,
}

impl ExtView {
    /// Every held navigation is cancelled when its view goes.
    pub(crate) fn cancel_pending(&mut self) {
        for (_, held) in self.pending.drain() {
            held.handler.call((WKNavigationActionPolicy::Cancel, NonNull::from(&*held.preferences)));
        }
    }
}

pub(crate) struct ExtState {
    inner: Weak<Inner>,
    world: World,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnExtensionMessageHandler"]
    #[ivars = ExtState]
    pub(crate) struct ExtHandler;

    unsafe impl NSObjectProtocol for ExtHandler {}

    unsafe impl WKScriptMessageHandler for ExtHandler {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn received(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            if let Some(inner) = self.ivars().inner.upgrade() {
                inner.ext_message(&self.ivars().world, message);
            }
        }
    }
);

impl ExtHandler {
    fn new(mtm: MainThreadMarker, inner: Weak<Inner>, world: World) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ExtState { inner, world });
        // SAFETY: NSObject's designated initializer on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

fn content_world(world: &World, mtm: MainThreadMarker) -> Retained<WKContentWorld> {
    // SAFETY: main thread (marker); a named world is created once per name and shared.
    unsafe { WKContentWorld::worldWithName(&NSString::from_str(world.name()), mtm) }
}

fn web_origin(frame: &WKFrameInfo) -> Option<String> {
    // SAFETY: WebKit passes a live frame on the main thread.
    let origin = unsafe { frame.securityOrigin() };
    crate::handler::security_origin(&origin).filter(|u| matches!(u.scheme(), "http" | "https")).map(|u| u.origin().ascii_serialization())
}

/// Every user script of the view again: Tarn's own site styles, then each install's guarded scripts.
pub(crate) fn install_scripts(built: &Built, mtm: MainThreadMarker) {
    // SAFETY: main thread (marker); the controller belongs to this view, the scripts are copied by
    // WebKit and run only in the shell's or the install's isolated world.
    unsafe {
        built.controller.removeAllUserScripts();
        if let Some(site) = &built.site_script {
            built.controller.addUserScript(site);
        }
        for world in &built.ext.worlds {
            let content = content_world(&world.world, mtm);
            let none = Decisions::default();
            for script in injections(world, built.ext.approved.get(&world.world).unwrap_or(&none), SETUP) {
                let time =
                    if script.run_at == RunAt::DocumentStart { WKUserScriptInjectionTime::AtDocumentStart } else { WKUserScriptInjectionTime::AtDocumentEnd };
                let user = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                    WKUserScript::alloc(mtm),
                    &NSString::from_str(&script.source),
                    time,
                    !script.all_frames,
                    &content,
                );
                built.controller.addUserScript(&user);
            }
        }
    }
}

impl Inner {
    fn extensions_allowed(&self, built: &Built) -> bool {
        matches!(store_kind(&self.profile()), StoreKind::Persistent(_)) && built.level != ShieldLevel::Lockdown
    }

    /// Brings the view's worlds, message handlers and network rules to what the shell set for its profile.
    pub fn apply_extensions(self: &Rc<Self>) {
        let profile = self.profile();
        let rules = {
            let mut built = self.built.borrow_mut();
            let Some(built) = built.as_mut() else { return };
            let on = self.extensions_allowed(built);
            let worlds = if on { tarn_ext::scripts(&profile) } else { Vec::new() };
            let same = worlds.len() == built.ext.worlds.len() && worlds.iter().zip(&built.ext.worlds).all(|(a, b)| Rc::ptr_eq(a, b));
            if !same {
                let name = NSString::from_str(HANDLER);
                let gone: Vec<_> = built.ext.handlers.extract_if(.., |(w, _)| !worlds.iter().any(|x| x.world == *w)).collect();
                for (world, _) in gone {
                    // SAFETY: main thread; the handler was added under this name for this world.
                    unsafe { built.controller.removeScriptMessageHandlerForName_contentWorld(&name, &content_world(&world, self.mtm)) };
                }
                let added: Vec<&Rc<WorldScripts>> = worlds.iter().filter(|x| !built.ext.handlers.iter().any(|(w, _)| *w == x.world)).collect();
                for world in added {
                    let handler = ExtHandler::new(self.mtm, Rc::downgrade(self), world.world.clone());
                    // SAFETY: main thread; no handler is registered under this name in this world (checked above).
                    unsafe {
                        built.controller.addScriptMessageHandler_contentWorld_name(
                            ProtocolObject::from_ref(&*handler),
                            &content_world(&world.world, self.mtm),
                            &name,
                        )
                    };
                    built.ext.handlers.push((world.world.clone(), handler));
                }
                built.ext.approved.retain(|w, _| worlds.iter().any(|x| x.world == *w));
                built.ext.worlds = worlds;
                install_scripts(built, self.mtm);
            }
            let rules = if on { tarn_ext::rules(&profile) } else { Vec::new() };
            if rules == built.ext.rules {
                return;
            }
            built.ext.rules.clone_from(&rules);
            rules
        };
        self.apply_rules(rules);
    }

    fn apply_rules(self: &Rc<Self>, rules: Vec<(String, String)>) {
        let ids: Vec<(String, String)> =
            rules.iter().map(|(install, json)| (format!("tarn.ext.{install}.{:016x}", crate::scripts::fnv(json)), json.clone())).collect();
        let missing: Vec<(String, String)> = ids.iter().filter(|(id, _)| !LISTS.with(|l| l.borrow().contains_key(id))).cloned().collect();
        if missing.is_empty() {
            let lists: Vec<Retained<WKContentRuleList>> = ids.iter().filter_map(|(id, _)| LISTS.with(|l| l.borrow().get(id).cloned().flatten())).collect();
            let mut built = self.built.borrow_mut();
            let Some(built) = built.as_mut().filter(|b| b.ext.rules == rules) else { return };
            // SAFETY: main thread; the lists belong to this view's controller and apply from the next load.
            unsafe {
                for old in built.ext.lists.drain(..) {
                    built.controller.removeContentRuleList(&old);
                }
                for list in &lists {
                    built.controller.addContentRuleList(list);
                }
            }
            built.ext.lists = lists;
            return;
        }
        // SAFETY: main thread (marker); the default store exists in an app container.
        let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(self.mtm) }) else { return };
        for (id, json) in missing {
            if !COMPILING.with(|c| c.borrow_mut().insert(id.clone())) {
                continue;
            }
            let (this, key, wanted) = (Rc::downgrade(self), id.clone(), rules.clone());
            let done = RcBlock::new(move |list: *mut WKContentRuleList, _error: *mut NSError| {
                // SAFETY: WebKit passes a valid rule list or null, retained for the duration of the call.
                let list = unsafe { Retained::retain(list) };
                COMPILING.with(|c| c.borrow_mut().remove(&key));
                LISTS.with(|l| l.borrow_mut().insert(key.clone(), list));
                if let Some(inner) = this.upgrade() {
                    inner.apply_rules(wanted.clone());
                }
            });
            // SAFETY: main thread; the arguments are valid and WebKit copies the block.
            unsafe {
                store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(
                    Some(&NSString::from_str(&id)),
                    Some(&NSString::from_str(&json)),
                    Some(&done),
                )
            };
        }
    }

    /// Holds a frame's allowed navigation while the shell's gate decides, on its own thread, which
    /// of each world's scripts may run in the document it brings. False: nothing to ask.
    pub(crate) fn defer_frame(&self, web: &WebView, action: &WKNavigationAction, preferences: &WKWebpagePreferences, handler: &ActionHandler) -> bool {
        // SAFETY: WebKit hands over a live navigation action on the main thread.
        let (main, raw) = unsafe { (action.targetFrame().is_none_or(|f| f.isMainFrame()), action.request().URL().and_then(|u| u.absoluteString())) };
        let url = raw.and_then(|s| Url::parse(&s.to_string()).ok()).filter(|u| matches!(u.scheme(), "http" | "https"));
        let top = if main { url.clone() } else { web.current_url() };
        let (tab, profile) = {
            let props = self.props.borrow();
            (props.tab.clone(), props.profile.clone())
        };
        let mut guard = self.built.borrow_mut();
        let Some(built) = guard.as_mut().filter(|b| !b.ext.worlds.is_empty()) else { return false };
        let (Some(url), Some(top)) = (url, top) else {
            if main && built.ext.approved.values().any(|t| !t.is_empty()) {
                built.ext.approved.clear();
                install_scripts(built, self.mtm);
            }
            return false;
        };
        let request = NEXT_FRAME_REQUEST.with(|n| n.replace(n.get() + 1));
        let worlds: Vec<&str> = built.ext.worlds.iter().map(|w| w.world.name()).collect();
        let data = json!({ "request": request, "tab": tab, "store": profile, "worlds": worlds, "url": url.as_str(), "top": top.as_str(), "main": main });
        built.ext.pending.insert(request, Held { handler: handler.copy(), preferences: preferences.retain(), url, top, main });
        drop(guard);
        self.emit(crate::ext_scripts::FRAME, data);
        true
    }

    /// The gate's answer to a held navigation: approve exactly what it returned, then let it go on.
    pub(crate) fn frame_answer(&self, request: u64, runs: &[(String, Vec<usize>)]) -> bool {
        let held = {
            let mut built = self.built.borrow_mut();
            let Some(built) = built.as_mut() else { return false };
            let Some(held) = built.ext.pending.remove(&request) else { return false };
            let mut changed = false;
            for (name, run) in runs {
                let Some(world) = World::parse(name).filter(|w| built.ext.worlds.iter().any(|x| x.world == *w)) else { continue };
                let decisions = built.ext.approved.entry(world).or_default();
                let before = decisions.clone();
                if held.main {
                    *decisions = Decisions::default();
                }
                decisions.record(&FrameNav { url: &held.url, top: &held.top, main_frame: held.main }, run.clone());
                changed |= *decisions != before;
            }
            if changed {
                install_scripts(built, self.mtm);
            }
            held
        };
        held.handler.call((WKNavigationActionPolicy::Allow, NonNull::from(&*held.preferences)));
        true
    }

    pub(crate) fn ext_message(&self, world: &World, message: &WKScriptMessage) {
        // SAFETY: WebKit passes a live message on the main thread.
        let (body, frame) = unsafe { (message.body(), message.frameInfo()) };
        let Some(text) = body.downcast_ref::<NSString>().map(|s| s.to_string()) else { return };
        if text.len() < TOKEN || text.len() > TOKEN + tarn_ext::MAX_MESSAGE_BYTES || !text.is_char_boundary(TOKEN) {
            return;
        }
        let (token, json) = text.split_at(TOKEN);
        let Some(origin) = web_origin(&frame).filter(|_| token.bytes().all(|b| b.is_ascii_hexdigit())) else { return };
        // SAFETY: a plain getter of a live frame on the main thread.
        let main = unsafe { frame.isMainFrame() };
        let sent = (!main).then(|| origin.clone());
        let id = {
            let mut built = self.built.borrow_mut();
            let Some(ext) = built.as_mut().map(|b| &mut b.ext) else { return };
            if main {
                ext.main.insert(world.clone(), frame);
                FrameId::MAIN.0
            } else {
                match ext.frames.get_mut(&(world.clone(), token.to_owned())) {
                    Some((id, info, known)) if *known == origin => {
                        *info = frame;
                        *id
                    }
                    Some(_) => return,
                    None if json.is_empty() => {
                        ext.next_frame += 1;
                        let id = ext.next_frame;
                        ext.frames.insert((world.clone(), token.to_owned()), (id, frame, origin));
                        id
                    }
                    None => return,
                }
            }
        };
        if !json.is_empty() {
            self.emit(crate::ext_scripts::MESSAGE, json!({ "world": world.name(), "frame": id, "origin": sent, "json": json }));
        }
    }

    /// The main frame committed a new document: its frames are new.
    pub(crate) fn ext_committed(&self) {
        if let Some(built) = self.built.borrow_mut().as_mut() {
            built.ext.frames.clear();
            built.ext.main.clear();
        }
    }

    pub(crate) fn ext_post(&self, args: &Value) {
        let Some(world) = args.get("world").and_then(Value::as_str).and_then(World::parse) else { return };
        let Some(json) = args.get("json").and_then(Value::as_str) else { return };
        let frame = args.get("frame").and_then(Value::as_u64);
        let (web, targets) = {
            let built = self.built.borrow();
            let Some(built) = built.as_ref().filter(|b| b.ext.worlds.iter().any(|w| w.world == world)) else { return };
            let subframes = built.ext.frames.iter().filter(|((w, _), _)| *w == world);
            let subframe = |((_, token), (_, info, _)): (&(World, String), &(u64, Retained<WKFrameInfo>, String))| (Some(info.clone()), Some(token.clone()));
            let targets: Vec<(Option<Retained<WKFrameInfo>>, Option<String>)> = match frame {
                None => std::iter::once((None, None)).chain(subframes.map(subframe)).collect(),
                Some(0) => vec![(None, None)],
                Some(n) => subframes.filter(|(_, (id, _, _))| *id == n).map(subframe).collect(),
            };
            (built.web.clone(), targets)
        };
        let content = content_world(&world, self.mtm);
        for (target, token) in targets {
            web.evaluate(&NSString::from_str(&engine::tarn_ext::reply(token.as_deref(), json)), target.as_deref(), &content, None);
        }
    }
}
