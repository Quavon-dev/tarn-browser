//! Runs the YouTube page script against player data in Node's `vm` sandbox, one fresh context per
//! host. Needs Node, so it runs only when asked: `cargo test -p shield --test youtube -- --ignored`.

use std::process::Command;

#[test]
#[ignore = "needs node"]
fn youtube_script_removes_ad_fields_only_on_youtube() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("youtube.js");
    std::fs::write(&script, shield::youtube::script(&shield::youtube::Config::default())).unwrap();
    let output = Command::new("node").arg(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/youtube_check.mjs")).arg(&script).output().expect("node");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success() && stdout.trim_end().ends_with("ALL OK"), "{stdout}{}", String::from_utf8_lossy(&output.stderr));
}
