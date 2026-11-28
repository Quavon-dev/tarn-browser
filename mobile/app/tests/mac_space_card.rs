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

fn inside(window: Size<Points>, b: Bounds<Points>) -> bool {
    b.origin.y.0 >= 0. && b.origin.y.0 + b.size.height.0 <= window.height.0 && b.origin.x.0 >= 0. && b.origin.x.0 + b.size.width.0 <= window.width.0
}

#[test]
fn a_tall_space_card_fits_above_its_button_and_keeps_its_footer() {
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
    // These checks read the Classic sidebar; tests/mac_modern.rs covers the Modern one.
    app.press("label:Classic >> nth=0").unwrap();
    app.wait_for("label:Organize Tabs").unwrap();
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();

    for window in [Size::new(Points(1133.), Points(744.)), Size::new(Points(744.), Points(1133.)), Size::new(Points(1376.), Points(1032.))] {
        app.set_viewport(window).unwrap();
        app.wait_for("label:New Space").unwrap();
        app.press("label:New Space").unwrap();
        app.wait_for("label:Create Space").unwrap();
        let (plus, card, create) = (bounds_of(&app, "label:New Space"), bounds(&app, "popover"), bounds_of(&app, "label:Create Space"));
        assert!(inside(window, card), "the card stays in the window {window:?}: {card:?}");
        assert!(card.origin.y.0 + card.size.height.0 <= plus.origin.y.0 + 0.5, "the card sits above +: {plus:?} {card:?}");
        assert!(inside(window, create) && create.origin.y.0 >= card.origin.y.0, "Create stays visible in the card: {create:?} {card:?}");
        app.press_key("escape").unwrap();
        app.wait_for_gone("label:Create Space", WAIT).unwrap();
    }

    app.set_viewport(Size::new(Points(1133.), Points(744.))).unwrap();
    app.press("label:New Space").unwrap();
    app.wait_for("label:Create Space").unwrap();
    app.press("label:Create Space").unwrap();
    app.wait_for_gone("label:Create Space", WAIT).unwrap();
    app.wait_for("label:Space 2").unwrap();
}
