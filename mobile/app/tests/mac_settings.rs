use nsh_core::geometry::{Bounds, Points, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

fn bounds_of(app: &Headless, selector: &str) -> Bounds<Points> {
    let node = app.screen().press_target(selector).unwrap().unwrap();
    app.screen().layout_nodes().into_iter().find(|n| n.id == node).map(|n| n.bounds).unwrap()
}

#[test]
fn the_mac_settings_window_shows_one_pane_at_a_time() {
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
    app.wait_for("label:Touch").unwrap();
    assert!(!app.screen().shows("Search the text of visited pages"), "only the Appearance pane shows");

    app.press("label:Search").unwrap();
    app.wait_for("Search the text of visited pages").unwrap();
    app.wait_for_gone("label:Touch", WAIT).unwrap();

    app.press("label:Shortcuts").unwrap();
    app.wait_for("Open location").unwrap();
    for row in ["Pin or unpin tab", "Toggle sidebar", "Select tab 1–8", "Switch to space 1–9", "Close command bar, popover or Settings"] {
        app.wait_for(row).unwrap();
    }
    assert!(app.screen().shows("⌘D") || app.screen().shows("Ctrl+D"), "rows show the shared table's keys");

    app.press("label:Profile").unwrap();
    app.wait_for("label:Change Design").unwrap();
    for part in ["label:Flip", "label:Share Card", "parameters removed", "HTTPS upgrades", "local visits blocked"] {
        app.wait_for(part).unwrap();
    }
    assert!(!app.screen().shows("Parameters removed"), "the card's numbers show once, beside the card");
    assert_eq!(app.screen().texts().iter().filter(|t| t.starts_with("Protected for ")).count(), 2, "the card and the column beside it say since when");
    let (card, flip) = (bounds_of(&app, "label:/^Shield card of/"), bounds_of(&app, "label:Flip"));
    assert!(card.origin.x.0 + card.size.width.0 <= flip.origin.x.0, "the actions sit beside the card in a wide pane: {card:?} {flip:?}");
    let design = bounds_of(&app, "label:Change Design");
    assert!(
        (design.origin.y.0 - flip.origin.y.0).abs() < 0.5 && (design.size.width.0 - flip.size.width.0).abs() < 0.5,
        "Flip and Change Design share a row: {flip:?} {design:?}"
    );
    let numbers: Vec<_> =
        app.screen().layout_nodes().into_iter().filter(|n| n.test_id.as_deref() == Some("profile-number") && !n.hidden).map(|n| n.bounds).collect();
    assert_eq!(numbers.len(), 3);
    assert!(numbers.iter().all(|b| (b.origin.y.0 - numbers[0].origin.y.0).abs() < 0.5), "the three numbers share one row: {numbers:?}");
    let levels = bounds_of(&app, "label:Lockdown");
    let below = numbers.iter().map(|b| b.origin.y.0 + b.size.height.0).fold(0., f32::max);
    assert!(levels.origin.x.0 >= card.origin.x.0 + card.size.width.0, "the Shield level sits in the column beside the card: {card:?} {levels:?}");
    assert!(levels.origin.y.0 >= below && levels.origin.y.0 + levels.size.height.0 <= flip.origin.y.0, "under the numbers, above the actions: {levels:?}");
    assert_eq!(app.screen().texts().iter().filter(|t| **t == "Shield level").count(), 1, "no second Shield level section");
    assert!(!app.screen().shows("Counted on this device only and never sent anywhere."), "the header badge already says it");
    app.press("label:Strict").unwrap();
    app.wait_until("Strict is the profile's level", WAIT, |s| s.selected("label:Strict").unwrap_or(false)).unwrap();
    app.press("label:Standard").unwrap();
    app.set_viewport(Size::new(Points(834.), Points(1194.))).unwrap();
    app.wait_until("the hero stacks in a narrow pane", WAIT, |s| {
        let found = |sel: &str| s.press_target(sel).ok().flatten().and_then(|id| s.layout_nodes().into_iter().find(|n| n.id == id).map(|n| n.bounds));
        matches!((found("label:/^Shield card of/"), found("label:Flip")), (Some(c), Some(f)) if f.origin.y.0 >= c.origin.y.0 + c.size.height.0)
    })
    .unwrap();
    app.set_viewport(Size::new(Points(1376.), Points(1032.))).unwrap();

    app.press("label:Privacy").unwrap();
    app.wait_for("Clear Browsing Data…").unwrap();

    app.press("label:General").unwrap();
    app.wait_for("Open-source licences").unwrap();
    app.press("Open-source licences").unwrap();
    app.wait_for("Open-Source Licences").unwrap();
    assert!(app.screen().matches("label:Close Settings").unwrap(), "licences open inside the Settings window");
    app.back().unwrap();
    app.wait_for("Sort downloads into folders").unwrap();
    assert!(app.screen().matches("label:Close Settings").unwrap(), "Back steps from the licences to the pane, not out of Settings");
    app.press("Open-source licences").unwrap();
    app.wait_for("Open-Source Licences").unwrap();
    app.press("label:Back").unwrap();
    app.wait_for("Sort downloads into folders").unwrap();

    app.press("label:Extensions").unwrap();
    app.wait_for_gone("Sort downloads into folders", WAIT).unwrap();
    assert!(app.screen().matches("label:Close Settings").unwrap(), "Extensions is a pane of the Settings window");

    app.press_key("escape").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();
    app.press_key("mod+,").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.press("label:Appearance").unwrap();
    app.wait_for("label:Touch").unwrap();
    app.press("label:Touch").unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    assert!(!app.screen().matches("label:Close Settings").unwrap(), "Touch goes back to the settings sheet");
    app.wait_for("Search the text of visited pages").unwrap();
}
