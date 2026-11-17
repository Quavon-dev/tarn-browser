//! The last accepted filter bundle on disk, re-verified on every read (S20).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::bundle::{self, BundleError, MANIFEST, MAX_MANIFEST_BYTES, SIGNATURE, VerifiedBundle};

/// A downloaded bundle older than this is refused, so a replayed old bundle cannot be a first install.
pub const MAX_AGE_SECS: u64 = 30 * 86_400;
/// Allowed clock skew for a bundle's version, which is its signing time.
pub const MAX_SKEW_SECS: u64 = 86_400;
const STATE: &str = "state.json";
const BUNDLE: &str = "bundle";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub version: Option<u64>,
    /// UTC seconds of the last update check, successful or not.
    pub checked: u64,
    pub created: String,
    /// Per rule-list group (`ads`, `cosmetic`, ...), the compiled lists WebKit keeps.
    #[serde(default)]
    pub compiled: BTreeMap<String, Stored>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    /// Hex `lists::fingerprint` of the group's input, naming its compiled rule lists.
    pub fingerprint: String,
    pub chunks: usize,
}

pub fn load_state(dir: &Path) -> State {
    fs::read(dir.join(STATE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_state(dir: &Path, state: &State) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join("state.json.tmp");
    fs::write(&tmp, serde_json::to_vec(state).map_err(io::Error::other)?)?;
    fs::rename(tmp, dir.join(STATE))
}

pub fn raise_floor(dir: &Path, version: u64) -> io::Result<()> {
    let mut state = load_state(dir);
    if state.version >= Some(version) {
        return Ok(());
    }
    state.version = Some(version);
    save_state(dir, &state)
}

pub fn check_age(version: u64, now: u64) -> Result<(), BundleError> {
    if version.saturating_add(MAX_AGE_SECS) < now || version > now.saturating_add(MAX_SKEW_SECS) {
        return Err(BundleError::Stale(version));
    }
    Ok(())
}

pub fn read_capped(reader: impl Read, max: u64) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    reader.take(max + 1).read_to_end(&mut data)?;
    if data.len() as u64 > max {
        return Err(io::Error::other("file too large"));
    }
    Ok(data)
}

fn read_file(path: &Path, max: u64) -> Option<Vec<u8>> {
    read_capped(fs::File::open(path).ok()?, max).ok()
}

/// After a crash between the two renames in `write`, only `bundle.old` holds a bundle.
fn restore_interrupted(dir: &Path) {
    let old = dir.join("bundle.old");
    if !dir.join(BUNDLE).exists() && old.exists() {
        fs::rename(old, dir.join(BUNDLE)).ok();
    }
}

/// The cached bundle, verified again so a modified cache is never compiled, and refused when older
/// than the version `state.json` records. That file is not signed: the floor stops a cache swapped
/// for an older signed bundle, not a process that can also rewrite the state.
pub fn read(dir: &Path, public_key: &[u8; 32]) -> Result<VerifiedBundle, BundleError> {
    restore_interrupted(dir);
    let floor = load_state(dir).version;
    let dir = dir.join(BUNDLE);
    let manifest = read_file(&dir.join(MANIFEST), MAX_MANIFEST_BYTES as u64).ok_or(BundleError::Missing(MANIFEST.into()))?;
    let signature = read_file(&dir.join(SIGNATURE), 64).ok_or(BundleError::Missing(SIGNATURE.into()))?;
    bundle::verify(&manifest, &signature, &|name, size| read_file(&dir.join(name), size.saturating_add(1)), public_key, floor.map(|v| v.saturating_sub(1)))
}

/// Replaces the cached bundle; after a crash `read` finds either the old or the new one.
pub fn write(dir: &Path, manifest: &[u8], signature: &[u8], verified: &VerifiedBundle) -> io::Result<()> {
    restore_interrupted(dir);
    let new = dir.join("bundle.new");
    let old = dir.join("bundle.old");
    for stale in [&new, &old] {
        if stale.exists() {
            fs::remove_dir_all(stale)?;
        }
    }
    fs::create_dir_all(&new)?;
    for list in &verified.lists {
        fs::write(new.join(&list.name), &list.data)?;
    }
    fs::write(new.join(SIGNATURE), signature)?;
    fs::write(new.join(MANIFEST), manifest)?;
    let current = dir.join(BUNDLE);
    if current.exists() {
        fs::rename(&current, &old)?;
    }
    fs::rename(&new, &current)?;
    if old.exists() {
        fs::remove_dir_all(&old)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{FORMAT, Kind, ListEntry, Manifest, VerifiedList, sha256_hex};
    use ed25519_dalek::{Signer, SigningKey};

    const LIST: &[u8] = b"||tracker.example^\n";

    fn signed(version: u64) -> (Vec<u8>, Vec<u8>, [u8; 32]) {
        let key = SigningKey::from_bytes(&[7; 32]);
        let manifest = serde_json::to_vec(&Manifest {
            format: FORMAT,
            version,
            created: String::new(),
            lists: vec![ListEntry {
                name: "easylist.txt".into(),
                sha256: sha256_hex(LIST),
                size: LIST.len() as u64,
                category: "ads".into(),
                kind: Kind::Filters,
                count: 1,
            }],
        })
        .unwrap();
        let sig = key.sign(&manifest).to_bytes().to_vec();
        (manifest, sig, key.verifying_key().to_bytes())
    }

    fn store(dir: &Path, version: u64) -> [u8; 32] {
        let (manifest, sig, pk) = signed(version);
        let lists = vec![VerifiedList { name: "easylist.txt".into(), category: "ads".into(), kind: Kind::Filters, count: 1, data: LIST.to_vec() }];
        write(dir, &manifest, &sig, &VerifiedBundle { version, created: String::new(), lists }).unwrap();
        pk
    }

    #[test]
    fn roundtrips_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        store(dir.path(), 5);
        let pk = store(dir.path(), 6);
        let bundle = read(dir.path(), &pk).unwrap();
        assert_eq!(bundle.version, 6);
        assert_eq!(bundle.lists[0].data, LIST);
        assert!(!dir.path().join("bundle.old").exists());
    }

    #[test]
    fn interrupted_write_falls_back_to_old_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let pk = store(dir.path(), 5);
        fs::rename(dir.path().join("bundle"), dir.path().join("bundle.old")).unwrap();
        assert_eq!(read(dir.path(), &pk).unwrap().version, 5);
    }

    #[test]
    fn tampered_cache_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let pk = store(dir.path(), 5);
        fs::write(dir.path().join("bundle/easylist.txt"), b"@@||tracker.example^\n").unwrap();
        assert!(
            matches!(read(dir.path(), &pk), Err(BundleError::SizeMismatch(_) | BundleError::Missing(_))),
            "a file larger than signed is not even read past its size"
        );
        assert_eq!(read(tempfile::tempdir().unwrap().path(), &pk).unwrap_err(), BundleError::Missing(MANIFEST.into()));
    }

    #[test]
    fn older_cached_bundle_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let pk = store(dir.path(), 5);
        let floor = |version| save_state(dir.path(), &State { version: Some(version), ..State::default() }).unwrap();
        floor(6);
        assert!(matches!(read(dir.path(), &pk), Err(BundleError::Rollback { version: 5, .. })));
        floor(5);
        assert_eq!(read(dir.path(), &pk).unwrap().version, 5);
    }

    #[test]
    fn state_roundtrips_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_state(dir.path()), State::default());
        let compiled = BTreeMap::from([("ads".to_owned(), Stored { fingerprint: "ab".into(), chunks: 2 })]);
        let state = State { version: Some(5), checked: 9, created: "2026-11-12T00:00:00Z".into(), compiled };
        save_state(dir.path(), &state).unwrap();
        assert_eq!(load_state(dir.path()), state);
        fs::write(dir.path().join(STATE), b"{").unwrap();
        assert_eq!(load_state(dir.path()), State::default());
    }

    #[test]
    fn the_floor_only_rises() {
        let dir = tempfile::tempdir().unwrap();
        raise_floor(dir.path(), 5).unwrap();
        assert_eq!(load_state(dir.path()).version, Some(5));
        save_state(dir.path(), &State { version: Some(7), checked: 3, ..State::default() }).unwrap();
        raise_floor(dir.path(), 6).unwrap();
        assert_eq!(load_state(dir.path()), State { version: Some(7), checked: 3, ..State::default() });
        raise_floor(dir.path(), 8).unwrap();
        assert_eq!(load_state(dir.path()), State { version: Some(8), checked: 3, ..State::default() });
    }

    #[test]
    fn age_window() {
        let now = 100 * 86_400;
        assert!(check_age(now - MAX_AGE_SECS, now).is_ok());
        assert!(check_age(now + MAX_SKEW_SECS, now).is_ok());
        assert_eq!(check_age(now - MAX_AGE_SECS - 1, now), Err(BundleError::Stale(now - MAX_AGE_SECS - 1)));
        assert!(check_age(now + MAX_SKEW_SECS + 1, now).is_err());
    }

    #[test]
    fn read_capped_limits() {
        assert_eq!(read_capped(&b"abc"[..], 3).unwrap(), b"abc");
        assert!(read_capped(&b"abcd"[..], 3).is_err());
    }
}
