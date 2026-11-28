use nsh_core::geometry::{Points, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::{Headless, WAIT};

fn palette(app: &mut Headless, typed: &str) {
    app.press_key("mod+t").unwrap();
    app.wait_for("id:palette-field").unwrap();
    if !typed.is_empty() {
        app.type_text("id:palette-field", typed).unwrap();
    }
}

fn run(app: &mut Headless, command: &str) {
    palette(app, command);
    app.wait_for(&format!("label:{command}")).unwrap();
    app.press(&format!("label:{command} >> nth=1")).unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
}

fn settings_pane(app: &mut Headless, pane: &str) {
    app.press_key("mod+,").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.press(&format!("label:{pane}")).unwrap();
}

#[test]
fn the_mac_layout_offers_assistant_tabs_on_top_density_and_capture() {
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
    app.wait_for("label:/^Space /").unwrap();
    assert!(!app.screen().matches("label:Play").unwrap(), "no mini-player without media");

    palette(&mut app, "");
    app.wait_for("QUICK ACTIONS").unwrap();
    app.press_key("right").unwrap();
    app.wait_for("label:Run").unwrap();
    assert!(!app.screen().shows("QUICK ACTIONS"), "the actions page replaces the list");
    app.press_key("left").unwrap();
    app.wait_for("QUICK ACTIONS").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();

    palette(&mut app, "https://example.com/");
    app.submit("id:palette-field").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    palette(&mut app, "capture");
    for entry in ["Capture Area", "Capture Element", "Capture Full Page"] {
        app.wait_for(&format!("label:{entry}")).unwrap();
    }
    app.press_key("escape").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();

    app.wait_for("label:Tidy").unwrap();
    run(&mut app, "Tabs on Top");
    app.wait_for_gone("label:Tidy", WAIT).expect("Tabs on Top replaces the sidebar with a strip");
    run(&mut app, "Tabs on Top");
    app.wait_for("label:Tidy").expect("Tabs on Top again brings the sidebar back");

    let row = |app: &Headless| {
        let id = app.screen().press_target("label:example.com").unwrap().unwrap();
        app.screen().layout_nodes().into_iter().find(|n| n.id == id).unwrap().bounds.size.height.0
    };
    let roomy = row(&app);
    settings_pane(&mut app, "Appearance");
    app.wait_for("label:Compact density: Off").unwrap();
    app.press("label:Compact density: Off").unwrap();
    app.wait_for("label:Compact density: On").unwrap();
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();
    assert!(row(&app) < roomy, "compact density shortens sidebar rows: {roomy} -> {}", row(&app));
    settings_pane(&mut app, "Appearance");
    app.press("label:Compact density: On").unwrap();
    app.wait_for("label:Compact density: Off").unwrap();
    app.press("label:Classic >> nth=0").unwrap();
    app.wait_for("label:Organize Tabs").expect("the Classic style brings back the sidebar's earlier header and list");
    assert!(!app.screen().matches("label:Tidy").unwrap() && !app.screen().matches("label:Pinned sites").unwrap());
    app.press("label:Modern").unwrap();
    app.wait_for("label:Tidy").expect("Modern puts Tidy back");
    app.wait_for("label:Dim sleeping tabs: Off").unwrap();

    app.press("label:AI").unwrap();
    app.wait_for("label:Use the assistant in this profile: Off").unwrap();
    app.press("label:Use the assistant in this profile: Off").unwrap();
    app.wait_for("label:Claude API").unwrap();
    assert!(app.screen().matches("label:API key").unwrap());
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();
    app.wait_for("label:Chats").expect("the sidebar lists chats once the profile's AI is on");
    run(&mut app, "Chats in Sidebar");
    app.wait_for_gone("label:Chats", WAIT).expect("the command bar hides them");
    run(&mut app, "Chats in Sidebar");
    app.wait_for("label:Chats").unwrap();

    run(&mut app, "Ask Assistant");
    app.wait_for("label:Attach Page").unwrap();
    let before = app.screen().commands().len();
    app.press("label:Attach Page").unwrap();
    assert!(app.screen().commands()[before..].iter().any(|(_, c)| format!("{c:?}").contains("readText")), "Attach Page asks the page for its text");
}
