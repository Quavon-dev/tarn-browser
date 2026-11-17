use nsh_core::geometry::{Points, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;
use tarn_nsh_shared::controls::{Density, density};

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[test]
fn a_tablet_switches_to_the_mac_layout_and_back() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut app = Headless::start(EngineConfig::new(Root::of::<tarn_mobile::App>())).unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("Search the text of visited pages").unwrap();
    assert!(!app.screen().shows("label:Mac"), "a phone offers no layout choice");
    app.press("Done").unwrap();
    app.wait_for_gone("Search the text of visited pages", WAIT).unwrap();

    app.set_viewport(Size::new(Points(1032.), Points(1376.))).unwrap();
    app.wait_for("label:Menu").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("label:Mac").unwrap();
    app.press("label:Mac").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    assert_eq!(density(), Density::Desktop);
    assert!(!app.screen().shows("label:Tabs 1"), "the touch toolbar gives way to the Mac one");
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();
    app.wait_for("label:New Tab").unwrap();
    app.wait_for("label:Close Tab").unwrap();
    app.wait_for("label:Back").unwrap();
    app.wait_for("id:address").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.back().unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();
    assert!(!app.screen().shows("Find in Page"), "Back from Settings returns to the page, not the touch Menu");
    app.press_key("mod+,").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();

    app.press("label:Settings").unwrap();
    app.wait_for("label:Touch").unwrap();
    app.press("label:Touch").unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    assert_eq!(density(), Density::Touch);
    assert!(!app.screen().shows("label:Close Settings"));
    app.press("Done").unwrap();
    app.wait_for_gone("Search the text of visited pages", WAIT).unwrap();

    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("label:Mac").unwrap();
    app.press("label:Mac").unwrap();
    app.wait_for("label:Close Settings").unwrap();
    app.set_viewport(Size::new(Points(430.), Points(932.))).unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    assert_eq!(density(), Density::Touch, "a phone-sized window keeps the touch layout");
}
