use std::path::{Path, PathBuf};

use browser_core::ext_package::tree_sha256;
use serde_json::{Value, json};

use super::*;

const INSTALL: &str = "fixture01";

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/tarn-ext")
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub core: Rc<Core<Data>>,
    pub profile: Id,
    pub tab: u64,
    pub path: PathBuf,
}

pub fn fixture(source: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let core = crate::browser::Browser::open_core(dir.path());
    let profile = core.disk.profiles().unwrap()[0].id;
    core.open_profile(false, profile, || Rc::new(|_| {}));
    let space = core.disk.spaces(profile).unwrap()[0].id;
    let tab = core.disk.create_tab(space, store::TabKind::Ephemeral, None, "http://127.0.0.1/page", "Page").unwrap().id as u64;
    let path = dir.path().join("extensions").join(INSTALL).join("1");
    copy(&fixture_dir(), &path);
    let row = InstalledExtension {
        profile_id: profile,
        install_id: INSTALL.to_owned(),
        source: source.to_owned(),
        path: path.to_string_lossy().into_owned(),
        name: "Tarn Fixture".to_owned(),
        version: "1.0".to_owned(),
        sha256: tree_sha256(&path).unwrap(),
        archive_sha256: String::new(),
        permissions: String::new(),
        enabled: true,
        all_sites: false,
        owner_id: 0,
        asset: String::new(),
        release_tag: String::new(),
    };
    core.disk.put_extension(&row).unwrap();
    with(|rt| {
        *rt = State::default();
        rt.core = Some(core.clone());
    });
    load_now(&core, profile);
    Fixture { dir, core, profile, tab, path }
}

pub fn shut_down() {
    drop(with(std::mem::take));
}

pub fn reload_now(f: &Fixture) {
    load_now(&f.core, f.profile);
}

fn load_now(core: &Core<Data>, profile: Id) {
    let seed = core.disk.profile(profile).unwrap().data_store_uuid;
    let rows = core.disk.extensions(profile).unwrap().into_iter().filter(|r| r.enabled).collect();
    apply(profile, &seed, runtime::load_rows(rows, runtime::row_dir, &seed, &["en".to_owned()]));
}

pub struct Attached {
    pub worlds: Vec<Rc<engine::tarn_ext::WorldScripts>>,
    pub rules: Vec<(String, String)>,
}

fn attach_tab(key: &StoreKey, tab: &str, level: ShieldLevel) -> Option<Attached> {
    let store = view::store_of(key, tab, level)?;
    with(|rt| {
        let profile = rt.profile_of(&store)?;
        let worlds = rt.rt.scripts(profile);
        (!worlds.is_empty()).then(|| Attached { worlds, rules: rt.rule_ids(profile) })
    })
}

fn attach(key: &StoreKey, tab: u64, level: ShieldLevel) -> Option<Attached> {
    attach_tab(key, &tab_prop(tab), level)
}

fn persistent(f: &Fixture) -> StoreKey {
    StoreKey::Persistent(f.core.disk.profile(f.profile).unwrap().data_store_uuid)
}

fn frame(f: &Fixture, url: &str) -> Vec<usize> {
    let world = World::for_install(INSTALL).unwrap();
    let url = Url::parse(url).unwrap();
    with(|rt| rt.gate(f.profile, f.tab, &world, &FrameNav { url: &url, top: &url, main_frame: true }))
}

pub fn asks() -> Vec<Note> {
    with(|rt| std::mem::take(&mut rt.sent)).into_iter().filter(|n| matches!(n, Note::Ask { .. })).collect()
}

#[test]
fn scripts_run_only_where_granted_after_one_prompt() {
    let f = fixture("file");
    let a = attach(&persistent(&f), f.tab, ShieldLevel::Standard).expect("a persistent tab runs the profile's extensions");
    assert_eq!(a.worlds.len(), 1);
    assert_eq!(a.worlds[0].world.name(), "tarn-ext-fixture01");
    assert!(a.rules.is_empty(), "no network rules act before a grant");
    asks();
    assert!(frame(&f, "http://127.0.0.1:8080/page").is_empty(), "nothing runs before the user answers");
    assert_eq!(asks(), [Note::Ask { profile: f.profile, tab: f.tab, install: INSTALL.into(), host: "127.0.0.1".into() }]);
    assert!(frame(&f, "http://127.0.0.1/again").is_empty());
    assert!(asks().is_empty(), "the prompt shows once per session");
    assert!(frame(&f, "https://other.test/").is_empty() && asks().is_empty(), "a host the extension does not want never asks");
    assert!(answer(f.profile, INSTALL, "127.0.0.1", true, true, f.tab));
    assert_eq!(f.core.disk.extension_hosts(f.profile, INSTALL).unwrap(), [("127.0.0.1".to_owned(), true)]);
    let a = attach(&persistent(&f), f.tab, ShieldLevel::Standard).unwrap();
    assert_eq!(frame(&f, "http://127.0.0.1:8080/page"), [0]);
    assert_eq!(a.rules.len(), 1, "the granted host gets the install's rules");
    assert!(a.rules[0].0.starts_with("tarn-ext-fixture01-") && a.rules[0].1.contains("blocked"), "{:?}", a.rules);
}

#[test]
fn private_burner_and_lockdown_views_get_nothing() {
    let f = fixture("file");
    answer(f.profile, INSTALL, "127.0.0.1", true, true, f.tab);
    assert!(attach(&StoreKey::Ephemeral("s".into()), f.tab, ShieldLevel::Standard).is_none());
    assert!(attach(&StoreKey::Burner("0123456789abcdef0123456789abcdef".into()), f.tab, ShieldLevel::Standard).is_none());
    assert!(attach_tab(&persistent(&f), &format!("true-{}", f.tab), ShieldLevel::Standard).is_none());
    assert!(attach(&persistent(&f), f.tab, ShieldLevel::Lockdown).is_none(), "a Lockdown view runs no extension");
    f.core.disk.set_site_override(f.profile, "127.0.0.1", Some(ShieldLevel::Lockdown)).unwrap();
    if let Some(rt) = f.core.profiles.borrow().get(&(false, f.profile)) {
        rt.overrides.borrow_mut().insert("127.0.0.1".into(), ShieldLevel::Lockdown);
    }
    assert!(attach(&persistent(&f), f.tab, ShieldLevel::Standard).is_some());
    assert!(frame(&f, "http://127.0.0.1/page").is_empty(), "a host at Lockdown is denied even when granted");
}

#[test]
fn a_changed_tree_turns_the_install_off_and_drops_its_grants() {
    let f = fixture("file");
    answer(f.profile, INSTALL, "127.0.0.1", true, true, f.tab);
    std::fs::write(f.path.join("content.js"), "document.title = 'tampered';").unwrap();
    load_now(&f.core, f.profile);
    let row = f.core.disk.extensions(f.profile).unwrap().remove(0);
    assert!(!row.enabled);
    assert!(f.core.disk.extension_hosts(f.profile, INSTALL).unwrap().is_empty());
    assert!(attach(&persistent(&f), f.tab, ShieldLevel::Standard).is_none());
}

fn post(f: &Fixture, json: Value) {
    message(f.profile, f.tab, "tarn-ext-fixture01", &json.to_string());
}

#[test]
fn content_scripts_store_only_on_granted_pages() {
    let f = fixture("file");
    post(&f, json!({"id": 1, "call": "storage.set", "args": [{"seen": "/page"}]}));
    assert!(f.core.disk.extension_storage(f.profile, INSTALL).unwrap().is_empty(), "an ungranted page's world is not heard");
    answer(f.profile, INSTALL, "127.0.0.1", true, false, f.tab);
    post(&f, json!({"id": 2, "call": "storage.set", "args": [{"seen": "/page"}]}));
    assert_eq!(f.core.disk.extension_storage(f.profile, INSTALL).unwrap(), [("seen".to_owned(), "\"/page\"".to_owned())]);
    post(&f, json!({"id": 3, "call": "tabs.query", "args": [{}]}));
    post(&f, json!({"id": 4, "call": "storage.set", "args": [{"other": 1}], "x": "y".repeat(2 << 20)}));
    assert_eq!(f.core.disk.extension_storage(f.profile, INSTALL).unwrap().len(), 1, "too large a message is dropped");
    tab_closed(f.tab);
    assert!(attach(&persistent(&f), f.tab, ShieldLevel::Standard).is_some());
    assert!(frame(&f, "http://127.0.0.1/page").is_empty(), "a Once grant ends with its tab");
}

#[test]
fn the_toolbar_shows_the_action_and_notes_open_tabs() {
    let f = fixture("official:fixture");
    let buttons = buttons(f.profile, Some(f.tab));
    assert_eq!(buttons.len(), 1);
    assert_eq!((buttons[0].name.as_str(), buttons[0].state.title.as_str(), buttons[0].popup), ("Tarn Fixture", "Fixture", true));
    assert_eq!(asking(f.profile, INSTALL).unwrap().badge, "Official");
    with(|rt| rt.sent.clear());
    carry_out(vec![Effect::OpenTab { profile: f.profile, url: Url::parse("https://a.test/").unwrap(), active: true }]);
    assert_eq!(with(|rt| rt.sent.clone()), [Note::OpenTab { profile: f.profile, url: Url::parse("https://a.test/").unwrap(), active: true }]);
    let _ = &f.dir;
}

#[test]
fn forgetting_site_answers_ends_this_sessions_answers_too() {
    let f = fixture("file");
    assert!(answer(f.profile, INSTALL, "127.0.0.1", true, false, f.tab));
    assert_eq!(frame(&f, "http://127.0.0.1/page"), [0]);
    asks();
    forget_answers(f.profile, INSTALL).unwrap();
    assert!(frame(&f, "http://127.0.0.1/page").is_empty(), "an Allow Once of this session is forgotten too");
    assert_eq!(asks().len(), 1, "it asks again");
    answer(f.profile, INSTALL, "127.0.0.1", true, true, f.tab);
    forget_answers(f.profile, INSTALL).unwrap();
    assert!(frame(&f, "http://127.0.0.1/page").is_empty(), "a remembered Allow is forgotten at once");
    assert!(f.core.disk.extension_hosts(f.profile, INSTALL).unwrap().is_empty());
}

fn set_profile_level(f: &Fixture, level: ShieldLevel) {
    f.core.disk.set_profile_shield_level(f.profile, level).unwrap();
    f.core.profiles.borrow_mut().get_mut(&(false, f.profile)).unwrap().profile.shield_level = level;
}

fn set_site_level(f: &Fixture, host: &str, level: Option<ShieldLevel>) {
    f.core.disk.set_site_override(f.profile, host, level).unwrap();
    let profiles = f.core.profiles.borrow();
    let mut overrides = profiles.get(&(false, f.profile)).unwrap().overrides.borrow_mut();
    match level {
        Some(level) => overrides.insert(host.into(), level),
        None => overrides.remove(host),
    };
}

fn sent() -> Vec<Note> {
    with(|rt| std::mem::take(&mut rt.sent))
}

#[test]
fn no_extension_page_opens_at_a_lockdown_profile() {
    let f = fixture("official:fixture");
    set_profile_level(&f, ShieldLevel::Lockdown);
    assert_eq!(open_popup(f.profile, INSTALL), None, "no popup opens");
    carry_out(vec![Effect::StartBackground { profile: f.profile, install_id: INSTALL.into() }]);
    assert_eq!(with(|rt| rt.rt.pages().count()), 0, "no background page starts");
    set_profile_level(&f, ShieldLevel::Standard);
    assert!(with(|rt| rt.page_spec(f.profile, INSTALL, PageKind::Popup).is_some() && rt.page_spec(f.profile, INSTALL, PageKind::Background).is_some()));
}

#[test]
fn pages_already_open_close_when_the_profile_moves_into_lockdown() {
    let f = fixture("official:fixture");
    assert!(with(|rt| rt.rt.page_opened(f.profile, INSTALL, PageKind::Background, Page(81), Instant::now())).is_none());
    assert!(with(|rt| rt.rt.page_opened(f.profile, INSTALL, PageKind::Popup, Page(82), Instant::now())).is_none());
    sent();
    set_profile_level(&f, ShieldLevel::Lockdown);
    levels_changed(f.profile);
    assert_eq!(with(|rt| rt.rt.pages().count()), 0);
    let notes = sent();
    assert!(notes.contains(&Note::PageClosed(81)) && notes.contains(&Note::PageClosed(82)), "{notes:?}");
}

#[test]
fn forgetting_answers_without_the_runtime_is_an_error() {
    let f = fixture("file");
    shut_down();
    assert!(forget_answers(f.profile, INSTALL).is_err(), "nothing was forgotten, so it does not report success");
}

#[test]
fn a_changed_level_works_the_rules_out_again() {
    let f = fixture("file");
    answer(f.profile, INSTALL, "127.0.0.1", true, true, f.tab);
    let rules = || attach(&persistent(&f), f.tab, ShieldLevel::Standard).map(|a| a.rules.len());
    assert_eq!(rules(), Some(1));
    set_site_level(&f, "127.0.0.1", Some(ShieldLevel::Lockdown));
    levels_changed(f.profile);
    assert_eq!(rules(), Some(0), "a host set to Lockdown takes no extension rules");
    set_site_level(&f, "127.0.0.1", None);
    levels_changed(f.profile);
    assert_eq!(rules(), Some(1));
    set_profile_level(&f, ShieldLevel::Lockdown);
    levels_changed(f.profile);
    assert_eq!(rules(), Some(0), "a Lockdown profile takes none");
    set_profile_level(&f, ShieldLevel::Standard);
    levels_changed(f.profile);
    assert_eq!(rules(), Some(1), "leaving Lockdown brings them back");
}

#[test]
fn a_page_the_runtime_closes_or_that_fails_leaves_the_ui() {
    let f = fixture("official:fixture");
    assert!(with(|rt| rt.rt.page_opened(f.profile, INSTALL, PageKind::Background, Page(78), Instant::now())).is_none());
    sent();
    on_event(Event::PageFailed { page: 78 });
    assert!(with(|rt| rt.rt.page(f.profile, INSTALL, PageKind::Background).is_none()), "a page that failed is gone");
    assert!(sent().contains(&Note::PageClosed(78)));
    assert!(with(|rt| rt.rt.page_opened(f.profile, INSTALL, PageKind::Popup, Page(77), Instant::now())).is_none());
    let mut row = f.core.disk.extensions(f.profile).unwrap().remove(0);
    row.enabled = false;
    f.core.disk.put_extension(&row).unwrap();
    reload_now(&f);
    assert!(sent().contains(&Note::PageClosed(77)), "a popup the runtime closes leaves the panel");
}
