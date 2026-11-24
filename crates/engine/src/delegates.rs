use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Instant, SystemTime};

use block2::{DynBlock, RcBlock};
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::date::CFDate;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::string::{CFString, CFStringRef};
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObjectProtocol, ProtocolObject, Sel};
use objc2::sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem, NSModalResponse, NSModalResponseOK, NSOpenPanel, NSUserInterfaceItemIdentification,
};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSHTTPURLResponse, NSObject, NSProgressReporting, NSRect, NSString, NSTimer, NSURL, NSURLAuthenticationChallenge,
    NSURLAuthenticationMethodClientCertificate, NSURLAuthenticationMethodHTTPBasic, NSURLAuthenticationMethodHTTPDigest, NSURLCredential,
    NSURLCredentialPersistence, NSURLProtectionSpace, NSURLQuarantinePropertiesKey, NSURLResponse, NSURLSessionAuthChallengeDisposition,
};
use objc2_web_kit::{
    WKDownload, WKDownloadDelegate, WKFrameInfo, WKMediaCaptureType, WKNavigation, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKNavigationResponse, WKNavigationResponsePolicy, WKNavigationType, WKOpenPanelParameters, WKPermissionDecision, WKSecurityOrigin, WKUIDelegate, WKWebView,
    WKWebViewConfiguration, WKWebpagePreferences, WKWebsiteDataStore, WKWindowFeatures,
};
use security_framework::identity::SecIdentity;
use shield::ShieldLevel;
use url::Url;

use crate::downloads::{self, Transfer};
use crate::nav_policy::{self, Decision, GateAction, Site, SiteFor, Source};
use crate::prompts::{self, Prompt, PromptKind, Reply};
use crate::webview::Popup;
use crate::{Event, Protection};

pub(crate) use crate::page::{DOWNLOAD_BURST, POPUP_BURST, PROMPT_BURST, blocked_message, bounded, serialized_origin, within_burst};
const MAX_AUTH_ATTEMPTS: isize = crate::view_policy::MAX_AUTH_ATTEMPTS as isize;

// WebKit hands over a forwarding proxy (WKNSURLAuthenticationChallenge) whose methods objc2's
// debug-build signature check cannot find, so these two getters are sent without that check.
fn protection_space(challenge: &NSURLAuthenticationChallenge) -> Option<Retained<NSURLProtectionSpace>> {
    type Getter = unsafe extern "C-unwind" fn(*const NSURLAuthenticationChallenge, Sel) -> *mut NSURLProtectionSpace;
    // SAFETY: `objc_msgSend` called with the exact signature of `-[NSURLAuthenticationChallenge
    // protectionSpace]`, which returns a +0 object or nil; it is retained before use.
    unsafe {
        let send: Getter = std::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn());
        Retained::retain(send(challenge, sel!(protectionSpace)))
    }
}

fn previous_failures(challenge: &NSURLAuthenticationChallenge) -> isize {
    type Getter = unsafe extern "C-unwind" fn(*const NSURLAuthenticationChallenge, Sel) -> isize;
    // SAFETY: `objc_msgSend` called with the exact signature of `-[NSURLAuthenticationChallenge
    // previousFailureCount]`, which returns an NSInteger.
    unsafe {
        let send: Getter = std::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn());
        send(challenge, sel!(previousFailureCount))
    }
}

/// The Keychain identities (certificate plus private key) valid now and, when the server names
/// the issuers it accepts, issued by one of them; each named by its certificate's subject.
fn keychain_identities(issuers: Option<Retained<NSArray<NSData>>>) -> (Vec<String>, Vec<SecIdentity>) {
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        static kSecClass: CFStringRef;
        static kSecClassIdentity: CFStringRef;
        static kSecReturnRef: CFStringRef;
        static kSecMatchLimit: CFStringRef;
        static kSecMatchLimitAll: CFStringRef;
        static kSecMatchIssuers: CFStringRef;
        static kSecMatchValidOnDate: CFStringRef;
        fn SecItemCopyMatching(query: CFDictionaryRef, result: *mut CFTypeRef) -> i32;
    }
    // SAFETY: the Security framework's constant keys are valid for the life of the process;
    // NSArray<NSData> is toll-free bridged to a CFArray of CFData, retained by `issuers` while the
    // query is built; SecItemCopyMatching returns a +1 CFArray of SecIdentityRef for a
    // kSecReturnRef, kSecMatchLimitAll identity query, or nothing on failure.
    unsafe {
        let key = |k: CFStringRef| CFString::wrap_under_get_rule(k);
        let mut query = vec![
            (key(kSecClass), key(kSecClassIdentity).into_CFType()),
            (key(kSecReturnRef), CFBoolean::true_value().into_CFType()),
            (key(kSecMatchLimit), key(kSecMatchLimitAll).into_CFType()),
            (key(kSecMatchValidOnDate), CFDate::now().into_CFType()),
        ];
        if let Some(issuers) = issuers.filter(|i| i.count() > 0) {
            query.push((key(kSecMatchIssuers), CFType::wrap_under_get_rule(Retained::as_ptr(&issuers).cast())));
        }
        let query = CFDictionary::from_CFType_pairs(&query);
        let mut found: CFTypeRef = std::ptr::null();
        if SecItemCopyMatching(query.as_concrete_TypeRef(), &mut found) != 0 || found.is_null() {
            return Default::default();
        }
        let found = CFArray::<SecIdentity>::wrap_under_create_rule(found as CFArrayRef);
        found.iter().filter_map(|identity| Some((identity.certificate().ok()?.subject_summary(), identity.clone()))).unzip()
    }
}

pub(crate) fn identity_credential(identity: &SecIdentity) -> Option<Retained<NSURLCredential>> {
    type Make = unsafe extern "C-unwind" fn(*const AnyClass, Sel, *const c_void, *const NSArray, NSURLCredentialPersistence) -> *mut NSURLCredential;
    // SAFETY: `objc_msgSend` called with the exact signature of `+[NSURLCredential
    // credentialWithIdentity:certificates:persistence:]` and a live SecIdentityRef; it returns a
    // +0 credential or nil, which is retained.
    unsafe {
        let send: Make = std::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn());
        let made = send(
            <NSURLCredential as objc2::ClassType>::class(),
            sel!(credentialWithIdentity:certificates:persistence:),
            identity.as_concrete_TypeRef().cast(),
            std::ptr::null(),
            NSURLCredentialPersistence::ForSession,
        );
        Retained::retain(made)
    }
}

fn navigation_id(navigation: Option<&WKNavigation>) -> usize {
    navigation.map_or(0, |n| n as *const WKNavigation as usize)
}

/// `None` for opaque origins (sandboxed frames, data:), which cannot be shown or remembered.
pub(crate) fn origin_of(frame: &WKFrameInfo) -> Option<String> {
    // SAFETY: main-thread reads of a live frame's security origin.
    let (scheme, host, port) = unsafe {
        let origin = frame.securityOrigin();
        (origin.protocol().to_string(), origin.host().to_string(), origin.port())
    };
    if scheme.is_empty() || host.is_empty() {
        return None;
    }
    let host = if host.contains(':') { format!("[{host}]") } else { host };
    let raw = if port == 0 { format!("{scheme}://{host}") } else { format!("{scheme}://{host}:{port}") };
    serialized_origin(&Url::parse(&raw).ok()?)
}

thread_local! {
    /// Destinations handed to downloads that have not finished, across all profiles.
    static RESERVED: RefCell<HashSet<PathBuf>> = RefCell::new(HashSet::new());
}

pub(crate) struct DelegateState {
    pub sink: RefCell<Rc<dyn Fn(Event)>>,
    /// Events from a popup that the shell has not adopted yet.
    pub pending: RefCell<Option<Vec<Event>>>,
    pub site_for: SiteFor,
    pub level: ShieldLevel,
    /// Built with the ad and tracker rules; see `WebView::filtered`.
    pub filtered: bool,
    pub downloads: Retained<DownloadDelegate>,
    /// URL of the load the shell itself started, so a level change can rebuild the view for it
    /// without turning page-initiated navigations into browser-initiated ones.
    /// The second field marks a link the page chose (a peek), which is not the user's navigation.
    pub shell_load: RefCell<Option<(Url, bool)>>,
    /// Files this view showed through `load_file`; only they get Open File's credit (S6).
    pub opened_files: RefCell<std::collections::HashSet<Url>>,
    pub agent_gate: RefCell<Option<crate::agent_scripts::Gate>>,
    /// Whether the main-frame navigation in flight was started by the main frame or the shell;
    /// a download response for one started by an iframe is cancelled.
    pub main_initiated: Cell<bool>,
    /// The main frame's navigation in flight was started by the user (shell or history);
    /// WebKit reports its redirect hops with the original action's type, `Other` for a shell load.
    pub user_navigation: Cell<bool>,
    /// Where a back, forward or reload the shell asked for may go; a script's `history.back()` or
    /// `location.reload()` reaches WebKit with the same navigation type.
    pub history_step: RefCell<Vec<Url>>,
    /// Shared with popups the page opens, so a chain of popups stays within one budget.
    pub recent_prompts: Rc<RefCell<VecDeque<Instant>>>,
    pub recent_popups: Rc<RefCell<VecDeque<Instant>>>,
    pub recent_downloads: Rc<RefCell<VecDeque<Instant>>>,
    /// Set when the user silences the page's dialogs; cleared on the next committed navigation.
    pub silenced: Rc<Cell<bool>>,
    /// Client certificate answers of the page being loaded or shown; a new page asks again.
    pub certificates: Rc<RefCell<prompts::Certificates>>,
    /// Address of the main-frame load between its start and its commit or failure; an external-scheme
    /// request then may come from a server redirect, so its source frame is not named.
    pub provisional: Cell<Option<usize>>,
    /// The navigation the shell started for the user with `load`; its redirect hops stay the user's.
    /// It is kept alive so that no later navigation can have its address.
    pub user_load: RefCell<Option<Retained<WKNavigation>>>,
    /// Origin of the last committed page; `URL` already points at a pending navigation's target.
    pub committed_origin: RefCell<Option<String>>,
    /// URL of the page on screen: set on commit and on same-document changes.
    pub committed_url: RefCell<Option<Url>>,
    /// The origin of the last page with one: `about:blank` and other origin-less pages keep it,
    /// so they cannot launder a public page's navigation to a private address (S10).
    pub web_origin: RefCell<Option<String>>,
    pub last_title: RefCell<String>,
    pub after_commit: RefCell<Option<Box<dyn FnOnce()>>>,
    /// When the user chose Open Link in Split View: the window WebKit asks for within moments opens
    /// beside the page, and a later one does not.
    pub split_link: Cell<Option<std::time::Instant>>,
    /// The commands a selection's menu offers after Ask Assistant about Selection; none hides both.
    pub assistant_menu: RefCell<Option<Vec<String>>>,
}

impl DelegateState {
    pub(crate) fn new(
        sink: Rc<dyn Fn(Event)>,
        site_for: SiteFor,
        level: ShieldLevel,
        filtered: bool,
        downloads: Retained<DownloadDelegate>,
        pending: bool,
    ) -> Self {
        DelegateState {
            sink: RefCell::new(sink),
            pending: RefCell::new(pending.then(Vec::new)),
            site_for,
            level,
            filtered,
            downloads,
            shell_load: RefCell::default(),
            opened_files: RefCell::default(),
            agent_gate: RefCell::default(),
            main_initiated: Cell::new(true),
            user_navigation: Cell::new(false),
            history_step: RefCell::default(),
            provisional: Cell::new(None),
            user_load: RefCell::default(),
            recent_prompts: Rc::default(),
            recent_popups: Rc::default(),
            recent_downloads: Rc::default(),
            silenced: Rc::default(),
            certificates: Rc::default(),
            committed_origin: RefCell::default(),
            committed_url: RefCell::default(),
            web_origin: RefCell::default(),
            last_title: RefCell::default(),
            after_commit: RefCell::default(),
            split_link: Cell::new(None),
            assistant_menu: RefCell::default(),
        }
    }
}

const PROGRESS_INTERVAL: f64 = 0.5;

pub(crate) struct ActiveDownload {
    id: u64,
    url: Option<Url>,
    path: PathBuf,
    started: SystemTime,
    transfer: Transfer<Retained<WKDownload>, Retained<NSData>>,
    done: u64,
    total: Option<u64>,
    /// The view a resumed download continues in, kept until it ends.
    view: Option<Retained<WKWebView>>,
    /// Kept to check where a resumed download comes from.
    source: Option<DownloadSource>,
    /// The user was already asked about the malware list for it.
    asked: bool,
}

impl ActiveDownload {
    fn is(&self, download: &WKDownload) -> bool {
        self.transfer.handle().is_some_and(|d| std::ptr::eq(&**d, download))
    }

    fn progress(&self) -> Event {
        Event::DownloadProgress { id: self.id, path: self.path.clone(), done: self.done, total: self.total, paused: !self.transfer.is_saving() }
    }
}

/// Where a download came from.
#[derive(Clone)]
pub(crate) struct DownloadSource {
    site_for: SiteFor,
    page: Option<Url>,
    tab: Weak<Delegate>,
}

pub(crate) struct DownloadState {
    pub sink: Rc<dyn Fn(Event)>,
    pub active: RefCell<Vec<ActiveDownload>>,
    sources: RefCell<HashMap<usize, DownloadSource>>,
    /// Ids of downloads the user declined at the malware prompt, which fail without a notice.
    declined: RefCell<HashSet<u64>>,
    next_id: Cell<u64>,
    /// New downloads go into a Downloads subfolder by type.
    pub sort: Cell<bool>,
    timer: RefCell<Option<Retained<NSTimer>>>,
}

fn download_key(download: &WKDownload) -> usize {
    download as *const WKDownload as usize
}

fn is_attachment(response: &NSURLResponse) -> bool {
    let Some(http) = response.downcast_ref::<NSHTTPURLResponse>() else { return false };
    http.valueForHTTPHeaderField(&NSString::from_str("Content-Disposition"))
        .is_some_and(|v| v.to_string().trim_start().to_ascii_lowercase().starts_with("attachment"))
}

/// The page title, cut to `MAX_TITLE` characters.
pub(crate) fn page_title(view: &WKWebView) -> String {
    // SAFETY: main-thread read of a live web view.
    let title = unsafe { view.title() }.map(|t| t.to_string()).unwrap_or_default();
    bounded(&title)
}

fn store_link_activated(action: &WKNavigationAction, shell_load: bool, page_link: bool, redirect: bool) -> bool {
    let known = action.respondsToSelector(sel!(_isRedirect)) && action.respondsToSelector(sel!(_isUserInitiated));
    // SAFETY: main-thread reads of the live action; the private selector exists (checked first) and returns a BOOL.
    let clicked = known && unsafe { action.navigationType() == WKNavigationType::LinkActivated && msg_send![action, _isUserInitiated] };
    nav_policy::store_link_asks(nav_policy::StoreNav { redirect, shell_load, page_link, clicked })
}

pub(crate) fn to_url(url: Option<Retained<NSURL>>) -> Option<Url> {
    let s = url?.absoluteString()?.to_string();
    Url::parse(&s).ok()
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnWebViewDelegate"]
    #[ivars = DelegateState]
    pub(crate) struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    impl Delegate {
        // WebKit's link menu opens a new window, which Tarn shows as a tab: the entry says so, and
        // Open Link in Split View takes the same path, with its pop-up checks, in `create_web_view`.
        #[unsafe(method_id(_webView:contextMenu:forElement:))]
        fn context_menu(&self, _web_view: &WKWebView, menu: &NSMenu, _element: &AnyObject) -> Retained<NSMenu> {
            self.ivars().split_link.set(None);
            let new_window = NSString::from_str("WKMenuItemIdentifierOpenLinkInNewWindow");
            let items = menu.itemArray();
            if let Some(at) = items.iter().position(|item| item.identifier().is_some_and(|id| *id == *new_window)) {
                let item = items.objectAtIndex(at as _);
                item.setTitle(&NSString::from_str("Open Link in New Tab"));
                let mtm = self.mtm();
                // SAFETY: `openLinkInSplitView:` is implemented below with the `(id)sender` signature AppKit sends;
                // the menu holds its target weakly, and WebKit keeps this delegate for as long as the view shows a menu.
                let split = unsafe {
                    let split = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str("Open Link in Split View"),
                        Some(sel!(openLinkInSplitView:)),
                        &NSString::new(),
                    );
                    split.setTarget(Some(self));
                    split.setRepresentedObject(Some(&item));
                    split
                };
                menu.insertItem_atIndex(&split, at as isize + 1);
            }
            let copy = NSString::from_str("WKMenuItemIdentifierCopy");
            let selected = menu.itemArray().iter().any(|item| item.identifier().is_some_and(|id| *id == *copy));
            if let Some(commands) = self.ivars().assistant_menu.borrow().as_ref().filter(|_| selected) {
                self.add_assistant_items(menu, commands);
            }
            menu.retain()
        }

        #[unsafe(method(askAssistant:))]
        fn ask_assistant(&self, sender: &NSMenuItem) {
            let command = (sender.tag() >= 0).then(|| sender.title().to_string());
            self.emit(Event::AssistantSelection(command));
        }

        #[unsafe(method(_webView:mouseDidMoveOverElement:withFlags:userInfo:))]
        fn mouse_over(&self, _web_view: &WKWebView, hit: Option<&NSObject>, _flags: usize, _info: Option<&AnyObject>) {
            let hit = hit.filter(|h| h.respondsToSelector(sel!(absoluteLinkURL)));
            // SAFETY: `_WKHitTestResult` answers `absoluteLinkURL` (checked above) with a nullable NSURL.
            let link: Option<Retained<NSURL>> = hit.and_then(|h| unsafe { msg_send![h, absoluteLinkURL] });
            let link = link.and_then(|u| u.absoluteString()).map(|s| if s.length() > crate::view_policy::MAX_HOVERED_LINK { s.substringToIndex(crate::view_policy::MAX_HOVERED_LINK) } else { s });
            self.emit(Event::LinkHovered(link.map(|s| s.to_string())));
        }

        /// The download button of WebKit's PDF viewer: saves the document on screen, with no new request.
        #[unsafe(method(_webView:saveDataToFile:suggestedFilename:mimeType:originatingURL:))]
        fn save_data_to_file(&self, web_view: &WKWebView, data: &NSData, name: &NSString, mime: &NSString, url: Option<&NSURL>) {
            if mime.to_string().eq_ignore_ascii_case("application/pdf") {
                // An `<embed>` or `<object>` PDF may report its page's address, so the PDF must be the
                // main frame's whole document, not only share its address.
                let known = web_view.respondsToSelector(sel!(_isDisplayingPDF));
                // SAFETY: `_isDisplayingPDF` (checked above) answers a BOOL for a live web view on the main thread.
                let whole_page = known && unsafe { msg_send![web_view, _isDisplayingPDF] };
                self.save_shown_pdf(data, &downloads::pdf_name(&name.to_string()), to_url(url.map(|u| u.retain())).filter(|_| whole_page));
            }
        }

        #[unsafe(method(openLinkInSplitView:))]
        fn open_link_in_split_view(&self, sender: &NSMenuItem) {
            let Some(item) = sender.representedObject().and_then(|o| o.downcast::<NSMenuItem>().ok()) else { return };
            let (Some(action), Some(target)) = (item.action(), item.target()) else { return };
            self.ivars().split_link.set(Some(std::time::Instant::now()));
            // SAFETY: sends WebKit's own action for its Open Link in New Window entry, with that entry as sender, as AppKit would.
            unsafe { NSApplication::sharedApplication(self.mtm()).sendAction_to_from(action, Some(&target), Some(&item)) };
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&AnyObject>,
            _context: *mut c_void,
        ) {
            let Some(view) = object.and_then(|o| o.downcast_ref::<WKWebView>()) else { return };
            if key_path.is_some_and(|k| matches!(k.to_string().as_str(), "URL" | "canGoBack" | "canGoForward")) {
                // Loads report their URL through `Committed`; this catches same-document changes
                // such as pushState.
                // SAFETY: main-thread reads of the observed, live web view.
                let (loading, url, can_go_back, can_go_forward) = unsafe { (view.isLoading(), view.URL(), view.canGoBack(), view.canGoForward()) };
                if !loading {
                    self.ivars().history_step.take();
                    if let Some(url) = to_url(url) {
                        *self.ivars().committed_url.borrow_mut() = Some(url.clone());
                        self.emit(Event::UrlChanged { url, can_go_back, can_go_forward });
                    }
                }
                return;
            }
            let title = page_title(view);
            if self.ivars().last_title.replace(title.clone()) != title {
                self.emit(Event::TitleChanged(title));
            }
        }
    }

    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:preferences:decisionHandler:))]
        fn decide_policy(
            &self,
            web_view: &WKWebView,
            action: &WKNavigationAction,
            preferences: &WKWebpagePreferences,
            handler: &DynBlock<dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>)>,
        ) {
            let prefs = NonNull::from(preferences);
            // SAFETY: WebKit hands us a live navigation action on the main thread.
            let is_main = unsafe { action.targetFrame() }
                // SAFETY: frame info is valid for the duration of this call.
                .is_some_and(|f| unsafe { f.isMainFrame() });
            // SAFETY: the action's request is a valid NSURLRequest.
            let url = to_url(unsafe { action.request() }.URL());
            // SAFETY: plain property read on the live action.
            let wants_download = unsafe { action.shouldPerformDownload() };
            // Only a server redirect continues the load that started it. Without WebKit's private
            // `_isRedirect` no hop counts as one.
            // SAFETY: the selector exists on this action (checked first) and returns a BOOL.
            let redirect = action.respondsToSelector(sel!(_isRedirect)) && unsafe { msg_send![action, _isRedirect] };
            let (mut history_step, mut link) = (Vec::new(), false);
            let from_shell = if is_main {
                // A redirect hop of the load before must not use up the mark of the one the shell just started.
                let mark = self.ivars().shell_load.borrow().clone().filter(|(expected, _)| Some(expected) == url.as_ref());
                if mark.is_some() || !redirect {
                    self.ivars().shell_load.take();
                }
                history_step = self.ivars().history_step.take();
                // SAFETY: frame info is valid for the duration of this call.
                let source_main = unsafe { action.sourceFrame() }.is_none_or(|f| unsafe { f.isMainFrame() });
                self.ivars().main_initiated.set(source_main);
                let from_shell = mark.is_some();
                if from_shell {
                    self.ivars().silenced.set(false);
                    link = mark.is_some_and(|(_, link)| link);
                }
                from_shell
            } else {
                false
            };
            let state = self.ivars();
            // The gate runs for every frame and every redirect hop, before the branches below.
            let site = url.as_ref().map(|u| (state.site_for)(u));
            if let (Some(u), Some(site)) = (&url, site) {
                match nav_policy::gate(site.blocked, is_main, from_shell, wants_download) {
                    GateAction::Allow => {}
                    GateAction::Warn(category) | GateAction::Notice(category) => {
                        self.emit(Event::Blocked { url: u.clone(), message: blocked_message(category, u), category: Some(category) });
                        handler.call((WKNavigationActionPolicy::Cancel, prefs));
                        return;
                    }
                    GateAction::Cancel => {
                        handler.call((WKNavigationActionPolicy::Cancel, prefs));
                        return;
                    }
                }
            }
            if !is_main {
                let top = state.committed_url.borrow().clone();
                let allow = url.as_ref().is_some_and(|u| u.scheme() != "tarn" && nav_policy::file_subresource_allowed(u, top.as_ref()));
                let policy = if allow && !wants_download {
                    if let Some(u) = &url {
                        crate::tarn_webkit::gate_frame(web_view, u, false);
                    }
                    WKNavigationActionPolicy::Allow
                } else {
                    WKNavigationActionPolicy::Cancel
                };
                handler.call((policy, prefs));
                return;
            }
            let (Some(url), Some(site)) = (url, site) else {
                handler.call((WKNavigationActionPolicy::Cancel, prefs));
                return;
            };
            if store_link_activated(action, from_shell, link, redirect)
                && let Some(id) = crate::tarn_ext::store_install(&url)
            {
                self.emit(Event::StoreInstall(id));
                handler.call((WKNavigationActionPolicy::Cancel, prefs));
                return;
            }
            if (!from_shell || link) && state.agent_gate.borrow().as_ref().is_some_and(|gate| !gate(&url)) {
                self.emit(Event::AgentHeld(url));
                handler.call((WKNavigationActionPolicy::Cancel, prefs));
                return;
            }
            // SAFETY: plain property reads on the live action.
            let (kind, modifiers) = unsafe { (action.navigationType(), action.modifierFlags()) };
            // WebKit reports a script's `a.click()`, `form.submit()`, `history.back()` or
            // `location.reload()` with the same types as the user's, so only the shell's own loads
            // and history steps count as the user's.
            let history_step = matches!(kind, WKNavigationType::BackForward | WKNavigationType::Reload) && history_step.contains(&url);
            let opened = state.opened_files.borrow().contains(&url);
            let source = if nav_policy::file_open(&url, opened && ((from_shell && !link) || history_step)) { Source::FileOpen } else { Source::Link };
            match nav_policy::decide(&url, source) {
                Decision::Allow => {}
                Decision::Prompt => {
                    let origin = if from_shell || self.ivars().provisional.get().is_some() {
                        None
                    } else {
                        // SAFETY: main-thread read of the live action; the frame comes back retained or nil.
                        unsafe { action.sourceFrame() }.and_then(|f| origin_of(&f))
                    };
                    if from_shell || within_burst(&state.recent_prompts, PROMPT_BURST) {
                        self.emit(Event::ExternalScheme { url, origin });
                    }
                    handler.call((WKNavigationActionPolicy::Cancel, prefs));
                    return;
                }
                Decision::Block => {
                    handler.call((WKNavigationActionPolicy::Cancel, prefs));
                    return;
                }
            }
            // A page without a committed origin (a new popup, about:blank, data:) is judged by the
            // frame that started the navigation, its opener's for a popup; opaque counts as public.
            // SAFETY: main-thread read of the live action; the frame comes back retained or nil.
            let left = state.committed_origin.borrow().clone().or_else(|| state.web_origin.borrow().clone()).or_else(|| unsafe { action.sourceFrame() }.and_then(|f| origin_of(&f)));
            let left = left.and_then(|o| Url::parse(&o).ok());
            let private_network = left.as_ref().map_or(state.level, |p| (state.site_for)(p).level).policy().block_private_network;
            if kind == WKNavigationType::LinkActivated && modifiers.contains(NSEventModifierFlags::Shift) && !wants_download {
                // A peek is a window the page could open with a synthetic Shift-click: it gets
                // the pop-up rules, their rate limit and, as the shell loads it, the S10 check here.
                let opener = state.committed_origin.borrow().as_deref().and_then(|o| Url::parse(o).ok()).map_or(Site::new(state.level), |o| (state.site_for)(&o));
                let local = nav_policy::into_private_network(&url, left.as_ref(), false, private_network);
                if within_burst(&state.recent_popups, POPUP_BURST) && nav_policy::popup_allowed(opener, Some(site)) && !local {
                    self.emit(Event::PeekLink(url, left));
                } else {
                    if local {
                        self.emit(Event::Protected(Protection::LocalNetwork, None));
                    }
                    self.emit(Event::PopupBlocked(Some(url)));
                }
                handler.call((WKNavigationActionPolicy::Cancel, prefs));
                return;
            }
            // A navigation the old document starts while the user's load is pending is the page's own
            // (S5, S10): only hops of the navigation the shell itself started are credited to it.
            let pending_user_load = state.provisional.get().is_some_and(|id| id != 0 && id == navigation_id(state.user_load.borrow().as_deref()));
            let user_started = nav_policy::user_started(from_shell, link, history_step, redirect, pending_user_load);
            if nav_policy::into_private_network(&url, left.as_ref(), user_started, private_network) {
                let message = nav_policy::LOCAL_NETWORK_BLOCKED.to_owned();
                self.emit(Event::Protected(Protection::LocalNetwork, None));
                self.emit(Event::Blocked { url, message, category: None });
                handler.call((WKNavigationActionPolicy::Cancel, prefs));
                return;
            }
            if wants_download {
                // SAFETY: frame info is valid for the duration of this call.
                let from_main = unsafe { action.sourceFrame() }.is_some_and(|f| unsafe { f.isMainFrame() });
                let policy = if from_main { WKNavigationActionPolicy::Download } else { WKNavigationActionPolicy::Cancel };
                handler.call((policy, prefs));
                return;
            }
            match nav_policy::level_action(site, state.level, state.filtered, from_shell, &url) {
                nav_policy::LevelAction::BlockInsecure => {
                    let message = "This site is set to HTTPS only; the insecure link was blocked.".to_owned();
                    self.emit(Event::Blocked { url, message, category: None });
                    handler.call((WKNavigationActionPolicy::Cancel, prefs));
                }
                nav_policy::LevelAction::Rebuild => {
                    self.emit(if user_started { Event::ShieldLevelChanged(url) } else { Event::RebuildForLink(url) });
                    handler.call((WKNavigationActionPolicy::Cancel, prefs));
                }
                nav_policy::LevelAction::Continue { javascript } => {
                    // A refused navigation leaves the pending load, and whose it is, as it was.
                    state.user_navigation.set(user_started);
                    // The view's `make-https` rule rewrites it; a redirect hop or history step is the same page.
                    if !redirect && !history_step && nav_policy::upgrades_to_https(&url, state.level.policy().https_only) {
                        self.emit(Event::Protected(Protection::HttpsUpgrade, url.host_str().map(str::to_owned)));
                    }
                    // SAFETY: the preferences object is owned by WebKit and mutable until we answer.
                    unsafe { preferences.setAllowsContentJavaScript(javascript) };
                    crate::tarn_webkit::gate_frame(web_view, &url, true);
                    handler.call((WKNavigationActionPolicy::Allow, prefs));
                }
            }
        }

        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_response(
            &self,
            _web_view: &WKWebView,
            response: &WKNavigationResponse,
            handler: &DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            // SAFETY: WebKit hands us a live navigation response on the main thread.
            let (main, download) = unsafe {
                (response.isForMainFrame(), !response.canShowMIMEType() || is_attachment(&response.response()))
            };
            let policy = match (download, main && self.ivars().main_initiated.get()) {
                (false, _) => WKNavigationResponsePolicy::Allow,
                (true, true) => WKNavigationResponsePolicy::Download,
                (true, false) => WKNavigationResponsePolicy::Cancel,
            };
            handler.call((policy,));
        }

        #[unsafe(method(webView:navigationAction:didBecomeDownload:))]
        fn action_became_download(&self, _web_view: &WKWebView, _action: &WKNavigationAction, download: &WKDownload) {
            self.adopt_download(download);
        }

        #[unsafe(method(webView:navigationResponse:didBecomeDownload:))]
        fn response_became_download(&self, _web_view: &WKWebView, _response: &WKNavigationResponse, download: &WKDownload) {
            self.adopt_download(download);
        }

        #[unsafe(method(webView:didReceiveAuthenticationChallenge:completionHandler:))]
        fn auth_challenge(
            &self,
            web_view: &WKWebView,
            challenge: &NSURLAuthenticationChallenge,
            handler: &DynBlock<dyn Fn(NSURLSessionAuthChallengeDisposition, *mut NSURLCredential)>,
        ) {
            let Some(space) = protection_space(challenge) else {
                handler.call((NSURLSessionAuthChallengeDisposition::PerformDefaultHandling, std::ptr::null_mut()));
                return;
            };
            let method = space.authenticationMethod();
            // SAFETY: reading Foundation's constant strings.
            let (http_auth, client_certificate) = unsafe {
                (
                    method.isEqualToString(NSURLAuthenticationMethodHTTPBasic) || method.isEqualToString(NSURLAuthenticationMethodHTTPDigest),
                    method.isEqualToString(NSURLAuthenticationMethodClientCertificate),
                )
            };
            // Server trust keeps WebKit's default handling (S7).
            if !http_auth && !client_certificate {
                handler.call((NSURLSessionAuthChallengeDisposition::PerformDefaultHandling, std::ptr::null_mut()));
                return;
            }
            let scheme = space.protocol().map(|p| p.to_string()).unwrap_or_default();
            let host = space.host().to_string();
            let host = if host.contains(':') && !host.starts_with('[') { format!("[{host}]") } else { host };
            let challenger = Url::parse(&format!("{scheme}://{host}:{}", space.port())).ok();
            // SAFETY: main-thread read of a live web view.
            let page = to_url(unsafe { web_view.URL() });
            // Only the page's own origin may ask for credentials, not embedded third parties or
            // other ports of the same host. WebKit moves the URL to a navigation's target before
            // its challenge arrives.
            let origin = challenger.as_ref().and_then(serialized_origin);
            // A refused certificate request goes on without one, which servers that only ask for
            // a certificate optionally accept.
            let refuse = if client_certificate {
                NSURLSessionAuthChallengeDisposition::PerformDefaultHandling
            } else {
                NSURLSessionAuthChallengeDisposition::CancelAuthenticationChallenge
            };
            let Some(origin) = origin.filter(|o| page.as_ref().and_then(serialized_origin).as_ref() == Some(o)) else {
                handler.call((refuse, std::ptr::null_mut()));
                return;
            };
            if previous_failures(challenge) >= MAX_AUTH_ATTEMPTS {
                handler.call((refuse, std::ptr::null_mut()));
                return;
            }
            if client_certificate && prompts::Certificates::join(&self.ivars().certificates, &origin, &handler.copy()) {
                return;
            }
            if self.ivars().silenced.get()
                || !within_burst(&self.ivars().recent_prompts, PROMPT_BURST)
            {
                handler.call((refuse, std::ptr::null_mut()));
                return;
            }
            let (kind, reply) = if client_certificate {
                let (names, identities) = keychain_identities(space.distinguishedNames());
                if identities.is_empty() {
                    handler.call((refuse, std::ptr::null_mut()));
                    return;
                }
                let state = self.ivars().certificates.clone();
                let page = {
                    let mut certificates = state.borrow_mut();
                    certificates.waiting.insert(origin.clone(), Vec::new());
                    certificates.page
                };
                let ask = prompts::CertificateAsk { origin: origin.clone(), page, identities, state };
                (PromptKind::ClientCertificate { names }, Reply::Certificate(handler.copy(), ask))
            } else {
                let realm = space.realm().map(|r| r.to_string()).unwrap_or_default();
                (PromptKind::Credentials { realm }, Reply::Credentials(handler.copy()))
            };
            let prompt = Prompt {
                origin,
                top_origin: None,
                insecure: !space.receivesCredentialSecurely(),
                message: String::new(),
                kind,
                reply: Some(reply),
                suppress: self.ivars().silenced.clone(),
            };
            self.emit(Event::Prompt(prompt));
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start_provisional(&self, _web_view: &WKWebView, navigation: Option<&WKNavigation>) {
            let id = navigation_id(navigation);
            self.ivars().provisional.set(Some(id));
            self.ivars().certificates.borrow_mut().next_page();
            if navigation_id(self.ivars().user_load.borrow().as_deref()) != id {
                self.ivars().user_load.take();
            }
            self.emit(Event::Loading(true));
        }

        #[unsafe(method(webView:didCommitNavigation:))]
        fn did_commit(&self, web_view: &WKWebView, navigation: Option<&WKNavigation>) {
            self.settle(navigation);
            self.ivars().silenced.set(false);
            self.ivars().user_navigation.set(false);
            self.ivars().history_step.take();
            self.run_after_commit();
            crate::tarn_webkit::committed(web_view);
            // SAFETY: main-thread read of a live web view.
            let committed = to_url(unsafe { web_view.URL() });
            let origin = committed.as_ref().and_then(serialized_origin);
            if origin.is_some() {
                self.ivars().web_origin.borrow_mut().clone_from(&origin);
            }
            *self.ivars().committed_origin.borrow_mut() = origin;
            *self.ivars().committed_url.borrow_mut() = committed;
            if let Some((url, title, back, forward)) = page_state(web_view) {
                self.emit(Event::Committed { url, title, can_go_back: back, can_go_forward: forward });
            }
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, web_view: &WKWebView, _navigation: Option<&WKNavigation>) {
            if let Some((url, title, back, forward)) = page_state(web_view) {
                self.emit(Event::Finished { url, title, can_go_back: back, can_go_forward: forward });
            }
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(&self, web_view: &WKWebView, navigation: Option<&WKNavigation>, error: &NSError) {
            self.settle(navigation);
            self.run_after_commit();
            self.failed(web_view, error);
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, web_view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            self.failed(web_view, error);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn process_terminated(&self, _web_view: &WKWebView) {
            self.emit(Event::ContentProcessTerminated);
        }
    }

    unsafe impl WKUIDelegate for Delegate {
        #[unsafe(method(webView:runJavaScriptAlertPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn js_alert(&self, _web_view: &WKWebView, message: &NSString, frame: &WKFrameInfo, handler: &DynBlock<dyn Fn()>) {
            self.page_prompt(frame, message, PromptKind::Alert, Reply::Alert(handler.copy()));
        }

        #[unsafe(method(webView:runJavaScriptConfirmPanelWithMessage:initiatedByFrame:completionHandler:))]
        fn js_confirm(&self, _web_view: &WKWebView, message: &NSString, frame: &WKFrameInfo, handler: &DynBlock<dyn Fn(Bool)>) {
            self.page_prompt(frame, message, PromptKind::Confirm, Reply::Confirm(handler.copy()));
        }

        #[unsafe(method(webView:runJavaScriptTextInputPanelWithPrompt:defaultText:initiatedByFrame:completionHandler:))]
        fn js_prompt(
            &self,
            _web_view: &WKWebView,
            message: &NSString,
            default: Option<&NSString>,
            frame: &WKFrameInfo,
            handler: &DynBlock<dyn Fn(*mut NSString)>,
        ) {
            let default = default.map(|d| d.to_string()).unwrap_or_default();
            self.page_prompt(frame, message, PromptKind::Text { default }, Reply::Text(handler.copy()));
        }

        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn media_permission(
            &self,
            _web_view: &WKWebView,
            _origin: &WKSecurityOrigin,
            frame: &WKFrameInfo,
            kind: WKMediaCaptureType,
            handler: &DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            let kind = match kind {
                WKMediaCaptureType::Camera => PromptKind::Camera,
                WKMediaCaptureType::Microphone => PromptKind::Microphone,
                _ => PromptKind::CameraAndMicrophone,
            };
            self.page_prompt(frame, &NSString::new(), kind, Reply::Permission(handler.copy()));
        }

        #[unsafe(method(webView:runOpenPanelWithParameters:initiatedByFrame:completionHandler:))]
        fn open_panel(&self, web_view: &WKWebView, parameters: &WKOpenPanelParameters, _frame: &WKFrameInfo, handler: &DynBlock<dyn Fn(*mut NSArray<NSURL>)>) {
            let panel = NSOpenPanel::openPanel(self.mtm());
            // SAFETY: main-thread reads of WebKit's live parameters.
            let (multiple, directories) = unsafe { (parameters.allowsMultipleSelection(), parameters.allowsDirectories()) };
            panel.setAllowsMultipleSelection(multiple);
            panel.setCanChooseDirectories(directories);
            panel.setCanChooseFiles(true);
            let handler = handler.copy();
            let chosen = panel.clone();
            let done = RcBlock::new(move |response: NSModalResponse| {
                let urls = (response == NSModalResponseOK).then(|| chosen.URLs());
                handler.call((urls.as_ref().map_or(std::ptr::null_mut(), |u| Retained::as_ptr(u).cast_mut()),));
            });
            match web_view.window() {
                Some(window) => panel.beginSheetModalForWindow_completionHandler(&window, &done),
                None => done.call((panel.runModal(),)),
            }
        }

        #[unsafe(method(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            web_view: &WKWebView,
            configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> *mut WKWebView {
            let split = self.ivars().split_link.take().is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(2));
            // SAFETY: the action's request is a valid NSURLRequest.
            let url = to_url(unsafe { action.request() }.URL());
            let decision = url.as_ref().map_or(Decision::Allow, |u| nav_policy::decide(u, Source::Popup));
            match decision {
                Decision::Allow => {}
                Decision::Prompt => {
                    if let Some(url) = url {
                        // SAFETY: main-thread read of the live action; the frame comes back retained or nil.
                        let origin = unsafe { action.sourceFrame() }.and_then(|f| origin_of(&f));
                        self.emit(Event::ExternalScheme { url, origin });
                    }
                    return std::ptr::null_mut();
                }
                Decision::Block => return std::ptr::null_mut(),
            }
            let state = self.ivars();
            if !within_burst(&state.recent_popups, POPUP_BURST) {
                return std::ptr::null_mut();
            }
            // The page on screen opens the window; `URL` already names a pending navigation's target (S5).
            let shown = state.committed_origin.borrow().clone().or_else(|| state.web_origin.borrow().clone()).and_then(|o| Url::parse(&o).ok());
            let mut opener = shown.as_ref().map_or(Site::new(state.level), |u| (state.site_for)(u));
            // A site's "Allow" covers its own pages, not the ad frames it embeds.
            // SAFETY: frame info is valid for the duration of this call.
            if !unsafe { action.sourceFrame() }.is_some_and(|f| unsafe { f.isMainFrame() }) && opener.popups == nav_policy::PopupRule::Allow {
                opener.popups = nav_policy::PopupRule::Default;
            }
            if !nav_policy::popup_allowed(opener, url.as_ref().map(|u| (state.site_for)(u))) {
                self.emit(Event::PopupBlocked(url));
                return std::ptr::null_mut();
            }
            // S10, as for top-level navigations: no window from a public page onto a private address.
            let opener_public = shown.as_ref().is_some_and(|o| !nav_policy::local_page(o));
            if opener_public && url.as_ref().is_some_and(shield::rules::local_target) {
                self.emit(Event::Protected(Protection::LocalNetwork, None));
                self.emit(Event::PopupBlocked(url));
                return std::ptr::null_mut();
            }
            if split {
                if let Some(url) = url {
                    self.emit(Event::OpenTab { url, from: shown, background: false, split: true });
                }
                return std::ptr::null_mut();
            }
            // The popup must use WebKit's configuration so it keeps its opener context
            // (referrer, Sec-Fetch-Site, SameSite cookies) instead of becoming a typed load.
            let mtm = self.mtm();
            let popup_state = DelegateState {
                recent_prompts: state.recent_prompts.clone(),
                recent_popups: state.recent_popups.clone(),
                // A blank popup counts as its opener until it commits a page with an origin.
                web_origin: RefCell::new(state.web_origin.borrow().clone()),
                // Until its first commit a popup shows the opener-writable blank document, whatever it was asked to load (S5).
                committed_url: RefCell::new(Url::parse("about:blank").ok()),
                agent_gate: RefCell::new(state.agent_gate.borrow().clone()),
                ..DelegateState::new(Rc::new(|_| {}), state.site_for.clone(), state.level, state.filtered, state.downloads.clone(), true)
            };
            // SAFETY: main thread; WebKit requires the returned view to use this configuration.
            let view = unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::ZERO, configuration) };
            let delegate = Delegate::new(mtm, popup_state);
            delegate.attach(&view);
            let ptr = Retained::autorelease_return(view.clone());
            // SAFETY: main-thread read of the live action.
            let link = unsafe { action.navigationType() } == WKNavigationType::LinkActivated;
            self.emit(Event::Popup(Popup { url, link, parts: Some((view, delegate)) }));
            ptr
        }
    }
);

impl Delegate {
    pub(crate) fn new(mtm: MainThreadMarker, state: DelegateState) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(state);
        // SAFETY: NSObject's designated initialiser on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }

    pub(crate) fn attach(&self, view: &WKWebView) {
        // SAFETY: main-thread setup; the WebView or Popup that owns `view` also owns this
        // delegate, and clears both references before dropping it.
        unsafe {
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(self)));
            view.setUIDelegate(Some(ProtocolObject::from_ref(self)));
            view.setAllowsBackForwardNavigationGestures(true);
            view.setHidden(true);
        }
    }

    pub(crate) fn filtered(&self) -> bool {
        self.ivars().filtered
    }

    pub(crate) fn level(&self) -> ShieldLevel {
        self.ivars().level
    }

    pub(crate) fn after_commit(&self, then: impl FnOnce() + 'static) {
        *self.ivars().after_commit.borrow_mut() = Some(Box::new(then));
    }

    /// Ends the pending main-frame load if `navigation` is it; a superseded load's late callback, or
    /// a nil navigation that cannot be told apart, leaves it pending.
    fn settle(&self, navigation: Option<&WKNavigation>) {
        let id = navigation_id(navigation);
        if id != 0 && self.ivars().provisional.get() == Some(id) {
            self.ivars().provisional.set(None);
        }
        if navigation_id(self.ivars().user_load.borrow().as_deref()) == id {
            self.ivars().user_load.take();
        }
    }

    pub(crate) fn run_after_commit(&self) {
        if let Some(then) = self.ivars().after_commit.take() {
            then();
        }
    }

    /// The URL to name for the page on screen. While a navigation the page started is pending,
    /// `URL` already names its target over the old content (S5), so the committed URL stands in.
    pub(crate) fn shown_url(&self, live: Option<Url>) -> Option<Url> {
        let state = self.ivars();
        shown_url(state.provisional.get().is_some() && !state.user_navigation.get(), &state.committed_url.borrow(), live)
    }

    pub(crate) fn mark_shell_load(&self, url: &Url) {
        *self.ivars().shell_load.borrow_mut() = Some((url.clone(), false));
    }

    /// A shell load of a link the page chose; while the view has no page, `opener` stands in for it.
    pub(crate) fn mark_link_load(&self, url: &Url, opener: Option<&Url>) {
        *self.ivars().shell_load.borrow_mut() = Some((url.clone(), true));
        if let Some(origin) = opener.and_then(serialized_origin) {
            *self.ivars().web_origin.borrow_mut() = Some(origin);
        }
    }

    pub(crate) fn set_user_load(&self, navigation: Option<Retained<WKNavigation>>) {
        *self.ivars().user_load.borrow_mut() = navigation;
    }

    pub(crate) fn refuse(&self, url: Option<Url>, message: String) {
        self.emit(Event::Failed { url, code: -1, message });
    }

    pub(crate) fn set_history_step(&self, urls: Vec<Url>) {
        *self.ivars().history_step.borrow_mut() = urls;
    }

    pub(crate) fn set_assistant_menu(&self, commands: Option<Vec<String>>) {
        *self.ivars().assistant_menu.borrow_mut() = commands;
    }

    /// Ask Assistant about Selection and the Ask Assistant to submenu, after a separator; each
    /// sends `askAssistant:` with its command's index as tag, or -1 for the question itself; the
    /// command is read from the item's title, as the list may change while the menu shows.
    fn add_assistant_items(&self, menu: &NSMenu, commands: &[String]) {
        let mtm = self.mtm();
        let item = |title: &str, tag: isize| {
            // SAFETY: `askAssistant:` is implemented above with the `(id)sender` signature AppKit
            // sends; the menu holds its target weakly, and WebKit keeps this delegate while it shows a menu.
            let item = unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(sel!(askAssistant:)),
                    &NSString::new(),
                );
                item.setTarget(Some(self));
                item
            };
            item.setTag(tag);
            item
        };
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        menu.addItem(&item(crate::view_policy::ASK_SELECTION, -1));
        if commands.is_empty() {
            return;
        }
        let title = NSString::from_str(crate::view_policy::ASSISTANT_COMMANDS);
        let sub = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
        for (ix, name) in commands.iter().enumerate() {
            sub.addItem(&item(name, ix as isize));
        }
        // SAFETY: a parent entry with no action; its submenu holds the commands.
        let parent = unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), &title, None, &NSString::new()) };
        parent.setSubmenu(Some(&sub));
        menu.addItem(&parent);
    }

    pub(crate) fn set_sink(&self, sink: Rc<dyn Fn(Event)>) {
        *self.ivars().sink.borrow_mut() = sink.clone();
        for event in self.ivars().pending.take().unwrap_or_default() {
            sink(event);
        }
    }

    pub(crate) fn emit(&self, event: Event) {
        if let Some(pending) = self.ivars().pending.borrow_mut().as_mut() {
            pending.push(event);
            return;
        }
        let sink = self.ivars().sink.borrow().clone();
        sink(event);
    }

    /// An extension's request to access `host`, shown in this view under the same limits as the
    /// page's own prompts.
    pub(crate) fn extension_prompt(&self, extension: String, key: String, host: String, reply: Box<dyn FnOnce(Option<bool>, bool)>) {
        let reply = Reply::Access(reply);
        if self.ivars().silenced.get() || !within_burst(&self.ivars().recent_prompts, PROMPT_BURST) {
            prompts::decline(Some(reply));
            return;
        }
        let kind = PromptKind::ExtensionAccess { extension, key };
        let suppress = self.ivars().silenced.clone();
        self.emit(Event::Prompt(Prompt { origin: host, top_origin: None, insecure: false, message: String::new(), kind, reply: Some(reply), suppress }));
    }

    fn page_prompt(&self, frame: &WKFrameInfo, message: &NSString, kind: PromptKind, reply: Reply) {
        let page = self.ivars().committed_origin.borrow().clone();
        let (Some(origin), Some(page)) = (origin_of(frame), page) else {
            prompts::decline(Some(reply));
            return;
        };
        if self.ivars().silenced.get() || !within_burst(&self.ivars().recent_prompts, PROMPT_BURST) {
            prompts::decline(Some(reply));
            return;
        }
        let top_origin = (page != origin).then_some(page);
        let suppress = self.ivars().silenced.clone();
        let message = message.to_string();
        self.emit(Event::Prompt(Prompt { origin, top_origin, insecure: false, message, kind, reply: Some(reply), suppress }));
    }

    /// Saves the PDF the main frame shows, under the same per-tab limit as a downloaded file and the
    /// malware-list prompt for its host.
    fn save_shown_pdf(&self, data: &NSData, name: &str, origin: Option<Url>) {
        let state = self.ivars();
        let without_fragment = |u: &Url| {
            let mut u = u.clone();
            u.set_fragment(None);
            u
        };
        let page = state.committed_url.borrow().clone();
        // Downloads come from the main frame only (S11); a PDF in a frame is not the page.
        if origin.is_none() || origin.as_ref().map(without_fragment) != page.as_ref().map(without_fragment) {
            self.emit(Event::DownloadFailed { id: None, message: "Only a PDF shown as the page itself can be saved.".into() });
            return;
        }
        if !within_burst(&state.recent_downloads, DOWNLOAD_BURST) {
            self.emit(Event::DownloadFailed { id: None, message: "Too many downloads from this page; blocked.".into() });
            return;
        }
        let bytes = data.to_vec();
        let page = page.filter(|u| matches!(u.scheme(), "http" | "https" | "file"));
        // A blob: or data: page is judged by the last real origin it came from.
        let judged = page.clone().or_else(|| state.web_origin.borrow().as_deref().and_then(|o| Url::parse(o).ok()));
        let downloads = state.downloads.clone();
        let Some(from) = downloads::flagged_source([judged], &*state.site_for) else {
            downloads.save_bytes(name, &bytes, page);
            return;
        };
        let pending = RefCell::new(Some((downloads, name.to_owned(), bytes, page)));
        self.ask_to_keep(
            &from,
            name,
            RcBlock::new(move |keep: Bool| {
                if let Some((downloads, name, bytes, page)) = pending.take().filter(|_| keep.as_bool()) {
                    downloads.save_bytes(&name, &bytes, page);
                }
            }),
        );
    }

    fn adopt_download(&self, download: &WKDownload) {
        let state = self.ivars();
        if !within_burst(&state.recent_downloads, DOWNLOAD_BURST) {
            // SAFETY: cancelling a live download; no completion handler needed.
            unsafe { download.cancel(None) };
            self.emit(Event::DownloadFailed { id: None, message: "Too many downloads from this page; blocked.".into() });
            return;
        }
        // SAFETY: WebKit holds the download delegate weakly; it is owned by the profile's
        // DataStore, which lives as long as the app, so it outlives every download.
        unsafe { download.setDelegate(Some(ProtocolObject::from_ref(&*state.downloads))) };
        let source = DownloadSource { site_for: state.site_for.clone(), page: state.committed_url.borrow().clone(), tab: Weak::new(self) };
        state.downloads.ivars().sources.borrow_mut().insert(download_key(download), source);
    }

    /// Asks whether to keep a download from a host on the malware list; it is cancelled unless kept.
    fn ask_to_keep(&self, from: &Url, file: &str, reply: RcBlock<dyn Fn(Bool)>) {
        let reply = Reply::Confirm(reply);
        let Some(origin) = serialized_origin(from).filter(|_| !self.ivars().silenced.get()) else {
            prompts::decline(Some(reply));
            return;
        };
        let message = downloads::malware_warning(from);
        let kind = PromptKind::MaliciousDownload { file: file.to_owned() };
        let suppress = self.ivars().silenced.clone();
        self.emit(Event::Prompt(Prompt { origin, top_origin: None, insecure: false, message, kind, reply: Some(reply), suppress }));
    }

    fn failed(&self, web_view: &WKWebView, error: &NSError) {
        self.ivars().user_navigation.set(false);
        self.ivars().history_step.take();
        // SAFETY: main-thread access to a live web view.
        let url = to_url(unsafe { web_view.URL() });
        let message = error.localizedDescription().to_string();
        self.emit(Event::Failed { url, code: error.code(), message });
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnDownloadDelegate"]
    #[ivars = DownloadState]
    pub(crate) struct DownloadDelegate;

    unsafe impl NSObjectProtocol for DownloadDelegate {}

    unsafe impl WKDownloadDelegate for DownloadDelegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        fn decide_destination(&self, download: &WKDownload, response: &NSURLResponse, suggested: &NSString, handler: &DynBlock<dyn Fn(*mut NSURL)>) {
            let resumed = self.ivars().active.borrow().iter().find(|a| a.is(download)).map(|a| (a.id, a.source.clone(), a.asked));
            let id = resumed.as_ref().map(|r| r.0);
            // SAFETY: reading the original request of a live download.
            let url = unsafe { download.originalRequest() }.and_then(|r| to_url(r.URL()));
            let name = downloads::sanitize_filename(&suggested.to_string());
            let (source, checked) = match resumed {
                Some((_, source, asked)) => (source, if asked { Vec::new() } else { vec![to_url(response.URL())] }),
                None => {
                    let source = self.ivars().sources.borrow_mut().remove(&download_key(download));
                    let page = source.as_ref().and_then(|s| s.page.clone());
                    (source, vec![url.clone(), to_url(response.URL()), page])
                }
            };
            let flagged = source.as_ref().and_then(|s| downloads::flagged_source(checked, &*s.site_for));
            let Some(from) = flagged else {
                self.keep(download, url, &name, source, handler);
                return;
            };
            let tab = source.as_ref().and_then(|s| s.tab.load());
            let pending = RefCell::new(Some((self.retain(), download.retain(), url, name.clone(), source, handler.copy())));
            let reply = RcBlock::new(move |keep: Bool| {
                let Some((this, download, url, name, source, handler)) = pending.take() else { return };
                if keep.as_bool() {
                    this.keep(&download, url, &name, source, &handler);
                    this.mark_asked(&download);
                } else {
                    if let Some(id) = id {
                        this.decline(id);
                    }
                    handler.call((std::ptr::null_mut(),));
                }
            });
            match tab {
                Some(tab) => tab.ask_to_keep(&from, &name, reply),
                None => prompts::decline(Some(Reply::Confirm(reply))),
            }
        }

        #[unsafe(method(downloadDidFinish:))]
        fn download_finished(&self, download: &WKDownload) {
            let Some(active) = self.take(download) else { return };
            RESERVED.with_borrow_mut(|r| r.remove(&active.path));
            let quarantined = quarantine(&active.path, active.url.as_ref());
            let ActiveDownload { id, url, path, started, .. } = active;
            (self.ivars().sink)(Event::DownloadFinished { id, url, path, started, quarantined });
            self.report_active();
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        fn download_failed(&self, download: &WKDownload, error: &NSError, _resume: Option<&NSData>) {
            let undecided = self.ivars().sources.borrow_mut().remove(&download_key(download)).is_some();
            let pausing = self.ivars().active.borrow().iter().any(|a| a.is(download) && !a.transfer.running());
            if pausing {
                return;
            }
            let Some(active) = self.take(download) else {
                // Without a source or an entry it failed after Tarn gave it no destination, which
                // a decline or `save` already answered.
                if undecided {
                    (self.ivars().sink)(Event::DownloadFailed { id: None, message: error.localizedDescription().to_string() });
                }
                return;
            };
            if self.ivars().declined.borrow_mut().remove(&active.id) {
                self.discard(&active);
                (self.ivars().sink)(Event::DownloadCancelled(active.id));
            } else {
                RESERVED.with_borrow_mut(|r| r.remove(&active.path));
                let message = error.localizedDescription().to_string();
                (self.ivars().sink)(Event::DownloadFailed { id: Some(active.id), message });
            }
            self.report_active();
        }
    }
);

impl DownloadDelegate {
    /// Continues into the file a resumed download had, or saves a new one.
    fn keep(&self, download: &WKDownload, url: Option<Url>, name: &str, source: Option<DownloadSource>, handler: &DynBlock<dyn Fn(*mut NSURL)>) {
        let resumed = self.ivars().active.borrow().iter().find(|a| a.is(download)).map(|a| a.path.clone());
        match resumed {
            Some(path) => {
                let ns_url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
                handler.call((Retained::as_ptr(&ns_url).cast_mut(),));
            }
            None => self.save(download, url, name, source, handler),
        }
    }

    fn save(&self, download: &WKDownload, url: Option<Url>, name: &str, source: Option<DownloadSource>, handler: &DynBlock<dyn Fn(*mut NSURL)>) {
        let Some(path) = self.reserve(name) else {
            handler.call((std::ptr::null_mut(),));
            return;
        };
        let ns_url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        let id = self.ivars().next_id.get();
        self.ivars().next_id.set(id + 1);
        let active = ActiveDownload {
            id,
            url,
            path,
            started: SystemTime::now(),
            transfer: Transfer::Saving(download.retain()),
            done: 0,
            total: None,
            view: None,
            source,
            asked: false,
        };
        let event = active.progress();
        self.ivars().active.borrow_mut().push(active);
        handler.call((Retained::as_ptr(&ns_url).cast_mut(),));
        (self.ivars().sink)(event);
        self.report_active();
    }

    /// A free path in Downloads (or its subfolder for the type) held until the caller releases it.
    fn reserve(&self, name: &str) -> Option<PathBuf> {
        let Some(home) = std::env::home_dir() else {
            (self.ivars().sink)(Event::DownloadFailed { id: None, message: "Tarn could not find the Downloads folder.".into() });
            return None;
        };
        let downloads_dir = home.join("Downloads");
        let sorted = self.ivars().sort.get().then(|| downloads::folder_for(name)).flatten().and_then(|f| downloads::sorted_dir(&downloads_dir, f));
        let dir = sorted.unwrap_or(downloads_dir);
        Some(RESERVED.with_borrow_mut(|reserved| {
            let path = downloads::free_path(&dir, name, |p| reserved.contains(p));
            reserved.insert(path.clone());
            path
        }))
    }

    /// Writes a file the page already holds, quarantined as a download from `from`.
    fn save_bytes(&self, name: &str, bytes: &[u8], from: Option<Url>) {
        let Some(path) = self.reserve(name) else { return };
        let started = SystemTime::now();
        let written = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).and_then(|mut f| std::io::Write::write_all(&mut f, bytes));
        RESERVED.with_borrow_mut(|r| r.remove(&path));
        let event = match written {
            Ok(()) => {
                let id = self.ivars().next_id.get();
                self.ivars().next_id.set(id + 1);
                let quarantined = quarantine(&path, from.as_ref());
                Event::DownloadFinished { id, url: from, path, started, quarantined }
            }
            Err(e) => {
                if e.kind() != std::io::ErrorKind::AlreadyExists {
                    std::fs::remove_file(&path).ok();
                }
                Event::DownloadFailed { id: None, message: format!("Tarn could not save {name}: {e}") }
            }
        };
        (self.ivars().sink)(event);
    }

    fn take(&self, download: &WKDownload) -> Option<ActiveDownload> {
        let mut active = self.ivars().active.borrow_mut();
        let index = active.iter().position(|a| a.is(download))?;
        Some(active.remove(index))
    }

    /// Cancels a saving download and keeps what WebKit needs to continue it, in memory only.
    pub(crate) fn pause(&self, id: u64) {
        let download = {
            let mut active = self.ivars().active.borrow_mut();
            let Some(a) = active.iter_mut().find(|a| a.id == id) else { return };
            if !a.transfer.pause() {
                return;
            }
            a.transfer.handle().cloned()
        };
        let Some(download) = download else { return };
        let this = self.retain();
        let done = RcBlock::new(move |data: *mut NSData| {
            // SAFETY: WebKit passes resume data or nil, valid for this call; it is retained here.
            this.paused(id, unsafe { Retained::retain(data) });
        });
        // SAFETY: cancelling a live download; WebKit copies the block.
        unsafe { download.cancel(Some(&done)) };
        self.report_progress(id);
        self.report_active();
    }

    fn paused(&self, id: u64, data: Option<Retained<NSData>>) {
        let mut active = self.ivars().active.borrow_mut();
        let Some(index) = active.iter().position(|a| a.id == id && matches!(a.transfer, Transfer::Pausing(_))) else { return };
        if let Some(data) = data {
            let a = &mut active[index];
            a.transfer.paused(data);
            quarantine(&a.path, a.url.as_ref());
            return;
        }
        let lost = active.remove(index);
        drop(active);
        self.discard(&lost);
        (self.ivars().sink)(Event::DownloadFailed { id: Some(id), message: "The server does not let this download pause, so it stopped.".into() });
    }

    /// Continues a paused download in a view of `store`. One never kept at the malware prompt is
    /// checked again first and stays paused until the user keeps it.
    pub(crate) fn resume(&self, id: u64, store: &WKWebsiteDataStore) {
        let flagged = self.ivars().active.borrow().iter().find(|a| a.id == id && !a.asked && matches!(a.transfer, Transfer::Paused(..))).and_then(|a| {
            let s = a.source.as_ref()?;
            let from = downloads::flagged_source([a.url.clone(), s.page.clone()], &*s.site_for)?;
            Some((from, s.tab.load(), a.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
        });
        let Some((from, tab, name)) = flagged else { return self.start_resume(id, store) };
        let pending = RefCell::new(Some((self.retain(), store.retain())));
        let reply = RcBlock::new(move |keep: Bool| {
            let Some((this, store)) = pending.take() else { return };
            if keep.as_bool() {
                if let Some(a) = this.ivars().active.borrow_mut().iter_mut().find(|a| a.id == id) {
                    a.asked = true;
                }
                this.start_resume(id, &store);
            } else {
                this.decline(id);
            }
        });
        match tab {
            Some(tab) => tab.ask_to_keep(&from, &name, reply),
            None => prompts::decline(Some(Reply::Confirm(reply))),
        }
    }

    fn start_resume(&self, id: u64, store: &WKWebsiteDataStore) {
        let Some(data) = self.ivars().active.borrow_mut().iter_mut().find(|a| a.id == id).and_then(|a| a.transfer.resume()) else { return };
        let mtm = self.mtm();
        // SAFETY: main-thread creation of a configuration and a view that only resumes the download.
        let view = unsafe {
            let config = WKWebViewConfiguration::new(mtm);
            config.setWebsiteDataStore(store);
            WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::ZERO, &config)
        };
        let (this, kept) = (self.retain(), view.clone());
        let done = RcBlock::new(move |download: NonNull<WKDownload>| {
            // SAFETY: WebKit passes the live download that continues the paused one.
            this.resumed(id, unsafe { download.as_ref() }, kept.clone());
        });
        // SAFETY: main thread; the data came from WebKit's own cancellation and WebKit copies the block.
        unsafe { view.resumeDownloadFromResumeData_completionHandler(&data, &done) };
        self.report_progress(id);
        self.report_active();
    }

    fn resumed(&self, id: u64, download: &WKDownload, view: Retained<WKWebView>) {
        let mut active = self.ivars().active.borrow_mut();
        let Some(a) = active.iter_mut().find(|a| a.id == id && matches!(a.transfer, Transfer::Resuming)) else {
            // SAFETY: cancelling a live download nothing tracks; no completion handler needed.
            unsafe { download.cancel(None) };
            return;
        };
        // SAFETY: as in `adopt_download`; the delegate outlives every download of its store.
        unsafe { download.setDelegate(Some(ProtocolObject::from_ref(self))) };
        a.transfer = Transfer::Saving(download.retain());
        a.view = Some(view);
    }

    /// The user's cancel: a saving download reports its failure, which removes it as declined.
    pub(crate) fn cancel(&self, id: u64) {
        let saving = self.ivars().active.borrow().iter().find(|a| a.id == id && a.transfer.running()).and_then(|a| a.transfer.handle().cloned());
        self.decline(id);
        if let Some(download) = saving {
            // SAFETY: cancelling a live download; its failure report removes it.
            unsafe { download.cancel(None) };
        }
    }

    fn mark_asked(&self, download: &WKDownload) {
        if let Some(a) = self.ivars().active.borrow_mut().iter_mut().find(|a| a.is(download)) {
            a.asked = true;
        }
    }

    /// Declines download `id` at the malware prompt in whatever state it is now: a saving one is
    /// noted for its failure report, which the caller brings about; any other is removed now.
    fn decline(&self, id: u64) {
        let mut active = self.ivars().active.borrow_mut();
        let Some(index) = active.iter().position(|a| a.id == id) else { return };
        if active[index].transfer.running() {
            self.ivars().declined.borrow_mut().insert(id);
            return;
        }
        let gone = active.remove(index);
        drop(active);
        self.discard(&gone);
        (self.ivars().sink)(Event::DownloadCancelled(id));
        self.report_active();
    }

    /// Removes the part a download that will not continue left behind.
    fn discard(&self, active: &ActiveDownload) {
        RESERVED.with_borrow_mut(|r| r.remove(&active.path));
        std::fs::remove_file(&active.path).ok();
    }

    /// For quit: quarantines the part of every download still saving and removes every other
    /// with its part, as nothing can continue it afterwards.
    pub(crate) fn quit(&self) {
        let stopped: Vec<ActiveDownload> = self.ivars().active.borrow_mut().extract_if(.., |a| !a.transfer.running()).collect();
        for a in &stopped {
            self.discard(a);
        }
        for a in self.ivars().active.borrow().iter() {
            quarantine(&a.path, a.url.as_ref());
        }
    }

    fn report_progress(&self, id: u64) {
        let event = self.ivars().active.borrow().iter().find(|a| a.id == id).map(ActiveDownload::progress);
        if let Some(event) = event {
            (self.ivars().sink)(event);
        }
    }

    fn tick(&self) {
        let events: Vec<Event> = self
            .ivars()
            .active
            .borrow_mut()
            .iter_mut()
            .filter_map(|a| {
                let Transfer::Saving(download) = &a.transfer else { return None };
                let progress = download.progress();
                let done = u64::try_from(progress.completedUnitCount()).unwrap_or(0);
                let total = u64::try_from(progress.totalUnitCount()).ok().filter(|t| *t > 0);
                if (done, total) == (a.done, a.total) {
                    return None;
                }
                (a.done, a.total) = (done, total);
                Some(a.progress())
            })
            .collect();
        for event in events {
            (self.ivars().sink)(event);
        }
    }

    /// Reports the number saving or paused, which keeps the data store alive, and polls progress
    /// while any is saving.
    fn report_active(&self) {
        let (count, saving) = {
            let active = self.ivars().active.borrow();
            (active.len(), active.iter().any(|a| a.transfer.is_saving()))
        };
        (self.ivars().sink)(Event::DownloadsActive(count));
        let mut timer = self.ivars().timer.borrow_mut();
        if !saving {
            if let Some(timer) = timer.take() {
                timer.invalidate();
            }
        } else if timer.is_none() {
            let this = Weak::new(self);
            let tick = RcBlock::new(move |_: NonNull<NSTimer>| {
                if let Some(this) = this.load() {
                    this.tick();
                }
            });
            // SAFETY: main thread; the run loop retains the timer, which copies the block.
            *timer = Some(unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(PROGRESS_INTERVAL, true, &tick) });
        }
    }

    pub(crate) fn new(mtm: MainThreadMarker, sink: Rc<dyn Fn(Event)>) -> Retained<Self> {
        let state = DownloadState {
            sink,
            active: RefCell::default(),
            sources: RefCell::default(),
            declined: RefCell::default(),
            next_id: Cell::new(1),
            sort: Cell::new(false),
            timer: RefCell::default(),
        };
        let this = Self::alloc(mtm).set_ivars(state);
        // SAFETY: NSObject's designated initialiser on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}

/// Marks a downloaded file so Gatekeeper checks it before it is opened (S11).
/// The record names where the file came from, without credentials, and when.
fn quarantine(path: &Path, from: Option<&Url>) -> bool {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let source = from.cloned().and_then(|mut u| {
        u.set_username("").ok()?;
        u.set_password(None).ok()?;
        NSURL::URLWithString(&NSString::from_str(u.as_str()))
    });
    let (agent, kind, now) = (NSString::from_str("Tarn"), NSString::from_str("LSQuarantineTypeWebDownload"), objc2_foundation::NSDate::now());
    let mut keys = vec![NSString::from_str("LSQuarantineAgentName"), NSString::from_str("LSQuarantineType"), NSString::from_str("LSQuarantineTimeStamp")];
    let mut values: Vec<&AnyObject> = vec![&agent, &kind, &now];
    if let Some(source) = &source {
        keys.push(NSString::from_str("LSQuarantineDataURL"));
        values.push(source);
    }
    let keys: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let props = NSDictionary::from_slices(&keys, &values);
    // SAFETY: the key is a valid Foundation constant and the value a property-list dictionary
    // of strings, which is what the quarantine property expects.
    unsafe { url.setResourceValue_forKey_error(Some(&props), NSURLQuarantinePropertiesKey) }.is_ok()
}

fn shown_url(page_navigation_pending: bool, committed: &Option<Url>, live: Option<Url>) -> Option<Url> {
    if page_navigation_pending { committed.clone() } else { live }
}

fn page_state(web_view: &WKWebView) -> Option<(Url, String, bool, bool)> {
    // SAFETY: main-thread reads of a live web view's properties.
    unsafe {
        let url = to_url(web_view.URL())?;
        Some((url, page_title(web_view), web_view.canGoBack(), web_view.canGoForward()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_limit_rejects_without_recording() {
        let recent = RefCell::new(VecDeque::new());
        assert!((0..3).all(|_| within_burst(&recent, 3)));
        assert!(!within_burst(&recent, 3));
        assert_eq!(recent.borrow().len(), 3);
    }

    #[test]
    fn a_pending_page_navigation_shows_the_committed_url() {
        let (old, target) = (Url::parse("https://a.test/").ok(), Url::parse("https://bank.test:8443/").ok());
        assert_eq!(shown_url(true, &old, target.clone()), old);
        assert_eq!(shown_url(true, &None, target.clone()), None);
        assert_eq!(shown_url(false, &old, target.clone()), target);
    }

    #[test]
    fn titles_are_cut_by_characters() {
        assert_eq!(bounded(&"é".repeat(crate::view_policy::MAX_TITLE + 5)).chars().count(), crate::view_policy::MAX_TITLE);
        assert_eq!(bounded("short"), "short");
    }
}
