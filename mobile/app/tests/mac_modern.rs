use nsh_core::geometry::{Bounds, Points, Size};
use nsh_core::platform::{Idiom, Os};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

fn bounds(app: &Headless, id: &str) -> Bounds<Points> {
    app.screen().layout_nodes().into_iter().find(|n| n.test_id.as_deref() == Some(id) && !n.hidden).map(|n| n.bounds).unwrap()
}

fn bounds_of(app: &Headless, selector: &str) -> Bounds<Points> {
    let node = app.screen().press_target(selector).unwrap().unwrap();
    app.screen().layout_nodes().into_iter().find(|n| n.id == node).map(|n| n.bounds).unwrap()
}

fn open(app: &mut Headless, url: &str) {
    app.press_key("mod+t").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", url).unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    app.wait_for(&format!("label:{}", url::Url::parse(url).unwrap().host_str().unwrap())).unwrap();
}

#[test]
fn the_modern_sidebar_has_tidy_cards_and_a_footer_with_the_space_and_profile_menus() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut config = EngineConfig::new(Root::of::<tarn_mobile::App>());
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).unwrap();
    app.set_viewport(Size::new(Points(1376.), Points(1032.))).unwrap();
    app.wait_for("label:Menu").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("label:Mac").unwrap();
    app.press("label:Mac").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    for option in ["Pinned sites", "Groups and folders", "label:Chats: On", "label:Mark changed tabs: On", "label:Dim sleeping tabs: Off"] {
        app.wait_for(option).unwrap();
    }
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();

    for part in ["label:Tidy", "label:/^Space /", "label:Downloads", "label:/^Profile /"] {
        app.wait_for(part).unwrap();
    }
    assert!(!app.screen().matches("label:Organize Tabs").unwrap(), "Tidy and its menu stand in for the organise button");
    assert!(!app.screen().shows("Chats"), "no chats while the profile's AI is off");

    app.press("label:Downloads").unwrap();
    app.wait_for("No downloads yet").unwrap();
    let (button, card) = (bounds_of(&app, "label:Downloads"), bounds(&app, "popover"));
    assert!(card.origin.y.0 + card.size.height.0 <= button.origin.y.0, "the footer's pop-overs open upwards: {button:?} {card:?}");
    app.press_key("escape").unwrap();
    app.wait_for_gone("No downloads yet", WAIT).unwrap();

    app.press("label:/^Space /").unwrap();
    app.wait_for("label:Edit Space…").unwrap();
    assert!(app.screen().matches("label:Home >> nth=1").unwrap(), "the switcher lists the spaces");
    app.press_key("escape").unwrap();
    app.wait_for_gone("label:Edit Space…", WAIT).unwrap();

    app.press("label:/^Profile /").unwrap();
    for entry in ["label:Settings…", "label:Show Library", "label:New Space…"] {
        app.wait_for(entry).unwrap();
    }
    app.press("label:Edit Profile…").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();

    open(&mut app, "https://example.com/");
    open(&mut app, "https://gitlab.com/b");
    open(&mut app, "https://github.com/a");
    app.press("label:Tidy").unwrap();
    app.wait_for("Sorted 2 tabs into groups").unwrap();
    app.wait_for("label:Development").unwrap();
    assert!(bounds_of(&app, "label:example.com").origin.y.0 < bounds_of(&app, "label:Development").origin.y.0, "loose tabs come before the group cards");
    app.press("label:Development").unwrap();
    app.wait_until("a closed group shows how many tabs it holds", WAIT, |s| s.shows("2")).unwrap();
    assert!(!app.screen().matches("label:Close Group and Its Tabs").unwrap(), "a closed card shows its summary in place of Close");

    app.long_press("label:Tidy").unwrap();
    app.press("label:New Folder").unwrap();
    app.wait_for("label:Folder 1").unwrap();
    let mut touch = None;
    app.set_environment("pointer", |e| {
        touch = Some((e.os, e.idiom));
        (e.os, e.idiom) = (Os::MacOs, Idiom::Desktop);
    })
    .unwrap();
    app.hover("label:example.com").unwrap();
    if !app.screen().shows("Drag tabs here") {
        app.press("label:Folder 1").unwrap();
    }
    app.wait_for("Drag tabs here").expect("an open, empty folder card offers itself as a drop target");
    app.set_environment("touch", |e| (e.os, e.idiom) = touch.unwrap()).unwrap();

    app.press_key("mod+shift+n").unwrap();
    app.wait_until("a private space", WAIT, |s| s.shows("Private")).unwrap();
    app.press("label:/^Profile /").unwrap();
    app.wait_for("label:Show Library").unwrap();
    assert!(!app.screen().matches("label:Edit Profile…").unwrap(), "a private space's profile is not edited from here");
}
