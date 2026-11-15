//! Signed filter-list bundles (S20): `manifest.json`, the list files it names and `manifest.sig`.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domains::{ENTRY_BYTES, MAX_ENTRIES};

/// Set by the maintainer to the key printed by `filter-bundle keygen`; all zeros rejects every bundle.
pub const FILTER_BUNDLE_PUBLIC_KEY: [u8; 32] =
    [172, 36, 119, 177, 98, 84, 108, 114, 136, 3, 118, 93, 99, 188, 221, 59, 237, 103, 140, 205, 108, 25, 46, 190, 220, 234, 160, 89, 101, 186, 5, 83];
/// `https://github.com/<owner>/<repo>/releases/download/filter-lists/`, set with the key; the app
/// downloads nothing while either is unset.
pub const FILTER_BUNDLE_URL: &str = "https://github.com/Quavon-dev/tarn-browser/releases/download/filter-lists/";
pub const FORMAT: u32 = 2;
pub const MAX_BUNDLE_BYTES: u64 = 32 << 20;
pub const MAX_MANIFEST_BYTES: usize = 64 << 10;
pub const MANIFEST: &str = "manifest.json";
pub const SIGNATURE: &str = "manifest.sig";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub version: u64,
    pub created: String,
    pub lists: Vec<ListEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListEntry {
    pub name: String,
    pub sha256: String,
    pub size: u64,
    /// A `Category` name, or `youtube` for its config; entries of a category this build does not
    /// know are verified and ignored, so a later category does not freeze older builds.
    pub category: String,
    pub kind: Kind,
    /// Rules, domains or set entries, as counted when the bundle was prepared; shown in Settings.
    pub count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// EasyList-syntax filter text.
    Filters,
    /// One domain per line, for content rules.
    Domains,
    /// A `domains::DomainSet` file.
    Set,
    /// JSON configuration.
    Config,
    /// Licence and attribution text.
    License,
    #[serde(other)]
    Other,
}

impl Kind {
    pub fn max_bytes(self) -> u64 {
        match self {
            Kind::Filters => 12 << 20,
            Kind::Domains => 2 << 20,
            Kind::Set => (MAX_ENTRIES * ENTRY_BYTES) as u64,
            Kind::Config => 64 << 10,
            Kind::License | Kind::Other => 1 << 20,
        }
    }
}

#[derive(Debug)]
pub struct VerifiedBundle {
    pub version: u64,
    pub created: String,
    /// In manifest order.
    pub lists: Vec<VerifiedList>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedList {
    pub name: String,
    pub category: String,
    pub kind: Kind,
    pub count: u64,
    pub data: Vec<u8>,
}

impl VerifiedBundle {
    pub fn of(&self, category: &str, kind: Kind) -> impl Iterator<Item = &VerifiedList> {
        let category = category.to_owned();
        self.lists.iter().filter(move |l| l.category == category && l.kind == kind)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum BundleError {
    NoPublicKey,
    BadSignature,
    BadManifest,
    UnknownFormat(u32),
    Rollback { version: u64, last: u64 },
    BadName(String),
    TooLarge,
    Missing(String),
    SizeMismatch(String),
    HashMismatch(String),
    Stale(u64),
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn valid_category(category: &str) -> bool {
    (1..=16).contains(&category.len()) && category.bytes().all(|b| b.is_ascii_lowercase())
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
        && name != MANIFEST
        && name != SIGNATURE
}

pub fn verify(
    manifest: &[u8],
    signature: &[u8],
    files: &dyn Fn(&str, u64) -> Option<Vec<u8>>,
    public_key: &[u8; 32],
    last_version: Option<u64>,
) -> Result<VerifiedBundle, BundleError> {
    if public_key == &[0; 32] {
        return Err(BundleError::NoPublicKey);
    }
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err(BundleError::TooLarge);
    }
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| BundleError::NoPublicKey)?;
    let signature = Signature::from_slice(signature).map_err(|_| BundleError::BadSignature)?;
    key.verify_strict(manifest, &signature).map_err(|_| BundleError::BadSignature)?;

    let parsed: Manifest = serde_json::from_slice(manifest).map_err(|_| BundleError::BadManifest)?;
    if parsed.format != FORMAT {
        return Err(BundleError::UnknownFormat(parsed.format));
    }
    if let Some(last) = last_version
        && parsed.version <= last
    {
        return Err(BundleError::Rollback { version: parsed.version, last });
    }
    let mut total = 0u64;
    for (i, entry) in parsed.lists.iter().enumerate() {
        if !valid_name(&entry.name) || !valid_category(&entry.category) || parsed.lists[..i].iter().any(|e| e.name == entry.name) {
            return Err(BundleError::BadName(entry.name.clone()));
        }
        if entry.size > entry.kind.max_bytes() {
            return Err(BundleError::TooLarge);
        }
        total = total.saturating_add(entry.size);
    }
    if total > MAX_BUNDLE_BYTES {
        return Err(BundleError::TooLarge);
    }

    let mut lists = Vec::with_capacity(parsed.lists.len());
    for entry in parsed.lists {
        let data = files(&entry.name, entry.size).ok_or_else(|| BundleError::Missing(entry.name.clone()))?;
        if data.len() as u64 != entry.size {
            return Err(BundleError::SizeMismatch(entry.name));
        }
        if sha256_hex(&data) != entry.sha256 {
            return Err(BundleError::HashMismatch(entry.name));
        }
        lists.push(VerifiedList { name: entry.name, category: entry.category, kind: entry.kind, count: entry.count, data });
    }
    Ok(VerifiedBundle { version: parsed.version, created: parsed.created, lists })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::collections::HashMap;

    const LIST: &[u8] = b"||tracker.example^\n";

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn manifest_json(format: u32, version: u64, lists: &[(&str, &[u8])]) -> Vec<u8> {
        let lists = lists.iter().map(|(name, data)| entry(name, data)).collect();
        serde_json::to_vec(&Manifest { format, version, created: "2026-11-12T00:00:00Z".into(), lists }).unwrap()
    }

    fn entry(name: &str, data: &[u8]) -> ListEntry {
        ListEntry { name: name.into(), sha256: sha256_hex(data), size: data.len() as u64, category: "ads".into(), kind: Kind::Filters, count: 1 }
    }

    fn check(manifest: &[u8], files: HashMap<&str, Vec<u8>>, last: Option<u64>) -> Result<VerifiedBundle, BundleError> {
        let sig = key().sign(manifest).to_bytes();
        verify(manifest, &sig, &|n, _| files.get(n).cloned(), &key().verifying_key().to_bytes(), last)
    }

    fn files() -> HashMap<&'static str, Vec<u8>> {
        HashMap::from([("easylist.txt", LIST.to_vec())])
    }

    #[test]
    fn accepts_valid_bundle() {
        let bundle = check(&manifest_json(FORMAT, 5, &[("easylist.txt", LIST)]), files(), Some(4)).unwrap();
        assert_eq!(bundle.version, 5);
        assert_eq!(
            bundle.lists,
            vec![VerifiedList { name: "easylist.txt".into(), category: "ads".into(), kind: Kind::Filters, count: 1, data: LIST.to_vec() }]
        );
        assert_eq!(bundle.of("ads", Kind::Filters).count(), 1);
        assert_eq!(bundle.of("ads", Kind::Set).count(), 0);
    }

    #[test]
    fn rejects_everything_with_zero_key() {
        let manifest = manifest_json(FORMAT, 5, &[("easylist.txt", LIST)]);
        let sig = key().sign(&manifest).to_bytes();
        let f = files();
        let result = verify(&manifest, &sig, &|n, _| f.get(n).cloned(), &[0; 32], None);
        assert_eq!(result.unwrap_err(), BundleError::NoPublicKey);
    }

    #[test]
    fn rejects_bad_signature() {
        let manifest = manifest_json(FORMAT, 5, &[("easylist.txt", LIST)]);
        let mut sig = key().sign(&manifest).to_bytes();
        sig[0] ^= 1;
        let pk = key().verifying_key().to_bytes();
        let f = files();
        assert_eq!(verify(&manifest, &sig, &|n, _| f.get(n).cloned(), &pk, None).unwrap_err(), BundleError::BadSignature);
        assert_eq!(verify(&manifest, &sig[..10], &|n, _| f.get(n).cloned(), &pk, None).unwrap_err(), BundleError::BadSignature);
        let other = SigningKey::from_bytes(&[8; 32]).sign(&manifest).to_bytes();
        assert_eq!(verify(&manifest, &other, &|n, _| f.get(n).cloned(), &pk, None).unwrap_err(), BundleError::BadSignature);
    }

    #[test]
    fn rejects_tampered_manifest() {
        let manifest = manifest_json(FORMAT, 5, &[("easylist.txt", LIST)]);
        let sig = key().sign(&manifest).to_bytes();
        let mut tampered = manifest.clone();
        tampered.push(b' ');
        let f = files();
        let result = verify(&tampered, &sig, &|n, _| f.get(n).cloned(), &key().verifying_key().to_bytes(), None);
        assert_eq!(result.unwrap_err(), BundleError::BadSignature);
    }

    #[test]
    fn rejects_malformed_manifest() {
        assert_eq!(check(b"{\"format\":1}", files(), None).unwrap_err(), BundleError::BadManifest);
    }

    #[test]
    fn rejects_unknown_format() {
        let manifest = manifest_json(1, 5, &[("easylist.txt", LIST)]);
        assert_eq!(check(&manifest, files(), None).unwrap_err(), BundleError::UnknownFormat(1));
    }

    #[test]
    fn rejects_rollback_and_replay() {
        let manifest = manifest_json(FORMAT, 5, &[("easylist.txt", LIST)]);
        assert_eq!(check(&manifest, files(), Some(5)).unwrap_err(), BundleError::Rollback { version: 5, last: 5 });
        assert_eq!(check(&manifest, files(), Some(9)).unwrap_err(), BundleError::Rollback { version: 5, last: 9 });
    }

    #[test]
    fn rejects_bad_names() {
        for name in ["../easylist.txt", "EasyList.txt", "a/b", "", ".hidden", "manifest.json", "manifest.sig", "a b"] {
            let manifest = manifest_json(FORMAT, 5, &[(name, LIST)]);
            let f = HashMap::from([(name, LIST.to_vec())]);
            assert_eq!(check(&manifest, f, None).unwrap_err(), BundleError::BadName(name.into()), "{name}");
        }
        let manifest = manifest_json(FORMAT, 5, &[("easylist.txt", LIST), ("easylist.txt", LIST)]);
        assert_eq!(check(&manifest, files(), None).unwrap_err(), BundleError::BadName("easylist.txt".into()));
    }

    #[test]
    fn rejects_oversized_bundle() {
        let lists = vec![ListEntry { size: MAX_BUNDLE_BYTES + 1, ..entry("big.txt", b"") }];
        let manifest = serde_json::to_vec(&Manifest { format: FORMAT, version: 5, created: String::new(), lists }).unwrap();
        assert_eq!(check(&manifest, HashMap::new(), None).unwrap_err(), BundleError::TooLarge);
        assert_eq!(check(&vec![b' '; MAX_MANIFEST_BYTES + 1], HashMap::new(), None).unwrap_err(), BundleError::TooLarge);
    }

    #[test]
    fn caps_each_kind_and_checks_categories() {
        let config = ListEntry { kind: Kind::Config, size: Kind::Config.max_bytes() + 1, ..entry("youtube.json", b"") };
        let manifest = serde_json::to_vec(&Manifest { format: FORMAT, version: 5, created: String::new(), lists: vec![config] }).unwrap();
        assert_eq!(check(&manifest, HashMap::new(), None).unwrap_err(), BundleError::TooLarge);
        for category in ["", "Ads", "a-b", "abcdefghijklmnopq"] {
            let bad = ListEntry { category: category.into(), ..entry("easylist.txt", LIST) };
            let manifest = serde_json::to_vec(&Manifest { format: FORMAT, version: 5, created: String::new(), lists: vec![bad] }).unwrap();
            assert_eq!(check(&manifest, files(), None).unwrap_err(), BundleError::BadName("easylist.txt".into()), "{category:?}");
        }
    }

    #[test]
    fn config_categories_of_later_features_are_valid() {
        assert!(valid_category("youtube") && valid_category(crate::revocations::CATEGORY));
        assert!(valid_name(crate::revocations::FILE));
    }

    #[test]
    fn unknown_kinds_and_categories_are_kept_for_the_caller_to_ignore() {
        let json = format!(
            r#"{{"format":{FORMAT},"version":5,"created":"","lists":[{{"name":"easylist.txt","sha256":"{}","size":{},"category":"later","kind":"future","count":3}}]}}"#,
            sha256_hex(LIST),
            LIST.len()
        );
        let bundle = check(json.as_bytes(), files(), None).unwrap();
        assert_eq!((bundle.lists[0].kind, bundle.lists[0].category.as_str()), (Kind::Other, "later"));
    }

    #[test]
    fn rejects_missing_size_and_hash_mismatch() {
        let manifest = manifest_json(FORMAT, 5, &[("easylist.txt", LIST)]);
        assert_eq!(check(&manifest, HashMap::new(), None).unwrap_err(), BundleError::Missing("easylist.txt".into()));
        let longer = HashMap::from([("easylist.txt", [LIST, b"x"].concat())]);
        assert_eq!(check(&manifest, longer, None).unwrap_err(), BundleError::SizeMismatch("easylist.txt".into()));
        let mut flipped = LIST.to_vec();
        flipped[0] = b'@';
        let changed = HashMap::from([("easylist.txt", flipped)]);
        assert_eq!(check(&manifest, changed, None).unwrap_err(), BundleError::HashMismatch("easylist.txt".into()));
    }
}
