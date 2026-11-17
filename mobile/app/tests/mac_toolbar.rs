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
fn the_mac_toolbar_opens_a_command_palette() {
    let panics = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (count, report) = (panics.clone(), std::panic::take_hook());
    std::panic::set_hook(Box::new(move |info| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        report(info);
    }));
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

    for name in ["label:Back", "label:Forward", "label:Reload", "label:Open Split View", "label:Share"] {
        app.wait_for(name).unwrap();
    }
    for viewport in [Size::new(Points(1032.), Points(1376.)), Size::new(Points(1376.), Points(1032.))] {
        app.set_viewport(viewport).unwrap();
        app.wait_for("label:Reload").unwrap();
        let (reload, field) = (bounds_of(&app, "label:Reload"), bounds(&app, "capsule"));
        assert!(reload.origin.x.0 + reload.size.width.0 <= field.origin.x.0, "Reload stays left of the address: {reload:?} {field:?}");
    }
    assert!(!app.screen().matches("label:Menu").unwrap(), "the Mac toolbar has no touch menu button");

    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    for hint in ["Run", "Move", "Close"] {
        assert!(app.screen().shows(hint), "{hint}");
    }
    app.type_text("id:palette-field", "example.com").unwrap();
    app.wait_for("Open").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();

    app.press("label:Hide Sidebar").unwrap();
    app.wait_for_gone("label:Library", WAIT).unwrap();
    app.wait_for("label:Show Sidebar").unwrap();
    app.press("label:Show Sidebar").unwrap();
    app.wait_for("label:Library").unwrap();
    app.press_key("mod+s").unwrap();
    app.wait_for_gone("label:Library", WAIT).unwrap();
    app.press_key("mod+s").unwrap();
    app.wait_for("label:Library").unwrap();
    assert_eq!(panics.load(std::sync::atomic::Ordering::SeqCst), 0, "hiding and showing the sidebar panics nothing");

    app.press_key("mod+l").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", "> pin").unwrap();
    app.wait_for("label:Pin Tab").unwrap();
    assert!(app.screen().shows("⌘D") || app.screen().shows("Ctrl+D"), "a command row shows its keys");
    app.clear("id:palette-field").unwrap();
    app.type_text("id:palette-field", "> reading").unwrap();
    app.wait_for("COMMANDS").unwrap();
    app.wait_for("label:Reading List").unwrap();
    assert!(!app.screen().shows("Search DuckDuckGo"), "a > list shows commands only");
    app.press_key("down").unwrap();
    app.press_key("up").unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    app.wait_for("Reading List").unwrap();
    app.back().unwrap();

    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.press("label:Close").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();

    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", "example.com").unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    app.wait_for("example.com").unwrap();
    app.wait_for("label:Site settings: Shield Standard, 0 stopped on this site").unwrap();

    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.clear("id:palette-field").unwrap();
    app.type_text("id:palette-field", "http://a.very.long.chain.of.subdomains.in.front.of.example.com/").unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    app.wait_for("Not Secure —").unwrap();
    app.wait_until("the host ends with its registrable domain", WAIT, |s| s.texts().iter().any(|t| t.starts_with('…') && t.ends_with("example.com"))).unwrap();
    app.set_viewport(Size::new(Points(700.), Points(1000.))).unwrap();
    app.wait_for("Not Secure —").unwrap();
    assert!(bounds(&app, "capsule").size.width.0 >= 200., "the address keeps room at the narrowest Mac window: {:?}", bounds(&app, "capsule"));

    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.clear("id:palette-field").unwrap();
    app.type_text("id:palette-field", "reading").unwrap();
    app.wait_for("label:Reading List").unwrap();
    app.press_key("down").unwrap();
    app.press("Search DuckDuckGo").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    app.wait_for("label:duckduckgo.com").unwrap();
    assert!(!app.screen().shows("Reading List"), "a click on the input row searches, though a command was picked with the keys");

    app.press_key("mod+l").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.clear("id:palette-field").unwrap();
    app.type_text("id:palette-field", "> pin").unwrap();
    app.wait_for("label:Pin Tab").unwrap();
    app.press_key("mod+d").unwrap();
    app.wait_for("label:Unpin Tab").unwrap();
    app.wait_for_gone("Pin tabs from their menu", WAIT).unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    app.wait_for("Pin tabs from their menu").unwrap();
}
