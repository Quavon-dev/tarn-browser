use nsh_core::geometry::{Points, Size};
use nsh_core::image::ImageData;
use nsh_core::picker::PickedFile;
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[test]
fn the_mac_layout_opens_a_file_from_the_palette_and_its_key() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this binary has one test, and it sets the variable before the engine starts.
    unsafe { std::env::set_var("TARN_DATA_DIR", dir.path()) };
    let mut app = Headless::start(EngineConfig::new(Root::of::<tarn_mobile::App>())).unwrap();
    app.set_viewport(Size::new(Points(1376.), Points(1032.))).unwrap();
    app.on_pick(|_| vec![PickedFile::new("note.txt", "text/plain", ImageData::new(b"hello".to_vec()))]);
    app.wait_for("label:Menu").unwrap();
    app.press("label:Menu").unwrap();
    app.press("label:Settings").unwrap();
    app.wait_for("label:Mac").unwrap();
    app.press("label:Mac").unwrap();
    app.press("label:Close Settings").unwrap();
    app.wait_for_gone("label:Close Settings", WAIT).unwrap();

    app.press_key("mod+l").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", "> open file").unwrap();
    app.wait_for("label:Open File…").unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for_gone("id:palette-field", WAIT).unwrap();
    app.wait_for("label:note.txt").unwrap();
    assert_eq!(app.picks().len(), 1, "the palette row asks for one file");
    assert!(std::fs::read_dir(dir.path().join("Opened")).unwrap().next().is_some(), "the picked bytes are staged where Open File keeps them");

    app.press_key("mod+o").unwrap();
    app.wait_until("two note.txt tabs", WAIT, |s| s.count("label:note.txt").unwrap_or(0) >= 2).unwrap();
    assert_eq!(app.picks().len(), 2);
}
