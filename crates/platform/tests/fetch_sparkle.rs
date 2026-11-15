#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

const SHA256: &str = "1a856d2a9edb8e628de88121d7be04cf0e2167f8641650b78fca00b14244cf27";

fn fetch(dir: &Path, host: &str, path: &str) -> Output {
    let lock = dir.join("sparkle.lock");
    fs::write(&lock, format!("sparkle_version=2.10.1\nrelease_date=2026-10-25\nhost={host}\nsha256={SHA256}\n")).unwrap();
    Command::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/fetch-sparkle.sh"))
        .env("SPARKLE_LOCK", &lock)
        .env("SPARKLE_DIR", dir.join("sparkle"))
        .env("PATH", path)
        .output()
        .unwrap()
}

#[test]
fn refuses_another_host() {
    let dir = tempfile::tempdir().unwrap();
    let out = fetch(dir.path(), "example.com", &std::env::var("PATH").unwrap());
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("another download host"));
}

#[test]
fn refuses_an_archive_with_another_hash() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let curl = bin.join("curl");
    fs::write(&curl, "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = -o ] && printf tampered > \"$2\"; shift; done\n").unwrap();
    fs::set_permissions(&curl, fs::Permissions::from_mode(0o755)).unwrap();
    let out = fetch(dir.path(), "github.com", &format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()));
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("SHA-256 mismatch"));
    assert!(fs::read_dir(dir.path().join("sparkle/downloads")).unwrap().next().is_none());
    assert!(!dir.path().join("sparkle/2.10.1").exists());
}
