use nsh_core::geometry::{Points, Size};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

fn tabs(app: &mut Headless, n: usize) {
    app.wait_until(&format!("{n} tabs"), WAIT, |s| s.count("id:tab").unwrap_or(0) == n).unwrap();
}

#[test]
fn the_mac_layout_answers_the_macos_keys() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut config = EngineConfig::new(Root::of::<tarn_mobile::App>());
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    app.press_key("mod+t").unwrap();
    app.wait_for("label:Tabs 1").unwrap();
    assert!(!app.screen().matches("label:Tabs 2").unwrap(), "the touch layout binds no browser keys");

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
    app.wait_for("id:palette-field").unwrap();
    tabs(&mut app, 1);
    app.press_key("escape").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    tabs(&mut app, 1);

    app.press_key("mod+t").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", "example.org").unwrap();
    app.submit("id:palette-field").unwrap();
    tabs(&mut app, 2);
    app.press_key("mod+k").unwrap();
    app.press_key("escape").unwrap();
    app.press_key("mod+1").unwrap();
    app.press_key("mod+w").unwrap();
    tabs(&mut app, 1);
    app.press_key("mod+shift+t").unwrap();
    tabs(&mut app, 2);
    let shown = |s: &nsh_testing::Screen| s.texts().iter().filter(|t| **t == "example.org").count();
    app.press_key("mod+1").unwrap();
    app.press_key("mod+9").unwrap();
    let last = shown(app.screen());
    app.press_key("ctrl+tab").unwrap();
    app.wait_for("id:tab-switcher").unwrap();
    app.press_key("escape").unwrap();
    app.wait_for_gone("id:tab-switcher", WAIT).unwrap();
    assert_eq!(shown(app.screen()), last, "escape keeps the tab");
    app.press_key("ctrl+tab").unwrap();
    app.wait_for("id:tab-switcher").unwrap();
    app.run_for(browser_core::switcher::COMMIT_DELAY).unwrap();
    app.wait_for_gone("id:tab-switcher", WAIT).unwrap();
    app.wait_until("a pause after ctrl+tab goes to the previously used tab", WAIT, |s| shown(s) != last).unwrap();
    app.press_key("ctrl+tab").unwrap();
    app.press_key("ctrl+tab").unwrap();
    app.press_key("ctrl+shift+tab").unwrap();
    app.wait_for("id:tab-switcher").unwrap();
    app.press_key("enter").unwrap();
    app.wait_for_gone("id:tab-switcher", WAIT).unwrap();
    app.wait_until("return switches to the selected tab", WAIT, |s| shown(s) == last).unwrap();
    app.press_key("ctrl+pagedown").unwrap();
    app.wait_until("ctrl+page down moves to the next tab", WAIT, |s| shown(s) != last).unwrap();
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

    app.press_key("mod+d").unwrap();
    app.wait_for("label:Pinned").unwrap();
    app.press_key("mod+d").unwrap();
    app.wait_for_gone("label:Pinned", WAIT).unwrap();

    app.long_press("label:New Tab >> nth=1").unwrap();
    app.press("label:New Group").unwrap();
    app.wait_for("label:Group 1").unwrap();
    app.press_key("mod+t").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", "example.com").unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for("label:example.com").unwrap();
    app.press("label:Group 1").unwrap();
    let active = |s: &nsh_testing::Screen| s.texts().iter().filter(|t| **t == "example.com").count() == 2;
    app.wait_until("the page stays active with its group collapsed", WAIT, active).unwrap();
    for keys in ["mod+1", "mod+9", "ctrl+tab", "ctrl+shift+tab"] {
        app.press_key(keys).unwrap();
        app.wait_until(&format!("{keys} reaches only visible tabs"), WAIT, active).unwrap();
    }
    assert_eq!(app.screen().count("id:tab").unwrap(), 1, "the collapsed group's tab stays hidden");

    app.press_key("mod+shift+n").unwrap();
    app.wait_until("a private space", WAIT, |s| s.shows("Private")).unwrap();
    assert!(!app.screen().matches("label:Private space: kept in memory only").unwrap(), "the sidebar already says it");
    app.press_key("mod+s").unwrap();
    app.wait_for("label:Private space: kept in memory only").unwrap();
}
