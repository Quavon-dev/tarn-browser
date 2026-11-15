use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn the_phone_shell_opens_tabs_spaces_and_settings() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut app = Headless::start(EngineConfig::new(Root::of::<tarn_mobile::App>())).unwrap();
    app.wait_for("label:Tabs 1").unwrap();

    app.type_text("id:address", "example.com").unwrap();
    app.submit("id:address").unwrap();
    app.wait_for("label:Share").unwrap();
    app.press("label:New Tab").unwrap();
    app.wait_for("label:Tabs 2").unwrap();
    app.wait_for("label:Cancel").unwrap();
    app.press("label:Cancel").unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    app.press("label:Menu").unwrap();
    app.press("label:History").unwrap();
    assert!(!app.screen().shows("example.com"), "a typed address is history only once a page commits");
    app.press("Done").unwrap();

    app.press("label:Menu").unwrap();
    app.press("New Tab >> nth=0").unwrap();
    app.wait_for("label:Tabs 2").unwrap();
    app.press("label:Tabs 2").unwrap();
    app.wait_for("label:Close Tab").unwrap();
    app.press("label:Close Tab >> nth=1").unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    app.press("Done").unwrap();
    app.press("label:Menu").unwrap();
    app.press("Reopen Closed Tab").unwrap();
    app.wait_for("label:Tabs 2").unwrap();

    app.press("label:Menu").unwrap();
    app.press("Private Space").unwrap();
    app.wait_for("label:Shield: Private, Standard").unwrap();
    app.press("label:Menu").unwrap();
    app.press("Close Private Space").unwrap();
    app.wait_for("label:Shield: Standard").unwrap();

    app.press("label:Menu").unwrap();
    app.press("Spaces and Profiles").unwrap();
    app.type_text("id:Name", "Work").unwrap();
    app.press("Add Space").unwrap();
    app.press("label:Tabs 1").unwrap();
    app.wait_for("Work").unwrap();
    app.press("Done").unwrap();

    app.press("label:Menu").unwrap();
    app.press("label:Settings").unwrap();
    app.press("label:Strict").unwrap();
    app.wait_for("label:Clear Browsing Data…").unwrap();
    app.press("label:Ads: On").unwrap();
    app.wait_for("label:Ads: Off").unwrap();
    app.press("label:Done").unwrap();
    app.wait_for("label:Shield: Strict").unwrap();
}
