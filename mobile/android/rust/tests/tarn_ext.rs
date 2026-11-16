use std::path::PathBuf;
use std::rc::Rc;

use browser_core::tarn_ext::{self, Access, Context, Resolver, resources::PAGE_CSP};
use shield::ShieldLevel;
use shield::dnr::Scope;
use tarn_mobile_android::tarn_ext::{self as android, Phase};

const STORE: &str = "6f1c2a9e-3b7d-4c55-9a0e-1d2f3a4b5c6d";
const NORMAL: Context = Context { private: false, burner: false, profile_level: ShieldLevel::Standard, view_level: ShieldLevel::Standard };
const SCRIPT: i32 = 4;
const ALLOW: i32 = 0;
const BLOCK: i32 = 1;

fn fixture() -> tarn_ext::Package {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tarn-ext");
    tarn_ext::load(&dir, None, "fx1", STORE, &[]).expect("the fixture loads")
}

#[test]
fn the_fixture_runs_through_the_android_engine_as_the_core_decides() {
    let package = Rc::new(fixture());
    let world = package.world.name().to_owned();
    let gated = package.clone();
    android::set_gate(Rc::new(move |_, _, world, nav| {
        if *world != gated.world {
            return Vec::new();
        }
        tarn_ext::frame_decision(&gated, &NORMAL, nav, |host| if host == "page.test" { Access::Granted } else { Access::Ask }).run
    }));
    android::set_scripts(STORE, vec![Rc::new(package.scripts.clone())]);

    let top = android::frame("n1", STORE, &world, "https://page.test/a", None, true, "https://page.test");
    let phases: Vec<Phase> = top.iter().map(|u| u.0).collect();
    assert_eq!(phases, [Phase::Start, Phase::Start, Phase::Start, Phase::Idle]);
    assert!(top[0].1.contains("__tarnExtPost") && top[0].1.contains(&package.origin_id), "the bootstrap of this install comes first");
    assert!(top[1].1.contains("rgb(0, 128, 0)"));
    assert!(top[2].1.contains("tarnFixture = 'content'") && top[3].1.contains("tarnFixtureFrame"));

    let excluded = android::frame("n1", STORE, &world, "https://page.test/private/x", None, true, "https://page.test");
    assert_eq!(excluded.iter().map(|u| u.0).collect::<Vec<_>>(), [Phase::Start, Phase::Idle], "exclude_matches drops the CSS and its script");

    let frame = android::frame("n1", STORE, &world, "https://page.test/frame", Some("https://page.test/a"), false, "https://page.test");
    assert_eq!(frame.len(), 2, "a subframe gets only the all_frames script");
    assert!(android::frame("n1", STORE, &world, "https://other.test/", None, true, "https://other.test").is_empty());
    assert!(android::frame("n1", STORE, &world, "https://page.test/a", None, true, "https://other.test").is_empty(), "the engine's origin wins");
}

#[test]
fn the_fixtures_rules_block_only_on_granted_sites() {
    let package = fixture();
    let list = tarn_ext::rule_list(&package, &NORMAL, &Scope::Hosts(vec!["page.test".into()])).expect("a rule list");
    let lockdown = Context { view_level: ShieldLevel::Lockdown, ..NORMAL };
    assert!(tarn_ext::rule_list(&package, &lockdown, &Scope::Every { except: Vec::new() }).is_none());
    android::set_rule_lists("rules-store", vec![(package.install_id.clone(), list)]);
    assert_eq!(android::verdict("rules-store", "https://ads.test/x.js", SCRIPT, &[(Some("https://page.test/"), 0)]), BLOCK);
    assert_eq!(android::verdict("rules-store", "https://ads.test/x.js", SCRIPT, &[(Some("https://elsewhere.test/"), 0)]), ALLOW);
    assert_eq!(android::verdict("rules-store", "https://ads.test/x.js", SCRIPT, &[(Some("https://page.test/"), 2)]), ALLOW);
}

#[test]
fn the_fixtures_pages_are_served_from_its_own_origin_only() {
    let package = fixture();
    let resolver = Resolver::default();
    package.serve(&resolver);
    android::serve("pages-store", resolver.handler());
    let (popup, csp) = android::resource("pages-store", package.popup_url().unwrap().as_str());
    assert_eq!((popup.status, popup.mime, popup.csp), (200, "text/html", Some(PAGE_CSP)));
    assert_eq!(csp.unwrap(), "script-src tarn-extension:; frame-src tarn-extension:; object-src 'none'", "the core's policy, with the scheme for 'self'");
    let (background, _) = android::resource("pages-store", package.background_url().unwrap().as_str());
    assert!(String::from_utf8(background.body).unwrap().contains("background.js"));
    let base = format!("tarn-extension://{}/", package.origin_id);
    assert_ne!(android::resource("pages-store", &format!("{base}..%2f..%2fCargo.toml")).0.status, 200);
    assert_eq!(android::resource("pages-store", "tarn-extension://0123/popup.html").0.status, 404);
    assert_eq!(android::resource("other-store", package.popup_url().unwrap().as_str()).0.status, 404);
}
