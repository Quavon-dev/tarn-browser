use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

#[test]
fn site_css_typed_in_the_site_sheet_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut app = Headless::start(EngineConfig::new(Root::of::<tarn_mobile::App>())).unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    app.type_text("id:address", "example.com").unwrap();
    app.submit("id:address").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("label:Shield and Settings for This Site").unwrap();
    app.press("label:Shield and Settings for This Site").unwrap();
    app.wait_for("id:Site CSS").unwrap();
    app.type_text("id:Site CSS", "h1{color:red}").unwrap();
    app.press("label:Save CSS").unwrap();
    app.wait_for("Site CSS saved").unwrap();
}
