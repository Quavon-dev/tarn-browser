use nsh_core::ViewKind;
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

#[test]
fn mac_popovers_anchor_under_their_buttons_and_close() {
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
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();

    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", "example.com").unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for("label:Site settings: Shield Standard, 0 stopped on this site").unwrap();
    let page_shown = |app: &Headless| app.screen().layout_nodes().iter().any(|n| n.kind == ViewKind::Native && !n.hidden && n.bounds.size.height.0 > 0.);
    assert!(page_shown(&app));
    app.press("label:Site settings: Shield Standard, 0 stopped on this site").unwrap();
    app.wait_for("SHIELD FOR THIS SITE").unwrap();
    assert!(page_shown(&app), "the page stays visible behind the Shield popover");
    assert!(!app.screen().shows("Done"), "the touch sheet gives way to the popover");
    let (chip, card) = (bounds_of(&app, "label:Site settings: Shield Standard, 0 stopped on this site"), bounds(&app, "popover"));
    assert_eq!(card.size.width, Points(340.));
    let levels = bounds(&app, "site-levels");
    assert!((levels.size.width.0 - (card.size.width.0 - 26.)).abs() < 1., "the Shield levels span the card like the rows below: {levels:?} {card:?}");
    let last = bounds_of(&app, "label:Lockdown");
    assert!((levels.origin.x.0 + levels.size.width.0 - (last.origin.x.0 + last.size.width.0)).abs() < 3., "the segments fill their track: {levels:?} {last:?}");
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
    assert!(page_shown(&app), "the page stays visible behind Downloads");
    app.press_key("escape").unwrap();
    app.wait_for_gone("No downloads yet", WAIT).unwrap();

    app.press("label:Share").unwrap();
    app.wait_for("label:Show QR Code").unwrap();
    assert!(page_shown(&app), "the page stays visible behind Share");
    app.press("label:Show QR Code").unwrap();
    app.wait_for("Scan to open on a phone").unwrap();
    assert!(page_shown(&app), "the page stays visible behind the QR code");
    app.press_key("escape").unwrap();
    app.wait_for_gone("Scan to open on a phone", WAIT).unwrap();
    app.press_key("mod+,").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    assert!(page_shown(&app), "the page stays visible behind the Settings window");
    app.press_key("escape").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();
    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    assert!(page_shown(&app), "the page stays visible behind the command palette");
    app.press_key("escape").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();

    assert_eq!(app.screen().count("label:Close Tab").unwrap(), 1);
    app.long_press("label:example.com").unwrap();
    app.wait_for("label:Copy Clean Link").unwrap();
    assert_eq!(app.screen().count("label:Close Tab").unwrap(), 2, "the tab menu ends with Close Tab");
    app.press_key("escape").unwrap();
    app.wait_for_gone("label:Copy Clean Link", WAIT).unwrap();

    let mut touch = None;
    app.set_environment("pointer", |e| {
        touch = Some((e.os, e.idiom));
        (e.os, e.idiom) = (Os::MacOs, Idiom::Desktop);
    })
    .unwrap();
    app.hover("label:example.com").unwrap();
    app.long_press("label:example.com").unwrap();
    app.wait_for("label:example.com").unwrap();
    assert!(!app.screen().matches("label:Copy Clean Link").unwrap(), "a held trackpad click opens the tab, not its menu");
    app.set_environment("touch", |e| (e.os, e.idiom) = touch.unwrap()).unwrap();
}
