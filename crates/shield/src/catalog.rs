//! The official catalogs of extensions and themes.
//! Each is `index.json` and `index.sig` on a release of its own repository, signed with a key of
//! its own and a context of its own, so neither catalog's signature verifies as the other.

use std::collections::BTreeSet;

use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;

/// Set by the maintainer; all zeros hides the Official section.
pub const EXTENSION_CATALOG_PUBLIC_KEY: [u8; 32] = [0; 32];
pub const CATALOG_REPO: &str = "Quavon-dev/tarn-extensions";
pub const INDEX_URL: &str = "https://github.com/Quavon-dev/tarn-extensions/releases/download/catalog/index.json";
pub const SIGNATURE_URL: &str = "https://github.com/Quavon-dev/tarn-extensions/releases/download/catalog/index.sig";
pub const MAX_INDEX_BYTES: usize = 256 << 10;
pub const KIND: &str = "tarn-extension-catalog";
pub const MAX_AGE_SECS: u64 = 30 * 86_400;
const MAX_SKEW_SECS: u64 = 86_400;
/// Signed before the index bytes, so no filter-bundle signature over the same bytes verifies.
const CONTEXT: &[u8] = b"tarn-extension-catalog\n";
/// Set by the maintainer; all zeros hides the official themes.
pub const THEME_CATALOG_PUBLIC_KEY: [u8; 32] = [0; 32];
/// The largest theme an official entry may name (`browser_core::theme::MAX_THEME_BYTES`).
pub const MAX_THEME_BYTES: u64 = 256 * 1024;

/// One catalog: what its index declares, what is signed before it, and where its files live.
#[derive(Debug, PartialEq, Eq)]
pub struct Catalog {
    pub kind: &'static str,
    context: &'static [u8],
    pub repo: &'static str,
    pub index_url: &'static str,
    pub signature_url: &'static str,
}

pub const EXTENSIONS: Catalog = Catalog { kind: KIND, context: CONTEXT, repo: CATALOG_REPO, index_url: INDEX_URL, signature_url: SIGNATURE_URL };

pub const THEMES: Catalog = Catalog {
    kind: "tarn-theme-catalog",
    context: b"tarn-theme-catalog\n",
    repo: "Quavon-dev/tarn-themes",
    index_url: "https://github.com/Quavon-dev/tarn-themes/releases/download/catalog/index.json",
    signature_url: "https://github.com/Quavon-dev/tarn-themes/releases/download/catalog/index.sig",
};

impl Catalog {
    pub fn signed_message(&self, index: &[u8]) -> Vec<u8> {
        [self.context, index].concat()
    }

    /// A release asset of this catalog's repository: packages are mirrored there, never fetched
    /// from their authors.
    fn valid_url(&self, url: &str) -> bool {
        let prefix = format!("https://github.com/{}/releases/download/", self.repo);
        url.strip_prefix(&prefix).is_some_and(|rest| {
            let parts: Vec<&str> = rest.split('/').collect();
            parts.len() == 2
                && parts.iter().all(|p| !p.is_empty() && *p != "." && *p != ".." && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b)))
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Index {
    pub kind: String,
    pub version: u64,
    /// Unix seconds.
    pub created: u64,
    #[serde(default)]
    pub extensions: Vec<Entry>,
    #[serde(default)]
    pub themes: Vec<ThemeEntry>,
    #[serde(default)]
    pub revoked: Vec<Revoked>,
}

/// An official theme: a `theme.css` mirrored in the theme catalog's repository.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ThemeEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    pub version: String,
    /// `dark` or `light`.
    pub scheme: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// Whether it brings website styles, and for which hosts; a theme with more is refused.
    #[serde(default)]
    pub page_css: bool,
    #[serde(default)]
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub permissions: Vec<String>,
    #[serde(default)]
    pub optional_permissions: Vec<String>,
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
pub struct Revoked {
    pub id: String,
    pub sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CatalogError {
    NoPublicKey,
    TooLarge,
    BadSignature,
    BadIndex,
    Rollback { version: u64, last: u64 },
    Stale,
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CatalogError::NoPublicKey => "no catalog key in this build",
            CatalogError::TooLarge => "the catalog is too large",
            CatalogError::BadSignature => "the catalog's signature does not check out",
            CatalogError::BadIndex => "the catalog is malformed",
            CatalogError::Rollback { .. } => "the offered catalog is older than the last one seen",
            CatalogError::Stale => "the offered catalog is out of date",
        })
    }
}

pub fn signed_message(index: &[u8]) -> Vec<u8> {
    EXTENSIONS.signed_message(index)
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_sha256(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The extension catalog: `verify_catalog` for `EXTENSIONS`.
pub fn verify(index: &[u8], signature: &[u8], public_key: &[u8; 32]) -> Result<Index, CatalogError> {
    verify_catalog(&EXTENSIONS, index, signature, public_key)
}

/// Checks the signature before parsing anything, then the index's own fields. An index carries
/// entries of its own catalog only.
pub fn verify_catalog(catalog: &Catalog, index: &[u8], signature: &[u8], public_key: &[u8; 32]) -> Result<Index, CatalogError> {
    if *public_key == [0; 32] {
        return Err(CatalogError::NoPublicKey);
    }
    if index.len() > MAX_INDEX_BYTES {
        return Err(CatalogError::TooLarge);
    }
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| CatalogError::NoPublicKey)?;
    let signature = Signature::from_slice(signature).map_err(|_| CatalogError::BadSignature)?;
    key.verify_strict(&catalog.signed_message(index), &signature).map_err(|_| CatalogError::BadSignature)?;
    let parsed: Index = serde_json::from_slice(index).map_err(|_| CatalogError::BadIndex)?;
    let host_ok = |h: &String| (1..=253).contains(&h.len()) && h.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-*".contains(&b));
    let entries_ok = if catalog == &THEMES {
        parsed.extensions.is_empty()
            && parsed.themes.iter().all(|e| {
                valid_id(&e.id)
                    && valid_sha256(&e.sha256)
                    && catalog.valid_url(&e.url)
                    && e.size <= MAX_THEME_BYTES
                    && matches!(e.scheme.as_str(), "dark" | "light")
                    && e.hosts.iter().all(host_ok)
                    && (e.page_css || e.hosts.is_empty())
            })
    } else {
        parsed.themes.is_empty() && parsed.extensions.iter().all(|e| valid_id(&e.id) && valid_sha256(&e.sha256) && catalog.valid_url(&e.url))
    };
    let mut ids = BTreeSet::new();
    let unique = parsed.extensions.iter().map(|e| e.id.as_str()).chain(parsed.themes.iter().map(|e| e.id.as_str())).all(|id| ids.insert(id));
    if parsed.kind != catalog.kind || !entries_ok || !unique || !parsed.revoked.iter().all(|r| valid_id(&r.id) && valid_sha256(&r.sha256)) {
        return Err(CatalogError::BadIndex);
    }
    Ok(parsed)
}

/// A fetched index must be at least as new as the last one seen and at most 30 days old; a
/// cached one is only verified again, since fetching a fresh one needs a click.
pub fn check_fetched(index: &Index, last: Option<u64>, now: u64) -> Result<(), CatalogError> {
    if let Some(last) = last.filter(|l| index.version < *l) {
        return Err(CatalogError::Rollback { version: index.version, last });
    }
    if index.created.saturating_add(MAX_AGE_SECS) < now || index.created > now.saturating_add(MAX_SKEW_SECS) {
        return Err(CatalogError::Stale);
    }
    Ok(())
}

pub fn is_old(index: &Index, now: u64) -> bool {
    index.created.saturating_add(MAX_AGE_SECS) < now
}

/// The stored revoked set (`id sha256` per line) with the index's entries added; it only grows.
pub fn merge_revoked(stored: &str, index: &Index) -> String {
    let mut set: BTreeSet<(String, String)> = parse_revoked(stored);
    set.extend(index.revoked.iter().map(|r| (r.id.clone(), r.sha256.clone())));
    set.into_iter().map(|(id, sha)| format!("{id} {sha}")).collect::<Vec<_>>().join("\n")
}

pub fn parse_revoked(stored: &str) -> BTreeSet<(String, String)> {
    stored.lines().filter_map(|l| l.split_once(' ')).map(|(id, sha)| (id.to_owned(), sha.to_owned())).collect()
}

/// Whether the loaded extension asks for nothing beyond what its entry lists.
pub fn covers(entry: &Entry, permissions: &[String], optional: &[String], hosts: &[String]) -> bool {
    let listed = |have: &[String], want: &[String]| want.iter().all(|w| have.contains(w));
    listed(&entry.permissions, permissions) && listed(&entry.optional_permissions, optional) && listed(&entry.hosts, hosts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const NOW: u64 = 1_800_000_000;

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[9; 32])
    }

    fn index(version: u64, created: u64) -> Vec<u8> {
        serde_json::json!({
            "kind": KIND, "version": version, "created": created, "future": true,
            "extensions": [{
                "id": "ublock-lite", "name": "uBlock Origin Lite", "description": "Blocks ads", "version": "1.0",
                "url": format!("https://github.com/{CATALOG_REPO}/releases/download/ublock-lite-1.0/ublock.zip"),
                "sha256": "a".repeat(64), "size": 10, "permissions": ["storage"], "hosts": ["<all_urls>"], "engines": ["webkit"]
            }],
            "revoked": [{"id": "old", "sha256": "b".repeat(64)}]
        })
        .to_string()
        .into_bytes()
    }

    fn signed(bytes: &[u8]) -> Vec<u8> {
        key().sign(&signed_message(bytes)).to_bytes().to_vec()
    }

    fn public() -> [u8; 32] {
        key().verifying_key().to_bytes()
    }

    #[test]
    fn a_signed_index_verifies_and_ignores_unknown_fields() {
        let bytes = index(2, NOW);
        let parsed = verify(&bytes, &signed(&bytes), &public()).unwrap();
        assert_eq!(parsed.extensions[0].id, "ublock-lite");
        assert!(check_fetched(&parsed, Some(2), NOW).is_ok());
    }

    #[test]
    fn refuses_bad_and_foreign_signatures() {
        let bytes = index(2, NOW);
        let mut sig = signed(&bytes);
        sig[0] ^= 1;
        assert_eq!(verify(&bytes, &sig, &public()).unwrap_err(), CatalogError::BadSignature);
        let filter_bundle_style = key().sign(&bytes).to_bytes();
        assert_eq!(verify(&bytes, &filter_bundle_style, &public()).unwrap_err(), CatalogError::BadSignature);
        assert_eq!(verify(&bytes, &signed(&bytes), &[0; 32]).unwrap_err(), CatalogError::NoPublicKey);
    }

    #[test]
    fn refuses_rollback_stale_and_oversized_indexes() {
        let old = verify(&index(1, NOW), &signed(&index(1, NOW)), &public()).unwrap();
        assert_eq!(check_fetched(&old, Some(2), NOW), Err(CatalogError::Rollback { version: 1, last: 2 }));
        let stale = verify(&index(3, NOW - MAX_AGE_SECS - 1), &signed(&index(3, NOW - MAX_AGE_SECS - 1)), &public()).unwrap();
        assert_eq!(check_fetched(&stale, Some(2), NOW), Err(CatalogError::Stale));
        assert!(is_old(&stale, NOW));
        let big = vec![b' '; MAX_INDEX_BYTES + 1];
        assert_eq!(verify(&big, &signed(&big), &public()).unwrap_err(), CatalogError::TooLarge);
    }

    #[test]
    fn refuses_wrong_kind_ids_and_urls() {
        for (from, to) in [
            (KIND.to_owned(), "tarn-filter-bundle".to_owned()),
            ("ublock-lite".to_owned(), "uBlock".to_owned()),
            (format!("{CATALOG_REPO}/releases"), "someone/else/releases".to_owned()),
        ] {
            let bytes = String::from_utf8(index(2, NOW)).unwrap().replacen(&from, &to, 1).into_bytes();
            assert_eq!(verify(&bytes, &signed(&bytes), &public()).unwrap_err(), CatalogError::BadIndex, "{to}");
        }
    }

    fn theme_index(entry: serde_json::Value) -> Vec<u8> {
        serde_json::json!({ "kind": THEMES.kind, "version": 4, "created": NOW, "themes": [entry], "revoked": [{"id": "bad", "sha256": "c".repeat(64)}] })
            .to_string()
            .into_bytes()
    }

    fn theme_entry() -> serde_json::Value {
        serde_json::json!({
            "id": "glacier", "name": "Glacier", "version": "1", "scheme": "light", "size": 900,
            "url": format!("https://github.com/{}/releases/download/glacier-1/theme.css", THEMES.repo),
            "sha256": "d".repeat(64), "page_css": true, "hosts": ["github.com", "*.example.org"]
        })
    }

    #[test]
    fn theme_and_extension_catalogs_never_verify_as_each_other() {
        let themes = theme_index(theme_entry());
        let theme_sig = key().sign(&THEMES.signed_message(&themes)).to_bytes();
        let parsed = verify_catalog(&THEMES, &themes, &theme_sig, &public()).unwrap();
        assert_eq!((parsed.themes[0].id.as_str(), parsed.revoked.len()), ("glacier", 1));
        assert_eq!(verify(&themes, &theme_sig, &public()).unwrap_err(), CatalogError::BadSignature, "a theme signature is not an extension one");
        let extensions = index(2, NOW);
        assert_eq!(verify_catalog(&THEMES, &extensions, &signed(&extensions), &public()).unwrap_err(), CatalogError::BadSignature);
        let as_theme_sig = key().sign(&THEMES.signed_message(&extensions)).to_bytes();
        assert_eq!(verify_catalog(&THEMES, &extensions, &as_theme_sig, &public()).unwrap_err(), CatalogError::BadIndex, "wrong kind");
        assert_eq!(verify_catalog(&THEMES, &themes, &theme_sig, &[0; 32]).unwrap_err(), CatalogError::NoPublicKey);
    }

    #[test]
    fn theme_entries_are_checked() {
        let mut bad = Vec::new();
        for (field, value) in [
            ("url", serde_json::json!(format!("https://github.com/{CATALOG_REPO}/releases/download/x/theme.css"))),
            ("size", serde_json::json!(MAX_THEME_BYTES + 1)),
            ("scheme", serde_json::json!("sepia")),
            ("hosts", serde_json::json!(["Evil Host"])),
            ("page_css", serde_json::json!(false)),
            ("sha256", serde_json::json!("x")),
        ] {
            let mut entry = theme_entry();
            entry[field] = value;
            bad.push((field, theme_index(entry)));
        }
        for (field, bytes) in bad {
            let sig = key().sign(&THEMES.signed_message(&bytes)).to_bytes();
            assert_eq!(verify_catalog(&THEMES, &bytes, &sig, &public()).unwrap_err(), CatalogError::BadIndex, "{field}");
        }
    }

    #[test]
    fn an_extension_may_not_ask_for_more_than_its_entry() {
        let bytes = index(2, NOW);
        let entry = verify(&bytes, &signed(&bytes), &public()).unwrap().extensions.remove(0);
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(covers(&entry, &s(&["storage"]), &[], &s(&["<all_urls>"])));
        assert!(!covers(&entry, &s(&["storage", "tabs"]), &[], &[]));
        assert!(!covers(&entry, &[], &s(&["cookies"]), &[]));
        assert!(!covers(&entry, &[], &[], &s(&["*://bank.test/*"])));
    }

    #[test]
    fn the_revoked_set_only_grows() {
        let bytes = index(2, NOW);
        let parsed = verify(&bytes, &signed(&bytes), &public()).unwrap();
        let merged = merge_revoked(&format!("x {}", "c".repeat(64)), &parsed);
        assert!(parse_revoked(&merged).contains(&("old".to_owned(), "b".repeat(64))));
        assert!(parse_revoked(&merged).contains(&("x".to_owned(), "c".repeat(64))));
        let mut older = parsed.clone();
        older.revoked.clear();
        assert_eq!(merge_revoked(&merged, &older), merged);
    }
}
