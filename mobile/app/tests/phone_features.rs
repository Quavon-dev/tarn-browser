use nsh_core::{KeyboardTaps, MountItem};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::{DialogAnswer, Headless, WAIT};

fn settings(app: &mut Headless, search: &str) {
    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    app.press("label:Settings").unwrap();
    app.type_text("label:Search settings", search).unwrap();
}

fn open(app: &mut Headless, address: &str) {
    app.press("label:New Tab").unwrap();
    app.wait_for("label:Cancel").unwrap();
    app.type_text("id:address", address).unwrap();
    app.submit("id:address").unwrap();
    app.wait_for_gone("label:Cancel", WAIT).unwrap();
}

#[test]
fn phone_snippets_assistant_tabs_capture_and_density() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut config = EngineConfig::new(Root::of::<tarn_mobile::App>());
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).unwrap();
    app.on_dialog(|_| DialogAnswer::Press("Delete".into()));
    let passes = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = passes.clone();
    app.observe(move |_, batch| {
        let handled = |item: &MountItem| matches!(item, MountItem::ScrollBehavior { behavior, .. } if behavior.keyboard_taps == KeyboardTaps::Handled);
        if batch.items.iter().any(handled) {
            seen.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    app.wait_for("label:Menu").unwrap();
    assert!(passes.load(std::sync::atomic::Ordering::Relaxed), "an address suggestion takes the first tap while the keyboard is up");
    assert!(!app.screen().matches("label:Play").unwrap(), "no mini-player without media");
    assert!(!app.screen().matches("label:Pause").unwrap());

    settings(&mut app, "Snippets");
    app.wait_for("No snippets yet.").unwrap();
    app.press("label:Add Snippet").unwrap();
    app.wait_for("label:Name").unwrap();
    app.type_text("label:Name", "sig").unwrap();
    app.type_text("label:Text", "Best, Ada").unwrap();
    app.press("label:Save").unwrap();
    app.wait_for("label:Edit sig").unwrap();
    app.press("label:Edit sig").unwrap();
    app.wait_for("label:Text").unwrap();
    app.clear("label:Text").unwrap();
    app.type_text("label:Text", "Best regards, Ada").unwrap();
    app.press("label:Save").unwrap();
    app.wait_for("Best regards, Ada").unwrap();
    app.press("label:Done").unwrap();

    app.press("label:New Tab").unwrap();
    app.wait_for("label:Cancel").unwrap();
    app.type_text("id:address", ";sig").unwrap();
    app.wait_for("Best regards, Ada").expect("the address bar offers the snippet for ;sig");
    app.press("label:Cancel").unwrap();
    app.wait_for_gone("label:Cancel", WAIT).unwrap();

    settings(&mut app, "Snippets");
    app.wait_for("label:Delete sig").unwrap();
    app.press("label:Delete sig").unwrap();
    app.wait_for("No snippets yet.").unwrap();
    app.press("label:Done").unwrap();

    settings(&mut app, "Density");
    app.wait_for("label:Compact density: Off").unwrap();
    app.press("label:Compact density: Off").unwrap();
    app.wait_for("label:Compact density: On").unwrap();
    app.press("label:Compact density: On").unwrap();
    app.wait_for("label:Compact density: Off").unwrap();
    app.press("label:Done").unwrap();

    open(&mut app, "example.com");
    open(&mut app, "example.org");
    app.press("label:Tabs 3").unwrap();
    app.wait_for("label:Clean Up").unwrap();
    assert!(!app.screen().matches("label:Group with Assistant").unwrap(), "Group with Assistant waits for AI in Settings");
    app.press("Done").unwrap();

    app.press("label:Menu").unwrap();
    app.wait_for("label:Find in Page").unwrap();
    assert!(!app.screen().matches("label:Ask Assistant").unwrap(), "Ask Assistant waits for AI in Settings");
    for entry in ["Take Screenshot", "Capture Area", "Capture Element", "Capture Full Page", "Insert Snippet…"] {
        assert!(app.screen().matches(&format!("label:{entry}")).unwrap(), "the Menu lists {entry}");
    }
    app.press("label:Menu").unwrap();
    app.wait_for_gone("label:Find in Page", WAIT).unwrap();

    settings(&mut app, "AI");
    app.wait_for("label:Use the assistant in this profile: Off").unwrap();
    assert!(!app.screen().shows("Provider"), "the provider waits for the switch");
    app.press("label:Use the assistant in this profile: Off").unwrap();
    app.wait_for("label:Use the assistant in this profile: On").unwrap();
    app.wait_for("Provider").unwrap();
    assert!(app.screen().matches("label:Claude API").unwrap());
    assert!(app.screen().matches("label:API key").unwrap(), "Claude API asks for its key");
    app.press("label:Done").unwrap();

    app.press("label:Tabs 3").unwrap();
    app.wait_for("label:Group with Assistant").expect("AI on offers Group with Assistant");
    assert!(app.screen().matches("label:Clean Up").unwrap());
    app.press("Done").unwrap();

    app.press("label:Menu").unwrap();
    app.wait_for("label:Ask Assistant").unwrap();
    app.press("label:Ask Assistant").unwrap();
    app.wait_for("label:Attach Page").unwrap();
    let before = app.screen().commands().len();
    app.press("label:Attach Page").unwrap();
    assert!(app.screen().commands()[before..].iter().any(|(_, c)| format!("{c:?}").contains("readText")), "Attach Page asks the page for its text");
    app.press("label:Done").unwrap();
}
