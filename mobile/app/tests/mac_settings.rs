use nsh_core::geometry::{Points, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

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

    app.press("label:Privacy").unwrap();
    app.wait_for("Clear Browsing Data…").unwrap();

    app.press("label:General").unwrap();
    app.wait_for("Open-source licences").unwrap();
    app.press("Open-source licences").unwrap();
    app.wait_for("Open-Source Licences").unwrap();
    assert!(app.screen().matches("label:Close Settings").unwrap(), "licences open inside the Settings window");
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
