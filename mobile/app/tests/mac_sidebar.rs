use nsh_core::geometry::{Points, Size, point};
use nsh_core::platform::{Idiom, Os};
use nsh_engine::{EngineConfig, Root};
use nsh_testing::Headless;

const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[test]
fn the_mac_sidebar_pins_groups_and_files_tabs_from_its_rows_and_menus() {
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

    for part in ["label:/^Space /", "label:Downloads", "label:New Tab Group", "label:New Folder", "label:Library", "label:New Space", "label:Close Tab"] {
        app.wait_for(part).unwrap();
    }
    assert!(app.screen().shows("Pin tabs from their menu"), "no pinned tab yet");
    assert!(!app.screen().matches("label:Clean Up").unwrap(), "one tab has nothing to sort");

    app.long_press("label:New Tab >> nth=1").unwrap();
    app.wait_for("label:Pin Tab").unwrap();
    app.wait_for("label:Add to Tiles").unwrap();
    app.wait_for("text:Close Other Tabs").unwrap();
    for entry in ["label:Duplicate", "label:Copy Link", "label:Rename"] {
        app.wait_for(entry).unwrap();
    }
    assert!(!app.screen().enabled("label:Duplicate").unwrap(), "an empty tab has nothing to duplicate");
    app.press("label:Pin Tab").unwrap();
    app.wait_for("label:Pinned").unwrap();
    app.wait_for_gone("Pin tabs from their menu", WAIT).unwrap();
    assert!(!app.screen().matches("label:Close Tab").unwrap(), "a pinned tab has no close button");

    app.long_press("label:New Tab >> nth=0").unwrap();
    app.wait_for("label:Unpin Tab").unwrap();
    app.wait_for("label:Unload Tab").unwrap();
    assert!(!app.screen().matches("text:Close Tab").unwrap(), "a pinned tab is never deleted from its menu");
    app.press("label:Add to Tiles").unwrap();
    app.wait_for_gone("label:Pinned", WAIT).unwrap();
    app.wait_for("Pin tabs from their menu").unwrap();

    app.long_press("label:New Tab >> nth=0").unwrap();
    app.wait_for("label:Unload Tab").unwrap();
    assert!(!app.screen().enabled("label:Rename").unwrap(), "a tile shows no title to rename");
    app.press("label:Unload Tab").unwrap();
    app.wait_for("label:Close Tab").unwrap();

    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.type_text("id:palette-field", "example.com").unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for("label:example.com").unwrap();
    let mut touch = None;
    app.set_environment("pointer", |e| {
        touch = Some((e.os, e.idiom));
        (e.os, e.idiom) = (Os::MacOs, Idiom::Desktop);
    })
    .unwrap();
    app.hover("label:example.com").unwrap();
    app.wait_for("Drag tabs here to pin them").unwrap();
    app.drag("label:example.com", point(Points(0.), Points(-74.))).unwrap();
    app.wait_for("label:Pinned").unwrap();
    app.wait_for_gone("Drag tabs here to pin them", WAIT).unwrap();
    app.set_environment("touch", |e| (e.os, e.idiom) = touch.unwrap()).unwrap();
    app.long_press("label:example.com").unwrap();
    app.press("label:Unpin Tab").unwrap();
    app.wait_for("Drag tabs here to pin them").unwrap();

    app.long_press("label:example.com").unwrap();
    app.wait_for("label:Duplicate").unwrap();
    app.press("label:Duplicate").unwrap();
    app.wait_for("label:example.com >> nth=1").unwrap();
    app.long_press("label:example.com >> nth=0").unwrap();
    app.press("label:Copy Link").unwrap();
    app.wait_for("Link copied").unwrap();
    assert_eq!(app.screen().clipboard(), Some("https://example.com/"));
    app.long_press("label:example.com >> nth=0").unwrap();
    app.press("label:Rename").unwrap();
    app.wait_for("label:Name").unwrap();
    app.submit("label:Name").unwrap();
    app.wait_for_gone("label:Name", WAIT).unwrap();
    app.long_press("label:example.com >> nth=0").unwrap();
    app.wait_for("label:Rename").unwrap();
    assert!(!app.screen().matches("label:Reset Title").unwrap(), "an unchanged name stores no custom title");
    app.press_key("escape").unwrap();
    app.wait_for_gone("label:Rename", WAIT).unwrap();
    app.long_press("label:example.com >> nth=0").unwrap();
    app.press("label:Rename").unwrap();
    app.wait_for("label:Name").unwrap();
    app.clear("label:Name").unwrap();
    app.type_text("label:Name", "Mail").unwrap();
    app.submit("label:Name").unwrap();
    app.wait_for("label:Mail").unwrap();
    app.long_press("label:Mail").unwrap();
    app.wait_for("label:Reset Title").unwrap();
    app.press("label:New Group").unwrap();
    app.wait_for("label:Close Group and Its Tabs").unwrap();
    app.press("label:Close Group and Its Tabs").unwrap();
    app.wait_for_gone("label:Close Group and Its Tabs", WAIT).unwrap();
    app.wait_for("Group closed").unwrap();
    app.press("label:Search or enter address").unwrap();
    app.wait_for("id:palette-field").unwrap();
    app.clear("id:palette-field").unwrap();
    app.type_text("id:palette-field", "> reopen").unwrap();
    app.wait_for("label:Reopen Closed Tab").unwrap();
    app.submit("id:palette-field").unwrap();
    app.wait_for("label:Close Group and Its Tabs").unwrap();
    app.wait_for("label:Group 1").unwrap();

    app.press("label:New Tab Group").unwrap();
    app.wait_for("label:Group 1").unwrap();
    app.long_press("label:Group 1").unwrap();
    app.wait_for("label:Ungroup").unwrap();
    app.press("label:Ungroup").unwrap();
    app.wait_for_gone("label:Group 1", WAIT).unwrap();

    app.press("label:New Folder").unwrap();
    app.wait_for("label:Folder 1").unwrap();
    app.long_press("label:Folder 1").unwrap();
    app.wait_for("label:New Subfolder").unwrap();
    app.press("label:New Subfolder").unwrap();
    app.wait_for("label:Folder 2").unwrap();
    app.press("label:Folder 1").unwrap();
    app.wait_for_gone("label:Folder 2", WAIT).unwrap();

    app.press("label:/^Space /").unwrap();
    app.wait_for("label:New Space…").unwrap();
    app.press("label:New Space…").unwrap();
    app.wait_for("Create Space").unwrap();
}
