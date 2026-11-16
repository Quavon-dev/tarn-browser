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

fn load_now(core: &Core<Data>, profile: Id) {
    let seed = core.disk.profile(profile).unwrap().data_store_uuid;
    let rows = core.disk.extensions(profile).unwrap().into_iter().filter(|r| r.enabled).collect();
    apply(profile, &seed, runtime::load_rows(rows, runtime::row_dir, &seed, &["en".to_owned()]));
}

fn attach(key: &StoreKey, tab: u64, level: ShieldLevel) -> Option<Attach> {
    with(|rt| rt.attach(&ViewInfo { key, tab: &tab_prop(tab), level })).map(|(_, a)| a)
}

fn persistent(f: &Fixture) -> StoreKey {
    StoreKey::Persistent(f.core.disk.profile(f.profile).unwrap().data_store_uuid)
}

fn frame(gate: &engine::tarn_ext::Gate, url: &str) -> Vec<usize> {
    let world = World::for_install(INSTALL).unwrap();
    let url = Url::parse(url).unwrap();
    gate(&world, &FrameNav { url: &url, top: &url, main_frame: true })
}

fn asks() -> Vec<Note> {
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
    assert!(frame(&a.gate, "http://127.0.0.1:8080/page").is_empty(), "nothing runs before the user answers");
    assert_eq!(asks(), [Note::Ask { profile: f.profile, tab: f.tab, install: INSTALL.into(), host: "127.0.0.1".into() }]);
    assert!(frame(&a.gate, "http://127.0.0.1/again").is_empty());
    assert!(asks().is_empty(), "the prompt shows once per session");
    assert!(frame(&a.gate, "https://other.test/").is_empty() && asks().is_empty(), "a host the extension does not want never asks");
    assert!(answer(f.profile, INSTALL, "127.0.0.1", true, true, f.tab));
    assert_eq!(f.core.disk.extension_hosts(f.profile, INSTALL).unwrap(), [("127.0.0.1".to_owned(), true)]);
    let a = attach(&persistent(&f), f.tab, ShieldLevel::Standard).unwrap();
    assert_eq!(frame(&a.gate, "http://127.0.0.1:8080/page"), [0]);
    assert_eq!(a.rules.len(), 1, "the granted host gets the install's rules");
    assert!(a.rules[0].0.starts_with("tarn-ext-fixture01-") && a.rules[0].1.contains("blocked"), "{:?}", a.rules);
}

#[test]
fn private_burner_and_lockdown_views_get_nothing() {
    let f = fixture("file");
    answer(f.profile, INSTALL, "127.0.0.1", true, true, f.tab);
    assert!(attach(&StoreKey::Ephemeral("s".into()), f.tab, ShieldLevel::Standard).is_none());
    assert!(attach(&StoreKey::Burner("0123456789abcdef0123456789abcdef".into()), f.tab, ShieldLevel::Standard).is_none());
    assert!(with(|rt| rt.attach(&ViewInfo { key: &persistent(&f), tab: &format!("true-{}", f.tab), level: ShieldLevel::Standard })).is_none());
    assert!(attach(&persistent(&f), f.tab, ShieldLevel::Lockdown).is_none(), "a Lockdown view runs no extension");
    f.core.disk.set_site_override(f.profile, "127.0.0.1", Some(ShieldLevel::Lockdown)).unwrap();
    if let Some(rt) = f.core.profiles.borrow().get(&(false, f.profile)) {
        rt.overrides.borrow_mut().insert("127.0.0.1".into(), ShieldLevel::Lockdown);
    }
    let a = attach(&persistent(&f), f.tab, ShieldLevel::Standard).unwrap();
    assert!(frame(&a.gate, "http://127.0.0.1/page").is_empty(), "a host at Lockdown is denied even when granted");
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
    let a = attach(&persistent(&f), f.tab, ShieldLevel::Standard).unwrap();
    assert!(frame(&a.gate, "http://127.0.0.1/page").is_empty(), "a Once grant ends with its tab");
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
    assert_eq!(with(|rt| rt.sent.clone()), [Note::OpenTab { url: Url::parse("https://a.test/").unwrap(), active: true }]);
    let _ = &f.dir;
}
