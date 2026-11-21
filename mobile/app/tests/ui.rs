use nsh_core::geometry::{Points, Size, point, px};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::{Headless, WAIT};

fn press_in_view(app: &mut Headless, selector: &str) {
    for _ in 0..40 {
        let screen = app.screen();
        let target = screen.press_target(selector).unwrap().unwrap();
        let nodes = screen.layout_nodes();
        let at = nodes.iter().find(|n| n.id == target).unwrap().bounds;
        let (top, bottom) = (at.origin.y.0, at.origin.y.0 + at.size.height.0);
        let port = nodes
            .iter()
            .filter(|n| n.scroll.is_some() && n.bounds.size.height.0 > 0. && n.bounds.origin.x.0 <= at.origin.x.0)
            .filter(|n| n.bounds.origin.x.0 + n.bounds.size.width.0 >= at.origin.x.0 + at.size.width.0)
            .map(|n| (n.bounds.origin.y.0, n.bounds.origin.y.0 + n.bounds.size.height.0))
            .next_back()
            .unwrap();
        if top >= port.0 && bottom <= port.1 {
            return app.press(selector).unwrap();
        }
        let by = if bottom > port.1 { (bottom - port.1 + 24.).min(300.) } else { -((port.0 - top + 24.).min(300.)) };
        app.scroll(selector, Points(by)).unwrap();
    }
    panic!("{selector} never came into view");
}

#[test]
fn the_phone_shell_opens_tabs_spaces_and_settings() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut config = EngineConfig::new(Root::of::<tarn_mobile::App>());
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    assert!(!app.screen().enabled("label:Back").unwrap(), "a new tab has nothing to go back to");
    assert!(!app.screen().enabled("label:Forward").unwrap());

    app.type_text("id:address", "example.com").unwrap();
    app.submit("id:address").unwrap();
    app.wait_for("label:Share").unwrap();
    app.press("label:New Tab").unwrap();
    app.wait_for("label:Tabs 2").unwrap();
    app.wait_for("label:Cancel").unwrap();
    app.press("label:Cancel").unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("label:History").unwrap();
    app.press("label:History").unwrap();
    app.wait_for("label:Back to Menu").unwrap();
    assert!(!app.screen().shows("example.com"), "a typed address is history only once a page commits");
    app.press("Done").unwrap();

    app.press("label:Menu").unwrap();
    app.wait_for("label:History").unwrap();
    app.press("label:History").unwrap();
    app.wait_for("label:Back to Menu").unwrap();
    app.press("label:Back to Menu").unwrap();
    app.wait_for("label:Find in Page").unwrap();
    assert!(!app.screen().shows("Reopen Closed Tab"), "the menu leaves out what has nothing to act on");
    app.press("label:History").unwrap();
    app.back().unwrap();
    app.wait_for("label:Find in Page").unwrap();
    app.back().unwrap();
    app.wait_for_gone("Find in Page", WAIT).expect("Back closes the Menu sheet");
    app.press("label:Menu").unwrap();
    app.wait_for("label:Find in Page").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for_gone("Find in Page", WAIT).expect("the Menu button closes its own sheet");

    app.press("label:Menu").unwrap();
    app.wait_for("label:Find in Page").unwrap();
    app.press("label:Find in Page").unwrap();
    app.wait_for("label:Next").unwrap();
    app.press_key("escape").unwrap();
    assert!(!app.screen().shows("label:Next"), "Esc closes the find bar");

    app.press("label:Menu").unwrap();
    app.wait_for("label:Find in Page").unwrap();
    app.press("New Tab >> nth=0").unwrap();
    app.wait_for("label:Tabs 2").unwrap();
    app.press("label:Tabs 2").unwrap();
    app.wait_for("label:Close Tab").unwrap();
    app.press("label:Close Tab >> nth=1").unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    app.press("Done").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("Reopen Closed Tab").unwrap();
    app.press("Reopen Closed Tab").unwrap();
    app.wait_for("label:Tabs 2").unwrap();

    app.press("label:Menu").unwrap();
    app.wait_for("Private Space").unwrap();
    app.press("Private Space").unwrap();
    app.wait_for("label:Shield: Private, Standard").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("Close Private Space").unwrap();
    app.press("Close Private Space").unwrap();
    app.wait_for("label:Shield: Standard").unwrap();

    app.press("label:Menu").unwrap();
    app.wait_for("Spaces and Profiles").unwrap();
    app.press("Spaces and Profiles").unwrap();
    app.wait_for("New Space").unwrap();
    app.press("New Space").unwrap();
    app.wait_for("id:Space Name").unwrap();
    app.type_text("id:Space Name", " Work").unwrap();
    app.scroll("Icon", Points(800.)).unwrap();
    app.press("Create Space").unwrap();
    app.press("label:Tabs 1").unwrap();
    app.wait_for("Space 2 Work").unwrap();
    app.press("Done").unwrap();

    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("label:Strict").unwrap();
    press_in_view(&mut app, "label:Strict");
    app.wait_for("label:Clear Browsing Data…").unwrap();
    press_in_view(&mut app, "label:Ads: On");
    app.wait_for("label:Ads: Off").unwrap();
    app.press("label:Done").unwrap();
    app.wait_for("label:Shield: Strict").unwrap();

    app.press("label:Menu").unwrap();
    app.wait_for("label:Find in Page").unwrap();
    app.press("New Tab >> nth=0").unwrap();
    app.wait_for("label:Tabs 2").unwrap();
    app.press("label:Tabs 2").unwrap();
    app.wait_for("label:Close Tab").unwrap();
    assert!(!app.screen().shows("Open in Split View"), "a phone shows one page at a time");
    app.press("Done").unwrap();
    app.set_viewport(Size::new(Points(1024.), Points(768.))).unwrap();
    app.press("label:Tabs 2").unwrap();
    app.wait_for("label:Open in Split View").unwrap();
    app.press("label:Open in Split View >> nth=0").unwrap();
    app.wait_for("label:Remove from Split View").unwrap();
    let divider_x = |app: &Headless| {
        let node = app.screen().layout_nodes().into_iter().find(|n| n.test_id.as_deref() == Some("column-divider"));
        node.expect("a column divider").bounds.origin.x.0
    };
    let even = divider_x(&app);
    app.drag("label:Column divider", point(px(120.), px(0.))).unwrap();
    let moved = divider_x(&app);
    assert!(moved > even + 60., "the divider follows the finger: {even} -> {moved}");
    app.drag("label:Column divider", point(px(0.), px(0.))).unwrap();
    app.drag("label:Column divider", point(px(0.), px(0.))).unwrap();
    assert!((divider_x(&app) - even).abs() < 1., "a double tap evens the panes out");
    let header_x = |app: &Headless| {
        let id = app.screen().press_target("label:Remove from Split View >> nth=0").unwrap().expect("a pane header");
        app.screen().layout_nodes().into_iter().find(|n| n.id == id).unwrap().bounds.origin.x.0
    };
    let before = header_x(&app);
    let across = if before < even { 600. } else { -600. };
    app.drag("label:Remove from Split View >> nth=0", point(px(across), px(200.))).unwrap();
    assert!((header_x(&app) - before).abs() > 200., "a header dropped on the other pane swaps the two");
    app.press("label:Tabs 2").unwrap();
    app.wait_for("Rotate Layout").unwrap();
    app.press("Rotate Layout").unwrap();
    app.wait_for("Close Split View").unwrap();
    app.press("Close Split View").unwrap();
    app.wait_for_gone("Close Split View", WAIT).expect("a closed split leaves its tabs open");
    app.press("label:Open in Split View >> nth=0").unwrap();
    app.wait_for("label:Remove from Split View").unwrap();
    app.press("label:Close Tab >> nth=1").unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    assert!(!app.screen().shows("Remove from Split View"), "a split left with one pane ends");
}
