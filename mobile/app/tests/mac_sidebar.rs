use nsh_core::geometry::{Points, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[test]
fn the_mac_sidebar_pins_groups_and_files_tabs_from_its_rows_and_menus() {
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

    for part in ["label:/^Space /", "label:Downloads", "label:New Tab Group", "label:New Folder", "label:Library", "label:New Space", "label:Close Tab"] {
        app.wait_for(part).unwrap();
    }
    assert!(app.screen().shows("Pin tabs from their menu"), "no pinned tab yet");
    assert!(!app.screen().shows("label:Clean Up"), "one tab has nothing to sort");

    app.long_press("label:New Tab >> nth=1").unwrap();
    app.wait_for("label:Pin Tab").unwrap();
    app.wait_for("label:Add to Tiles").unwrap();
    app.wait_for("text:Close Other Tabs").unwrap();
    app.press("label:Pin Tab").unwrap();
    app.wait_for("label:Pinned").unwrap();
    app.wait_for_gone("Pin tabs from their menu", WAIT).unwrap();
    assert!(!app.screen().shows("label:Close Tab"), "a pinned tab has no close button");

    app.long_press("label:New Tab >> nth=0").unwrap();
    app.wait_for("label:Unpin Tab").unwrap();
    assert!(!app.screen().shows("text:Close Tab"), "a pinned tab is never deleted from its menu");
    app.press("label:Add to Tiles").unwrap();
    app.wait_for_gone("label:Pinned", WAIT).unwrap();
    app.wait_for("Pin tabs from their menu").unwrap();

    app.press("label:New Tab Group").unwrap();
    app.wait_for("label:Group 1").unwrap();
    app.long_press("label:Group 1").unwrap();
    app.wait_for("label:Ungroup").unwrap();
    app.press("label:Ungroup").unwrap();
    app.wait_for_gone("label:Group 1", WAIT).unwrap();

    app.press("label:New Folder").unwrap();
    app.wait_for("label:Folder 1").unwrap();
    app.long_press("label:Folder 1").unwrap();
    app.wait_for("label:New Subfolder").unwrap();
    app.press("label:New Subfolder").unwrap();
    app.wait_for("label:Folder 2").unwrap();
    app.press("label:Folder 1").unwrap();
    app.wait_for_gone("label:Folder 2", WAIT).unwrap();

    app.press("label:/^Space /").unwrap();
    app.wait_for("label:New Space…").unwrap();
    app.press("label:New Space…").unwrap();
    app.wait_for("Spaces and Profiles").unwrap();
}
