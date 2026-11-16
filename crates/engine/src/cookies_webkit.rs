use std::cell::RefCell;
use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSDate, NSDictionary, NSHTTPCookie, NSHTTPCookieDomain, NSHTTPCookieExpires, NSHTTPCookieName, NSHTTPCookiePath, NSHTTPCookieSameSiteLax,
    NSHTTPCookieSameSitePolicy, NSHTTPCookieSameSiteStrict, NSHTTPCookieSecure, NSHTTPCookieValue, NSNumber, NSObject, NSString,
};
use objc2_web_kit::{WKHTTPCookieStore, WKHTTPCookieStoreObserver};

use crate::DataStore;
use crate::cookies::{Cookie, SameSite};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnCookieObserver"]
    #[ivars = Box<dyn Fn()>]
    struct Observer;

    unsafe impl NSObjectProtocol for Observer {}

    unsafe impl WKHTTPCookieStoreObserver for Observer {
        #[unsafe(method(cookiesDidChangeInCookieStore:))]
        fn cookies_did_change(&self, _store: &WKHTTPCookieStore) {
            (self.ivars())();
        }
    }
);

/// A profile's WebKit cookie jar, as the cookie bridge reads and writes it.
pub(crate) struct WebKitCookies {
    store: Retained<WKHTTPCookieStore>,
    observer: RefCell<Option<Retained<Observer>>>,
}

impl WebKitCookies {
    pub(crate) fn new(data: &DataStore) -> Self {
        // SAFETY: `DataStore` lives on the main thread; the store is a live WKWebsiteDataStore.
        WebKitCookies { store: unsafe { data.store.httpCookieStore() }, observer: RefCell::new(None) }
    }

    pub(crate) fn read(&self, done: impl FnOnce(Vec<Cookie>) + 'static) {
        let done = RefCell::new(Some(done));
        let handler = RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
            // SAFETY: WebKit passes a valid array for the duration of the call.
            let cookies = unsafe { cookies.as_ref() };
            if let Some(done) = done.take() {
                done(cookies.iter().map(|c| from_ns(&c)).collect());
            }
        });
        // SAFETY: main thread (the store is main-thread only); WebKit copies the block.
        unsafe { self.store.getAllCookies(&handler) };
    }

    /// `done` gets false without a write when WebKit cannot hold the cookie as it is.
    pub(crate) fn set(&self, cookie: &Cookie, done: impl FnOnce(bool) + 'static) {
        let Some(ns) = to_ns(cookie).filter(|ns| from_ns(ns) == *cookie) else { return done(false) };
        let done = RefCell::new(Some(done));
        let handler = RcBlock::new(move || {
            if let Some(done) = done.take() {
                done(true);
            }
        });
        // SAFETY: main thread; the cookie is valid and WebKit copies the block.
        unsafe { self.store.setCookie_completionHandler(&ns, Some(&handler)) };
    }

    pub(crate) fn delete(&self, cookie: &Cookie, done: impl FnOnce() + 'static) {
        let Some(ns) = to_ns(cookie) else { return done() };
        let done = RefCell::new(Some(done));
        let handler = RcBlock::new(move || {
            if let Some(done) = done.take() {
                done();
            }
        });
        // SAFETY: main thread; WebKit matches the cookie by name, domain and path and copies the block.
        unsafe { self.store.deleteCookie_completionHandler(&ns, Some(&handler)) };
    }

    /// Calls `changed` whenever WebKit's jar changes, until `self` is dropped.
    pub(crate) fn observe(&self, changed: impl Fn() + 'static) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let observer = Observer::alloc(mtm).set_ivars(Box::new(changed) as Box<dyn Fn()>);
        // SAFETY: NSObject's designated initialiser on a freshly allocated instance.
        let observer: Retained<Observer> = unsafe { msg_send![super(observer), init] };
        // SAFETY: main thread; WebKit holds the observer weakly and `Drop` removes it.
        unsafe { self.store.addObserver(ProtocolObject::from_ref(&*observer)) };
        if let Some(old) = self.observer.replace(Some(observer)) {
            // SAFETY: as above.
            unsafe { self.store.removeObserver(ProtocolObject::from_ref(&*old)) };
        }
    }
}

impl Drop for WebKitCookies {
    fn drop(&mut self) {
        if let Some(observer) = self.observer.take() {
            // SAFETY: main thread (the store is main-thread only).
            unsafe { self.store.removeObserver(ProtocolObject::from_ref(&*observer)) };
        }
    }
}

fn from_ns(c: &NSHTTPCookie) -> Cookie {
    let domain = c.domain().to_string().to_ascii_lowercase();
    let same_site = same_site(c.sameSitePolicy().map(|p| p.to_string()).as_deref());
    let partitioned = c.properties().is_some_and(|p| p.allKeys().iter().any(|k| k.to_string().to_ascii_lowercase().contains("partition")));
    Cookie {
        name: c.name().to_string(),
        value: c.value().to_string(),
        host_only: !domain.starts_with('.'),
        domain: domain.trim_start_matches('.').to_owned(),
        path: c.path().to_string(),
        secure: c.isSecure(),
        http_only: c.isHTTPOnly(),
        same_site,
        expires: (!c.isSessionOnly()).then(|| c.expiresDate().map(|d| d.timeIntervalSince1970() as i64)).flatten(),
        partitioned,
    }
}

/// WebKit has no "unspecified": it reports a cookie without `SameSite` as `none`, the same as an
/// explicit `SameSite=None`. Both are read as `Lax`, as Chromium reads an unspecified one, so the
/// bridge never writes a cookie to Chromium with fewer restrictions than Chromium's default.
fn same_site(policy: Option<&str>) -> SameSite {
    if policy.is_some_and(|p| p.eq_ignore_ascii_case("strict")) { SameSite::Strict } else { SameSite::Lax }
}

/// `HttpOnly` has no public property key; WebKit's own conversion uses this one, and `set`
/// refuses a cookie that does not read back with every attribute it was given.
fn to_ns(c: &Cookie) -> Option<Retained<NSHTTPCookie>> {
    let domain = if c.host_only { c.domain.clone() } else { format!(".{}", c.domain) };
    let yes = || -> Retained<AnyObject> { NSNumber::new_bool(true).into() };
    // SAFETY: Foundation's exported constant strings.
    let mut pairs: Vec<(&NSString, Retained<AnyObject>)> = unsafe {
        vec![
            (NSHTTPCookieName, NSString::from_str(&c.name).into()),
            (NSHTTPCookieValue, NSString::from_str(&c.value).into()),
            (NSHTTPCookieDomain, NSString::from_str(&domain).into()),
            (NSHTTPCookiePath, NSString::from_str(&c.path).into()),
        ]
    };
    let http_only = NSString::from_str("HttpOnly");
    // SAFETY: as above.
    unsafe {
        if c.secure {
            pairs.push((NSHTTPCookieSecure, yes()));
        }
        if let Some(expires) = c.expires {
            pairs.push((NSHTTPCookieExpires, NSDate::dateWithTimeIntervalSince1970(expires as f64).into()));
        }
        match c.same_site {
            SameSite::Lax => pairs.push((NSHTTPCookieSameSitePolicy, NSHTTPCookieSameSiteLax.retain().into())),
            SameSite::Strict => pairs.push((NSHTTPCookieSameSitePolicy, NSHTTPCookieSameSiteStrict.retain().into())),
            SameSite::None => {}
        }
    }
    if c.http_only {
        pairs.push((&http_only, yes()));
    }
    let keys: Vec<&NSString> = pairs.iter().map(|(k, _)| *k).collect();
    let values: Vec<Retained<AnyObject>> = pairs.into_iter().map(|(_, v)| v).collect();
    let properties = NSDictionary::<NSString, AnyObject>::from_retained_objects(&keys, &values);
    // SAFETY: the dictionary holds only NSHTTPCookie property keys with values of the documented types.
    unsafe { NSHTTPCookie::cookieWithProperties(&properties) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webkits_none_reads_as_lax() {
        assert_eq!(same_site(Some("none")), SameSite::Lax, "WebKit reports a cookie without SameSite as none");
        assert_eq!(same_site(None), SameSite::Lax);
        assert_eq!(same_site(Some("lax")), SameSite::Lax);
        assert_eq!(same_site(Some("strict")), SameSite::Strict);
    }
}
