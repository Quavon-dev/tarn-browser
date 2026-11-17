//! `tarn.extpage`: an install's own page (its hidden background page or its popup) served from
//! `tarn-extension://`, with the extension bridge in that page's main frame only.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use block2::{DynBlock, RcBlock};
use engine::tarn_ext::SCHEME;
use nsh_platform_ios::{ComponentContext, Components, NativeComponent};
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSData, NSDictionary, NSError, NSHTTPURLResponse, NSObject, NSString, NSURL, NSURLRequest};
use objc2_ui_kit::{UIView, UIViewAutoresizing};
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKContentWorld, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKScriptMessage,
    WKScriptMessageHandler, WKURLSchemeHandler, WKURLSchemeTask, WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebViewConfiguration,
};
use serde_json::{Value, json};
use shield::ShieldLevel;
use shield::rules::builtin_rules;
use url::Url;

use crate::data::{StoreKind, store_kind};
use crate::ext_scripts::{HANDLER, MESSAGE, POST, page_script};
use crate::webkit::WebView;
use crate::{stores, tarn_ext};

type ActionHandler = DynBlock<dyn Fn(WKNavigationActionPolicy)>;

#[derive(Clone, Default, PartialEq)]
struct Props {
    profile: String,
    world: String,
    url: String,
    bootstrap: String,
    level: String,
}

struct Built {
    web: Retained<WebView>,
    controller: Retained<WKUserContentController>,
    _delegate: Retained<PageDelegate>,
    _scheme: Retained<SchemeHandler>,
    props: Props,
}

pub(crate) struct Page {
    mtm: MainThreadMarker,
    context: ComponentContext,
    root: Retained<UIView>,
    props: RefCell<Props>,
    built: RefCell<Option<Built>>,
    compiling: Cell<bool>,
}

fn ns_url(url: Option<Retained<NSURL>>) -> Option<Url> {
    url.and_then(|u| u.absoluteString()).and_then(|s| Url::parse(&s.to_string()).ok())
}

impl Page {
    fn origin(&self) -> Option<String> {
        let url = Url::parse(&self.built.borrow().as_ref()?.props.url).ok()?;
        url.host_str().map(str::to_owned)
    }

    fn update(self: &Rc<Self>) {
        let props = self.props.borrow().clone();
        if self.built.borrow().as_ref().is_some_and(|b| b.props == props) || self.compiling.get() {
            return;
        }
        self.teardown();
        let level = ShieldLevel::parse(&props.level).unwrap_or(ShieldLevel::Lockdown);
        let persistent = matches!(store_kind(&props.profile), StoreKind::Persistent(_));
        let url = Url::parse(&props.url).ok().filter(|u| u.scheme() == SCHEME && u.host_str().is_some());
        let (Some(url), true, false) = (url, persistent, level == ShieldLevel::Lockdown) else { return };
        if props.bootstrap.is_empty() || !props.world.starts_with(engine::tarn_ext::WORLD_PREFIX) {
            return;
        }
        match stores::rule_list(level) {
            Some(list) => self.build(props, &url, &list),
            None => self.compile(level),
        }
    }

    fn compile(self: &Rc<Self>, level: ShieldLevel) {
        // SAFETY: main thread (marker); the default store exists in an app container.
        let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(self.mtm) }) else { return };
        self.compiling.set(true);
        let this = Rc::downgrade(self);
        let done = RcBlock::new(move |list: *mut WKContentRuleList, _error: *mut NSError| {
            // SAFETY: WebKit passes a valid rule list or null, retained for the duration of the call.
            let list = unsafe { Retained::retain(list) };
            let Some(page) = this.upgrade() else { return };
            page.compiling.set(false);
            if let Some(list) = list {
                stores::keep_rule_list(level, list);
                page.update();
            }
        });
        let rules = NSString::from_str(&builtin_rules(&level.policy()));
        let id = NSString::from_str(&format!("tarn.builtin.{}", level.as_str()));
        // SAFETY: main thread; the arguments are valid and WebKit copies the block.
        unsafe { store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(Some(&id), Some(&rules), Some(&done)) };
    }

    fn build(self: &Rc<Self>, props: Props, url: &Url, list: &WKContentRuleList) {
        let mtm = self.mtm;
        let origin = url.host_str().unwrap_or_default().to_owned();
        let scheme = SchemeHandler::new(mtm, props.profile.clone(), origin.clone());
        let delegate = PageDelegate::new(mtm, Rc::downgrade(self));
        // SAFETY: plain allocations and setters on the main thread; the web view copies the
        // configuration. The page script runs only in this page's main frame on its own origin.
        let configuration = unsafe {
            let controller = WKUserContentController::new(mtm);
            controller.addContentRuleList(list);
            let world = WKContentWorld::pageWorld(mtm);
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                WKUserScript::alloc(mtm),
                &NSString::from_str(&page_script(&origin, &props.bootstrap)),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
                &world,
            );
            controller.addUserScript(&script);
            controller.addScriptMessageHandler_contentWorld_name(ProtocolObject::from_ref(&*delegate), &world, &NSString::from_str(HANDLER));
            let configuration = WKWebViewConfiguration::new(mtm);
            configuration.setWebsiteDataStore(&stores::store_for(&props.profile, mtm));
            configuration.setUserContentController(&controller);
            configuration.setURLSchemeHandler_forURLScheme(Some(ProtocolObject::from_ref(&*scheme)), &NSString::from_str(SCHEME));
            configuration.preferences().setJavaScriptCanOpenWindowsAutomatically(false);
            configuration
        };
        // SAFETY: main thread (marker); the configuration copied the controller, and this getter returns that copy.
        let controller = unsafe { configuration.userContentController() };
        let web = WebView::init_with(WebView::alloc(mtm), self.root.bounds(), &configuration);
        web.setAutoresizingMask(UIViewAutoresizing::FlexibleWidth | UIViewAutoresizing::FlexibleHeight);
        web.set_navigation_delegate(Some(ProtocolObject::from_ref(&*delegate)));
        self.root.addSubview(&web);
        if let Some(ns_url) = NSURL::URLWithString(&NSString::from_str(url.as_str())) {
            web.load_request(&NSURLRequest::requestWithURL(&ns_url));
        }
        *self.built.borrow_mut() = Some(Built { web, controller, _delegate: delegate, _scheme: scheme, props });
    }

    fn teardown(&self) {
        if let Some(old) = self.built.borrow_mut().take() {
            // SAFETY: main thread; the handler was added under this name in the page world.
            unsafe { old.controller.removeScriptMessageHandlerForName_contentWorld(&NSString::from_str(HANDLER), &WKContentWorld::pageWorld(self.mtm)) };
            old.web.set_navigation_delegate(None);
            old.web.stop_loading();
            old.web.removeFromSuperview();
        }
    }

    fn message(&self, message: &WKScriptMessage) {
        // SAFETY: WebKit passes a live message on the main thread; its frame is valid during the call.
        let (body, main, origin) = unsafe {
            let frame = message.frameInfo();
            let origin = frame.securityOrigin();
            (message.body(), frame.isMainFrame(), (origin.protocol().to_string(), origin.host().to_string()))
        };
        if !main || origin.0 != SCHEME || Some(origin.1) != self.origin() {
            return;
        }
        let Some(json) = body.downcast_ref::<NSString>().map(|s| s.to_string()).filter(|j| j.len() <= tarn_ext::MAX_MESSAGE_BYTES) else { return };
        let world = self.built.borrow().as_ref().map(|b| b.props.world.clone()).unwrap_or_default();
        self.context.emit(MESSAGE, json!({ "world": world, "frame": 0, "json": json }));
    }

    fn post(&self, args: &Value) {
        let built = self.built.borrow();
        let Some(built) = built.as_ref() else { return };
        if args.get("world").and_then(Value::as_str) != Some(built.props.world.as_str()) {
            return;
        }
        let Some(json) = args.get("json").and_then(Value::as_str) else { return };
        // SAFETY: main thread (marker); the extension's own page runs in the page world.
        let world = unsafe { WKContentWorld::pageWorld(self.mtm) };
        built.web.evaluate(&NSString::from_str(&engine::tarn_ext::reply(None, json)), None, &world, None);
    }
}

pub(crate) struct DelegateState {
    page: Weak<Page>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnExtensionPageDelegate"]
    #[ivars = DelegateState]
    pub(crate) struct PageDelegate;

    unsafe impl NSObjectProtocol for PageDelegate {}

    unsafe impl WKNavigationDelegate for PageDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide(&self, _web: &WebView, action: &WKNavigationAction, handler: &ActionHandler) {
            // SAFETY: WebKit hands over a live navigation action on the main thread.
            let (main, url) = unsafe { (action.targetFrame().is_none_or(|f| f.isMainFrame()), ns_url(action.request().URL())) };
            let origin = self.ivars().page.upgrade().and_then(|p| p.origin());
            let allowed = origin.zip(url).is_some_and(|(origin, url)| engine::tarn_ext::page_loads(&url, &origin, main));
            handler.call((if allowed { WKNavigationActionPolicy::Allow } else { WKNavigationActionPolicy::Cancel },));
        }
    }

    unsafe impl WKScriptMessageHandler for PageDelegate {
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn received(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            if let Some(page) = self.ivars().page.upgrade() {
                page.message(message);
            }
        }
    }
);

impl PageDelegate {
    fn new(mtm: MainThreadMarker, page: Weak<Page>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateState { page });
        // SAFETY: NSObject's designated initializer on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

pub(crate) struct SchemeState {
    store: String,
    origin: String,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnExtensionSchemeHandler"]
    #[ivars = SchemeState]
    pub(crate) struct SchemeHandler;

    unsafe impl NSObjectProtocol for SchemeHandler {}

    unsafe impl WKURLSchemeHandler for SchemeHandler {
        #[unsafe(method(webView:startURLSchemeTask:))]
        fn start(&self, _web: &WebView, task: &ProtocolObject<dyn WKURLSchemeTask>) {
            self.serve(task);
        }

        #[unsafe(method(webView:stopURLSchemeTask:))]
        fn stop(&self, _web: &WebView, _task: &ProtocolObject<dyn WKURLSchemeTask>) {}
    }
);

impl SchemeHandler {
    fn new(mtm: MainThreadMarker, store: String, origin: String) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SchemeState { store, origin });
        // SAFETY: NSObject's designated initializer on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }

    /// Answers the task at once with the bytes the profile's resolver gives for its URL, for the
    /// page's own origin only.
    fn serve(&self, task: &ProtocolObject<dyn WKURLSchemeTask>) {
        // SAFETY: WebKit hands over a live task on the main thread.
        let ns = unsafe { task.request() }.URL();
        let url = ns_url(ns.clone()).filter(|u| u.host_str() == Some(self.ivars().origin.as_str()));
        let resource = url.as_ref().zip(tarn_ext::resources(&self.ivars().store)).map(|(url, resources)| resources(url));
        let response = ns.zip(resource.as_ref()).and_then(|(ns, r)| {
            let mut keys = vec![NSString::from_str("Content-Type"), NSString::from_str("X-Content-Type-Options")];
            let mut values = vec![NSString::from_str(r.mime), NSString::from_str("nosniff")];
            if let Some(csp) = r.csp {
                keys.push(NSString::from_str("Content-Security-Policy"));
                values.push(NSString::from_str(csp));
            }
            let headers = NSDictionary::from_slices(&keys.iter().map(|k| &**k).collect::<Vec<_>>(), &values.iter().map(|v| &**v).collect::<Vec<_>>());
            NSHTTPURLResponse::initWithURL_statusCode_HTTPVersion_headerFields(
                NSHTTPURLResponse::alloc(),
                &ns,
                isize::try_from(r.status).unwrap_or(500),
                Some(&NSString::from_str("HTTP/1.1")),
                Some(&headers),
            )
        });
        // SAFETY: the task is live until it is finished or failed, each called once here on the main thread.
        unsafe {
            match (response, resource) {
                (Some(response), Some(resource)) => {
                    task.didReceiveResponse(&response);
                    task.didReceiveData(&NSData::from_vec(resource.body));
                    task.didFinish();
                }
                _ => task.didFailWithError(&NSError::errorWithDomain_code_userInfo(&NSString::from_str("NSURLErrorDomain"), -1100, None)),
            }
        }
    }
}

struct TarnExtPage(Rc<Page>);

impl NativeComponent for TarnExtPage {
    fn view(&self) -> &UIView {
        &self.0.root
    }

    fn set_prop(&mut self, name: &str, value: Option<&Value>) {
        let text = value.and_then(Value::as_str).unwrap_or_default().to_owned();
        {
            let mut props = self.0.props.borrow_mut();
            match name {
                "profile" => props.profile = text,
                "world" => props.world = text,
                "url" => props.url = text,
                "bootstrap" => props.bootstrap = text,
                "level" => props.level = text,
                _ => return,
            }
        }
        self.0.update();
    }

    fn command(&mut self, name: &str, args: &Value) {
        if name == POST {
            self.0.post(args);
        }
    }
}

impl Drop for TarnExtPage {
    fn drop(&mut self) {
        self.0.teardown();
    }
}

pub fn register(components: &Components) {
    components.register(
        "tarn.extpage",
        Rc::new(|context: ComponentContext| -> Box<dyn NativeComponent> {
            let mtm = MainThreadMarker::new().expect("components are made on the main thread");
            Box::new(TarnExtPage(Rc::new(Page {
                mtm,
                context,
                root: UIView::new(mtm),
                props: RefCell::new(Props::default()),
                built: RefCell::new(None),
                compiling: Cell::new(false),
            })))
        }),
    );
}
