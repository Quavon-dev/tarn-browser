use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::{Rc, Weak};

use block2::DynBlock;
use engine::view_policy;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSDictionary, NSError, NSHTTPURLResponse, NSInteger, NSObject, NSString, NSURL, NSURLAuthenticationChallenge, NSURLCredential};
use objc2_web_kit::{
    WKDownload, WKFrameInfo, WKMediaCaptureType, WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationResponse,
    WKNavigationResponsePolicy, WKNavigationType, WKPermissionDecision, WKSecurityOrigin, WKUIDelegate, WKWebViewConfiguration, WKWebpagePreferences,
    WKWindowFeatures,
};
use serde_json::json;
use shield::ShieldLevel;
use url::Url;

use crate::component::{DISPOSITION_REJECT, Inner, MediaRequest, Pending, settle};
use crate::webkit::WebView;

pub struct State {
    inner: Weak<Inner>,
    level: ShieldLevel,
}

type ActionHandler = DynBlock<dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>)>;
type ResponseHandler = DynBlock<dyn Fn(WKNavigationResponsePolicy)>;
type AuthHandler = DynBlock<dyn Fn(NSInteger, *mut NSURLCredential)>;

const DISPOSITION_DEFAULT: NSInteger = 1;
const CANCELLED: [(&str, isize); 2] = [("NSURLErrorDomain", -999), ("WebKitErrorDomain", 102)];
const PASSWORD_METHODS: [&str; 3] = ["NSURLAuthenticationMethodHTTPBasic", "NSURLAuthenticationMethodHTTPDigest", "NSURLAuthenticationMethodNTLM"];

fn ns_url(url: Option<Retained<NSURL>>) -> Option<Url> {
    url.and_then(|u| u.absoluteString()).and_then(|s| Url::parse(&s.to_string()).ok())
}

fn frame_origin(frame: &WKFrameInfo) -> Option<Url> {
    // SAFETY: WebKit passes a live frame on the main thread.
    let origin = unsafe { frame.securityOrigin() };
    security_origin(&origin)
}

pub(crate) fn security_origin(origin: &WKSecurityOrigin) -> Option<Url> {
    // SAFETY: plain getters of a live origin on the main thread.
    let (scheme, host, port) = unsafe { (origin.protocol().to_string(), origin.host().to_string(), origin.port()) };
    let port = if port > 0 { format!(":{port}") } else { String::new() };
    Url::parse(&format!("{scheme}://{host}{port}/")).ok()
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnWebViewHandler"]
    #[ivars = State]
    pub struct Handler;

    unsafe impl NSObjectProtocol for Handler {}

    unsafe impl WKNavigationDelegate for Handler {
        #[unsafe(method(webView:decidePolicyForNavigationAction:preferences:decisionHandler:))]
        fn decide_action(&self, web: &WebView, action: &WKNavigationAction, preferences: &WKWebpagePreferences, handler: &ActionHandler) {
            let policy = match self.inner() {
                Some(inner) => {
                    let policy = inner.decide(web, action, preferences, self.ivars().level);
                    if policy != WKNavigationActionPolicy::Allow {
                        inner.end_popup();
                    } else if inner.defer_frame(web, action, preferences, handler) {
                        return;
                    }
                    policy
                }
                None => WKNavigationActionPolicy::Cancel,
            };
            handler.call((policy, NonNull::from(preferences)));
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(&self, _web: &WebView, response: &WKNavigationResponse, handler: &ResponseHandler) {
            handler.call((if is_download(response) { WKNavigationResponsePolicy::Download } else { WKNavigationResponsePolicy::Allow },));
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start_provisional(&self, _web: &WebView, _navigation: Option<&WKNavigation>) {
            self.emit("started", json!({}));
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, web: &WebView, _navigation: Option<&WKNavigation>) {
            if let Some(inner) = self.inner() {
                inner.committed();
                inner.emit("committed", web.page());
            }
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web: &WebView, _navigation: Option<&WKNavigation>) {
            if let Some(inner) = self.inner() {
                inner.page_finished();
            }
            self.emit("finished", web.page());
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(&self, _web: &WebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            if let Some(inner) = self.inner() {
                inner.end_popup();
            }
            self.failed(error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, _web: &WebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            self.failed(error);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn terminated(&self, _web: &WebView) {
            self.emit("crashed", serde_json::Value::Null);
        }

        #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
        fn challenge(&self, web: &WebView, challenge: &NSURLAuthenticationChallenge, handler: &AuthHandler) {
            let space = challenge.protectionSpace();
            let method = space.authenticationMethod().to_string();
            let failures = u32::try_from(challenge.previousFailureCount()).unwrap_or(u32::MAX);
            let origin = space.protocol().and_then(|scheme| Url::parse(&format!("{scheme}://{}:{}/", space.host(), space.port())).ok());
            if !PASSWORD_METHODS.contains(&method.as_str()) {
                return handler.call((DISPOSITION_DEFAULT, std::ptr::null_mut()));
            }
            let allowed = origin.as_ref().is_some_and(|o| view_policy::may_ask_credentials(o, web.current_url().as_ref(), failures));
            let Some(inner) = self.inner().filter(|_| allowed) else {
                return handler.call((DISPOSITION_REJECT, std::ptr::null_mut()));
            };
            let fields = json!({ "realm": space.realm().map(|r| r.to_string()), "insecure": !space.receivesCredentialSecurely() });
            inner.ask("credentials", origin, fields, Pending::Auth(handler.copy()), true);
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_download(&self, _web: &WebView, _action: &WKNavigationAction, download: &WKDownload) {
            self.adopt(download);
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_download(&self, _web: &WebView, _response: &WKNavigationResponse, download: &WKDownload) {
            self.adopt(download);
        }
    }

    unsafe impl WKUIDelegate for Handler {
        #[unsafe(method(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create(&self, web: &WebView, _configuration: &WKWebViewConfiguration, action: &WKNavigationAction, _features: &WKWindowFeatures) -> *mut WebView {
            if let Some(inner) = self.inner() {
                // SAFETY: WebKit passes a live navigation action on the main thread.
                let (url, kind) = unsafe { (ns_url(action.request().URL()), action.navigationType()) };
                inner.popup(web, url, kind == WKNavigationType::LinkActivated);
            }
            std::ptr::null_mut()
        }

        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn alert(&self, _web: &WebView, message: &NSString, frame: &WKFrameInfo, handler: &DynBlock<dyn Fn()>) {
            let pending = Pending::Alert(handler.copy());
            self.dialog("alert", frame, json!({ "message": message.to_string() }), pending);
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn confirm(&self, _web: &WebView, message: &NSString, frame: &WKFrameInfo, handler: &DynBlock<dyn Fn(Bool)>) {
            let pending = Pending::Confirm(handler.copy());
            self.dialog("confirm", frame, json!({ "message": message.to_string() }), pending);
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn text(&self, _web: &WebView, prompt: &NSString, default: Option<&NSString>, frame: &WKFrameInfo, handler: &DynBlock<dyn Fn(*mut NSString)>) {
            let pending = Pending::Text(handler.copy());
            self.dialog("text", frame, json!({ "message": prompt.to_string(), "default": default.map(|d| d.to_string()) }), pending);
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media(&self, _web: &WebView, origin: &WKSecurityOrigin, _frame: &WKFrameInfo, kind: WKMediaCaptureType, handler: &DynBlock<dyn Fn(WKPermissionDecision)>) {
            let Some(inner) = self.inner() else { return handler.call((WKPermissionDecision::Deny,)) };
            let kinds: &[&str] = match kind {
                WKMediaCaptureType::Camera => &["camera"],
                WKMediaCaptureType::Microphone => &["microphone"],
                _ => &["camera", "microphone"],
            };
            let request = Rc::new(MediaRequest { block: handler.copy(), remaining: Cell::new(kinds.len() as u8), granted: Cell::new(true) });
            for kind in kinds {
                inner.ask(kind, security_origin(origin), json!({}), Pending::Media(Rc::clone(&request)), true);
            }
        }
    }

    impl Handler {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe(&self, path: Option<&NSString>, object: Option<&AnyObject>, _change: Option<&NSDictionary>, _context: *mut c_void) {
            let Some(web) = object.and_then(|o| o.downcast_ref::<WebView>()) else { return };
            if path.is_some_and(|p| p.to_string() == "title") {
                self.emit("title", json!({ "title": web.title().map(|t| t.to_string()).unwrap_or_default() }));
            } else {
                self.emit("history", json!({ "canGoBack": web.can_go_back(), "canGoForward": web.can_go_forward() }));
            }
        }
    }
);

fn is_download(response: &WKNavigationResponse) -> bool {
    // SAFETY: WebKit passes a live response on the main thread.
    let (main, shows, response) = unsafe { (response.isForMainFrame(), response.canShowMIMEType(), response.response()) };
    if !main {
        return false;
    }
    let attachment = response
        .downcast_ref::<NSHTTPURLResponse>()
        .and_then(|r| r.valueForHTTPHeaderField(&NSString::from_str("Content-Disposition")))
        .is_some_and(|v| v.to_string().trim_start().to_ascii_lowercase().starts_with("attachment"));
    attachment || !shows
}

impl Handler {
    pub fn new(mtm: MainThreadMarker, inner: Weak<Inner>, level: ShieldLevel) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(State { inner, level });
        // SAFETY: NSObject's designated initializer on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }

    fn inner(&self) -> Option<Rc<Inner>> {
        self.ivars().inner.upgrade()
    }

    fn emit(&self, name: &str, data: serde_json::Value) {
        if let Some(inner) = self.inner() {
            inner.emit(name, data);
        }
    }

    fn dialog(&self, kind: &str, frame: &WKFrameInfo, fields: serde_json::Value, pending: Pending) {
        match self.inner() {
            Some(inner) => inner.ask(kind, frame_origin(frame), fields, pending, true),
            None => settle(pending, false, &serde_json::Value::Null),
        }
    }

    fn failed(&self, error: &NSError) {
        let (domain, code) = (error.domain().to_string(), error.code());
        if CANCELLED.iter().any(|(d, c)| *d == domain && *c == code) {
            return self.emit("stopped", json!({}));
        }
        let key = NSString::from_str("NSErrorFailingURLStringKey");
        let url = error.userInfo().objectForKey(&key).and_then(|v| v.downcast::<NSString>().ok()).map(|s| s.to_string());
        self.emit("failed", json!({ "url": url, "message": error.localizedDescription().to_string() }));
    }

    fn adopt(&self, download: &WKDownload) {
        crate::transfers::adopt(download, self.ivars().inner.clone(), self.mtm());
    }
}
