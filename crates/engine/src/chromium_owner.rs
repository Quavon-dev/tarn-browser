//! Chromium keeps one persistent profile, owned by one Tarn profile:
//! `OWNER` in Chromium's root folder names that profile's own data-store UUID.

use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

fn valid(uuid: &str) -> bool {
    uuid.len() == 36 && uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

enum Owner {
    None,
    Invalid,
    Is(String),
    Unreadable,
}

fn read(root: &Path) -> Owner {
    match fs::read_to_string(root.join("OWNER")) {
        Ok(owner) if valid(&owner) => Owner::Is(owner),
        Ok(_) => Owner::Invalid,
        Err(e) if e.kind() == ErrorKind::NotFound => Owner::None,
        Err(e) if e.kind() == ErrorKind::InvalidData => Owner::Invalid,
        Err(_) => Owner::Unreadable,
    }
}

fn write(root: &Path, profile: &str) -> std::io::Result<std::path::PathBuf> {
    let staged = root.join(format!("OWNER.{}", std::process::id()));
    let _ = fs::remove_file(&staged);
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged)?;
    f.write_all(profile.as_bytes())?;
    f.sync_all()?;
    Ok(staged)
}

/// True if `profile` owns Chromium, claiming it when no valid owner is recorded.
pub fn claim(root: &Path, profile: &str) -> bool {
    if !valid(profile) || fs::DirBuilder::new().recursive(true).mode(0o700).create(root).is_err() {
        return false;
    }
    match read(root) {
        Owner::Is(owner) => return owner == profile,
        Owner::Unreadable => return false,
        Owner::Invalid => {
            let _ = fs::remove_file(root.join("OWNER"));
        }
        Owner::None => {}
    }
    let Ok(staged) = write(root, profile) else { return false };
    let linked = fs::hard_link(&staged, root.join("OWNER"));
    let _ = fs::remove_file(&staged);
    match linked {
        Ok(()) => true,
        Err(e) if e.kind() == ErrorKind::AlreadyExists => matches!(read(root), Owner::Is(o) if o == profile),
        Err(_) => false,
    }
}

/// "Delete website data" in the owning profile: ownership moves to `to` (its new store, or the
/// same UUID when the store is shared), and `prepare` deletes Chromium's data once CEF is gone.
pub fn transfer(root: &Path, from: &str, to: &str) {
    if !valid(to) || !matches!(read(root), Owner::Is(o) if o == from) {
        return;
    }
    if fs::write(root.join("PURGE"), b"").is_err() {
        return;
    }
    if let Ok(staged) = write(root, to)
        && fs::rename(&staged, root.join("OWNER")).is_err()
    {
        let _ = fs::remove_file(&staged);
    }
}

/// Records `host` as owing a site clear in Chromium; the record survives a quit and a purge
/// deletes it with everything else.
pub fn record_clear(root: &Path, host: &str) {
    if host.is_empty() || host.contains(char::is_whitespace) {
        return;
    }
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).mode(0o600).open(root.join("CLEAR")) {
        let _ = writeln!(file, "{host}");
    }
}

pub fn clears(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join("CLEAR")).unwrap_or_default().lines().filter(|l| !l.is_empty()).map(str::to_owned).collect()
}

pub fn take_clears(root: &Path) -> Vec<String> {
    let hosts = clears(root);
    let _ = fs::remove_file(root.join("CLEAR"));
    hosts
}

/// True if `claim` would succeed for `profile`, without claiming.
pub fn may_claim(root: &Path, profile: &str) -> bool {
    valid(profile)
        && match read(root) {
            Owner::Is(owner) => owner == profile,
            Owner::Unreadable => false,
            Owner::None | Owner::Invalid => true,
        }
}

/// True if `profile` is the recorded owner.
pub fn owns(root: &Path, profile: &str) -> bool {
    matches!(read(root), Owner::Is(o) if o == profile)
}

/// Before CEF starts or once it is gone: deletes everything Chromium keeps when a purge is
/// pending or the recorded owner is no longer among `live` profiles; in the second case the owner
/// record goes too. Chromium has a single profile, so its root-level state (`Local State`,
/// component data, GPU caches, logs) goes with it, and so does the disk cache Chromium moves to
/// `~/Library/Caches` for a root under `~/Library/Application Support`.
pub fn prepare(root: &Path, live: &[String]) {
    let orphan = matches!(read(root), Owner::Is(o) if !live.contains(&o));
    let purge = root.join("PURGE").exists();
    if !orphan && !purge {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else { return };
    let mut left = false;
    for path in entries.flatten().map(|e| e.path()).filter(|p| !p.ends_with("OWNER") && !p.ends_with("PURGE")) {
        let removed = if path.is_dir() && !path.is_symlink() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
        left |= removed.is_err() && path.symlink_metadata().is_ok();
    }
    if let Some(cache) = std::env::home_dir().and_then(|home| cache_twin(root, &home)) {
        let _ = fs::remove_dir_all(cache);
    }
    if left {
        return;
    }
    let _ = fs::remove_file(root.join("PURGE"));
    if orphan {
        let _ = fs::remove_file(root.join("OWNER"));
    }
}

fn cache_twin(root: &Path, home: &Path) -> Option<std::path::PathBuf> {
    let relative = root.strip_prefix(home.join("Library/Application Support")).ok()?;
    let plain = relative.components().all(|c| matches!(c, std::path::Component::Normal(_)));
    (plain && !relative.as_os_str().is_empty()).then(|| home.join("Library/Caches").join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "8ab0422f-0b55-494b-be4f-33dd4512e785";
    const B: &str = "5940bbff-6f34-4ca5-a03f-c6b43bff8a13";

    #[test]
    fn first_profile_claims_and_others_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Chromium");
        assert!(claim(&root, A));
        assert!(claim(&root, A));
        assert!(!claim(&root, B));
        assert!(!claim(&root, "not-a-uuid"));
        assert!(may_claim(&root, A) && !may_claim(&root, B) && !may_claim(&root, "not-a-uuid"));
        assert_eq!(fs::read_to_string(root.join("OWNER")).unwrap(), A);
    }

    #[test]
    fn a_broken_owner_record_does_not_lock_chromium() {
        let dir = tempfile::tempdir().unwrap();
        assert!(may_claim(dir.path(), A));
        fs::write(dir.path().join("OWNER"), "").unwrap();
        assert!(may_claim(dir.path(), B));
        assert!(claim(dir.path(), B));
        assert!(!claim(dir.path(), A));
    }

    #[test]
    fn a_deleted_owner_releases_chromium_with_its_data() {
        let dir = tempfile::tempdir().unwrap();
        assert!(claim(dir.path(), A));
        fs::create_dir(dir.path().join("Default")).unwrap();
        fs::write(dir.path().join("Local State"), "{}").unwrap();
        prepare(dir.path(), &[A.to_owned()]);
        assert!(dir.path().join("Default").exists());
        prepare(dir.path(), &[B.to_owned()]);
        assert!(!dir.path().join("Default").exists() && !dir.path().join("Local State").exists());
        assert!(claim(dir.path(), B));
    }

    #[test]
    fn a_deleted_owner_leaves_no_chromium_directory_after_relaunch() {
        let dir = tempfile::tempdir().unwrap();
        assert!(claim(dir.path(), A));
        fs::create_dir_all(dir.path().join("Default/Local Storage/leveldb")).unwrap();
        fs::write(dir.path().join("Default/Cookies"), "x").unwrap();
        transfer(dir.path(), A, A);
        prepare(dir.path(), &[A.to_owned()]);
        assert!(!dir.path().join("Default").exists() && claim(dir.path(), A) && !owns(dir.path(), B));
        prepare(dir.path(), &[B.to_owned()]);
        let left: Vec<_> = fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn site_clears_are_recorded_until_taken_and_purged_with_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        assert!(claim(dir.path(), A));
        record_clear(dir.path(), "a.test");
        record_clear(dir.path(), "bad host");
        record_clear(dir.path(), "b.test");
        assert_eq!(clears(dir.path()), ["a.test", "b.test"]);
        assert_eq!(take_clears(dir.path()), ["a.test", "b.test"]);
        assert!(clears(dir.path()).is_empty());
        record_clear(dir.path(), "a.test");
        transfer(dir.path(), A, A);
        prepare(dir.path(), &[A.to_owned()]);
        assert!(clears(dir.path()).is_empty());
    }

    #[test]
    fn the_disk_cache_twin_is_found_only_under_application_support() {
        let home = Path::new("/Users/x");
        let twin = cache_twin(Path::new("/Users/x/Library/Application Support/Tarn/Chromium"), home);
        assert_eq!(twin.as_deref(), Some(Path::new("/Users/x/Library/Caches/Tarn/Chromium")));
        assert!(cache_twin(Path::new("/tmp/data/Chromium"), home).is_none());
        assert!(cache_twin(Path::new("/Users/x/Library/Application Support"), home).is_none());
        assert!(cache_twin(Path::new("/Users/x/Library/Application Support/../../Documents"), home).is_none());
    }

    #[test]
    fn a_new_store_keeps_ownership_and_purges_at_next_launch() {
        let dir = tempfile::tempdir().unwrap();
        assert!(claim(dir.path(), A));
        fs::create_dir(dir.path().join("Default")).unwrap();
        transfer(dir.path(), B, A);
        assert!(!dir.path().join("PURGE").exists());
        transfer(dir.path(), A, A);
        assert!(dir.path().join("PURGE").exists() && claim(dir.path(), A));
        transfer(dir.path(), A, B);
        assert!(claim(dir.path(), B));
        assert!(!claim(dir.path(), A));
        prepare(dir.path(), &[B.to_owned()]);
        assert!(!dir.path().join("Default").exists() && !dir.path().join("PURGE").exists());
        assert!(claim(dir.path(), B));
    }

    #[test]
    fn an_unreadable_owner_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("OWNER")).unwrap();
        assert!(!claim(dir.path(), A));
        assert!(dir.path().join("OWNER").is_dir());
    }
}
