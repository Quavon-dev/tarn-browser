//! Reading and writing a Chromium cookie jar in the cookie rules' terms, on CEF's UI thread.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use cef::*;

use crate::cookies::{self as rules, Key, SameSite};

/// Seconds from 1601-01-01, CEF's time base, to the Unix epoch.
const WINDOWS_EPOCH: i64 = 11_644_473_600;

type Done = RefCell<Option<Box<dyn FnOnce(Option<Vec<rules::Cookie>>)>>>;

struct Visit {
    delete: HashSet<Key>,
    cookies: RefCell<Vec<rules::Cookie>>,
    ok: Cell<bool>,
    /// The jar size CEF reported, so a visit cut short is not taken for a smaller jar.
    total: Cell<Option<usize>>,
    done: Done,
}

impl Drop for Visit {
    fn drop(&mut self) {
        if let Some(done) = self.done.take() {
            let cookies = self.cookies.take();
            let complete = self.total.get().is_none_or(|total| total == cookies.len());
            done((self.ok.get() && complete).then_some(cookies));
        }
    }
}

wrap_cookie_visitor! {
    struct Visitor {
        visit: Rc<Visit>,
    }

    impl CookieVisitor {
        fn visit(&self, cookie: Option<&cef::Cookie>, _count: i32, total: i32, delete_cookie: Option<&mut i32>) -> i32 {
            self.visit.total.set(usize::try_from(total).ok());
            if let Some(cookie) = cookie.map(from_cef) {
                if let Some(delete) = delete_cookie.filter(|_| self.visit.delete.contains(&cookie.key())) {
                    *delete = 1;
                }
                self.visit.cookies.borrow_mut().push(cookie);
            }
            1
        }
    }
}

/// Visits `manager`'s whole jar, deleting the cookies in `delete`; `done` gets `None` when the jar
/// could not be read or the visit stopped early. CEF releases the visitor on the UI thread once
/// the last cookie was visited, or at once for an empty jar.
pub(crate) fn visit(manager: Option<CookieManager>, delete: HashSet<Key>, done: impl FnOnce(Option<Vec<rules::Cookie>>) + 'static) {
    let visit = Rc::new(Visit { delete, cookies: RefCell::default(), ok: Cell::new(true), total: Cell::new(None), done: RefCell::new(Some(Box::new(done))) });
    let mut visitor = Visitor::new(visit.clone());
    let started = manager.is_some_and(|m| m.visit_all_cookies(Some(&mut visitor)) == 1);
    visit.ok.set(started);
}

/// The URL CEF sets `c` for.
pub(crate) fn url(c: &rules::Cookie) -> CefString {
    let scheme = if c.secure { "https" } else { "http" };
    CefString::from(format!("{scheme}://{}{}", c.domain, c.path).as_str())
}

pub(crate) fn from_cef(c: &cef::Cookie) -> rules::Cookie {
    let domain = c.domain.to_string().to_ascii_lowercase();
    let same_site = if c.same_site == CookieSameSite::STRICT_MODE {
        SameSite::Strict
    } else if c.same_site == CookieSameSite::NO_RESTRICTION {
        SameSite::None
    } else {
        SameSite::Lax
    };
    rules::Cookie {
        name: c.name.to_string(),
        value: c.value.to_string(),
        host_only: !domain.starts_with('.'),
        domain: domain.trim_start_matches('.').to_owned(),
        path: c.path.to_string(),
        secure: c.secure != 0,
        http_only: c.httponly != 0,
        same_site,
        expires: (c.has_expires != 0).then(|| c.expires.val / 1_000_000 - WINDOWS_EPOCH),
        partitioned: false,
    }
}

/// An empty domain makes CEF set a host-only cookie for the URL's host.
pub(crate) fn to_cef(c: &rules::Cookie) -> cef::Cookie {
    cef::Cookie {
        name: CefString::from(c.name.as_str()),
        value: CefString::from(c.value.as_str()),
        domain: CefString::from(if c.host_only { String::new() } else { format!(".{}", c.domain) }.as_str()),
        path: CefString::from(c.path.as_str()),
        secure: i32::from(c.secure),
        httponly: i32::from(c.http_only),
        has_expires: i32::from(c.expires.is_some()),
        expires: Basetime { val: c.expires.map_or(0, |e| e.saturating_add(WINDOWS_EPOCH).saturating_mul(1_000_000)) },
        same_site: match c.same_site {
            SameSite::None => CookieSameSite::NO_RESTRICTION,
            SameSite::Lax => CookieSameSite::LAX_MODE,
            SameSite::Strict => CookieSameSite::STRICT_MODE,
        },
        priority: CookiePriority::MEDIUM,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cookie_survives_the_round_trip_through_cef() {
        let c = rules::Cookie {
            name: "sid".into(),
            value: "v".into(),
            domain: "a.test".into(),
            host_only: false,
            path: "/x".into(),
            secure: true,
            http_only: true,
            same_site: SameSite::Strict,
            expires: Some(1_800_000_000),
            partitioned: false,
        };
        assert_eq!(from_cef(&to_cef(&c)), c);
        assert_eq!(url(&c).to_string(), "https://a.test/x");
        let session = rules::Cookie { expires: None, ..c };
        assert_eq!(from_cef(&to_cef(&session)).expires, None);
    }
}
