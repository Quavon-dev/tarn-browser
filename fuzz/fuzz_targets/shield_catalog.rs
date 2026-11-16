#![no_main]

use ed25519_dalek::{Signer, SigningKey};
use libfuzzer_sys::fuzz_target;
use shield::catalog::{EXTENSIONS, THEMES, check_fetched, covers, is_old, merge_revoked, parse_revoked, verify_catalog};

fuzz_target!(|data: &[u8]| {
    let Some((&which, rest)) = data.split_first() else { return };
    let catalog = if which & 1 == 0 { &EXTENSIONS } else { &THEMES };
    let key = SigningKey::from_bytes(&[7; 32]);
    let public = key.verifying_key().to_bytes();
    let (signature, index) = rest.split_at(rest.len().min(64));
    let _ = verify_catalog(catalog, index, signature, &public);
    let signature = key.sign(&catalog.signed_message(rest)).to_bytes();
    let Ok(parsed) = verify_catalog(catalog, rest, &signature, &public) else { return };
    assert_eq!(parsed.kind, catalog.kind);
    let _ = check_fetched(&parsed, Some(u64::from(which)), parsed.created);
    let _ = is_old(&parsed, u64::MAX);
    let merged = merge_revoked("", &parsed);
    assert_eq!(parse_revoked(&merged).len(), parse_revoked(&merge_revoked(&merged, &parsed)).len());
    for entry in &parsed.extensions {
        assert!(covers(entry, &entry.permissions, &entry.optional_permissions, &entry.hosts));
    }
});
