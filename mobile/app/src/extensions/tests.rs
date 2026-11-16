use std::path::Path;
use std::time::{Duration, Instant};

use browser_core::ext_package;
use browser_core::tarn_ext::router::IDLE;
use engine::tarn_ext::FrameNav;
use serde_json::json;
use store::InstalledExtension;

use super::*;
use crate::data::Memory;

type TestShell = Shell<Memory>;
const ID: &str = "0123456789abcdef0123456789abcdef";

fn fixture(dir: &Path) {
    let manifest = json!({
        "manifest_version": 3, "name": "Shop Helper", "version": "1.0",
        "browser_specific_settings": {"tarn": {"runtime": 1}},
        "permissions": ["storage", "declarativeNetRequest"],
        "content_scripts": [{"matches": ["*://*.shop.test/*"], "js": ["c.js"], "css": ["c.css"]}],
        "declarative_net_request": {"rule_resources": [{"id": "r", "enabled": true, "path": "rules.json"}]},
        "background": {"service_worker": "bg.js"},
        "action": {"default_popup": "popup.html", "default_title": "Helper"},
    });
    let files = [
        ("manifest.json", manifest.to_string()),
        ("c.js", "chrome.storage.local.set({seen: location.host});".to_owned()),
        ("c.css", "body { outline: 1px solid red }".to_owned()),
        ("bg.js", "chrome.runtime.onMessage.addListener((m, s, r) => r('pong'));".to_owned()),
        ("popup.html", "<!doctype html><script src=\"popup.js\"></script>".to_owned()),
        ("popup.js", "chrome.action.setBadgeText({text: '1'});".to_owned()),
        ("rules.json", r#"[{"id":1,"action":{"type":"block"},"condition":{"urlFilter":"||ads.test^","resourceTypes":["script"]}}]"#.to_owned()),
    ];
    std::fs::create_dir_all(dir).unwrap();
    for (name, body) in files {
        std::fs::write(dir.join(name), body).unwrap();
    }
}

struct Setup {
    _dir: tempfile::TempDir,
    shell: TestShell,
    profile: Id,
    store_id: String,
    tab: TabKey,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let mut shell = TestShell::open(dir.path());
    let profile = shell.extension_profile().unwrap();
    let path = shell.ext.root.join(profile.to_string()).join(ID).join("1");
    fixture(&path);
    let row = InstalledExtension {
        profile_id: profile,
        install_id: ID.into(),
        source: "file".into(),
        path: path.to_string_lossy().into_owned(),
        name: "Shop Helper".into(),
        version: "1.0".into(),
        sha256: ext_package::tree_sha256(&path).unwrap(),
        archive_sha256: String::new(),
        permissions: String::new(),
        enabled: true,
        all_sites: false,
        owner_id: 0,
        asset: String::new(),
        release_tag: String::new(),
    };
    shell.core.disk.put_extension(&row).unwrap();
    shell.load_extensions(profile);
    let store_id = shell.store_id(false, profile).unwrap();
    shell.navigate("https://shop.test/cart");
    let tab = shell.model.active.unwrap();
    let page = json!({ "url": "https://shop.test/cart", "title": "Cart", "canGoBack": false, "canGoForward": false });
    shell.event(tab, "committed", &page, 1);
    Setup { _dir: dir, shell, profile, store_id, tab }
}

fn world() -> World {
    World::for_install(ID).unwrap()
}

fn gate(s: &mut Setup, url: &str, now: Instant) -> Vec<usize> {
    let url = Url::parse(url).unwrap();
    let tab = format!("n{}", s.tab.id);
    let store = s.store_id.clone();
    s.shell.gate(&tab, &store, &world(), &FrameNav { url: &url, top: &url, main_frame: true }, now)
}

fn message(s: &mut Setup, body: Value) -> Vec<Effect> {
    let data = json!({ "world": world().name(), "frame": 0, "json": body.to_string() });
    let tab = s.tab;
    s.shell.event(tab, MESSAGE, &data, 2)
}

fn posts_to_tab(effects: &[Effect], tab: TabKey) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Command(k, POST, args) if *k == tab => serde_json::from_str(args["json"].as_str()?).ok(),
            _ => None,
        })
        .collect()
}

#[test]
fn loading_hands_the_engine_scripts_pages_and_no_rules_without_a_grant() {
    let s = setup();
    let scripts = ext::SCRIPTS.with(|m| m.borrow().get(&s.store_id).cloned()).unwrap();
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].world, world());
    assert_eq!(scripts[0].scripts[0].css, ["body { outline: 1px solid red }"]);
    let rules = ext::RULES.with(|m| m.borrow().get(&s.store_id).cloned()).unwrap();
    assert!(rules.is_empty(), "no DNR rules before the user allows a site: {rules:?}");
    let package = s.shell.ext.rt.loaded(s.profile, ID).unwrap().package.clone();
    let served = ext::SERVED.with(|m| m.borrow().get(&s.store_id).cloned()).unwrap();
    let popup = served(&package.popup_url().unwrap());
    assert_eq!((popup.status, popup.csp), (200, Some(browser_core::tarn_ext::resources::PAGE_CSP)));
    let outside = Url::parse(&format!("tarn-extension://{}/%2e%2e/%2e%2e/secret", package.origin_id)).unwrap();
    assert_ne!(served(&outside).status, 200);
}

#[test]
fn a_site_runs_scripts_only_after_the_user_allows_it() {
    let mut s = setup();
    let now = Instant::now();
    assert!(gate(&mut s, "https://shop.test/cart", now).is_empty());
    let ask = s.shell.active_ask().cloned().unwrap();
    assert_eq!((ask.host.as_str(), ask.badge.as_str()), ("shop.test", "Unverified · file"));
    assert!(gate(&mut s, "https://shop.test/cart", now).is_empty());
    assert_eq!(s.shell.ext.asks.len(), 1, "asked once per session");
    assert!(gate(&mut s, "https://news.test/", now).is_empty(), "a site the extension does not list");
    let effects = s.shell.answer_ask(true, false, now);
    assert_eq!(effects, vec![Effect::Command(s.tab, "reload", Value::Null)]);
    assert_eq!(gate(&mut s, "https://shop.test/cart", now), vec![0]);
    let rules = ext::RULES.with(|m| m.borrow().get(&s.store_id).cloned()).unwrap();
    assert_eq!(rules.len(), 1, "the rules act where the extension holds a grant");
    assert!(rules[0].1.contains("shop.test"), "{}", rules[0].1);
    assert!(gate(&mut s, "https://shop.test/cart", now + Duration::from_secs(3601)).is_empty(), "Allow Once ends after an hour");
    s.shell.extensions_tick(now + Duration::from_secs(3601));
    assert!(ext::RULES.with(|m| m.borrow().get(&s.store_id).cloned()).unwrap().is_empty());
}

#[test]
fn a_remembered_answer_is_stored_and_never_asked_again() {
    let mut s = setup();
    let now = Instant::now();
    gate(&mut s, "https://shop.test/", now);
    s.shell.answer_ask(false, true, now);
    assert_eq!(s.shell.core.disk.extension_hosts(s.profile, ID).unwrap(), [("shop.test".to_owned(), false)]);
    s.shell.ext.rt.session.forget(s.profile, None);
    assert!(gate(&mut s, "https://shop.test/", now).is_empty());
    assert!(s.shell.ext.asks.is_empty());
}

#[test]
fn storage_and_messages_go_through_the_router() {
    let mut s = setup();
    let now = Instant::now();
    gate(&mut s, "https://shop.test/", now);
    s.shell.answer_ask(true, true, now);
    let effects = message(&mut s, json!({"id": 1, "call": "storage.set", "args": [{"seen": "shop.test"}]}));
    let replies = posts_to_tab(&effects, s.tab);
    assert!(replies.iter().any(|r| r["id"] == 1 && r.get("error").is_none()), "{replies:?}");
    assert_eq!(s.shell.core.disk.extension_storage(s.profile, ID).unwrap(), [("seen".to_owned(), "\"shop.test\"".to_owned())]);
    let effects = message(&mut s, json!({"id": 2, "call": "runtime.sendMessage", "args": ["ping"]}));
    assert!(posts_to_tab(&effects, s.tab).is_empty(), "the reply waits for the background page");
    let page = s.shell.page_views().pop().expect("the background page starts on the first message");
    assert_eq!((page.kind, page.world.as_str()), (PageKind::Background, world().name()));
    assert!(page.url.ends_with("/_tarn/background.html") && page.bootstrap.contains("\"page\":true"));
    let ready = json!({"world": world().name(), "json": json!({"id": null, "call": "ready", "args": []}).to_string()});
    let delivered = s.shell.page_message(s.profile, ID, PageKind::Background, &ready, now);
    let Some(Effect::PagePost(_, PageKind::Background, json)) = delivered.first() else { panic!("{delivered:?}") };
    let delivery: Value = serde_json::from_str(json).unwrap();
    assert_eq!(delivery["message"], "ping");
    let reply = json!({"world": world().name(), "json": json!({"id": null, "call": "reply", "args": [delivery["delivery"], "pong"]}).to_string()});
    let effects = s.shell.page_message(s.profile, ID, PageKind::Background, &reply, now);
    assert!(posts_to_tab(&effects, s.tab).iter().any(|r| r["id"] == 2 && r["result"] == "pong"));
    s.shell.extensions_tick(Instant::now() + IDLE + Duration::from_secs(1));
    assert!(s.shell.page_views().is_empty(), "an idle background page stops");
}

#[test]
fn the_page_world_cannot_speak_for_an_extension() {
    let mut s = setup();
    let forged = json!({ "world": "", "frame": 0, "json": json!({"id": 1, "call": "storage.set", "args": [{"x": 1}]}).to_string() });
    let tab = s.tab;
    assert!(s.shell.event(tab, MESSAGE, &forged, 2).is_empty());
    let ungranted = message(&mut s, json!({"id": 1, "call": "storage.set", "args": [{"x": 1}]}));
    assert!(ungranted.is_empty(), "a main frame without a grant is not heard");
    let from_page = json!({"world": "tarn-ext-ffff", "json": "{}"});
    assert!(s.shell.page_message(s.profile, ID, PageKind::Popup, &from_page, Instant::now()).is_empty(), "no popup is open");
}

#[test]
fn the_action_opens_its_popup_and_shows_its_badge() {
    let mut s = setup();
    let now = Instant::now();
    assert_eq!(s.shell.action_items()[0].state.title, "Helper");
    s.shell.click_action(ID, now);
    let popup = s.shell.popup().unwrap();
    assert!(popup.url.ends_with("/popup.html"));
    let badge = json!({"world": world().name(), "json": json!({"id": 5, "call": "action.setBadgeText", "args": [{"text": "1"}]}).to_string()});
    s.shell.page_message(s.profile, ID, PageKind::Popup, &badge, now);
    assert_eq!(s.shell.action_items()[0].state.badge_text, "1");
    s.shell.close_popup();
    assert!(s.shell.popup().is_none());
}

#[test]
fn nothing_runs_in_a_private_space() {
    let mut s = setup();
    s.shell.enter_private();
    assert!(s.shell.model.private());
    assert_eq!(s.shell.extension_profile(), None);
    assert!(s.shell.action_items().is_empty());
    let private_store = s.shell.views()[0].profile.clone();
    assert!(ext::SCRIPTS.with(|m| m.borrow().get(&private_store).is_none()));
    let url = Url::parse("https://shop.test/").unwrap();
    let tab = format!("p{}", s.shell.model.active.unwrap().id);
    assert!(s.shell.gate(&tab, &private_store, &world(), &FrameNav { url: &url, top: &url, main_frame: true }, Instant::now()).is_empty());
    let key = s.shell.model.active.unwrap();
    let data = json!({ "world": world().name(), "frame": 0, "json": "{}" });
    assert!(s.shell.event(key, MESSAGE, &data, 2).is_empty());
}

#[test]
fn a_changed_install_is_turned_off_with_its_grants() {
    let mut s = setup();
    let now = Instant::now();
    gate(&mut s, "https://shop.test/", now);
    s.shell.answer_ask(true, true, now);
    let dir = s.shell.ext.rt.loaded(s.profile, ID).unwrap().package.dir.clone();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.join("c.js"), std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(dir.join("c.js"), "steal()").unwrap();
    s.shell.load_extensions(s.profile);
    assert!(s.shell.notice.as_deref().unwrap().contains("changed on disk"));
    assert!(!s.shell.core.disk.extensions(s.profile).unwrap()[0].enabled);
    assert!(s.shell.core.disk.extension_hosts(s.profile, ID).unwrap().is_empty());
    assert!(ext::SCRIPTS.with(|m| m.borrow().get(&s.store_id).cloned()).unwrap().is_empty());
    assert!(gate(&mut s, "https://shop.test/", now).is_empty());
}

#[test]
fn closing_the_tab_ends_its_once_grant() {
    let mut s = setup();
    let now = Instant::now();
    gate(&mut s, "https://shop.test/", now);
    s.shell.answer_ask(true, false, now);
    assert_eq!(gate(&mut s, "https://shop.test/", now), vec![0]);
    let tab = s.tab;
    s.shell.new_tab(None);
    s.shell.close_tab(tab);
    assert!(s.shell.ext.rt.session.next_expiry(now).is_none());
}
