use nsh_core::geometry::{Bounds, Points, Size};
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

#[test]
fn mac_popovers_anchor_under_their_buttons_and_close() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut app = Headless::start(EngineConfig::new(Root::of::<tarn_mobile::App>())).unwrap();
    app.set_viewport(Size::new(Points(1376.), Points(1032.))).unwrap();
    app.wait_for("label:Menu").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("label:Mac").unwrap();
    app.press("label:Mac").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();

    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", "example.com").unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for("label:Site settings: Shield Standard, 0 stopped on this site").unwrap();
    app.press("label:Site settings: Shield Standard, 0 stopped on this site").unwrap();
    app.wait_for("SHIELD FOR THIS SITE").unwrap();
    assert!(!app.screen().shows("Done"), "the touch sheet gives way to the popover");
    let (chip, card) = (bounds_of(&app, "label:Site settings: Shield Standard, 0 stopped on this site"), bounds(&app, "popover"));
    assert_eq!(card.size.width, Points(340.));
    assert!((card.origin.y.0 - (chip.origin.y.0 + chip.size.height.0 + 4.)).abs() < 0.5, "4 pt under the chip: {chip:?} {card:?}");
    let centre = |b: Bounds<Points>| b.origin.x.0 + b.size.width.0 / 2.;
    assert!((centre(card) - centre(chip)).abs() < 0.5 || card.origin.x.0 <= 8.5, "centred under the chip: {chip:?} {card:?}");
    app.press_key("escape").unwrap();
    app.wait_for_gone("SHIELD FOR THIS SITE", WAIT).unwrap();

    app.wait_for("label:Downloads").unwrap();
    app.press("label:Downloads").unwrap();
    app.wait_for("No downloads yet").unwrap();
    let (button, card) = (bounds_of(&app, "label:Downloads"), bounds(&app, "popover"));
    assert_eq!(card.size.width, Points(320.));
    assert!((card.origin.y.0 - (button.origin.y.0 + button.size.height.0 + 4.)).abs() < 0.5, "4 pt under the button: {button:?} {card:?}");
    assert!(!app.screen().shows("Done"));
    app.press_key("escape").unwrap();
    app.wait_for_gone("No downloads yet", WAIT).unwrap();

    assert_eq!(app.screen().count("label:Close Tab").unwrap(), 1);
    app.long_press("label:https://example.com/").unwrap();
    app.wait_for("label:Copy Clean Link").unwrap();
    assert_eq!(app.screen().count("label:Close Tab").unwrap(), 2, "the tab menu ends with Close Tab");
    app.press_key("escape").unwrap();
    app.wait_for_gone("label:Copy Clean Link", WAIT).unwrap();
}
