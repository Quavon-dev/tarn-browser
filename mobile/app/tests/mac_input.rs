use nsh_core::geometry::{Points, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

fn tabs(app: &mut Headless, n: usize) {
    app.wait_until(&format!("{n} tabs"), WAIT, |s| s.count("label:Close Tab").unwrap_or(0) == n).unwrap();
}

#[test]
fn the_mac_layout_answers_the_macos_keys() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut app = Headless::start(EngineConfig::new(Root::of::<tarn_mobile::App>())).unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    app.press_key("mod+t").unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    assert!(!app.screen().shows("label:Tabs 2"), "the touch layout binds no browser keys");

    app.set_viewport(Size::new(Points(1376.), Points(1032.))).unwrap();
    app.wait_for("label:Menu").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("label:Mac").unwrap();
    app.press("label:Mac").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();
    tabs(&mut app, 1);

    app.press_key("mod+t").unwrap();
    tabs(&mut app, 2);
    app.press_key("escape").unwrap();
    tabs(&mut app, 1);

    app.press_key("mod+t").unwrap();
    tabs(&mut app, 2);
    app.press_key("mod+k").unwrap();
    app.press_key("escape").unwrap();
    app.press_key("mod+1").unwrap();
    app.press_key("mod+w").unwrap();
    tabs(&mut app, 1);
    app.press_key("mod+shift+t").unwrap();
    tabs(&mut app, 2);
    app.press_key("ctrl+tab").unwrap();
    app.press_key("mod+9").unwrap();
    app.press_key("mod+w").unwrap();
    tabs(&mut app, 1);

    app.press_key("mod+f").unwrap();
    app.wait_for("label:Next").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("label:Next", WAIT).unwrap();

    app.press_key("mod+y").unwrap();
    app.wait_for("label:Back to Menu").unwrap();
    app.press_key("mod+y").unwrap();
    app.wait_for_gone("label:Back to Menu", WAIT).unwrap();

    app.press_key("mod+,").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.press_key("mod+,").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();

    app.press_key("mod+shift+n").unwrap();
    app.wait_until("a private space", WAIT, |s| s.shows("Private")).unwrap();
}
