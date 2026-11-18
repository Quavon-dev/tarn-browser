use std::ptr::NonNull;

use block2::DynBlock;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadOnly, extern_class, extern_methods};
use objc2_core_foundation::CGRect;
use objc2_foundation::{NSData, NSError, NSObject, NSString, NSURL, NSURLRequest};
use objc2_ui_kit::{UIResponder, UIView};
use objc2_web_kit::{
    WKContentWorld, WKDownload, WKFindConfiguration, WKFindResult, WKFrameInfo, WKNavigation, WKNavigationDelegate, WKUIDelegate, WKWebViewConfiguration,
};

extern_class!(
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WKWebView"]
    pub struct WebView;
);

// SAFETY: the signatures match WKWebView's UIKit headers (objc2-web-kit 0.3.2 declares the class only
// for macOS); all are main-thread methods of a main-thread-only class.
impl WebView {
    extern_methods!(
        #[unsafe(method(initWithFrame:configuration:))]
        pub fn init_with(this: Allocated<Self>, frame: CGRect, configuration: &WKWebViewConfiguration) -> Retained<Self>;
        #[unsafe(method(setNavigationDelegate:))]
        pub fn set_navigation_delegate(&self, delegate: Option<&ProtocolObject<dyn WKNavigationDelegate>>);
        #[unsafe(method(setUIDelegate:))]
        pub fn set_ui_delegate(&self, delegate: Option<&ProtocolObject<dyn WKUIDelegate>>);
        #[unsafe(method(setAllowsBackForwardNavigationGestures:))]
        pub fn set_back_forward_gestures(&self, allow: bool);
        #[unsafe(method(loadRequest:))]
        pub fn load_request(&self, request: &NSURLRequest) -> Option<Retained<WKNavigation>>;
        #[unsafe(method(loadFileURL:allowingReadAccessToURL:))]
        pub fn load_file_url(&self, url: &NSURL, read_access: &NSURL) -> Option<Retained<WKNavigation>>;
        #[unsafe(method(goBack))]
        pub fn go_back(&self) -> Option<Retained<WKNavigation>>;
        #[unsafe(method(goForward))]
        pub fn go_forward(&self) -> Option<Retained<WKNavigation>>;
        #[unsafe(method(reload))]
        pub fn reload(&self) -> Option<Retained<WKNavigation>>;
        #[unsafe(method(reloadFromOrigin))]
        pub fn reload_from_origin(&self) -> Option<Retained<WKNavigation>>;
        #[unsafe(method(stopLoading))]
        pub fn stop_loading(&self);
        #[unsafe(method(canGoBack))]
        pub fn can_go_back(&self) -> bool;
        #[unsafe(method(canGoForward))]
        pub fn can_go_forward(&self) -> bool;
        #[unsafe(method(URL))]
        pub fn url(&self) -> Option<Retained<NSURL>>;
        #[unsafe(method(title))]
        pub fn title(&self) -> Option<Retained<NSString>>;
        #[unsafe(method(evaluateJavaScript:inFrame:inContentWorld:completionHandler:))]
        pub fn evaluate(
            &self,
            script: &NSString,
            frame: Option<&WKFrameInfo>,
            world: &WKContentWorld,
            done: Option<&DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>>,
        );
        #[unsafe(method(callAsyncJavaScript:arguments:inFrame:inContentWorld:completionHandler:))]
        pub fn call_async(
            &self,
            body: &NSString,
            arguments: Option<&AnyObject>,
            frame: Option<&WKFrameInfo>,
            world: &WKContentWorld,
            done: Option<&DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>>,
        );
        #[unsafe(method(resumeDownloadFromResumeData:completionHandler:))]
        pub fn resume_download(&self, data: &NSData, done: &DynBlock<dyn Fn(NonNull<WKDownload>)>);
        #[unsafe(method(findString:withConfiguration:completionHandler:))]
        pub fn find(&self, text: &NSString, configuration: Option<&WKFindConfiguration>, done: &DynBlock<dyn Fn(NonNull<WKFindResult>)>);
        #[unsafe(method(pauseAllMediaPlaybackWithCompletionHandler:))]
        pub fn pause_media(&self, done: Option<&DynBlock<dyn Fn()>>);
        #[unsafe(method(addObserver:forKeyPath:options:context:))]
        pub fn add_observer(&self, observer: &AnyObject, path: &NSString, options: usize, context: *mut std::ffi::c_void);
        #[unsafe(method(removeObserver:forKeyPath:))]
        pub fn remove_observer(&self, observer: &AnyObject, path: &NSString);
    );
}

impl WebView {
    pub fn page(&self) -> serde_json::Value {
        let url = self.url().and_then(|u| u.absoluteString()).map(|s| s.to_string());
        let title = self.title().map(|t| t.to_string()).unwrap_or_default();
        serde_json::json!({ "url": url, "title": title, "canGoBack": self.can_go_back(), "canGoForward": self.can_go_forward() })
    }

    pub fn current_url(&self) -> Option<url::Url> {
        self.url().and_then(|u| u.absoluteString()).and_then(|s| url::Url::parse(&s.to_string()).ok())
    }
}
