//! Writes imported cookies into a real WebKit jar and reads them back. Needs the main thread, so
//! it has its own `main`: `cargo test -p engine --test import_cookies`.

#[cfg(target_os = "macos")]
fn main() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
    use engine::DataStore;
    use engine::cookies::{Cookie, SameSite};

    fn wait_for<T>(slot: &RefCell<Option<T>>) -> T {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(v) = slot.take() {
                return v;
            }
            assert!(Instant::now() < deadline, "WebKit did not answer");
            // SAFETY: Core Foundation's exported constant run-loop mode.
            CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, Duration::from_millis(50), true);
        }
    }

    let store = DataStore::ephemeral(Rc::new(|_| {})).expect("the main thread");
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let cookie = |name: &str, domain: &str, expires: i64| Cookie {
        name: name.into(),
        value: "v".into(),
        domain: domain.into(),
        host_only: false,
        path: "/".into(),
        secure: true,
        http_only: true,
        same_site: SameSite::Lax,
        expires: Some(expires),
        partitioned: false,
    };
    let good = cookie("sid", "a.test", now + 3600);
    let refused = [cookie("suffix", "co.uk", now + 3600), cookie("old", "a.test", now - 1), cookie("ip", "127.0.0.1", now + 3600)];

    let counts = Rc::new(RefCell::new(None));
    let slot = counts.clone();
    store.import_cookies([vec![good.clone()], refused.to_vec()].concat(), now, move |ok, refused| *slot.borrow_mut() = Some((ok, refused)));
    assert_eq!(wait_for(&counts), (1, 3), "a public-suffix, an expired and an IP domain cookie are refused");

    let weaker = Cookie { value: "w".into(), secure: false, http_only: false, ..good.clone() };
    let replacement = Cookie { value: "other account".into(), ..good.clone() };
    let shadow = Cookie { value: "other account".into(), path: "/account".into(), ..good.clone() };
    let slot = counts.clone();
    store.import_cookies(vec![replacement, shadow], now, move |ok, refused| *slot.borrow_mut() = Some((ok, refused)));
    assert_eq!(wait_for(&counts), (0, 2), "a cookie the jar holds is neither replaced nor shadowed on a longer path");
    let readable = Cookie { name: "pref".into(), http_only: false, expires: Some(now + 365 * 86_400), ..good.clone() };
    let slot = counts.clone();
    store.import_cookies(vec![weaker, readable], now, move |ok, refused| *slot.borrow_mut() = Some((ok, refused)));
    assert_eq!(wait_for(&counts), (1, 1), "a cookie that would drop HttpOnly and Secure from one in the jar is refused");

    let many: Vec<Cookie> =
        (0..engine::cookies::MAX_COOKIES_PER_SITE + 5).map(|i| Cookie { name: format!("c{i}"), ..cookie("x", "b.test", now + 3600) }).collect();
    let slot = counts.clone();
    store.import_cookies(many, now, move |ok, refused| *slot.borrow_mut() = Some((ok, refused)));
    assert_eq!(wait_for(&counts), (engine::cookies::MAX_COOKIES_PER_SITE, 5), "a site stays within the bridge's cap");

    let jar = Rc::new(RefCell::new(None));
    let slot = jar.clone();
    store.cookies(move |all| *slot.borrow_mut() = Some(all));
    let all = wait_for(&jar);
    assert_eq!(all.len(), 2 + engine::cookies::MAX_COOKIES_PER_SITE);
    let sid = all.iter().find(|c| c.name == "sid").unwrap();
    assert!(sid.http_only && sid.secure, "HttpOnly and Secure survive");
    assert_eq!(*sid, good);
    let pref = all.iter().find(|c| c.name == "pref").unwrap();
    assert!(pref.expires.unwrap() <= now + engine::cookies::SCRIPT_COOKIE_LIFETIME, "a script-readable cookie gets the bridge's lifetime");
    println!("import_cookies: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
