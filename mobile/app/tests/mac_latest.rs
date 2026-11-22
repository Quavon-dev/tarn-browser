use nsh_core::geometry::{Bounds, Points, Size};
use nsh_core::platform::{Idiom, Os};
use nsh_core::time::Clock;
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
fn the_mac_layout_follows_the_latest_macos_sidebar_settings_and_cards() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut config = EngineConfig::new(Root::of::<tarn_mobile::App>());
    config.clock = Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).unwrap();
    app.set_viewport(Size::new(Points(1376.), Points(1032.))).unwrap();
    app.wait_for("label:Menu").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("label:Mac").unwrap();
    app.press("label:Mac").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.press("label:General").unwrap();
    app.wait_for("Profiles").unwrap();
    app.wait_for("This window").unwrap();
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();

    app.wait_for("label:/^Space /").unwrap();
    assert!(!app.screen().matches("label:Personal").unwrap(), "the sidebar header has no profile button");

    app.press("label:Shield: Standard").unwrap();
    app.wait_for("label:Strict").unwrap();
    app.press("label:Strict").unwrap();
    app.wait_for("label:Shield: Strict").unwrap();

    app.press("label:New Space").unwrap();
    app.wait_for("Create Space").unwrap();
    let (plus, card) = (bounds_of(&app, "label:New Space"), bounds(&app, "popover"));
    assert!(card.origin.y.0 + card.size.height.0 <= plus.origin.y.0, "the space card opens above +: {plus:?} {card:?}");
    app.press_key("escape").unwrap();
    app.wait_for_gone("Create Space", WAIT).unwrap();
    app.press_key("mod+shift+s").unwrap();
    app.wait_for("Create Space").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("Create Space", WAIT).unwrap();
    app.press_key("mod+s").unwrap();
    app.wait_for_gone("label:Organize Tabs", WAIT).unwrap();
    app.press("label:Home").unwrap();
    app.press("label:New Space…").unwrap();
    app.wait_for("Create Space").unwrap();
    let (toggle, card) = (bounds_of(&app, "label:Show Sidebar"), bounds(&app, "popover"));
    assert!(card.origin.y.0 >= toggle.origin.y.0 + toggle.size.height.0, "with the sidebar hidden the card hangs from the sidebar toggle: {toggle:?} {card:?}");
    app.press_key("escape").unwrap();
    app.wait_for_gone("Create Space", WAIT).unwrap();
    app.press_key("mod+s").unwrap();
    app.wait_for("label:Organize Tabs").unwrap();

    app.press("label:Organize Tabs").unwrap();
    app.press("label:New Folder").unwrap();
    app.wait_for("label:Folder 1").unwrap();
    app.long_press("label:Folder 1").unwrap();
    app.press("label:Change Icon…").unwrap();
    app.wait_for("label:Emoji").unwrap();
    app.type_text("label:Emoji", "🚀").unwrap();
    app.submit("label:Emoji").unwrap();
    app.wait_for("🚀").unwrap();
    app.long_press("label:Folder 1").unwrap();
    app.press("label:Change Icon…").unwrap();
    app.press("label:Remove").unwrap();
    app.wait_for_gone("🚀", WAIT).unwrap();

    open(&mut app, "https://example.com/");
    open(&mut app, "https://gitlab.com/b");
    open(&mut app, "https://github.com/a");
    app.press("label:Organize Tabs").unwrap();
    app.press("label:Clean Up").unwrap();
    app.wait_for("Sorted 2 tabs into groups").unwrap();
    app.wait_for("label:Development").unwrap();
    app.press("label:Undo Sort").unwrap();
    app.wait_for_gone("label:Development", WAIT).unwrap();
    open(&mut app, "https://example.org/");

    let mut touch = None;
    app.set_environment("pointer", |e| {
        touch = Some((e.os, e.idiom));
        (e.os, e.idiom) = (Os::MacOs, Idiom::Desktop);
    })
    .unwrap();
    let before = app.screen().count("label:Close Tab").unwrap();
    app.hover("label:github.com").unwrap();
    app.wait_until("a close button on the hovered row", WAIT, |s| s.count("label:Close Tab").unwrap_or(0) == before + 1).unwrap();
    app.press("label:github.com >> label:Close Tab").unwrap();
    app.wait_for_gone("label:github.com", WAIT).unwrap();
    assert_eq!(app.screen().count("label:Close Tab").unwrap(), before + 1, "the row that slid under the pointer shows its close button at once");
    app.press("label:gitlab.com >> label:Close Tab").unwrap();
    app.wait_for_gone("label:gitlab.com", WAIT).unwrap();
    assert_eq!(app.screen().count("label:Close Tab").unwrap(), before + 1, "closing tab after tab in place keeps a close button under the pointer");
    app.set_environment("touch", |e| (e.os, e.idiom) = touch.unwrap()).unwrap();
}
