use std::time::{Duration, Instant};

use engine::content_rules::{self, ResourceKind, RuleList, Verdict};
use engine::tarn_ext::{FrameId, FrameNav, PageKind};
use serde_json::{Value, json};
use shield::ShieldLevel;
use shield::dnr::Scope;
use store::{InstalledExtension, Store};
use url::Url;

use super::grants::{Answer, Install};
use super::router::{Env, From, MAX_DELIVERIES, MAX_QUEUED, MAX_RATE, Out, QUOTA_BYTES, REPLY_TIMEOUT, Router, Sender, TabInfo, Target};
use super::*;

const ID: &str = "0a1b2c";

fn package(extra: Value) -> (tempfile::TempDir, Package) {
    let dir = tempfile::tempdir().unwrap();
    let mut manifest = json!({
        "manifest_version": 3, "name": "__MSG_name__", "version": "1.0", "default_locale": "en",
        "browser_specific_settings": {"tarn": {"runtime": 1}},
        "permissions": ["storage", "declarativeNetRequest"],
        "host_permissions": ["*://*.news.test/*"],
        "content_scripts": [
            {"matches": ["*://*.shop.test/*"], "js": ["c.js"], "css": ["c.css"], "run_at": "document_start"},
            {"matches": ["*://*.shop.test/*"], "exclude_matches": ["*://shop.test/private/*"], "js": ["f.js"], "all_frames": true}
        ],
        "declarative_net_request": {"rule_resources": [{"id": "r", "enabled": true, "path": "rules.json"}]},
        "background": {"service_worker": "bg.js"},
    });
    if let (Some(m), Some(e)) = (manifest.as_object_mut(), extra.as_object()) {
        m.extend(e.clone());
    }
    let files: [(&str, &str); 6] = [
        ("c.js", "window.c = 1;"),
        ("f.js", "window.f = 1;"),
        ("c.css", "body{}"),
        ("bg.js", ""),
        ("rules.json", r#"[{"id":1,"action":{"type":"block"},"condition":{"urlFilter":"||ads.test^","resourceTypes":["script"]}}]"#),
        ("_locales/en/messages.json", r#"{"name":{"message":"Shop Helper"}}"#),
    ];
    for (path, text) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
    let loaded = load(dir.path(), None, ID, "SEED", &["de".to_owned()]).unwrap();
    (dir, loaded)
}

fn url(s: &str) -> Url {
    Url::parse(s).unwrap()
}

const NORMAL: Context = Context { private: false, burner: false, profile_level: ShieldLevel::Standard, view_level: ShieldLevel::Standard };

#[test]
fn loads_only_the_tree_it_recorded() {
    let (dir, p) = package(json!({}));
    let tree = crate::ext_package::tree_sha256(dir.path()).unwrap();
    assert!(load(dir.path(), Some(&tree), ID, "SEED", &[]).is_ok());
    assert_eq!(load(dir.path(), Some("0".repeat(64).as_str()), ID, "SEED", &[]).err(), Some(LoadError::Changed));
    assert_eq!(load(dir.path(), Some(""), ID, "SEED", &[]).err(), Some(LoadError::Changed));
    assert_eq!(p.name, "Shop Helper");
    assert_eq!(p.world.name(), "tarn-ext-0a1b2c");
    assert_eq!(p.scripts.scripts[0].js, ["window.c = 1;"]);
    assert_eq!(p.scripts.scripts[0].css, ["body{}"]);
    assert!(p.scripts.bootstrap.contains(&format!("\"id\":\"{}\"", p.origin_id)) && p.scripts.bootstrap.contains("\"page\":false"));
    assert!(p.bootstrap(true).contains("\"page\":true"));
    assert_eq!(p.background_url().unwrap().as_str(), format!("tarn-extension://{}/_tarn/background.html", p.origin_id));
    assert!(is_tarn_dir(dir.path()));
}

#[test]
fn a_missing_file_or_bad_rule_refuses_the_package() {
    let (dir, _) = package(json!({}));
    std::fs::write(dir.path().join("rules.json"), r#"[{"id":7,"action":{"type":"redirect"}}]"#).unwrap();
    let Err(LoadError::Refused(why)) = load(dir.path(), None, ID, "SEED", &[]) else { panic!("loaded") };
    assert!(why.contains("rule 7") && why.contains("redirect"), "{why}");
    std::fs::remove_file(dir.path().join("rules.json")).unwrap();
    assert!(matches!(load(dir.path(), None, ID, "SEED", &[]), Err(LoadError::Refused(_))));
}

fn row(source: &str, all_sites: bool) -> InstalledExtension {
    InstalledExtension { profile_id: 1, install_id: ID.into(), source: source.into(), all_sites, enabled: true, ..Default::default() }
}

#[test]
fn scripts_run_only_where_granted_and_never_private_burner_or_lockdown() {
    let (_dir, p) = package(json!({}));
    let page = url("https://www.shop.test/cart");
    fn frame<'a>(u: &'a Url, top: &'a Url, main: bool) -> FrameNav<'a> {
        FrameNav { url: u, top, main_frame: main }
    }
    let granted = |_: &str| Access::Granted;
    assert_eq!(frame_decision(&p, &NORMAL, &frame(&page, &page, true), granted).run, [0, 1]);
    let sub = url("https://cdn.shop.test/widget");
    assert_eq!(frame_decision(&p, &NORMAL, &frame(&sub, &page, false), granted).run, [1], "only all_frames scripts reach frames");
    let excluded = url("https://shop.test/private/x");
    assert_eq!(frame_decision(&p, &NORMAL, &FrameNav { url: &excluded, top: &excluded, main_frame: true }, granted).run, [0]);
    for context in [
        Context { private: true, ..NORMAL },
        Context { burner: true, ..NORMAL },
        Context { profile_level: ShieldLevel::Lockdown, ..NORMAL },
        Context { view_level: ShieldLevel::Lockdown, ..NORMAL },
    ] {
        assert!(frame_decision(&p, &context, &frame(&page, &page, true), granted).run.is_empty(), "{context:?}");
        assert_eq!(rule_list(&p, &context, &Scope::Every { except: Vec::new() }), None);
    }
    let asked = frame_decision(&p, &NORMAL, &frame(&page, &page, true), |_| Access::Ask);
    assert_eq!(asked, FrameDecision { run: Vec::new(), ask: Some("www.shop.test".into()) });
    assert!(frame_decision(&p, &NORMAL, &frame(&sub, &page, false), |_| Access::Ask).ask.is_none(), "frames never prompt");
    let tarn_page = url("tarn-extension://x/popup.html");
    assert!(frame_decision(&p, &NORMAL, &FrameNav { url: &tarn_page, top: &tarn_page, main_frame: true }, granted).run.is_empty());
    let blank = url("about:blank");
    assert!(frame_decision(&p, &NORMAL, &FrameNav { url: &blank, top: &page, main_frame: false }, granted).run.is_empty());
}

#[test]
fn grants_follow_source_switches_answers_and_lockdown() {
    let (_dir, p) = package(json!({}));
    let now = Instant::now();
    let mut session = Session::default();
    let lockdown = ["locked.shop.test".to_owned()];
    let stored = [("deny.shop.test".to_owned(), false)];
    let unverified = row("file", true);
    let install = Install { row: &unverified, manifest: &p.manifest, every_site_pref: true, stored: &stored, lockdown_hosts: &lockdown };
    assert_eq!(session.access(&install, "www.shop.test", now), Access::Ask, "Unverified always asks");
    assert_eq!(session.access(&install, "locked.shop.test", now), Access::Denied);
    assert_eq!(session.access(&install, "deny.shop.test", now), Access::Denied);
    assert_eq!(session.access(&install, "other.test", now), Access::Denied, "unlisted hosts are not asked about");
    assert!(session.ask(1, ID, "www.shop.test") && !session.ask(1, ID, "www.shop.test"));
    assert_eq!(session.access(&install, "www.shop.test", now), Access::Denied, "asked once per session");
    let answer = Answer { profile: 1, install_id: ID.into(), host: "www.shop.test".into(), allow: true, remember: false, tab: Some(4) };
    assert_eq!(session.answer(&answer, now), None);
    assert_eq!(session.access(&install, "www.shop.test", now), Access::Granted);
    assert_eq!(session.scope(&install, now), Scope::Hosts(vec!["www.shop.test".into()]));
    assert_eq!(session.access(&install, "www.shop.test", now + Duration::from_secs(3601)), Access::Denied, "Once lasts an hour");
    session.tab_closed(4);
    assert_eq!(session.access(&install, "www.shop.test", now), Access::Denied, "Once ends with its tab");
    let kept = session.answer(&Answer { remember: true, ..answer.clone() }, now);
    assert_eq!(kept, Some(("www.shop.test".into(), true)));
    let official = row("official:x", false);
    let trusted = Install { row: &official, manifest: &p.manifest, every_site_pref: true, stored: &[], lockdown_hosts: &lockdown };
    assert_eq!(session.access(&trusted, "a.news.test", now), Access::Granted);
    assert_eq!(session.access(&trusted, "locked.shop.test", now), Access::Denied, "Lockdown hosts stay denied");
    assert_eq!(session.scope(&trusted, now), Scope::Every { except: vec!["locked.shop.test".into()] });
    session.answer(&Answer { host: "no.news.test".into(), allow: false, remember: false, tab: None, ..answer.clone() }, now);
    let denied = Install { stored: &stored, ..trusted };
    let except = vec!["deny.shop.test".to_owned(), "locked.shop.test".into(), "no.news.test".into()];
    assert_eq!(session.scope(&denied, now), Scope::Every { except }, "denied hosts stay out of every-site rules");
    let switched_off = Install { every_site_pref: false, ..trusted };
    assert_eq!(session.access(&switched_off, "a.news.test", now), Access::Ask);
}

#[test]
fn network_rules_act_only_on_granted_top_sites() {
    let (_dir, p) = package(json!({}));
    let lists = |scope: &Scope| rule_list(&p, &NORMAL, scope).map(|json| vec![RuleList::parse(&json).unwrap()]).unwrap_or_default();
    let verdict = |lists: &[RuleList], top: &str| {
        let (script, top) = (url("https://ads.test/x.js"), url(top));
        content_rules::evaluate(lists, &content_rules::Request { url: &script, top: Some(&top), kind: ResourceKind::Script })
    };
    let granted = lists(&Scope::Hosts(vec!["news.test".into()]));
    assert_eq!(verdict(&granted, "https://news.test/"), Verdict::Block);
    assert_eq!(verdict(&granted, "https://www.news.test/"), Verdict::Allow, "a grant names one host");
    assert_eq!(verdict(&granted, "https://other.test/"), Verdict::Allow);
    assert!(lists(&Scope::Hosts(Vec::new())).is_empty());
    assert_eq!(verdict(&lists(&Scope::Every { except: Vec::new() }), "https://other.test/"), Verdict::Block);
    assert_eq!(verdict(&lists(&Scope::Every { except: vec!["other.test".into()] }), "https://other.test/"), Verdict::Allow);
}

#[test]
fn a_dropped_prompt_asks_again() {
    let (_dir, p) = package(json!({}));
    let mut session = Session::default();
    let unverified = row("file", false);
    let install = Install { row: &unverified, manifest: &p.manifest, every_site_pref: false, stored: &[], lockdown_hosts: &[] };
    assert!(session.ask(1, ID, "www.shop.test"));
    assert_eq!(session.access(&install, "www.shop.test", Instant::now()), Access::Denied);
    session.unask(1, ID, "WWW.shop.test.");
    assert_eq!(session.access(&install, "www.shop.test", Instant::now()), Access::Ask);
}

struct World {
    router: Router,
    tabs: Vec<TabInfo>,
    store: Store,
    _dir: tempfile::TempDir,
    package: Package,
}

fn world(extra: Value) -> World {
    let (dir, package) = package(extra);
    let store = Store::open_in_memory().unwrap();
    let profile = store.create_profile("P", "SEED", false).unwrap().id;
    let mut router = Router::new(profile);
    router.add(&package);
    let tabs = vec![
        TabInfo { id: 1, url: Some(url("https://www.shop.test/cart")), title: "Cart".into(), active: true, current_window: true },
        TabInfo { id: 2, url: Some(url("https://bank.test/")), title: "Bank".into(), active: false, current_window: true },
    ];
    World { router, tabs, store, _dir: dir, package }
}

fn granted(_: &str, host: &str) -> Access {
    if host.ends_with("shop.test") { Access::Granted } else { Access::Denied }
}

impl World {
    fn send(&mut self, from: From, json: Value, now: Instant) -> Vec<Out> {
        self.send_raw(from, &json.to_string(), now)
    }

    fn send_raw(&mut self, from: From, json: &str, now: Instant) -> Vec<Out> {
        let env = Env { tabs: &self.tabs, access: &granted, store: Some(&self.store), now };
        self.router.handle(&Sender { install_id: ID.into(), from, origin: None }, json, &env)
    }
}

const CONTENT: From = From::Tab { tab: 1, frame: FrameId::MAIN };
const BACKGROUND: From = From::Page(PageKind::Background);

fn posts(out: &[Out]) -> Vec<(Target, Value)> {
    out.iter()
        .filter_map(|o| match o {
            Out::Post { target, json, .. } => Some((*target, serde_json::from_str(json).unwrap())),
            _ => None,
        })
        .collect()
}

#[test]
fn messages_start_the_background_wait_for_it_and_find_their_way_back() {
    let mut w = world(json!({}));
    let now = Instant::now();
    let out = w.send(CONTENT, json!({"id": 5, "call": "runtime.sendMessage", "args": [{"hi": 1}]}), now);
    assert_eq!(out, [Out::StartBackground(ID.into())]);
    w.router.page_opened(ID, PageKind::Background, now);
    let flushed = posts(&w.send(BACKGROUND, json!({"id": null, "call": "ready", "args": []}), now));
    let (to, delivered) = &flushed[0];
    assert_eq!(*to, Target::Page(PageKind::Background));
    assert_eq!(delivered["message"], json!({"hi": 1}));
    assert_eq!(delivered["sender"]["tab"]["url"], "https://www.shop.test/cart");
    assert_eq!(delivered["sender"]["id"], w.package.origin_id);
    let delivery = delivered["delivery"].clone();
    assert!(w.send(From::Tab { tab: 1, frame: FrameId(3) }, json!({"id": null, "call": "reply", "args": [delivery, "forged"]}), now).is_empty());
    let answer = posts(&w.send(BACKGROUND, json!({"id": null, "call": "reply", "args": [delivery, {"ok": true}]}), now));
    assert_eq!(answer, [(Target::Frame { tab: 1, frame: FrameId::MAIN }, json!({"id": 5, "result": {"ok": true}}))]);
    assert!(w.router.tick(now + Duration::from_secs(29)).is_empty());
    assert_eq!(w.router.tick(now + Duration::from_secs(30)), [Out::StopBackground(ID.into())]);
}

#[test]
fn identity_comes_from_the_engine_and_ungranted_tabs_are_dropped() {
    let mut w = world(json!({}));
    let now = Instant::now();
    let claim = json!({"id": 1, "call": "tabs.query", "args": [{}], "sender": "background", "install": "other"});
    let refused = posts(&w.send(CONTENT, claim, now));
    assert!(refused[0].1["error"].as_str().unwrap().contains("not available"), "a content script is not a page whatever it claims");
    assert!(w.send(From::Tab { tab: 2, frame: FrameId::MAIN }, json!({"id": 1, "call": "storage.get", "args": [null]}), now).is_empty());
    let env = Env { tabs: &w.tabs, access: &granted, store: None, now };
    assert!(
        w.router
            .handle(&Sender { install_id: "unknown".into(), from: CONTENT, origin: None }, r#"{"id":1,"call":"storage.get","args":[null]}"#, &env)
            .is_empty()
    );
    assert!(w.send_raw(CONTENT, "not json", now).is_empty());
}

#[test]
fn a_subframe_is_heard_only_with_a_grant_for_its_own_origin() {
    let mut w = world(json!({}));
    let now = Instant::now();
    let get = json!({"id": 1, "call": "storage.get", "args": [null]}).to_string();
    let mut from_frame = |origin: Option<&str>| {
        let env = Env { tabs: &w.tabs, access: &granted, store: None, now };
        let sender = Sender { install_id: ID.into(), from: From::Tab { tab: 2, frame: FrameId(7) }, origin: origin.map(url) };
        posts(&w.router.handle(&sender, &get, &env))
    };
    assert_eq!(from_frame(Some("https://cdn.shop.test")).len(), 1, "a granted frame inside an ungranted page");
    assert!(from_frame(Some("https://bank.test")).is_empty(), "its grant ran out or was denied");
    assert!(from_frame(None).is_empty(), "no origin from the engine");
}

#[test]
fn a_background_page_that_never_gets_ready_fails_its_callers() {
    let mut w = world(json!({}));
    let now = Instant::now();
    assert_eq!(w.send(CONTENT, json!({"id": 5, "call": "runtime.sendMessage", "args": [1]}), now), [Out::StartBackground(ID.into())]);
    w.router.page_opened(ID, PageKind::Background, now);
    for n in 0..MAX_QUEUED + 10 {
        w.send(CONTENT, json!({"id": null, "call": "runtime.sendMessage", "args": [n]}), now);
    }
    assert!(w.router.tick(now + Duration::from_secs(29)).is_empty());
    let out = w.router.tick(now + Duration::from_secs(30));
    assert_eq!(out[0], Out::StopBackground(ID.into()));
    assert_eq!(posts(&out), [(Target::Frame { tab: 1, frame: FrameId::MAIN }, json!({"id": 5, "error": "The extension's background page did not start."}))]);
    assert_eq!(w.send(CONTENT, json!({"id": 6, "call": "runtime.sendMessage", "args": [1]}), now), [Out::StartBackground(ID.into())]);
    w.router.page_opened(ID, PageKind::Background, now);
    for n in 0..MAX_QUEUED + 10 {
        w.send(CONTENT, json!({"id": null, "call": "runtime.sendMessage", "args": [n]}), now);
    }
    assert_eq!(w.send(BACKGROUND, json!({"id": null, "call": "ready", "args": []}), now).len(), MAX_QUEUED);
}

#[test]
fn a_message_beyond_the_queue_is_answered_and_the_background_still_stops() {
    let mut w = world(json!({}));
    let now = Instant::now();
    w.send(CONTENT, json!({"id": null, "call": "runtime.sendMessage", "args": [0]}), now);
    w.router.page_opened(ID, PageKind::Background, now);
    for n in 1..MAX_QUEUED {
        assert!(w.send(CONTENT, json!({"id": null, "call": "runtime.sendMessage", "args": [n]}), now).is_empty());
    }
    let out = w.send(CONTENT, json!({"id": 7, "call": "runtime.sendMessage", "args": ["late"]}), now);
    assert_eq!(
        posts(&out),
        [(Target::Frame { tab: 1, frame: FrameId::MAIN }, json!({"id": 7, "error": "Too many messages are waiting for the extension's page."}))]
    );
    assert_eq!(w.send(BACKGROUND, json!({"id": null, "call": "ready", "args": []}), now).len(), MAX_QUEUED);
    assert_eq!(w.router.tick(now + Duration::from_secs(30)), [Out::StopBackground(ID.into())]);
}

#[test]
fn size_and_rate_caps_answer_with_last_error() {
    let mut w = world(json!({}));
    let now = Instant::now();
    let big = format!(r#"{{"id":9,"call":"storage.set","args":[{{"k":"{}"}}]}}"#, "x".repeat(1 << 20));
    assert!(posts(&w.send_raw(CONTENT, &big, now))[0].1["error"].as_str().unwrap().contains("1 MiB"));
    let get = json!({"id": 1, "call": "storage.get", "args": ["k"]});
    for _ in 0..MAX_RATE {
        w.send(CONTENT, get.clone(), now);
    }
    assert_eq!(posts(&w.send(CONTENT, get.clone(), now))[0].1["error"], "Too many messages.");
    assert!(posts(&w.send(From::Tab { tab: 3, frame: FrameId::MAIN }, get.clone(), now)).is_empty(), "unknown tab");
    assert!(posts(&w.send(BACKGROUND, get.clone(), now))[0].1.get("result").is_some(), "another tab has its own budget");
    assert!(posts(&w.send(CONTENT, get, now + Duration::from_secs(1)))[0].1.get("result").is_some());
}

#[test]
fn storage_keeps_its_quota_persists_and_reports_changes() {
    let mut w = world(json!({}));
    let now = Instant::now();
    let out = posts(&w.send(CONTENT, json!({"id": 1, "call": "storage.set", "args": [{"a": [1, 2], "b": "x"}]}), now));
    assert_eq!(out[0], (Target::AllTabs, json!({"event": "storage.onChanged", "changes": {"a": {"newValue": [1, 2]}, "b": {"newValue": "x"}}})));
    assert_eq!(out[1].1, json!({"id": 1, "result": null}));
    assert_eq!(w.store.extension_storage(w.router_profile(), ID).unwrap().len(), 2);
    let got = posts(&w.send(CONTENT, json!({"id": 2, "call": "storage.get", "args": [{"a": 0, "z": 9}]}), now));
    assert_eq!(got[0].1["result"], json!({"a": [1, 2], "z": 9}));
    let chunk = "y".repeat(900 << 10);
    for n in 0..QUOTA_BYTES / (900 << 10) {
        let fits = posts(&w.send(CONTENT, json!({"id": 3, "call": "storage.set", "args": [{format!("c{n}"): chunk}]}), now));
        assert!(fits.last().unwrap().1.get("error").is_none());
    }
    let over = posts(&w.send(CONTENT, json!({"id": 3, "call": "storage.set", "args": [{"last": chunk}]}), now));
    assert_eq!(over, [(Target::Frame { tab: 1, frame: FrameId::MAIN }, json!({"id": 3, "error": "QUOTA_BYTES quota exceeded"}))]);
    let long_key = posts(&w.send(CONTENT, json!({"id": 4, "call": "storage.set", "args": [{"k".repeat(1025): 1}]}), now));
    assert!(long_key[0].1["error"].as_str().unwrap().contains("1 KiB"));
    let removed = posts(&w.send(CONTENT, json!({"id": 5, "call": "storage.clear", "args": []}), now));
    assert_eq!(removed[0].1["changes"]["a"], json!({"oldValue": [1, 2]}));
    w.send(CONTENT, json!({"id": 5, "call": "storage.set", "args": [{"b": "x"}]}), now);
    let bytes = posts(&w.send(CONTENT, json!({"id": 6, "call": "storage.getBytesInUse", "args": [null]}), now));
    assert_eq!(bytes[0].1["result"], json!(1 + "\"x\"".len()));
}

#[test]
fn storage_needs_its_permission_and_stays_in_memory_when_private() {
    let mut w = world(json!({"permissions": ["declarativeNetRequest"]}));
    let out = posts(&w.send(CONTENT, json!({"id": 1, "call": "storage.set", "args": [{"a": 1}]}), Instant::now()));
    assert!(out[0].1["error"].is_string());
    let mut w = world(json!({}));
    let env = Env { tabs: &w.tabs, access: &granted, store: None, now: Instant::now() };
    w.router.handle(&Sender { install_id: ID.into(), from: CONTENT, origin: None }, r#"{"id":1,"call":"storage.set","args":[{"a":1}]}"#, &env);
    assert!(w.store.extension_storage(w.router_profile(), ID).unwrap().is_empty());
}

impl World {
    fn router_profile(&self) -> store::Id {
        self.store.profiles().unwrap()[0].id
    }
}

#[test]
fn tabs_query_shows_urls_only_of_granted_or_clicked_tabs() {
    let mut w = world(json!({"permissions": ["storage", "activeTab", "declarativeNetRequest"]}));
    let now = Instant::now();
    let all = posts(&w.send(BACKGROUND, json!({"id": 1, "call": "tabs.query", "args": [{}]}), now));
    assert_eq!(all[0].1["result"], json!([{"id": 1, "active": true, "url": "https://www.shop.test/cart", "title": "Cart"}, {"id": 2, "active": false}]));
    let by_url = posts(&w.send(BACKGROUND, json!({"id": 2, "call": "tabs.query", "args": [{"url": "*://bank.test/*"}]}), now));
    assert_eq!(by_url[0].1["result"], json!([]), "an ungranted tab is not found by its address");
    let bank = w.tabs[1].clone();
    let env = Env { tabs: &w.tabs, access: &granted, store: None, now };
    w.router.clicked(ID, &bank, &env);
    let clicked = posts(&w.send(BACKGROUND, json!({"id": 3, "call": "tabs.query", "args": [{"active": false}]}), now));
    assert_eq!(clicked[0].1["result"][0]["url"], "https://bank.test/");
    w.router.navigated(2);
    let after = posts(&w.send(BACKGROUND, json!({"id": 4, "call": "tabs.query", "args": [{"active": false}]}), now));
    assert!(after[0].1["result"][0].get("url").is_none());
    let to_bank = posts(&w.send(BACKGROUND, json!({"id": 5, "call": "tabs.sendMessage", "args": [2, "hi"]}), now));
    assert!(to_bank[0].1["error"].as_str().unwrap().contains("Receiving end"));
    let to_shop = posts(&w.send(BACKGROUND, json!({"id": 6, "call": "tabs.sendMessage", "args": [1, "hi", {"frameId": 0}]}), now));
    assert_eq!(to_shop[0].0, Target::Frame { tab: 1, frame: FrameId::MAIN });
}

#[test]
fn tabs_create_opens_web_pages_at_the_popup_rate_only() {
    let mut w = world(json!({}));
    let now = Instant::now();
    let create = |u: &str| json!({"id": 1, "call": "tabs.create", "args": [{"url": u, "active": false}]});
    for bad in ["http://192.168.1.1/", "http://localhost/", "file:///etc/passwd", "javascript:alert(1)", "tarn-extension://x/a.html", "popup.html"] {
        let out = w.send(BACKGROUND, create(bad), now);
        assert!(!out.iter().any(|o| matches!(o, Out::OpenTab { .. })), "{bad}");
    }
    for _ in 0..3 {
        let out = w.send(BACKGROUND, create("https://example.com/"), now);
        assert!(out.contains(&Out::OpenTab { url: url("https://example.com/"), active: false }));
    }
    let fourth = w.send(BACKGROUND, create("https://example.com/"), now);
    assert!(!fourth.iter().any(|o| matches!(o, Out::OpenTab { .. })));
    assert!(w.send(BACKGROUND, create("https://example.com/"), now + Duration::from_secs(10)).iter().any(|o| matches!(o, Out::OpenTab { .. })));
    assert!(!w.send(CONTENT, create("https://example.com/"), now + Duration::from_secs(20)).iter().any(|o| matches!(o, Out::OpenTab { .. })), "pages only");
}

#[test]
fn badges_are_short_and_per_tab_over_global() {
    let mut w = world(json!({"action": {"default_title": "Helper"}}));
    let now = Instant::now();
    let out = w.send(BACKGROUND, json!({"id": 1, "call": "action.setBadgeText", "args": [{"text": "12"}]}), now);
    assert!(out.contains(&Out::ActionChanged(ID.into())));
    w.send(BACKGROUND, json!({"id": 2, "call": "action.setBadgeText", "args": [{"text": "7", "tabId": 1}]}), now);
    w.send(BACKGROUND, json!({"id": 3, "call": "action.setBadgeBackgroundColor", "args": [{"color": "#f00"}]}), now);
    let long = posts(&w.send(BACKGROUND, json!({"id": 4, "call": "action.setBadgeText", "args": [{"text": "12345"}]}), now));
    assert!(long[0].1["error"].is_string());
    assert_eq!(w.router.action(ID, Some(1)).badge_text, "7");
    let global = w.router.action(ID, Some(2));
    assert_eq!((global.badge_text.as_str(), global.badge_color, global.title.as_str()), ("12", Some([255, 0, 0, 255]), "Helper"));
    w.router.tab_closed(1);
    assert_eq!(w.router.action(ID, Some(1)).badge_text, "12");
}

#[test]
fn unanswered_calls_expire_and_are_capped() {
    let mut w = world(json!({}));
    let now = Instant::now();
    w.send(CONTENT, json!({"id": 1, "call": "runtime.sendMessage", "args": [1]}), now);
    w.router.page_opened(ID, PageKind::Background, now);
    w.send(BACKGROUND, json!({"id": null, "call": "ready", "args": []}), now);
    assert!(w.router.tick(now + Duration::from_secs(29)).is_empty());
    let out = w.router.tick(now + REPLY_TIMEOUT);
    assert_eq!(
        posts(&out),
        [(Target::Frame { tab: 1, frame: FrameId::MAIN }, json!({"id": 1, "error": "The message port closed before a response was received."}))]
    );
    let later = now + REPLY_TIMEOUT + Duration::from_secs(1);
    for n in 0..MAX_DELIVERIES as u64 {
        w.send(BACKGROUND, json!({"id": n, "call": "tabs.sendMessage", "args": [1, n]}), later + Duration::from_secs(n / MAX_RATE as u64));
    }
    let over = posts(&w.send(BACKGROUND, json!({"id": 999, "call": "tabs.sendMessage", "args": [1, "x"]}), later + Duration::from_secs(5)));
    assert_eq!(over[0].1["error"], "Too many messages are waiting for an answer.");
}

#[test]
fn the_background_stays_up_only_for_events_to_it() {
    let mut w = world(json!({}));
    let now = Instant::now();
    w.send(CONTENT, json!({"id": null, "call": "runtime.sendMessage", "args": [1]}), now);
    w.router.page_opened(ID, PageKind::Background, now);
    w.send(BACKGROUND, json!({"id": null, "call": "ready", "args": []}), now);
    for s in 1..40 {
        w.send(BACKGROUND, json!({"id": s, "call": "storage.get", "args": [null]}), now + Duration::from_secs(s));
    }
    assert_eq!(w.router.tick(now + Duration::from_secs(40)), [Out::StopBackground(ID.into())]);
}

#[test]
fn a_tab_answer_without_a_value_waits_for_the_other_frames() {
    let mut w = world(json!({}));
    let now = Instant::now();
    let sent = posts(&w.send(BACKGROUND, json!({"id": 4, "call": "tabs.sendMessage", "args": [1, "hi"]}), now));
    assert_eq!(sent[0].0, Target::Tab(1));
    let delivery = sent[0].1["delivery"].clone();
    assert!(w.send(CONTENT, json!({"id": null, "call": "reply", "args": [delivery, null]}), now).is_empty());
    let frame = Sender { install_id: ID.into(), from: From::Tab { tab: 1, frame: FrameId(5) }, origin: Some(url("https://cdn.shop.test")) };
    let env = Env { tabs: &w.tabs, access: &granted, store: None, now };
    let answer = posts(&w.router.handle(&frame, &json!({"id": null, "call": "reply", "args": [delivery, "real"]}).to_string(), &env));
    assert_eq!(answer, [(Target::Page(PageKind::Background), json!({"id": 4, "result": "real"}))]);
    let sent = posts(&w.send(BACKGROUND, json!({"id": 5, "call": "tabs.sendMessage", "args": [1, "hi"]}), now));
    w.send(CONTENT, json!({"id": null, "call": "reply", "args": [sent[0].1["delivery"], null]}), now);
    let settled = posts(&w.router.tick(now + Duration::from_secs(1)));
    assert_eq!(settled, [(Target::Page(PageKind::Background), json!({"id": 5, "result": null}))]);
}

#[test]
fn badges_need_an_open_tab_and_main_frames_their_tab_origin() {
    let mut w = world(json!({"action": {"default_title": "Helper"}}));
    let now = Instant::now();
    let gone = posts(&w.send(BACKGROUND, json!({"id": 1, "call": "action.setBadgeText", "args": [{"text": "1", "tabId": 99}]}), now));
    assert!(gone[0].1["error"].is_string());
    assert_eq!(w.router.action(ID, Some(99)).badge_text, "");
    let get = json!({"id": 1, "call": "storage.get", "args": [null]}).to_string();
    let env = Env { tabs: &w.tabs, access: &granted, store: None, now };
    let main = |origin: &str| Sender { install_id: ID.into(), from: CONTENT, origin: Some(url(origin)) };
    assert!(w.router.handle(&main("https://evil.test"), &get, &env).is_empty(), "the engine's origin is not the tab's");
    assert_eq!(posts(&w.router.handle(&main("https://www.shop.test"), &get, &env)).len(), 1);
}

#[test]
fn an_https_only_extension_does_not_reach_the_http_page_of_a_granted_host() {
    let mut w = world(json!({
        "host_permissions": ["https://*.shop.test/*"],
        "content_scripts": [{"matches": ["https://*.shop.test/*"], "js": ["c.js"]}],
    }));
    w.tabs[0].url = Some(url("http://www.shop.test/cart"));
    let now = Instant::now();
    assert!(w.send(CONTENT, json!({"id": 1, "call": "storage.get", "args": [null]}), now).is_empty());
    let sent = posts(&w.send(BACKGROUND, json!({"id": 2, "call": "tabs.sendMessage", "args": [1, "hi"]}), now));
    assert!(sent[0].1["error"].as_str().unwrap().contains("Receiving end"));
    let mut w = world(json!({}));
    w.tabs[0].url = Some(url("http://www.shop.test/cart"));
    assert_eq!(posts(&w.send(CONTENT, json!({"id": 1, "call": "storage.get", "args": [null]}), now)).len(), 1, "a *:// pattern names http");
}
