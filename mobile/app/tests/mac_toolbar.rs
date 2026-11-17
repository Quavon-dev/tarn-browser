use nsh_core::geometry::{Points, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[test]
fn the_mac_toolbar_opens_a_command_palette() {
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
    assert!(!app.screen().shows("label:Menu"), "the Mac toolbar has no touch menu button");

    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    for hint in ["Run", "Move", "Close"] {
        assert!(app.screen().shows(hint), "{hint}");
    }
    app.type_text("id:palette-field", "example.com").unwrap();
    app.wait_for("Open").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();

    app.press_key("mod+l").unwrap();
    app.wait_for("id:palette-field").unwrap();
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
}
