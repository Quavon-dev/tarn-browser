use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Instant;

use block2::RcBlock;
use engine::nav_policy::{self, Decision, LevelAction, Site, Source};
use engine::view_policy::{self, Levels};
use nsh_platform_ios::{ComponentContext, Components, NativeComponent};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, Message};
use objc2_foundation::{NSError, NSInteger, NSNumber, NSOperationQueue, NSString, NSURL, NSURLCredential, NSURLCredentialPersistence, NSURLRequest};
use objc2_ui_kit::{UIView, UIViewAutoresizing};
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKContentWorld, WKFindConfiguration, WKFindResult, WKNavigationAction, WKNavigationActionPolicy,
    WKNavigationType, WKPermissionDecision, WKUserContentController, WKWebViewConfiguration, WKWebpagePreferences,
};
use serde_json::{Value, json};
use shield::ShieldLevel;
use shield::rules::builtin_rules;
use url::Url;

use crate::find;
use crate::handler::Handler;
use crate::stores;
use crate::webkit::WebView;
use crate::zoom;

const MAX_DIALOGS: u32 = 10;
/// The web view's properties the handler reports: the title, and the history a same-document
/// navigation (`pushState`, a fragment) changes without a `committed`.
const OBSERVED: [&str; 3] = ["title", "canGoBack", "canGoForward"];

thread_local! {
    static NEXT_PROMPT: Cell<u64> = const { Cell::new(1) };
}
const DISPOSITION_USE: NSInteger = 0;
pub const DISPOSITION_REJECT: NSInteger = 3;

pub struct MediaRequest {
    pub block: RcBlock<dyn Fn(WKPermissionDecision)>,
    pub remaining: Cell<u8>,
    pub granted: Cell<bool>,
}

pub enum Pending {
    Alert(RcBlock<dyn Fn()>),
    Confirm(RcBlock<dyn Fn(Bool)>),
    Text(RcBlock<dyn Fn(*mut NSString)>),
    Media(Rc<MediaRequest>),
    Auth(RcBlock<dyn Fn(NSInteger, *mut NSURLCredential)>),
}

#[derive(Clone, Default)]
pub(crate) struct Props {
    pub(crate) styles: Vec<(String, String)>,
    pub(crate) theme_styles: Vec<(String, String)>,
    pub(crate) zaps: Vec<(String, String)>,
    pub(crate) sort_downloads: bool,
    pub(crate) profile: String,
    level: ShieldLevel,
    levels: Value,
    load: Option<(String, u64)>,
    opener: Option<String>,
    active: bool,
}

impl Props {
    pub(crate) fn site(&self, url: &Url) -> ShieldLevel {
        Levels::parse(&self.levels, self.level).level(url)
    }
}

pub(crate) struct Built {
    pub(crate) web: Retained<WebView>,
    pub(crate) handler: Retained<Handler>,
    pub(crate) controller: Retained<WKUserContentController>,
    pub(crate) styles: Option<Vec<(String, String)>>,
    pub(crate) zaps: Option<String>,
    pub(crate) zap_list: Option<Retained<WKContentRuleList>>,
    profile: String,
    pub(crate) level: ShieldLevel,
}

pub struct Inner {
    pub(crate) mtm: MainThreadMarker,
    pub context: ComponentContext,
    root: Retained<UIView>,
    pub(crate) props: RefCell<Props>,
    pub(crate) built: RefCell<Option<Built>>,
    level_dirty: Cell<bool>,
    loaded_seq: Cell<u64>,
    scheduled: Cell<bool>,
    compiling: Cell<bool>,
    shell_load: RefCell<Option<Url>>,
    opener: RefCell<Option<Url>>,
    shell_history: Cell<bool>,
    find_text: RefCell<String>,
    popups: RefCell<view_policy::Burst>,
    downloads: RefCell<view_policy::Burst>,
    zoom: Cell<f64>,
    pending: RefCell<HashMap<u64, Pending>>,
    pub(crate) closed: Cell<bool>,
    pub dialogs: Cell<u32>,
}

fn origin_of(url: &Url) -> String {
    url.origin().ascii_serialization()
}

fn page_origin(web: &WebView) -> String {
    web.current_url().and_then(|u| view_policy::origin(&u)).unwrap_or_default()
}

impl Inner {
    pub fn emit(&self, name: &str, data: Value) {
        self.context.emit(name, data);
    }

    /// Whether this tab may start another download now (`view_policy::DOWNLOAD_BURST`).
    pub fn admit_download(&self, now: std::time::Instant) -> bool {
        self.downloads.borrow_mut().admit(view_policy::DOWNLOAD_BURST, now)
    }

    pub fn profile(&self) -> String {
        self.props.borrow().profile.clone()
    }

    pub fn web(&self) -> Option<Retained<WebView>> {
        self.built.borrow().as_ref().map(|b| b.web.clone())
    }

    fn schedule(self: &Rc<Self>) {
        if self.scheduled.replace(true) {
            return;
        }
        let this = Rc::clone(self);
        let block = RcBlock::new(move || this.flush());
        // SAFETY: the main queue runs the block on the main thread, where every captured object lives.
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
    }

    fn flush(self: &Rc<Self>) {
        self.scheduled.set(false);
        if self.compiling.get() || self.closed.get() {
            return;
        }
        crate::transfers::flush(self);
        self.apply_content();
        let props = self.props.borrow().clone();
        self.apply_active(props.active);
        let load = props.load.clone().filter(|(_, seq)| *seq != self.loaded_seq.get());
        let target = match &load {
            Some((url, _)) => Some(url.clone()),
            None if self.level_dirty.get() => self.web().and_then(|w| w.current_url()).map(|u| u.to_string()),
            None => None,
        };
        let Some(raw) = target else {
            self.level_dirty.set(false);
            return;
        };
        let Ok(url) = Url::parse(&raw) else {
            if let Some((_, seq)) = load {
                self.loaded_seq.set(seq);
            }
            return self.emit("failed", json!({ "url": raw, "message": "This is not a web address." }));
        };
        let level = props.site(&url);
        let same = self.built.borrow().as_ref().is_some_and(|b| b.profile == props.profile && b.level == level);
        if !same {
            let Some(list) = stores::rule_list(level) else { return self.compile(level, load.map(|(_, seq)| seq)) };
            self.build(&props.profile, level, &list);
        } else if load.is_none() {
            self.level_dirty.set(false);
            return;
        }
        self.level_dirty.set(false);
        let opener = load.as_ref().and(props.opener.as_deref()).and_then(|o| Url::parse(o).ok());
        if let Some((_, seq)) = load {
            self.loaded_seq.set(seq);
        }
        self.load(&url, opener);
    }

    fn apply_active(&self, active: bool) {
        if !active && let Some(web) = self.web() {
            web.pause_media(None);
        }
    }

    fn compile(self: &Rc<Self>, level: ShieldLevel, seq: Option<u64>) {
        // SAFETY: main thread (marker); the default store exists in an app container.
        let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(self.mtm) }) else {
            return self.rules_failed(seq, "no content rule list store".into());
        };
        self.compiling.set(true);
        let this = Rc::clone(self);
        let done = RcBlock::new(move |list: *mut WKContentRuleList, error: *mut NSError| {
            this.compiling.set(false);
            // SAFETY: WebKit passes a valid rule list or null, retained for the duration of the call.
            match unsafe { Retained::retain(list) } {
                Some(list) => {
                    stores::keep_rule_list(level, list);
                    this.flush();
                }
                None => {
                    // SAFETY: WebKit passes a valid error or null.
                    let message = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_default();
                    this.rules_failed(seq, message);
                }
            }
        });
        let rules = NSString::from_str(&builtin_rules(&level.policy()));
        let id = NSString::from_str(&format!("tarn.builtin.{}", level.as_str()));
        // SAFETY: main thread; the arguments are valid and WebKit copies the block.
        unsafe { store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(Some(&id), Some(&rules), Some(&done)) };
    }

    fn rules_failed(&self, seq: Option<u64>, message: String) {
        if let Some(seq) = seq {
            self.loaded_seq.set(seq);
        }
        self.level_dirty.set(false);
        self.emit("failed", json!({ "url": null, "message": format!("Shield's rules could not be compiled: {message}") }));
    }

    fn build(self: &Rc<Self>, profile: &str, level: ShieldLevel, list: &WKContentRuleList) {
        self.teardown();
        let mtm = self.mtm;
        let store = stores::store_for(profile, mtm);
        // SAFETY: plain allocations and setters on the main thread; the web view copies the configuration.
        let configuration = unsafe {
            let controller = WKUserContentController::new(mtm);
            controller.addContentRuleList(list);
            let preferences = WKWebpagePreferences::new(mtm);
            preferences.setAllowsContentJavaScript(level.policy().javascript);
            let configuration = WKWebViewConfiguration::new(mtm);
            configuration.setWebsiteDataStore(&store);
            configuration.setUserContentController(&controller);
            configuration.setDefaultWebpagePreferences(Some(&preferences));
            configuration.preferences().setJavaScriptCanOpenWindowsAutomatically(false);
            configuration
        };
        // SAFETY: main thread (marker); the configuration copied the controller, and this getter returns that copy.
        let controller = unsafe { configuration.userContentController() };
        let web = WebView::init_with(WebView::alloc(mtm), self.root.bounds(), &configuration);
        web.setAutoresizingMask(UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight);
        web.set_back_forward_gestures(true);
        let handler = Handler::new(mtm, Rc::downgrade(self), level);
        web.set_navigation_delegate(Some(ProtocolObject::from_ref(&*handler)));
        web.set_ui_delegate(Some(ProtocolObject::from_ref(&*handler)));
        for path in OBSERVED {
            web.add_observer(&handler, &NSString::from_str(path), 1, std::ptr::null_mut());
        }
        self.root.addSubview(&web);
        *self.built.borrow_mut() = Some(Built { web, handler, controller, styles: None, zaps: None, zap_list: None, profile: profile.to_owned(), level });
        self.apply_content();
    }

    fn teardown(&self) {
        if let Some(old) = self.built.borrow_mut().take() {
            for path in OBSERVED {
                old.web.remove_observer(&old.handler, &NSString::from_str(path));
            }
            old.web.set_navigation_delegate(None);
            old.web.set_ui_delegate(None);
            old.web.stop_loading();
            old.web.removeFromSuperview();
        }
    }

    fn load(&self, url: &Url, opener: Option<Url>) {
        let Some(web) = self.web() else { return };
        match nav_policy::decide(url, if opener.is_some() { Source::Popup } else { Source::Typed }) {
            Decision::Allow => {}
            Decision::Prompt => {
                return self.emit("external", json!({ "url": url.as_str(), "origin": opener.as_ref().and_then(view_policy::origin).unwrap_or_default() }));
            }
            Decision::Block => return self.emit("blocked", json!({ "url": url.as_str(), "reason": "nav_policy" })),
        }
        let block_private_network = self.props.borrow().site(url).policy().block_private_network;
        if opener.as_ref().is_some_and(|from| nav_policy::into_private_network(url, Some(from), false, block_private_network)) {
            self.emit("blocked", json!({ "url": url.as_str(), "reason": "local_network" }));
            return self.emit("shielded", json!({ "kind": "local_network", "url": url.as_str() }));
        }
        let Some(ns_url) = NSURL::URLWithString(&NSString::from_str(url.as_str())) else { return };
        *self.shell_load.borrow_mut() = Some(url.clone());
        *self.opener.borrow_mut() = opener;
        web.load_request(&NSURLRequest::requestWithURL(&ns_url));
    }

    pub fn decide(&self, web: &WebView, action: &WKNavigationAction, preferences: &WKWebpagePreferences, view: ShieldLevel) -> WKNavigationActionPolicy {
        // SAFETY: WebKit hands over a live navigation action on the main thread; its frames are valid during the call.
        let (main, raw, kind, download) = unsafe {
            let main = action.targetFrame().is_none_or(|frame| frame.isMainFrame());
            let raw = action.request().URL().and_then(|url| url.absoluteString()).map(|s| s.to_string());
            (main, raw, action.navigationType(), action.shouldPerformDownload())
        };
        if !main {
            return WKNavigationActionPolicy::Allow;
        }
        let history = self.shell_history.take() && kind == WKNavigationType::BackForward;
        let Some(url) = raw.as_deref().and_then(|s| Url::parse(s).ok()) else {
            self.emit("blocked", json!({ "url": raw, "reason": "nav_policy" }));
            return WKNavigationActionPolicy::Cancel;
        };
        match nav_policy::decide(&url, Source::Link) {
            Decision::Allow => {}
            Decision::Prompt => {
                self.emit("external", json!({ "url": url.as_str(), "origin": page_origin(web) }));
                return WKNavigationActionPolicy::Cancel;
            }
            Decision::Block => {
                self.emit("blocked", json!({ "url": url.as_str(), "reason": "nav_policy" }));
                return WKNavigationActionPolicy::Cancel;
            }
        }
        let from_shell = self.shell_load.take().is_some_and(|expected| expected == url);
        let opener = self.opener.borrow().clone();
        let site = Site::new(self.props.borrow().site(&url));
        let policy = site.level.max(view).policy();
        let user_started = (from_shell && opener.is_none()) || history;
        if nav_policy::into_private_network(&url, opener.or_else(|| web.current_url()).as_ref(), user_started, policy.block_private_network) {
            self.emit("blocked", json!({ "url": url.as_str(), "reason": "local_network" }));
            self.emit("shielded", json!({ "kind": "local_network", "url": url.as_str() }));
            return WKNavigationActionPolicy::Cancel;
        }
        if nav_policy::upgrades_to_https(&url, policy.https_only) && !history {
            self.emit("shielded", json!({ "kind": "https_upgrade", "url": url.as_str() }));
        }
        match nav_policy::level_action(site, view, true, from_shell, &url) {
            LevelAction::Continue { javascript } => {
                // SAFETY: the preferences belong to this pending navigation and stay mutable until the handler is called.
                unsafe { preferences.setAllowsContentJavaScript(javascript) };
                if download { WKNavigationActionPolicy::Download } else { WKNavigationActionPolicy::Allow }
            }
            LevelAction::BlockInsecure => {
                self.emit("blocked", json!({ "url": url.as_str(), "reason": "https_only" }));
                WKNavigationActionPolicy::Cancel
            }
            LevelAction::Rebuild => {
                self.emit("blocked", json!({ "url": url.as_str(), "reason": "rebuild" }));
                WKNavigationActionPolicy::Cancel
            }
        }
    }

    pub fn committed(&self) {
        self.dialogs.set(0);
        self.end_popup();
    }

    pub fn end_popup(&self) {
        self.opener.borrow_mut().take();
    }

    pub fn popup(&self, web: &WebView, url: Option<Url>, link: bool) {
        let Some(url) = url else { return };
        if !self.popups.borrow_mut().admit(view_policy::POPUP_BURST, Instant::now()) {
            if let Some(page) = web.current_url() {
                self.emit("shielded", json!({ "kind": "popup", "url": page.as_str() }));
            }
            return;
        }
        match nav_policy::decide(&url, Source::Popup) {
            Decision::Allow => {}
            Decision::Prompt => return self.emit("external", json!({ "url": url.as_str(), "origin": page_origin(web) })),
            Decision::Block => return self.emit("blocked", json!({ "url": url.as_str(), "reason": "nav_policy" })),
        }
        let policy = self.props.borrow().site(&url).policy();
        if nav_policy::into_private_network(&url, web.current_url().as_ref(), false, policy.block_private_network) {
            self.emit("blocked", json!({ "url": url.as_str(), "reason": "shield", "popup": true }));
            return self.emit("shielded", json!({ "kind": "local_network", "url": url.as_str() }));
        }
        self.emit("popup", json!({ "url": url.as_str(), "link": link }));
    }

    pub fn ask(&self, kind: &str, origin: Option<Url>, fields: Value, pending: Pending, dialog: bool) {
        if dialog && self.dialogs.replace(self.dialogs.get() + 1) >= MAX_DIALOGS {
            return settle(pending, false, &Value::Null);
        }
        let id = NEXT_PROMPT.with(|n| n.replace(n.get() + 1));
        let mut data = json!({ "id": id, "kind": kind, "origin": origin.as_ref().map(origin_of).unwrap_or_default() });
        let top = self.web().and_then(|w| w.current_url()).map(|u| origin_of(&u));
        if let (Some(top), Some(origin)) = (top, origin.as_ref().map(origin_of))
            && top != origin
        {
            data["topOrigin"] = json!(top);
        }
        if let (Some(data), Value::Object(fields)) = (data.as_object_mut(), fields) {
            data.extend(fields);
        }
        self.pending.borrow_mut().insert(id, pending);
        self.emit("prompt", data);
    }

    fn command(self: &Rc<Self>, name: &str, args: &Value) {
        if self.script_command(name, args) || crate::transfers::command(name, args, self.mtm) {
            return;
        }
        if name == "answer" {
            let pending = args.get("id").and_then(Value::as_u64).and_then(|id| self.pending.borrow_mut().remove(&id));
            if let Some(pending) = pending {
                settle(pending, args.get("accept").and_then(Value::as_bool).unwrap_or(false), args);
            }
            return;
        }
        let Some(web) = self.web() else { return };
        match name {
            "back" if web.can_go_back() => {
                self.shell_history.set(true);
                web.go_back();
            }
            "forward" if web.can_go_forward() => {
                self.shell_history.set(true);
                web.go_forward();
            }
            "reload" => drop(web.reload()),
            "stop" => web.stop_loading(),
            "zoom" => {
                self.zoom.set(zoom::clamp(args.get("factor").and_then(Value::as_f64).unwrap_or(1.0)));
                self.apply_zoom(&web);
            }
            "find" => self.find(&web, args),
            _ => {}
        }
    }

    pub fn page_finished(&self) {
        if !zoom::is_actual_size(self.zoom.get())
            && let Some(web) = self.web()
        {
            self.apply_zoom(&web);
        }
    }

    fn apply_zoom(&self, web: &WebView) {
        // SAFETY: main thread (marker); the script runs in the shell's isolated world, not the page's.
        let world = unsafe { WKContentWorld::defaultClientWorld(self.mtm) };
        web.evaluate(&NSString::from_str(&zoom::script(self.zoom.get())), None, &world, None);
    }

    fn find(&self, web: &WebView, args: &Value) {
        let text = args.get("text").and_then(Value::as_str).unwrap_or_default().to_owned();
        if text.is_empty() {
            self.find_text.borrow_mut().clear();
            return;
        }
        // SAFETY: main thread (marker); plain setters on a fresh configuration.
        let configuration = unsafe {
            let configuration = WKFindConfiguration::new(self.mtm);
            configuration.setBackwards(args.get("backwards").and_then(Value::as_bool).unwrap_or(false));
            configuration.setWraps(true);
            configuration
        };
        let context = self.context.clone();
        let target = web.retain();
        let script = NSString::from_str(&find::count_script(&text));
        // No script of Tarn's runs on a Lockdown page, so the native result stands as one match.
        let lockdown = self.built.borrow().as_ref().is_none_or(|b| b.level == ShieldLevel::Lockdown);
        let mtm = self.mtm;
        let done = RcBlock::new(move |result: NonNull<WKFindResult>| {
            // SAFETY: WebKit passes a valid result for the duration of the call, on the main thread.
            let found = unsafe { result.as_ref().matchFound() };
            if !found || lockdown {
                return context.emit("found", json!({ "matched": u64::from(found) }));
            }
            let context = context.clone();
            let counted = RcBlock::new(move |value: *mut AnyObject, _error: *mut NSError| {
                // SAFETY: WebKit passes the script's result, or null, on the main thread for the duration of the call.
                let count = unsafe { value.as_ref() }.and_then(|v| v.downcast_ref::<NSNumber>()).map(|n| n.as_f64());
                context.emit("found", json!({ "matched": find::matched(true, count) }));
            });
            // SAFETY: main thread (marker); the count runs in the shell's isolated world.
            let world = unsafe { WKContentWorld::defaultClientWorld(mtm) };
            target.evaluate(&script, None, &world, Some(&counted));
        });
        let query = NSString::from_str(&text);
        if *self.find_text.borrow() == text || lockdown {
            return web.find(&query, Some(&configuration), &done);
        }
        // WebKit finds from the selection, the previous match: a new text starts again at the top.
        *self.find_text.borrow_mut() = text;
        let again = web.retain();
        let restart = RcBlock::new(move |_: *mut AnyObject, _: *mut NSError| again.find(&query, Some(&configuration), &done));
        // SAFETY: main thread (marker); the script runs in the shell's isolated world and only clears the selection.
        let world = unsafe { WKContentWorld::defaultClientWorld(self.mtm) };
        web.evaluate(&NSString::from_str(find::RESTART), None, &world, Some(&restart));
    }

    fn close(&self) {
        self.closed.set(true);
        self.teardown();
        for (_, pending) in self.pending.borrow_mut().drain() {
            settle(pending, false, &Value::Null);
        }
    }
}

pub fn settle(pending: Pending, accept: bool, args: &Value) {
    let field = |name: &str| args.get(name).and_then(Value::as_str);
    match pending {
        Pending::Alert(block) => block.call(()),
        Pending::Confirm(block) => block.call((Bool::new(accept),)),
        Pending::Text(block) => match accept.then(|| NSString::from_str(field("text").unwrap_or_default())) {
            Some(text) => block.call((Retained::as_ptr(&text).cast_mut(),)),
            None => block.call((std::ptr::null_mut(),)),
        },
        Pending::Media(request) => {
            if request.remaining.get() == 0 {
                return;
            }
            request.granted.set(request.granted.get() && accept);
            request.remaining.set(if accept { request.remaining.get() - 1 } else { 0 });
            if request.remaining.get() == 0 {
                request.block.call((if request.granted.get() { WKPermissionDecision::Grant } else { WKPermissionDecision::Deny },));
            }
        }
        Pending::Auth(block) => match (accept, field("user"), field("password")) {
            (true, Some(user), Some(password)) => {
                let credential = NSURLCredential::credentialWithUser_password_persistence(
                    &NSString::from_str(user),
                    &NSString::from_str(password),
                    NSURLCredentialPersistence::ForSession,
                );
                block.call((DISPOSITION_USE, Retained::as_ptr(&credential).cast_mut()));
            }
            _ => block.call((DISPOSITION_REJECT, std::ptr::null_mut())),
        },
    }
}

struct TarnWebView(Rc<Inner>);

impl NativeComponent for TarnWebView {
    fn view(&self) -> &UIView {
        &self.0.root
    }

    fn set_prop(&mut self, name: &str, value: Option<&Value>) {
        let text = || value.and_then(Value::as_str).unwrap_or_default().to_owned();
        {
            let mut props = self.0.props.borrow_mut();
            match name {
                "profile" => {
                    let profile = text();
                    self.0.level_dirty.set(self.0.level_dirty.get() || props.profile != profile);
                    props.profile = profile;
                }
                "level" => {
                    let level = ShieldLevel::parse(&text()).unwrap_or(ShieldLevel::Lockdown);
                    self.0.level_dirty.set(self.0.level_dirty.get() || props.level != level);
                    props.level = level;
                }
                "levels" => {
                    let levels = value.cloned().unwrap_or(Value::Null);
                    self.0.level_dirty.set(self.0.level_dirty.get() || props.levels != levels);
                    props.levels = levels;
                }
                "load" => {
                    props.load = value.and_then(|v| Some((v.get("url")?.as_str()?.to_owned(), v.get("seq")?.as_u64()?)));
                    props.opener = value.and_then(|v| v.get("opener")?.as_str().map(str::to_owned));
                }
                "active" => props.active = value.and_then(Value::as_bool).unwrap_or(false),
                "styles" => props.styles = crate::scripts::host_map(value).into_iter().filter_map(|(h, v)| Some((h, v.as_str()?.to_owned()))).collect(),
                "themeStyles" => props.theme_styles = value.map(engine::page_scripts::theme_sheets).unwrap_or_default(),
                "zaps" => {
                    props.zaps = crate::scripts::host_map(value)
                        .into_iter()
                        .flat_map(|(h, v)| v.as_array().into_iter().flatten().filter_map(Value::as_str).map(|s| (h.clone(), s.to_owned())).collect::<Vec<_>>())
                        .collect()
                }
                "sortDownloads" => props.sort_downloads = value.and_then(Value::as_bool).unwrap_or(false),
                _ => return,
            }
        }
        self.0.schedule();
    }

    fn command(&mut self, name: &str, args: &Value) {
        self.0.command(name, args);
    }
}

impl Drop for TarnWebView {
    fn drop(&mut self) {
        self.0.close();
    }
}

pub fn register(components: &Components) {
    components.register(
        "tarn.webview",
        Rc::new(|context: ComponentContext| -> Box<dyn NativeComponent> {
            let mtm = MainThreadMarker::new().expect("components are made on the main thread");
            let inner = Rc::new_cyclic(|weak| Inner {
                mtm,
                context,
                root: crate::page_root::PageRoot::view(mtm, weak.clone()),
                props: RefCell::new(Props::default()),
                built: RefCell::new(None),
                level_dirty: Cell::new(false),
                loaded_seq: Cell::new(0),
                scheduled: Cell::new(false),
                compiling: Cell::new(false),
                shell_load: RefCell::new(None),
                opener: RefCell::new(None),
                shell_history: Cell::new(false),
                find_text: RefCell::default(),
                popups: RefCell::default(),
                downloads: RefCell::default(),
                zoom: Cell::new(1.0),
                pending: RefCell::new(HashMap::new()),
                closed: Cell::new(false),
                dialogs: Cell::new(0),
            });
            crate::transfers::register(&inner);
            Box::new(TarnWebView(inner))
        }),
    );
}
