use nsh_core::geometry::{Points, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

fn open(app: &mut Headless, url: &str, host: &str) {
    app.press_key("mod+t").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", url).unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    app.wait_for(&format!("label:{host}")).unwrap();
}

#[test]
fn a_pinned_tab_closed_from_its_split_pane_is_unloaded_in_the_mac_layout() {
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

    open(&mut app, "example.com", "example.com");
    app.press_key("mod+d").unwrap();
    app.wait_for_gone("Pin tabs from their menu", WAIT).unwrap();
    open(&mut app, "example.org", "example.org");
    app.long_press("label:example.com").unwrap();
    app.press("label:Open in Split View").unwrap();
    app.wait_for("id:pane >> nth=1").unwrap();
    let pinned = (0..2).find(|n| app.screen().matches(&format!("id:pane >> nth={n} >> label:example.com")).unwrap()).unwrap();
    app.press(&format!("id:pane >> nth={pinned} >> label:Close Tab")).unwrap();
    app.wait_for_gone("id:pane", WAIT).unwrap();
    assert!(!app.screen().shows("Pin tabs from their menu"), "the pinned tab stays pinned");
    assert_eq!(app.screen().count("id:tab").unwrap(), 3, "the pinned tab keeps its row beside the start tab and example.org");
}
