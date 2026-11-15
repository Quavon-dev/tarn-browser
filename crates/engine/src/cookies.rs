//! The cookie bridge's rules: which cookies belong to a site, and the
//! three-way merge between WebKit, Chromium and what the bridge last confirmed on both sides.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::net::IpAddr;

pub const MAX_COOKIES_PER_SITE: usize = 100;
pub const MAX_BYTES_PER_SITE: usize = 64 * 1024;
pub const SCRIPT_COOKIE_LIFETIME: i64 = 7 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SameSite {
    None,
    Lax,
    Strict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    WebKit,
    Chromium,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    /// Lowercase, without a leading dot.
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: SameSite,
    /// Unix seconds; `None` for a session cookie.
    pub expires: Option<i64>,
    pub partitioned: bool,
}

impl fmt::Debug for Cookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cookie")
            .field("name", &self.name)
            .field("domain", &self.domain)
            .field("host_only", &self.host_only)
            .field("path", &self.path)
            .field("secure", &self.secure)
            .field("http_only", &self.http_only)
            .field("same_site", &self.same_site)
            .field("expires", &self.expires)
            .field("partitioned", &self.partitioned)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub name: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
}

impl Cookie {
    pub fn key(&self) -> Key {
        Key { name: self.name.clone(), domain: self.domain.clone(), host_only: self.host_only, path: self.path.clone() }
    }

    pub fn site(&self) -> String {
        site(&self.domain)
    }

    fn same_content(&self, other: &Cookie) -> bool {
        (&self.value, self.secure, self.http_only, self.same_site, self.partitioned)
            == (&other.value, other.secure, other.http_only, other.same_site, other.partitioned)
    }

    pub(crate) fn bytes(&self) -> usize {
        self.name.len() + self.value.len()
    }

    fn prefixed(&self, prefix: &str) -> bool {
        self.name.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix))
    }

    /// Whether this cookie may be written for `site` at all: the `__Secure-` and `__Host-`
    /// prefixes, no `Domain` cookie on a public suffix or an IP address, nothing outside the site.
    pub(crate) fn valid_for(&self, site_key: &str) -> bool {
        let prefixes = (!self.prefixed("__Secure-") || self.secure) && (!self.prefixed("__Host-") || (self.secure && self.host_only && self.path == "/"));
        let domain_ok = self.host_only || (!is_ip(&self.domain) && psl::suffix_str(&self.domain) != Some(self.domain.as_str()));
        prefixes && domain_ok && !self.partitioned && !self.domain.is_empty() && self.path.starts_with('/') && site(&self.domain) == site_key
    }

    /// Whether this cookie and `other` could both be sent on one request: same name, one domain
    /// inside the other's reach, one path inside the other.
    pub(crate) fn overlaps(&self, other: &Cookie) -> bool {
        fn reaches(outer: &Cookie, inner: &Cookie) -> bool {
            inner.domain == outer.domain || (!outer.host_only && inner.domain.ends_with(&format!(".{}", outer.domain)))
        }
        fn within(outer: &str, inner: &str) -> bool {
            inner == outer || (inner.starts_with(outer) && (outer.ends_with('/') || inner[outer.len()..].starts_with('/')))
        }
        self.name == other.name && (reaches(self, other) || reaches(other, self)) && (within(&self.path, &other.path) || within(&other.path, &self.path))
    }

    /// True if writing `self` would take a protection away from a cookie already in the target.
    pub(crate) fn downgrades(&self, existing: &Cookie) -> bool {
        self.overlaps(existing) && ((existing.http_only && !self.http_only) || (existing.secure && !self.secure))
    }
}

fn is_ip(host: &str) -> bool {
    host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>().is_ok()
}

/// The bridge's unit: a host's registrable domain under the public-suffix list (its private
/// section included), or the host itself for an IP address, a single label or a public suffix.
pub fn site(host: &str) -> String {
    let host = host.trim_start_matches('.').trim_end_matches('.').to_ascii_lowercase();
    if is_ip(&host) || !host.contains('.') {
        return host;
    }
    psl::domain_str(&host).map_or_else(|| host.clone(), str::to_owned)
}

/// Chromium evicts down to 3000 cookies once a profile holds more than 3300.
pub const JAR_FULL: usize = 3000;

pub struct Jar {
    pub cookies: Vec<Cookie>,
    sites: Vec<String>,
    /// A missing cookie may have been evicted, so it is not a deletion.
    pub full: bool,
}

impl Jar {
    pub fn new(cookies: Vec<Cookie>) -> Self {
        let full = cookies.len() >= JAR_FULL;
        let sites = cookies.iter().map(Cookie::site).collect();
        Jar { cookies, sites, full }
    }

    fn of_site<'a>(&'a self, site_key: &'a str) -> impl Iterator<Item = &'a Cookie> + 'a {
        self.cookies.iter().zip(&self.sites).filter(move |(_, s)| *s == site_key).map(|(c, _)| c)
    }

    fn get<'a>(&'a self, site_key: &'a str, key: &Key) -> Option<&'a Cookie> {
        self.of_site(site_key).find(|c| c.key() == *key)
    }

    fn over_cap(&self, site_key: &str) -> bool {
        let (count, bytes) = self.of_site(site_key).fold((0, 0), |(n, b), c| (n + 1, b + c.bytes()));
        count > MAX_COOKIES_PER_SITE || bytes > MAX_BYTES_PER_SITE
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Op {
    Set(Cookie),
    Delete(Cookie),
}

#[derive(Debug, Default)]
pub struct Plan {
    pub webkit: Vec<Op>,
    pub chromium: Vec<Op>,
    /// Cookies that cannot be written faithfully; never written or deleted again this session.
    pub skipped: Vec<Key>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct OverCap;

/// What the bridge last confirmed on both sides, per cookie: WebKit's copy and Chromium's copy.
/// Each side is compared with its own copy, so a cookie written with a capped expiry is not seen
/// as a change on the next merge.
#[derive(Default)]
pub struct Base {
    agreed: HashMap<Key, (Cookie, Cookie)>,
    skipped: HashSet<Key>,
}

enum State<'a> {
    Absent,
    New(&'a Cookie),
    Same,
    Changed(&'a Cookie),
    Deleted,
    /// Gone because it expired or the jar was full: not a deletion.
    Vanished,
}

fn state<'a>(now: Option<&'a Cookie>, base: Option<&Cookie>, full: bool, time: i64) -> State<'a> {
    match (now, base) {
        (Some(c), Some(b)) if c == b => State::Same,
        (Some(c), Some(_)) => State::Changed(c),
        (Some(c), None) => State::New(c),
        (None, Some(b)) if full || b.expires.is_some_and(|e| e <= time) => State::Vanished,
        (None, Some(_)) => State::Deleted,
        (None, None) => State::Absent,
    }
}

impl Base {
    /// Records what both sides hold after a plan was applied: cookies equal on both sides are
    /// agreed, and an agreed cookie missing only from a full jar stays agreed, so it is neither
    /// deleted nor copied back. A cookie the plan set is refused, and skipped, when its target afterwards holds no
    /// cookie of that name, domain and path (unless that jar is full or read empty, which a
    /// written jar cannot be) or still the one it held
    /// before; a third value is the site changing it meanwhile.
    pub fn confirm(&mut self, site_key: &str, plan: &Plan, before: [&Jar; 2], after: [&Jar; 2]) {
        self.skipped.extend(plan.skipped.iter().cloned());
        for (i, ops) in [&plan.webkit, &plan.chromium].into_iter().enumerate() {
            for op in ops {
                let Op::Set(c) = op else { continue };
                let now = after[i].get(site_key, &c.key());
                if (after[i].full || after[i].cookies.is_empty()) && now.is_none() {
                    continue;
                }
                if now.is_none_or(|t| !t.same_content(c) && Some(t) == before[i].get(site_key, &c.key())) {
                    self.skipped.insert(c.key());
                }
            }
        }
        let vanished = |k: &Key| {
            [0, 1].into_iter().any(|i| (before[i].full || after[i].full) && after[i].get(site_key, k).is_none() && after[1 - i].get(site_key, k).is_some())
        };
        self.agreed.retain(|k, _| site(&k.domain) != site_key || vanished(k));
        let chromium: HashMap<Key, &Cookie> = after[1].of_site(site_key).map(|c| (c.key(), c)).collect();
        for w in after[0].of_site(site_key) {
            if let Some(c) = chromium.get(&w.key()).filter(|c| c.same_content(w)) {
                self.agreed.insert(w.key(), (w.clone(), (*c).clone()));
            }
        }
    }

    /// Forgets a site, as when the bridge is suspended for a clear.
    pub fn forget(&mut self, site_key: &str) {
        self.agreed.retain(|k, _| site(&k.domain) != site_key);
        self.skipped.retain(|k| site(&k.domain) != site_key);
    }
}

/// Three-way merge of one site. `active` wins where both sides changed a cookie; `now` is Unix
/// seconds.
pub fn merge(site_key: &str, base: &Base, webkit: &Jar, chromium: &Jar, active: Side, now: i64) -> Result<Plan, OverCap> {
    if webkit.over_cap(site_key) || chromium.over_cap(site_key) {
        return Err(OverCap);
    }
    let w_map: HashMap<Key, &Cookie> = webkit.of_site(site_key).map(|c| (c.key(), c)).collect();
    let c_map: HashMap<Key, &Cookie> = chromium.of_site(site_key).map(|c| (c.key(), c)).collect();
    let keys: BTreeSet<&Key> = w_map.keys().chain(c_map.keys()).chain(base.agreed.keys().filter(|k| site(&k.domain) == site_key)).collect();
    let mut plan = Plan::default();
    for key in keys {
        if base.skipped.contains(key) {
            continue;
        }
        let agreed = base.agreed.get(key);
        let w = state(w_map.get(key).copied(), agreed.map(|a| &a.0), webkit.full, now);
        let c = state(c_map.get(key).copied(), agreed.map(|a| &a.1), chromium.full, now);
        let to_chromium = |plan: &mut Plan, cookie: &Cookie| write(plan, cookie, Side::Chromium, &chromium.cookies, site_key, now);
        let to_webkit = |plan: &mut Plan, cookie: &Cookie| write(plan, cookie, Side::WebKit, &webkit.cookies, site_key, now);
        let delete = |plan: &mut Plan, side: Side| match side {
            Side::WebKit => plan.webkit.extend(w_map.get(key).map(|c| Op::Delete((*c).clone()))),
            Side::Chromium => plan.chromium.extend(c_map.get(key).map(|c| Op::Delete((*c).clone()))),
        };
        use State::*;
        match (w, c) {
            (New(w), Absent) | (Changed(w), Same | Vanished) => to_chromium(&mut plan, w),
            (Absent, New(c)) | (Same | Vanished, Changed(c)) => to_webkit(&mut plan, c),
            (New(w), New(c)) | (Changed(w), Changed(c)) if !w.same_content(c) => match active {
                Side::WebKit => to_chromium(&mut plan, w),
                Side::Chromium => to_webkit(&mut plan, c),
            },
            (Deleted, Same) => delete(&mut plan, Side::Chromium),
            (Deleted, Vanished) => plan.chromium.extend(agreed.map(|a| Op::Delete(a.1.clone()))),
            (Same, Deleted) => delete(&mut plan, Side::WebKit),
            (Changed(w), Deleted) => match active {
                Side::WebKit => to_chromium(&mut plan, w),
                Side::Chromium => delete(&mut plan, Side::WebKit),
            },
            (Deleted, Changed(c)) => match active {
                Side::WebKit => delete(&mut plan, Side::Chromium),
                Side::Chromium => to_webkit(&mut plan, c),
            },
            _ => {}
        }
    }
    Ok(plan)
}

/// A site's first merge in a session. WebKit's jar is the one at rest, so its cookies replace
/// whatever Chromium collected for the site before it was bridged (redirects, script navigations,
/// cross-site posts), which is deleted rather than copied into WebKit.
pub fn first_bridge(site_key: &str, webkit: &Jar, chromium: &Jar, now: i64) -> Result<Plan, OverCap> {
    if webkit.over_cap(site_key) {
        return Err(OverCap);
    }
    let mut plan = Plan::default();
    for cookie in webkit.of_site(site_key) {
        write(&mut plan, cookie, Side::Chromium, &[], site_key, now);
    }
    let set: HashSet<Key> = plan.chromium.iter().filter_map(|op| if let Op::Set(c) = op { Some(c.key()) } else { None }).collect();
    plan.chromium.extend(chromium.of_site(site_key).filter(|c| !set.contains(&c.key())).map(|c| Op::Delete(c.clone())));
    Ok(plan)
}

/// WebKit cannot hold an explicit `SameSite=None` (it reads back as unspecified, which the bridge
/// reads as `Lax`), and Chromium refuses `None` without `Secure`.
fn write(plan: &mut Plan, cookie: &Cookie, target: Side, existing: &[Cookie], site_key: &str, now: i64) {
    let same_site_ok = cookie.same_site != SameSite::None || (target == Side::Chromium && cookie.secure);
    let faithful = cookie.valid_for(site_key) && same_site_ok;
    if !faithful {
        plan.skipped.push(cookie.key());
        return;
    }
    if existing.iter().any(|e| cookie.downgrades(e)) {
        return;
    }
    let mut cookie = cookie.clone();
    if target == Side::WebKit && !cookie.http_only {
        cookie.expires = cookie.expires.map(|e| e.min(now + SCRIPT_COOKIE_LIFETIME));
    }
    match target {
        Side::WebKit => plan.webkit.push(Op::Set(cookie)),
        Side::Chromium => plan.chromium.push(Op::Set(cookie)),
    }
}

pub const IMPORTED_COOKIE_LIFETIME: i64 = 400 * 24 * 60 * 60;

/// The cookies from another browser a jar holding `existing` may take, with lifetimes capped;
/// the rest are counted as refused: expired at `now` (Unix seconds), not valid for their own site,
/// one that could be sent alongside a cookie the jar holds (Tarn's own sign-in wins), or past the
/// bridge's per-site caps. Script-readable cookies keep at most the bridge's lifetime, others
/// Chromium's 400 days.
pub fn admit_imported(cookies: Vec<Cookie>, existing: &[Cookie], now: i64) -> (Vec<Cookie>, usize) {
    let mut sites: HashMap<String, (usize, usize)> = HashMap::new();
    for e in existing {
        let site = sites.entry(e.site()).or_default();
        *site = (site.0 + 1, site.1 + e.bytes());
    }
    let mut by_name: HashMap<&str, Vec<&Cookie>> = HashMap::new();
    for e in existing {
        by_name.entry(e.name.as_str()).or_default().push(e);
    }
    let total = cookies.len();
    let mut admitted: Vec<Cookie> = cookies
        .into_iter()
        .filter(|c| {
            let fits = c.expires.is_none_or(|e| e > now)
                && c.valid_for(&c.site())
                && !by_name.get(c.name.as_str()).is_some_and(|same| same.iter().any(|e| c.overlaps(e)));
            let site = sites.entry(c.site()).or_default();
            let room = site.0 < MAX_COOKIES_PER_SITE && site.1 + c.bytes() <= MAX_BYTES_PER_SITE;
            if fits && room {
                *site = (site.0 + 1, site.1 + c.bytes());
            }
            fits && room
        })
        .collect();
    for c in &mut admitted {
        let lifetime = if c.http_only { IMPORTED_COOKIE_LIFETIME } else { SCRIPT_COOKIE_LIFETIME };
        c.expires = c.expires.map(|e| e.min(now + lifetime));
    }
    let refused = total - admitted.len();
    (admitted, refused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Not;

    const NOW: i64 = 1_800_000_000;
    const SITE: &str = "example.com";

    fn cookie(name: &str, value: &str) -> Cookie {
        Cookie {
            name: name.into(),
            value: value.into(),
            domain: "example.com".into(),
            host_only: true,
            path: "/".into(),
            secure: true,
            http_only: false,
            same_site: SameSite::Lax,
            expires: Some(NOW + 3600),
            partitioned: false,
        }
    }

    fn session(value: &str) -> Cookie {
        Cookie { http_only: true, ..cookie("session", value) }
    }

    fn jar(cookies: &[Cookie]) -> Jar {
        Jar { full: false, ..Jar::new(cookies.to_vec()) }
    }

    fn run(base: &Base, webkit: &[Cookie], chromium: &[Cookie], active: Side) -> Plan {
        merge(SITE, base, &jar(webkit), &jar(chromium), active, NOW).unwrap()
    }

    fn agreed(cookies: &[Cookie]) -> Base {
        let mut base = Base::default();
        confirm(&mut base, &Plan::default(), &[], &[], cookies, cookies);
        base
    }

    fn confirm(base: &mut Base, plan: &Plan, w0: &[Cookie], c0: &[Cookie], w1: &[Cookie], c1: &[Cookie]) {
        base.confirm(SITE, plan, [&jar(w0), &jar(c0)], [&jar(w1), &jar(c1)]);
    }

    fn set(c: &Cookie) -> Op {
        Op::Set(c.clone())
    }

    #[test]
    fn site_is_registrable_domain_with_private_suffixes() {
        assert_eq!(site("www.example.com"), "example.com");
        assert_eq!(site(".Example.COM."), "example.com");
        assert_eq!(site("a.b.example.co.uk"), "example.co.uk");
        assert_eq!(site("alice.github.io"), "alice.github.io");
        assert_eq!(site("bob.github.io"), "bob.github.io");
        assert_eq!(site("github.io"), "github.io");
        assert_eq!(site("localhost"), "localhost");
        assert_eq!(site("127.0.0.1"), "127.0.0.1");
        assert_eq!(site("[::1]"), "[::1]");
        assert_ne!(site("10.0.0.1"), site("10.0.0.2"));
    }

    #[test]
    fn debug_never_prints_the_value() {
        let text = format!("{:?} {:?}", session("s3cr3t-value"), Op::Set(session("s3cr3t-value")));
        assert!(!text.contains("s3cr3t-value"));
        assert!(text.contains("session"));
    }

    #[test]
    fn new_cookies_copy_both_ways() {
        let (a, b) = (cookie("a", "1"), cookie("b", "2"));
        let plan = run(&Base::default(), std::slice::from_ref(&a), std::slice::from_ref(&b), Side::WebKit);
        assert_eq!(plan.chromium, vec![set(&a)]);
        assert_eq!(plan.webkit, vec![set(&b)]);
    }

    #[test]
    fn other_sites_are_ignored() {
        let other = Cookie { domain: "example.org".into(), ..cookie("a", "1") };
        let sub = Cookie { domain: "www.example.com".into(), ..cookie("b", "1") };
        let plan = run(&Base::default(), &[other, sub.clone()], &[], Side::WebKit);
        assert_eq!(plan.chromium, vec![set(&sub)]);
    }

    #[test]
    fn unchanged_cookies_produce_nothing() {
        let a = cookie("a", "1");
        let plan = run(&agreed(std::slice::from_ref(&a)), std::slice::from_ref(&a), std::slice::from_ref(&a), Side::Chromium);
        assert!(plan.webkit.is_empty() && plan.chromium.is_empty());
    }

    #[test]
    fn a_change_on_one_side_wins() {
        let (old, new) = (cookie("a", "1"), cookie("a", "2"));
        let base = agreed(std::slice::from_ref(&old));
        let plan = run(&base, std::slice::from_ref(&new), std::slice::from_ref(&old), Side::Chromium);
        assert_eq!((plan.chromium, plan.webkit), (vec![set(&new)], vec![]));
        let plan = run(&base, std::slice::from_ref(&old), std::slice::from_ref(&new), Side::WebKit);
        assert_eq!((plan.webkit, plan.chromium), (vec![set(&new)], vec![]));
    }

    #[test]
    fn a_change_on_both_sides_goes_to_the_active_engine() {
        let (old, w, c) = (cookie("a", "1"), cookie("a", "w"), cookie("a", "c"));
        let base = agreed(std::slice::from_ref(&old));
        let plan = run(&base, std::slice::from_ref(&w), std::slice::from_ref(&c), Side::WebKit);
        assert_eq!((plan.chromium, plan.webkit), (vec![set(&w)], vec![]));
        let plan = run(&base, std::slice::from_ref(&w), std::slice::from_ref(&c), Side::Chromium);
        assert_eq!((plan.webkit, plan.chromium), (vec![set(&c)], vec![]));
        let plan = run(&Base::default(), std::slice::from_ref(&w), std::slice::from_ref(&c), Side::Chromium);
        assert_eq!(plan.webkit, vec![set(&c)]);
    }

    #[test]
    fn the_same_change_on_both_sides_produces_nothing() {
        let (old, new) = (cookie("a", "1"), cookie("a", "2"));
        let plan = run(&agreed(&[old]), std::slice::from_ref(&new), std::slice::from_ref(&new), Side::WebKit);
        assert!(plan.webkit.is_empty() && plan.chromium.is_empty());
    }

    #[test]
    fn a_deletion_in_base_propagates() {
        let s = session("v");
        let base = agreed(std::slice::from_ref(&s));
        let plan = run(&base, std::slice::from_ref(&s), &[], Side::WebKit);
        assert_eq!(plan.webkit, vec![Op::Delete(s.clone())]);
        let plan = run(&base, &[], std::slice::from_ref(&s), Side::Chromium);
        assert_eq!(plan.chromium, vec![Op::Delete(s)]);
    }

    #[test]
    fn a_cookie_never_confirmed_is_not_deleted() {
        let s = session("v");
        let plan = run(&Base::default(), std::slice::from_ref(&s), &[], Side::Chromium);
        assert_eq!((plan.chromium, plan.webkit), (vec![set(&s)], vec![]));
    }

    #[test]
    fn an_expiry_is_not_a_deletion() {
        let mut short = session("v");
        short.expires = Some(NOW - 1);
        let long = Cookie { expires: Some(NOW + 3600), ..short.clone() };
        let mut base = Base::default();
        base.agreed.insert(short.key(), (long.clone(), short));
        let plan = run(&base, std::slice::from_ref(&long), &[], Side::Chromium);
        assert!(plan.webkit.is_empty() && plan.chromium.is_empty());
    }

    #[test]
    fn a_jar_at_the_eviction_level_is_full() {
        let many: Vec<Cookie> = (0..JAR_FULL).map(|i| Cookie { domain: format!("s{i}.example.org"), ..cookie("a", "1") }).collect();
        assert!(Jar::new(many[..JAR_FULL - 1].to_vec()).full.not());
        assert!(Jar::new(many).full);
    }

    #[test]
    fn a_cookie_missing_from_a_full_jar_never_comes_back_and_a_later_deletion_still_counts() {
        let s = session("v");
        let mut base = agreed(std::slice::from_ref(&s));
        let webkit = jar(std::slice::from_ref(&s));
        let full = Jar { full: true, ..Jar::new(Vec::new()) };
        for _ in 0..2 {
            let plan = merge(SITE, &base, &webkit, &full, Side::Chromium, NOW).unwrap();
            assert!(plan.webkit.is_empty() && plan.chromium.is_empty());
            base.confirm(SITE, &plan, [&webkit, &full], [&webkit, &full]);
        }
        let plan = merge(SITE, &base, &webkit, &jar(&[]), Side::Chromium, NOW).unwrap();
        assert_eq!(plan.webkit, vec![Op::Delete(s)]);
    }

    #[test]
    fn a_webkit_logout_during_an_unsure_chromium_read_still_deletes_there() {
        let s = session("v");
        let base = agreed(std::slice::from_ref(&s));
        let unsure = Jar { full: true, ..Jar::new(Vec::new()) };
        let plan = merge(SITE, &base, &jar(&[]), &unsure, Side::WebKit, NOW).unwrap();
        assert_eq!(plan.chromium, vec![Op::Delete(s)]);
    }

    #[test]
    fn a_jar_full_before_a_write_keeps_agreement_after_it() {
        let s = session("v");
        let other = cookie("other", "1");
        let mut base = agreed(std::slice::from_ref(&s));
        let webkit = jar(&[s.clone(), other.clone()]);
        let full = Jar { full: true, ..Jar::new(Vec::new()) };
        let plan = merge(SITE, &base, &webkit, &full, Side::Chromium, NOW).unwrap();
        assert_eq!(plan.chromium, vec![set(&other)]);
        base.confirm(SITE, &plan, [&webkit, &full], [&webkit, &jar(std::slice::from_ref(&other))]);
        let plan = merge(SITE, &base, &webkit, &jar(std::slice::from_ref(&other)), Side::Chromium, NOW).unwrap();
        assert_eq!(plan.webkit, vec![Op::Delete(s)]);
        assert!(plan.chromium.is_empty());
    }

    #[test]
    fn an_eviction_from_a_full_jar_is_not_a_deletion() {
        let s = session("v");
        let base = agreed(std::slice::from_ref(&s));
        let full = Jar { full: true, ..Jar::new(vec![Cookie { domain: "example.org".into(), ..cookie("other", "1") }]) };
        let plan = merge(SITE, &base, &jar(std::slice::from_ref(&s)), &full, Side::Chromium, NOW).unwrap();
        assert!(plan.webkit.is_empty() && plan.chromium.is_empty());
    }

    #[test]
    fn a_refused_write_is_skipped_and_not_deleted() {
        let s = session("v");
        let mut base = Base::default();
        let plan = run(&base, std::slice::from_ref(&s), &[], Side::WebKit);
        assert_eq!(plan.chromium, vec![set(&s)]);
        let other = Cookie { domain: "example.org".into(), ..cookie("x", "1") };
        confirm(&mut base, &plan, std::slice::from_ref(&s), &[], std::slice::from_ref(&s), std::slice::from_ref(&other));
        assert!(base.skipped.contains(&s.key()));
        let mut unread = Base::default();
        let full = Jar { full: true, ..Jar::new(Vec::new()) };
        unread.confirm(SITE, &plan, [&jar(std::slice::from_ref(&s)), &jar(&[])], [&jar(std::slice::from_ref(&s)), &full]);
        assert!(!unread.skipped.contains(&s.key()));
        unread.confirm(SITE, &plan, [&jar(std::slice::from_ref(&s)), &jar(&[])], [&jar(std::slice::from_ref(&s)), &jar(&[])]);
        assert!(!unread.skipped.contains(&s.key()));
        unread.confirm(SITE, &plan, [&jar(std::slice::from_ref(&s)), &jar(&[])], [&jar(std::slice::from_ref(&s)), &jar(std::slice::from_ref(&other))]);
        assert!(unread.skipped.contains(&s.key()));
        let plan = run(&base, std::slice::from_ref(&s), &[], Side::Chromium);
        assert!(plan.webkit.is_empty() && plan.chromium.is_empty());
    }

    #[test]
    fn a_confirmed_write_enters_base_and_a_later_deletion_propagates() {
        let s = session("v");
        let mut base = Base::default();
        let plan = run(&base, std::slice::from_ref(&s), &[], Side::WebKit);
        confirm(&mut base, &plan, std::slice::from_ref(&s), &[], std::slice::from_ref(&s), std::slice::from_ref(&s));
        assert!(!base.skipped.contains(&s.key()));
        let plan = run(&base, std::slice::from_ref(&s), &[], Side::Chromium);
        assert_eq!(plan.webkit, vec![Op::Delete(s)]);
    }

    #[test]
    fn http_only_is_never_replaced_by_a_script_cookie() {
        let real = session("real");
        let evil = Cookie { http_only: false, ..session("evil") };
        let base = agreed(std::slice::from_ref(&real));
        let plan = run(&base, std::slice::from_ref(&real), std::slice::from_ref(&evil), Side::Chromium);
        assert!(plan.webkit.is_empty() && plan.chromium.is_empty() && plan.skipped.is_empty());
        let shadow = Cookie { host_only: false, path: "/app".into(), ..evil.clone() };
        let plan = run(&base, std::slice::from_ref(&real), &[real.clone(), shadow], Side::Chromium);
        assert!(plan.webkit.is_empty());
    }

    #[test]
    fn secure_is_never_replaced_by_an_insecure_cookie() {
        let real = session("real");
        let plain = Cookie { secure: false, ..session("evil") };
        let plan = run(&agreed(std::slice::from_ref(&real)), std::slice::from_ref(&real), std::slice::from_ref(&plain), Side::Chromium);
        assert!(plan.webkit.is_empty());
        let plan = run(&agreed(std::slice::from_ref(&real)), std::slice::from_ref(&plain), std::slice::from_ref(&real), Side::WebKit);
        assert!(plan.chromium.is_empty());
    }

    #[test]
    fn host_prefix_is_never_replaced_by_a_domain_cookie() {
        let host = Cookie { http_only: true, ..cookie("__Host-id", "real") };
        let domain = Cookie { host_only: false, ..cookie("__Host-id", "evil") };
        let plan = run(&agreed(std::slice::from_ref(&host)), std::slice::from_ref(&host), &[host.clone(), domain.clone()], Side::Chromium);
        assert!(plan.webkit.is_empty());
        assert_eq!(plan.skipped, vec![domain.key()]);
    }

    #[test]
    fn a_stronger_cookie_may_replace_a_weaker_one() {
        let weak = Cookie { http_only: false, ..session("old") };
        let strong = session("new");
        let plan = run(&agreed(std::slice::from_ref(&weak)), std::slice::from_ref(&strong), std::slice::from_ref(&weak), Side::Chromium);
        assert_eq!(plan.chromium, vec![set(&strong)]);
    }

    #[test]
    fn prefixes_and_public_suffixes_are_checked_before_writing() {
        let bad = [
            Cookie { secure: false, ..cookie("__Secure-a", "1") },
            Cookie { secure: false, ..cookie("__host-b", "1") },
            Cookie { path: "/x".into(), ..cookie("__Host-c", "1") },
            Cookie { host_only: false, domain: "co.uk".into(), ..cookie("d", "1") },
            Cookie { host_only: false, domain: "github.io".into(), ..cookie("e", "1") },
            Cookie { host_only: false, domain: "10.0.0.1".into(), ..cookie("f", "1") },
            Cookie { partitioned: true, ..cookie("g", "1") },
            Cookie { path: "".into(), ..cookie("h", "1") },
        ];
        for c in bad {
            let s = c.site();
            let plan = merge(&s, &Base::default(), &jar(std::slice::from_ref(&c)), &jar(&[]), Side::WebKit, NOW).unwrap();
            assert!(plan.chromium.is_empty(), "{c:?}");
            assert_eq!(plan.skipped, vec![c.key()], "{c:?}");
        }
        let ok = Cookie { host_only: false, ..cookie("__Secure-z", "1") };
        assert_eq!(run(&Base::default(), std::slice::from_ref(&ok), &[], Side::WebKit).chromium, vec![set(&ok)]);
    }

    #[test]
    fn unfaithful_cookies_are_skipped_never_weakened() {
        let none_plain = Cookie { same_site: SameSite::None, secure: false, ..cookie("a", "1") };
        let plan = run(&Base::default(), std::slice::from_ref(&none_plain), &[], Side::WebKit);
        assert!(plan.chromium.is_empty());
        assert_eq!(plan.skipped, vec![none_plain.key()]);
        let none_secure = Cookie { same_site: SameSite::None, ..cookie("b", "1") };
        let plan = run(&Base::default(), &[], std::slice::from_ref(&none_secure), Side::Chromium);
        assert!(plan.webkit.is_empty());
        assert_eq!(plan.skipped, vec![none_secure.key()]);
        let plan = run(&Base::default(), std::slice::from_ref(&none_secure), &[], Side::WebKit);
        assert_eq!(plan.chromium, vec![set(&none_secure)]);
    }

    #[test]
    fn script_cookies_from_chromium_are_capped_at_seven_days() {
        let long = Cookie { expires: Some(NOW + 400 * 86_400), ..cookie("a", "1") };
        let plan = run(&Base::default(), &[], std::slice::from_ref(&long), Side::Chromium);
        assert_eq!(plan.webkit, vec![Op::Set(Cookie { expires: Some(NOW + SCRIPT_COOKIE_LIFETIME), ..long.clone() })]);
        let http_only = Cookie { http_only: true, ..long.clone() };
        assert_eq!(run(&Base::default(), &[], std::slice::from_ref(&http_only), Side::Chromium).webkit, vec![set(&http_only)]);
        assert_eq!(run(&Base::default(), std::slice::from_ref(&long), &[], Side::WebKit).chromium, vec![set(&long)]);
    }

    #[test]
    fn a_capped_expiry_does_not_loop() {
        let long = Cookie { expires: Some(NOW + 400 * 86_400), ..cookie("a", "1") };
        let mut base = Base::default();
        let plan = run(&base, &[], std::slice::from_ref(&long), Side::Chromium);
        let Op::Set(capped) = &plan.webkit[0] else { panic!() };
        let webkit = vec![capped.clone()];
        confirm(&mut base, &plan, &[], std::slice::from_ref(&long), &webkit, std::slice::from_ref(&long));
        let plan = run(&base, &webkit, std::slice::from_ref(&long), Side::Chromium);
        assert!(plan.webkit.is_empty() && plan.chromium.is_empty());
    }

    #[test]
    fn deletion_against_a_change_goes_to_the_active_engine() {
        let (old, new) = (cookie("a", "1"), cookie("a", "2"));
        let base = agreed(std::slice::from_ref(&old));
        let plan = run(&base, std::slice::from_ref(&new), &[], Side::WebKit);
        assert_eq!((plan.chromium, plan.webkit), (vec![set(&new)], vec![]));
        let plan = run(&base, std::slice::from_ref(&new), &[], Side::Chromium);
        assert_eq!((plan.webkit, plan.chromium), (vec![Op::Delete(new.clone())], vec![]));
        let plan = run(&base, &[], std::slice::from_ref(&new), Side::WebKit);
        assert_eq!((plan.chromium, plan.webkit), (vec![Op::Delete(new.clone())], vec![]));
        let plan = run(&base, &[], std::slice::from_ref(&new), Side::Chromium);
        assert_eq!((plan.webkit, plan.chromium), (vec![set(&new)], vec![]));
    }

    #[test]
    fn a_site_over_the_caps_stops_syncing() {
        let many: Vec<Cookie> = (0..=MAX_COOKIES_PER_SITE).map(|i| cookie(&format!("c{i}"), "v")).collect();
        assert_eq!(merge(SITE, &Base::default(), &jar(&[]), &jar(&many), Side::Chromium, NOW).unwrap_err(), OverCap);
        let big = vec![cookie("a", &"x".repeat(MAX_BYTES_PER_SITE))];
        assert_eq!(merge(SITE, &Base::default(), &jar(&big), &jar(&[]), Side::WebKit, NOW).unwrap_err(), OverCap);
        let at_cap: Vec<Cookie> = many[..MAX_COOKIES_PER_SITE].to_vec();
        assert!(merge(SITE, &Base::default(), &jar(&[]), &jar(&at_cap), Side::Chromium, NOW).is_ok());
        let other = vec![Cookie { domain: "example.org".into(), ..cookie("a", &"x".repeat(MAX_BYTES_PER_SITE)) }];
        assert!(merge(SITE, &Base::default(), &jar(&other), &jar(&[]), Side::WebKit, NOW).is_ok());
    }

    #[test]
    fn host_only_and_domain_cookies_are_different_cookies() {
        let host = cookie("a", "1");
        let domain = Cookie { host_only: false, ..cookie("a", "2") };
        let plan = run(&agreed(std::slice::from_ref(&host)), &[host.clone(), domain.clone()], std::slice::from_ref(&host), Side::WebKit);
        assert_eq!(plan.chromium, vec![set(&domain)]);
    }

    #[test]
    fn a_write_overtaken_by_the_site_is_not_refused() {
        let (old, new, rotated) = (session("old"), session("new"), session("rotated"));
        let base = agreed(std::slice::from_ref(&old));
        let plan = run(&base, std::slice::from_ref(&new), std::slice::from_ref(&old), Side::WebKit);
        let mut overtaken = agreed(std::slice::from_ref(&old));
        confirm(&mut overtaken, &plan, std::slice::from_ref(&new), std::slice::from_ref(&old), std::slice::from_ref(&new), std::slice::from_ref(&rotated));
        assert!(!overtaken.skipped.contains(&old.key()));
        let mut refused = agreed(std::slice::from_ref(&old));
        confirm(&mut refused, &plan, std::slice::from_ref(&new), std::slice::from_ref(&old), std::slice::from_ref(&new), std::slice::from_ref(&old));
        assert!(refused.skipped.contains(&old.key()));
    }

    #[test]
    fn a_first_bridge_replaces_what_chromium_collected_before() {
        let login = session("real");
        let planted = Cookie { http_only: true, ..cookie("csrf-login", "attacker") };
        let stale = session("stale");
        let plan = first_bridge(SITE, &jar(std::slice::from_ref(&login)), &jar(&[stale, planted.clone()]), NOW).unwrap();
        assert!(plan.webkit.is_empty());
        assert_eq!(plan.chromium, vec![set(&login), Op::Delete(planted)]);
        let many: Vec<Cookie> = (0..=MAX_COOKIES_PER_SITE).map(|i| cookie(&format!("c{i}"), "v")).collect();
        assert_eq!(first_bridge(SITE, &jar(&many), &jar(&[]), NOW).unwrap_err(), OverCap);
    }

    #[test]
    fn forget_drops_a_site_from_base() {
        let s = session("v");
        let mut base = agreed(std::slice::from_ref(&s));
        base.skipped.insert(cookie("x", "1").key());
        base.forget(SITE);
        assert!(!base.skipped.contains(&cookie("x", "1").key()));
        let plan = run(&base, std::slice::from_ref(&s), &[], Side::Chromium);
        assert_eq!(plan.chromium, vec![set(&s)]);
    }

    #[test]
    fn an_import_keeps_the_jar_s_cookies_the_caps_and_short_lifetimes() {
        let held = session("tarn");
        let on = |domain: &str, c: Cookie| Cookie { domain: domain.into(), host_only: false, ..c };
        let refused = [
            Cookie { expires: Some(NOW), ..cookie("old", "v") },
            on("co.uk", cookie("suffix", "v")),
            on("127.0.0.1", cookie("ip", "v")),
            session("other account"),
            Cookie { path: "/account".into(), ..session("shadow") },
            Cookie { path: "/a".into(), ..cookie("__Host-x", "v") },
        ];
        let long = Cookie { expires: Some(NOW + 1000 * 86_400), ..cookie("pref", "v") };
        let signed_in = Cookie { name: "sid".into(), ..Cookie { expires: Some(NOW + 1000 * 86_400), ..session("v") } };
        let fresh = Cookie { expires: None, ..cookie("tmp", "v") };
        let input = [refused.to_vec(), vec![long, signed_in, fresh.clone()]].concat();
        let (admitted, skipped) = admit_imported(input, std::slice::from_ref(&held), NOW);
        assert_eq!(skipped, refused.len());
        assert_eq!(admitted.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["pref", "sid", "tmp"]);
        assert_eq!(admitted[0].expires, Some(NOW + SCRIPT_COOKIE_LIFETIME), "a script-readable cookie gets the bridge's lifetime");
        assert_eq!(admitted[1].expires, Some(NOW + IMPORTED_COOKIE_LIFETIME), "an HttpOnly cookie gets Chromium's 400 days");
        assert_eq!(admitted[2], fresh, "a session cookie stays one");

        let many: Vec<Cookie> = (0..MAX_COOKIES_PER_SITE + 5).map(|i| cookie(&format!("c{i}"), "v")).collect();
        let (admitted, skipped) = admit_imported(many, std::slice::from_ref(&held), NOW);
        assert_eq!((admitted.len(), skipped), (MAX_COOKIES_PER_SITE - 1, 6), "the jar's own cookie counts toward the cap");
        let big = vec![cookie("a", &"x".repeat(MAX_BYTES_PER_SITE - 1)), cookie("b", "v")];
        assert_eq!(admit_imported(big, &[], NOW).1, 1, "a site stays within the byte cap");
    }
}
