use nsh_core::{KeyboardTaps, MountItem};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::{DialogAnswer, Headless, WAIT};

#[test]
fn a_note_is_written_edited_and_deleted_from_phone_settings() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut config = EngineConfig::new(Root::of::<tarn_mobile::App>());
    config.clock = nsh_core::time::Clock::manual(std::time::SystemTime::now());
    let mut app = Headless::start(config).unwrap();
    app.on_dialog(|_| DialogAnswer::Press("Delete".into()));
    app.wait_for("label:Menu").unwrap();
    app.press("label:Menu").unwrap();
    app.wait_for("label:Settings").unwrap();
    let passes = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = passes.clone();
    app.observe(move |_, batch| {
        let handled = |item: &MountItem| matches!(item, MountItem::ScrollBehavior { behavior, .. } if behavior.keyboard_taps == KeyboardTaps::Handled);
        if batch.items.iter().any(handled) {
            seen.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    app.press("label:Settings").unwrap();
    app.type_text("label:Search settings", "Notes").unwrap();
    assert!(passes.load(std::sync::atomic::Ordering::Relaxed), "with the keyboard up, a tap on New Note reaches it instead of only closing the keyboard");
    app.wait_for("No notes yet.").unwrap();
    app.press("label:New Note").unwrap();
    app.wait_for("label:Title").expect("New Note opens the editor");
    app.type_text("label:Title", "Groceries").unwrap();
    app.type_text("label:Note", "Milk and bread").unwrap();
    app.press("label:Save").unwrap();
    app.wait_for("label:Edit Groceries").unwrap();
    assert!(!app.screen().shows("No notes yet."));
    app.press("label:Edit Groceries").unwrap();
    app.wait_for("label:Title").unwrap();
    app.clear("label:Title").unwrap();
    app.type_text("label:Title", "Shopping").unwrap();
    app.press("label:Save").unwrap();
    app.wait_for("label:Edit Shopping").unwrap();
    app.press("label:Delete Shopping").unwrap();
    app.wait_for("No notes yet.").unwrap();
    app.wait_for_gone("label:Edit Shopping", WAIT).unwrap();
}
