//! The cookie bridge between the owning profile's WebKit store and Chromium. Everything here runs on the main thread, which is CEF's UI thread, except the
//! request hold, which CEF calls on its IO thread and which therefore touches only `GATES`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cef::*;
use url::Url;

use super::jar;
use crate::cookies::{self as rules, Base, Jar, Key, Op, Side};
use crate::cookies_webkit::WebKitCookies;
use crate::{DataStore, Event};

const HOLD_LIMIT: Duration = Duration::from_secs(2);
const DEBOUNCE: Duration = Duration::from_millis(300);

enum Gate {
    /// The bridging attempt the gate waits for, and the requests it holds.
    Waiting(u64, Vec<Held>),
    Open,
    Closed,
}

struct Held(Callback);

// SAFETY: CEF callbacks are atomically reference counted, and a resource request's callback may be
// continued or cancelled from any thread.
unsafe impl Send for Held {}

/// The first main-frame request to a site that is being bridged waits here until WebKit's
/// cookies are in Chromium. Read on CEF's IO thread.
static GATES: LazyLock<Mutex<HashMap<String, Gate>>> = LazyLock::new(Mutex::default);

fn gates() -> std::sync::MutexGuard<'static, HashMap<String, Gate>> {
    GATES.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Opens or closes a site's gate if it still waits for `attempt` (any attempt for `None`); true
/// if it did.
fn release(site: &str, open: bool, attempt: Option<u64>) -> bool {
    let mut gates = gates();
    if !matches!(gates.get(site), Some(Gate::Waiting(a, _)) if attempt.is_none_or(|n| n == *a)) {
        return false;
    }
    let Some(Gate::Waiting(_, held)) = gates.insert(site.to_owned(), if open { Gate::Open } else { Gate::Closed }) else { return false };
    drop(gates);
    for Held(callback) in held {
        if open {
            callback.cont();
        } else {
            callback.cancel();
        }
    }
    true
}

/// Called on CEF's IO thread for every main-frame navigation request: the bridged site it
/// must wait for, if any.
pub(crate) fn hold_for(url: &str) -> Option<String> {
    let site = Url::parse(url).ok()?.host_str().map(rules::site)?;
    matches!(gates().get(&site), Some(Gate::Waiting(..))).then_some(site)
}

/// Holds a request of a bridged site until WebKit's cookies are in Chromium.
pub(crate) fn hold(site: &str, callback: Option<&mut Callback>) -> ReturnValue {
    let mut gates = gates();
    match (gates.get_mut(site), callback) {
        (Some(Gate::Waiting(_, held)), Some(callback)) => {
            held.push(Held(callback.clone()));
            ReturnValue::CONTINUE_ASYNC
        }
        (Some(Gate::Open), _) => ReturnValue::CONTINUE,
        _ => ReturnValue::CANCEL,
    }
}

type Waiter = Box<dyn FnOnce(bool)>;
type Then<T> = RefCell<Option<Box<dyn FnOnce(T)>>>;

struct Bridge {
    webkit: WebKitCookies,
    /// The WebKit store's identifier, so clears of other profiles' stores leave the bridge alone.
    store: String,
    base: RefCell<Base>,
    /// Bridged sites, with the page and tab that bridged each, for notices.
    sites: RefCell<HashMap<String, (Url, u64)>>,
    /// Bridged sites whose first merge has not succeeded yet.
    fresh: RefCell<HashSet<String>>,
    /// Chromium's jar was at its eviction level at the last read, so a cookie missing now may have
    /// been evicted in between.
    evicting: Cell<bool>,
    /// The last Chromium read was empty. An empty read deletes nothing until the next read
    /// confirms it, since a visit CEF dropped unvisited looks the same.
    empty_once: Cell<bool>,
    over_cap: RefCell<HashSet<String>>,
    /// Sites cleared this session; they never bridge again before a relaunch.
    excluded: RefCell<HashSet<String>>,
    suspended: Cell<bool>,
    busy: Cell<bool>,
    again: Cell<Option<Side>>,
    waiting: RefCell<Vec<Waiter>>,
    last_webkit: RefCell<Option<Vec<rules::Cookie>>>,
    debouncing: Cell<bool>,
}

thread_local! {
    static BRIDGE: RefCell<Option<Rc<Bridge>>> = const { RefCell::new(None) };
    static ATTEMPTS: Cell<u64> = const { Cell::new(0) };
}

fn bridge() -> Option<Rc<Bridge>> {
    BRIDGE.with_borrow(|b| b.clone()).filter(|b| !b.suspended.get())
}

/// Starts the bridge for the profile that owns Chromium; later calls keep the first one. A store
/// without an identifier (an ephemeral one) never bridges.
pub(crate) fn attach(data: &DataStore) {
    if BRIDGE.with_borrow(Option::is_some) {
        return;
    }
    let Some(store) = data.identifier() else { return };
    let bridge = Rc::new(Bridge {
        webkit: WebKitCookies::new(data),
        store,
        base: RefCell::default(),
        sites: RefCell::default(),
        fresh: RefCell::default(),
        evicting: Cell::new(false),
        empty_once: Cell::new(false),
        over_cap: RefCell::default(),
        excluded: RefCell::default(),
        suspended: Cell::new(false),
        busy: Cell::new(false),
        again: Cell::new(None),
        waiting: RefCell::default(),
        last_webkit: RefCell::default(),
        debouncing: Cell::new(false),
    });
    let weak = Rc::downgrade(&bridge);
    bridge.webkit.observe(move || {
        let Some(bridge) = weak.upgrade() else { return };
        if bridge.sites.borrow().is_empty() || bridge.debouncing.replace(true) {
            return;
        }
        super::runtime::after(DEBOUNCE, move || {
            bridge.debouncing.set(false);
            bridge.sync(Side::WebKit, None);
        });
    });
    BRIDGE.set(Some(bridge));
}

/// A site whose bridging failed is bridged again by the next shell load or gesture navigation.
fn retry_notice(site: &str, tab: u64, url: Url) {
    if let Some(bridge) = BRIDGE.with_borrow(|b| b.clone()) {
        bridge.sites.borrow_mut().remove(site);
        bridge.fresh.borrow_mut().remove(site);
    }
    super::view::notify(
        tab,
        Event::Blocked {
            url,
            message: "This page was stopped because its sign-in could not be brought over from WebKit. Reload to try again.".into(),
            category: None,
        },
    );
}

/// A top-level navigation in tab `tab` started by the shell or by a user gesture: bridges its
/// site, and holds its request until WebKit's cookies are written to Chromium; after
/// `HOLD_LIMIT`, or when that fails, the request is cancelled with a notice.
pub(crate) fn bridge_navigation(url: &Url, tab: u64) {
    let Some(bridge) = bridge() else { return };
    let Some(site) = url.host_str().map(rules::site).filter(|_| matches!(url.scheme(), "http" | "https")) else { return };
    if bridge.excluded.borrow().contains(&site) || bridge.sites.borrow().contains_key(&site) {
        return;
    }
    bridge.sites.borrow_mut().insert(site.clone(), (url.clone(), tab));
    bridge.fresh.borrow_mut().insert(site.clone());
    let attempt = ATTEMPTS.get() + 1;
    ATTEMPTS.set(attempt);
    gates().insert(site.clone(), Gate::Waiting(attempt, Vec::new()));
    let (opened, failed) = (site.clone(), url.clone());
    bridge.sync(
        Side::WebKit,
        Some(Box::new(move |ok| {
            if release(&opened, ok, Some(attempt)) && !ok {
                retry_notice(&opened, tab, failed);
            }
        })),
    );
    let url = url.clone();
    super::runtime::after(HOLD_LIMIT, move || {
        if release(&site, false, Some(attempt)) {
            retry_notice(&site, tab, url);
        }
    });
}

/// Chromium may have changed cookies: a load ended, a tab was hidden, or the periodic tick.
pub(crate) fn changed_in_chromium() {
    if let Some(bridge) = bridge().filter(|b| !b.sites.borrow().is_empty()) {
        bridge.sync(Side::Chromium, None);
    }
}

/// Before a clear of the WebKit store `store`: `None` stops the bridge for the rest of the
/// session; a host stops its site. The clear is then never read as the user deleting cookies.
pub(crate) fn suspend(store: &str, host: Option<&str>) {
    if let Some(bridge) = BRIDGE.with_borrow(|b| b.clone()).filter(|b| b.store.eq_ignore_ascii_case(store)) {
        stop(&bridge, host);
    }
}

/// Before a clear of Chromium's own data, which belongs to the bridged profile.
pub(crate) fn suspend_chromium(host: Option<&str>) {
    if let Some(bridge) = BRIDGE.with_borrow(|b| b.clone()) {
        stop(&bridge, host);
    }
}

fn stop(bridge: &Bridge, host: Option<&str>) {
    let sites: Vec<String> = match host {
        Some(host) => vec![rules::site(host)],
        None => {
            bridge.suspended.set(true);
            bridge.sites.borrow().keys().cloned().collect()
        }
    };
    for site in sites {
        bridge.sites.borrow_mut().remove(&site);
        bridge.base.borrow_mut().forget(&site);
        bridge.fresh.borrow_mut().remove(&site);
        bridge.excluded.borrow_mut().insert(site.clone());
        release(&site, false, None);
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

impl Bridge {
    fn sync(self: &Rc<Self>, active: Side, waiter: Option<Waiter>) {
        self.waiting.borrow_mut().extend(waiter);
        if self.busy.replace(true) {
            let side = match (self.again.get(), active) {
                (Some(Side::WebKit), _) | (_, Side::WebKit) => Side::WebKit,
                _ => Side::Chromium,
            };
            self.again.set(Some(side));
            return;
        }
        let waiters = std::mem::take(&mut *self.waiting.borrow_mut());
        let this = self.clone();
        self.webkit.read(move |webkit| {
            read_chromium(move |chromium| match chromium {
                Some(chromium) if !this.suspended.get() => this.apply(active, webkit, chromium, waiters),
                _ => this.finish(waiters, false),
            });
        });
    }

    /// The eviction level is Chromium's; WebKit's jar is the one at rest and never counts as full.
    fn jars(&self, webkit: Vec<rules::Cookie>, chromium: Vec<rules::Cookie>) -> Rc<[Jar; 2]> {
        let mut webkit = Jar::new(webkit);
        webkit.full = false;
        let mut chromium = Jar::new(chromium);
        let evicted = self.evicting.replace(chromium.full);
        let empty = chromium.cookies.is_empty();
        let unconfirmed = empty && !self.empty_once.replace(empty);
        chromium.full |= evicted || unconfirmed;
        Rc::new([webkit, chromium])
    }

    fn apply(self: &Rc<Self>, active: Side, webkit: Vec<rules::Cookie>, chromium: Vec<rules::Cookie>, waiters: Vec<Waiter>) {
        let before = self.jars(webkit, chromium);
        let [webkit, chromium] = &*before;
        let mut plans = Vec::new();
        let sites: Vec<(String, Url, u64)> = self.sites.borrow().iter().map(|(s, (u, tab))| (s.clone(), u.clone(), *tab)).collect();
        let time = now();
        for (site, url, tab) in sites {
            if self.over_cap.borrow().contains(&site) {
                continue;
            }
            let planned = if self.fresh.borrow().contains(&site) {
                rules::first_bridge(&site, webkit, chromium, time)
            } else {
                rules::merge(&site, &self.base.borrow(), webkit, chromium, active, time)
            };
            match planned {
                Ok(plan) => plans.push((site, plan)),
                Err(rules::OverCap) => {
                    self.over_cap.borrow_mut().insert(site);
                    super::view::notify(
                        tab,
                        Event::Blocked {
                            url,
                            message: "This site keeps too many cookies to share between WebKit and Chromium, so its sign-in is no longer kept in step.".into(),
                            category: None,
                        },
                    );
                }
            }
        }
        if plans.iter().all(|(_, p)| p.webkit.is_empty() && p.chromium.is_empty()) {
            self.confirm(&plans, &before, &before);
            return self.finish(waiters, true);
        }
        let this = self.clone();
        let plans = Rc::new(plans);
        let applied = plans.clone();
        let countdown = Countdown::new(move || {
            let reader = this.clone();
            reader.webkit.read(move |webkit| {
                read_chromium(move |chromium| match chromium {
                    Some(chromium) if !this.suspended.get() => {
                        let after = this.jars(webkit, chromium);
                        this.confirm(&applied, &before, &after);
                        this.finish(waiters, true);
                    }
                    _ => this.finish(waiters, false),
                });
            });
        });
        for (_, plan) in plans.iter() {
            for op in &plan.webkit {
                let done = countdown.clone();
                match op {
                    Op::Set(cookie) => self.webkit.set(cookie, move |_| drop(done)),
                    Op::Delete(cookie) => self.webkit.delete(cookie, move || drop(done)),
                }
            }
        }
        write_chromium(plans.iter().flat_map(|(_, p)| &p.chromium), countdown);
    }

    fn confirm(&self, plans: &[(String, rules::Plan)], before: &Rc<[Jar; 2]>, after: &Rc<[Jar; 2]>) {
        let mut base = self.base.borrow_mut();
        for (site, plan) in plans {
            base.confirm(site, plan, [&before[0], &before[1]], [&after[0], &after[1]]);
            self.fresh.borrow_mut().remove(site);
        }
        *self.last_webkit.borrow_mut() = Some(after[0].cookies.clone());
    }

    fn finish(self: &Rc<Self>, waiters: Vec<Waiter>, ok: bool) {
        self.busy.set(false);
        for waiter in waiters {
            waiter(ok);
        }
        if let Some(active) = self.again.take() {
            self.sync(active, None);
        }
    }
}

/// Runs its closure once every clone is dropped.
#[derive(Clone)]
struct Countdown {
    _last: Rc<Once>,
}

impl Countdown {
    fn new(then: impl FnOnce() + 'static) -> Self {
        Countdown { _last: Rc::new(Once(RefCell::new(Some(Box::new(move |_| then()))))) }
    }
}

fn manager() -> Option<CookieManager> {
    request_context_get_global_context()?.cookie_manager(None)
}

/// An empty jar reads as empty, which `Bridge::jars` treats as possibly unread.
fn visit_chromium(delete: HashSet<Key>, done: impl FnOnce(Option<Vec<rules::Cookie>>) + 'static) {
    jar::visit(manager(), delete, done);
}

fn read_chromium(done: impl FnOnce(Option<Vec<rules::Cookie>>) + 'static) {
    visit_chromium(HashSet::new(), done);
}

wrap_set_cookie_callback! {
    struct SetDone {
        countdown: Countdown,
    }

    impl SetCookieCallback {
        fn on_complete(&self, _success: i32) {}
    }
}

/// Learns once whether a deletion finished; a callback CEF releases without calling counts as failed.
struct Once(Then<bool>);

impl Once {
    fn call(&self, ok: bool) {
        if let Some(then) = self.0.borrow_mut().take() {
            then(ok);
        }
    }
}

impl Drop for Once {
    fn drop(&mut self) {
        self.call(false);
    }
}

wrap_delete_cookies_callback! {
    struct DeleteDone {
        then: Rc<Once>,
    }

    impl DeleteCookiesCallback {
        fn on_complete(&self, _num_deleted: i32) {
            self.then.call(true);
        }
    }
}

fn write_chromium<'a>(ops: impl Iterator<Item = &'a Op>, countdown: Countdown) {
    let Some(manager) = manager() else { return };
    let mut delete = HashSet::new();
    for op in ops {
        match op {
            Op::Set(cookie) => {
                let mut done = SetDone::new(countdown.clone());
                manager.set_cookie(Some(&jar::url(cookie)), Some(&jar::to_cef(cookie)), Some(&mut done));
            }
            Op::Delete(cookie) => {
                delete.insert(cookie.key());
            }
        }
    }
    if !delete.is_empty() {
        visit_chromium(delete, move |_| drop(countdown));
    }
}

/// Empties Chromium's jar when its context is created; `then` learns whether that worked.
pub(crate) fn empty_chromium(then: impl FnOnce(bool) + 'static) {
    let then = Rc::new(Once(RefCell::new(Some(Box::new(then)))));
    let mut done = DeleteDone::new(then.clone());
    if !manager().is_some_and(|m| m.delete_cookies(None, None, Some(&mut done)) == 1) {
        then.call(false);
    }
}

/// On quit: Chromium's changes are merged into WebKit against WebKit's last read jar, without
/// waiting for WebKit to confirm, then Chromium's jar is emptied. The returned flag turns true
/// once Chromium answered the deletion.
pub(crate) fn finish() -> Rc<Cell<bool>> {
    let settled = Rc::new(Cell::new(false));
    let flag = settled.clone();
    let empty = move || empty_chromium(move |_| flag.set(true));
    let Some(bridge) = bridge().filter(|b| !b.sites.borrow().is_empty()) else {
        empty();
        return settled;
    };
    bridge.suspended.set(true);
    read_chromium(move |chromium| {
        if let (Some(chromium), Some(webkit)) = (chromium, bridge.last_webkit.take()) {
            let jars = bridge.jars(webkit, chromium);
            let base = bridge.base.borrow();
            let fresh = bridge.fresh.borrow();
            for site in bridge.sites.borrow().keys().filter(|s| !bridge.over_cap.borrow().contains(*s) && !fresh.contains(*s)) {
                for op in rules::merge(site, &base, &jars[0], &jars[1], Side::Chromium, now()).map(|p| p.webkit).unwrap_or_default() {
                    match op {
                        Op::Set(cookie) => bridge.webkit.set(&cookie, |_| {}),
                        Op::Delete(cookie) => bridge.webkit.delete(&cookie, || {}),
                    }
                }
            }
        }
        empty();
    });
    settled
}
