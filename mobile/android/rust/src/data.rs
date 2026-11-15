use std::rc::Rc;

use browser_core::WebsiteData;
use engine::Event;
use engine::cookies::{Cookie, SameSite};

use crate::host::{self, Space};

pub const PRIVATE_PREFIX: &str = "private:";

/// A profile's website data in the system WebView. A persistent store is the `ProfileStore`
/// profile `tarn-<uuid>`; an ephemeral one is `private:<session>`, which `tarn.webview` maps to the
/// `:private` process's own data directory there, and to a throwaway profile deleted at the next
/// launch in the main process (Lockdown).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Data {
    store: Store,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Store {
    Persistent(String),
    Ephemeral(String),
}

impl Data {
    /// The `profile` prop of `tarn.webview`.
    pub fn store_id(&self) -> String {
        match &self.store {
            Store::Persistent(uuid) => uuid.clone(),
            Store::Ephemeral(session) => format!("{PRIVATE_PREFIX}{session}"),
        }
    }

    pub fn is_ephemeral(&self) -> bool {
        matches!(self.store, Store::Ephemeral(_))
    }

    fn open_persistent(space: &Space, uuid: &str) -> Option<Self> {
        match space {
            Space::Private { .. } => None,
            Space::Normal => canonical_uuid(uuid).map(|u| Data { store: Store::Persistent(u) }),
        }
    }

    fn open_ephemeral(space: &Space) -> Self {
        let session = match space {
            Space::Private { session } => session.clone(),
            Space::Normal => uuid::Uuid::new_v4().simple().to_string(),
        };
        Data { store: Store::Ephemeral(session) }
    }
}

impl Drop for Data {
    fn drop(&mut self) {
        if let (Store::Ephemeral(session), Space::Normal) = (&self.store, host::space()) {
            host::release_store(session);
        }
    }
}

fn canonical_uuid(uuid: &str) -> Option<String> {
    uuid::Uuid::try_parse(uuid).ok().filter(|u| uuid.len() == 36 && !u.is_nil()).map(|u| u.hyphenated().to_string())
}

impl WebsiteData for Data {
    /// Rules are applied per request by `tarn.webview` through this crate, not compiled per store.
    type RuleLists = ();

    fn persistent(uuid: &str, _own_store: &str, _downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        Self::open_persistent(&host::space(), uuid)
    }

    fn ephemeral(_downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        Some(Self::open_ephemeral(&host::space()))
    }

    fn wipe_persistent(uuid: &str) {
        if let Some(uuid) = canonical_uuid(uuid) {
            host::wipe_store(&uuid);
        }
    }

    fn import_cookies(&self, cookies: Vec<Cookie>, now: i64, done: Box<dyn FnOnce(usize, usize)>) {
        let total = cookies.len();
        let (cookies, _) = engine::cookies::admit_imported(cookies, &[], now);
        let (urls, values): (Vec<String>, Vec<String>) = cookies.iter().filter_map(|c| set_cookie(c, now)).unzip();
        let written = if urls.is_empty() { 0 } else { host::import_cookies(&self.store_id(), &urls, &values).min(urls.len()) };
        done(written, total - written);
    }
}

/// The URL and `Set-Cookie` value that recreate `cookie`; `None` once it has expired.
fn set_cookie(cookie: &Cookie, now: i64) -> Option<(String, String)> {
    let max_age = match cookie.expires {
        Some(at) if at <= now => return None,
        Some(at) => Some(at - now),
        None => None,
    };
    let host = cookie.domain.trim_start_matches('.');
    if host.is_empty() || cookie.name.contains([';', '=']) || cookie.value.contains(';') {
        return None;
    }
    let path = if cookie.path.starts_with('/') { cookie.path.as_str() } else { "/" };
    let scheme = if cookie.secure { "https" } else { "http" };
    let mut value = format!("{}={}; Path={path}", cookie.name, cookie.value);
    if !cookie.host_only {
        value.push_str(&format!("; Domain={host}"));
    }
    if let Some(age) = max_age {
        value.push_str(&format!("; Max-Age={age}"));
    }
    if cookie.secure {
        value.push_str("; Secure");
    }
    if cookie.http_only {
        value.push_str("; HttpOnly");
    }
    value.push_str(match cookie.same_site {
        SameSite::None => "; SameSite=None",
        SameSite::Lax => "; SameSite=Lax",
        SameSite::Strict => "; SameSite=Strict",
    });
    if cookie.partitioned {
        value.push_str("; Partitioned");
    }
    Some((format!("{scheme}://{host}{path}"), value))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "6f1c2a9e-3b7d-4c55-9a0e-1d2f3a4b5c6d";

    fn downloads() -> Rc<dyn Fn(Event)> {
        Rc::new(|_| {})
    }

    #[test]
    fn a_persistent_store_is_the_profiles_canonical_uuid() {
        let data = Data::persistent(&UUID.to_uppercase(), UUID, downloads()).unwrap();
        assert_eq!(data.store_id(), UUID);
        assert!(!data.is_ephemeral());
        for other in [
            "../Default",
            "00000000-0000-0000-0000-000000000000",
            "6f1c2a9e3b7d4c559a0e1d2f3a4b5c6d",
            "{6f1c2a9e-3b7d-4c55-9a0e-1d2f3a4b5c6d}",
            "urn:uuid:6f1c2a9e-3b7d-4c55-9a0e-1d2f3a4b5c6d",
        ] {
            assert!(Data::persistent(other, UUID, downloads()).is_none(), "only a hyphenated, non-nil UUID names a profile: {other}");
        }
    }

    #[test]
    fn the_private_process_never_opens_a_persistent_store() {
        let session = "0123456789abcdef0123456789abcdef".to_owned();
        let space = Space::Private { session: session.clone() };
        assert!(Data::open_persistent(&space, UUID).is_none());
        assert_eq!(Data::open_ephemeral(&space).store_id(), format!("private:{session}"), "its ephemeral store is the process's own session");
    }

    #[test]
    fn each_ephemeral_store_in_the_main_process_is_new() {
        let a = Data::ephemeral(downloads()).unwrap();
        let b = Data::ephemeral(downloads()).unwrap();
        assert!(a.is_ephemeral());
        assert_ne!(a, b);
        let session = a.store_id().strip_prefix(PRIVATE_PREFIX).unwrap().to_owned();
        assert!(host::valid_session(&session), "{session}");
    }

    fn cookie(name: &str, expires: Option<i64>) -> Cookie {
        Cookie {
            name: name.into(),
            value: "v".into(),
            domain: "example.com".into(),
            host_only: false,
            path: "/a".into(),
            secure: true,
            http_only: true,
            same_site: SameSite::Lax,
            expires,
            partitioned: false,
        }
    }

    #[test]
    fn cookies_become_set_cookie_values() {
        let (url, value) = set_cookie(&cookie("sid", Some(1_100)), 1_000).unwrap();
        assert_eq!(url, "https://example.com/a");
        assert_eq!(value, "sid=v; Path=/a; Domain=example.com; Max-Age=100; Secure; HttpOnly; SameSite=Lax");
        let session = Cookie { host_only: true, secure: false, http_only: false, path: String::new(), ..cookie("s", None) };
        assert_eq!(set_cookie(&session, 0).unwrap(), ("http://example.com/".into(), "s=v; Path=/; SameSite=Lax".into()));
        assert!(set_cookie(&cookie("old", Some(999)), 1_000).is_none());
        assert!(set_cookie(&cookie("a;b", None), 0).is_none());
    }

    #[test]
    fn without_a_host_imported_cookies_are_refused() {
        let data = Data::persistent(UUID, UUID, downloads()).unwrap();
        let result = Rc::new(std::cell::Cell::new((9, 9)));
        let seen = result.clone();
        data.import_cookies(vec![cookie("a", None), cookie("old", Some(1))], 1_000, Box::new(move |w, r| seen.set((w, r))));
        assert_eq!(result.get(), (0, 2));
    }
}
