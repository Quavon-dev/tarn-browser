use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use cef::*;
use url::Url;

use crate::request_filter::{self, RequestAction, Shield};

#[cfg(target_os = "macos")]
static SHIELD: RwLock<Option<Arc<Shield>>> = RwLock::new(None);
/// Each profile's choices on Windows, by `DataStore::shield_key`.
#[cfg(windows)]
static SHIELDS: RwLock<BTreeMap<String, Arc<Shield>>> = RwLock::new(BTreeMap::new());

/// Requests other than page loads the filter cancelled, by Shield key (empty on macOS, where one
/// profile owns Chromium) and host of the page shown.
static BLOCKED: Mutex<BTreeMap<(String, String), i64>> = Mutex::new(BTreeMap::new());

/// Requests other than page loads the filter cancelled since the last call, per host of the page shown.
#[cfg(not(windows))]
pub fn take_blocked() -> BTreeMap<String, i64> {
    let mut hosts = BTreeMap::new();
    for ((_, host), n) in std::mem::take(&mut *BLOCKED.lock().unwrap_or_else(PoisonError::into_inner)) {
        *hosts.entry(host).or_default() += n;
    }
    hosts
}

/// As `take_blocked`, for the profile with this Shield key.
#[cfg(windows)]
pub(crate) fn take_blocked_for(key: &str) -> BTreeMap<String, i64> {
    let mut blocked = BLOCKED.lock().unwrap_or_else(PoisonError::into_inner);
    let mine: Vec<(String, String)> = blocked.keys().filter(|(k, _)| k == key).cloned().collect();
    mine.into_iter().filter_map(|k| blocked.remove(&k).map(|n| (k.1, n))).collect()
}

/// Drops the counts of a profile whose Shield key is gone.
#[cfg(windows)]
pub(crate) fn forget_blocked(key: &str) {
    BLOCKED.lock().unwrap_or_else(PoisonError::into_inner).retain(|(k, _), _| k != key);
}

/// Replaces the Shield choices Chromium's requests are checked against. Until the first call
/// every request is cancelled.
#[cfg(target_os = "macos")]
pub fn set_shield(shield: Shield) {
    *SHIELD.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(shield));
}

/// As `set_shield`, for the requests of one data store.
#[cfg(windows)]
pub(crate) fn set_shield_for(key: &str, shield: Shield) {
    SHIELDS.write().unwrap_or_else(PoisonError::into_inner).insert(key.to_owned(), Arc::new(shield));
}

#[cfg(windows)]
pub(crate) fn drop_shield(key: &str) {
    SHIELDS.write().unwrap_or_else(PoisonError::into_inner).remove(key);
    forget_blocked(key);
}

/// The choices for the requests of `profile`'s data store; one set for all on macOS, where one
/// profile owns Chromium.
#[cfg(target_os = "macos")]
pub(crate) fn shield_for(_profile: &str) -> Option<Arc<Shield>> {
    SHIELD.read().unwrap_or_else(PoisonError::into_inner).clone()
}

#[cfg(windows)]
pub(crate) fn shield_for(profile: &str) -> Option<Arc<Shield>> {
    SHIELDS.read().unwrap_or_else(PoisonError::into_inner).get(profile).cloned()
}

const KINDS: [(ResourceType, &str); 20] = [
    (ResourceType::MAIN_FRAME, "main_frame"),
    (ResourceType::NAVIGATION_PRELOAD_MAIN_FRAME, "main_frame"),
    (ResourceType::SUB_FRAME, "sub_frame"),
    (ResourceType::NAVIGATION_PRELOAD_SUB_FRAME, "sub_frame"),
    (ResourceType::STYLESHEET, "stylesheet"),
    (ResourceType::SCRIPT, "script"),
    (ResourceType::WORKER, "script"),
    (ResourceType::SHARED_WORKER, "script"),
    (ResourceType::SERVICE_WORKER, "script"),
    (ResourceType::IMAGE, "image"),
    (ResourceType::FAVICON, "image"),
    (ResourceType::FONT_RESOURCE, "font"),
    (ResourceType::OBJECT, "object"),
    (ResourceType::PLUGIN_RESOURCE, "object"),
    (ResourceType::MEDIA, "media"),
    (ResourceType::XHR, "xhr"),
    (ResourceType::PING, "ping"),
    (ResourceType::CSP_REPORT, "csp_report"),
    (ResourceType::PREFETCH, "other"),
    (ResourceType::SUB_RESOURCE, "other"),
];

fn parse(s: &CefStringUserfree) -> Option<Url> {
    Url::parse(&CefString::from(s).to_string()).ok()
}

wrap_resource_request_handler! {
    pub(crate) struct TarnFilter {
        hold: Option<String>,
        profile: String,
        lockdown_view: bool,
    }

    impl ResourceRequestHandler {
        // Runs on CEF's IO thread: reads only the snapshot and CEF's own thread-safe objects.
        fn on_before_resource_load(&self, browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, callback: Option<&mut Callback>) -> ReturnValue {
            if super::outdated() {
                return ReturnValue::CANCEL;
            }
            let (Some(request), Some(shield)) = (request, shield_for(&self.profile)) else { return ReturnValue::CANCEL };
            let Some(url) = parse(&request.url()) else { return ReturnValue::CANCEL };
            let kind = if matches!(url.scheme(), "ws" | "wss") {
                "websocket"
            } else {
                let resource = request.resource_type();
                KINDS.iter().find(|(t, _)| *t == resource).map_or("other", |(_, k)| k)
            };
            let first_party = parse(&request.first_party_for_cookies());
            let top = browser.and_then(|b| b.main_frame()).and_then(|f| parse(&f.url())).or_else(|| first_party.clone());
            let referrer = parse(&request.referrer_url());
            match request_filter::decide(&shield, &url, kind, top.as_ref(), first_party.as_ref(), referrer.as_ref(), self.lockdown_view) {
                RequestAction::Cancel => {
                    if let Some(host) = top.as_ref().and_then(Url::host_str).filter(|_| kind != "main_frame") {
                        *BLOCKED.lock().unwrap_or_else(PoisonError::into_inner).entry((self.profile.clone(), host.to_owned())).or_default() += 1;
                    }
                    ReturnValue::CANCEL
                }
                RequestAction::Continue { upgrade, strip_referrer } => {
                    if strip_referrer {
                        request.set_referrer(None, ReferrerPolicy::NO_REFERRER);
                    }
                    if let Some(upgrade) = upgrade {
                        request.set_url(Some(&CefString::from(upgrade.as_str())));
                    }
                    self.hold.as_deref().map_or(ReturnValue::CONTINUE, |site| super::cookies::hold(site, callback))
                }
            }
        }
    }
}

wrap_request_context_handler! {
    pub(crate) struct TarnContextFilter {
        profile: String,
    }

    impl RequestContextHandler {
        fn resource_request_handler(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>, _is_navigation: i32, _is_download: i32, _request_initiator: Option<&CefString>, _disable_default_handling: Option<&mut i32>) -> Option<ResourceRequestHandler> {
            Some(TarnFilter::new(None, self.profile.clone(), false))
        }
    }
}

/// Requests of service workers have no browser, so CEF hands them to a handler of any request
/// context sharing the profile's storage; this context exists only to be that handler.
#[cfg(target_os = "macos")]
pub(crate) fn worker_filter(global: &mut RequestContext) -> Option<RequestContext> {
    request_context_cef_create_context_shared(Some(global), Some(&mut TarnContextFilter::new(String::new())))
}
