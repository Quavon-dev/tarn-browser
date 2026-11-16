#![no_main]

use ed25519_dalek::{Signer, SigningKey};
use libfuzzer_sys::fuzz_target;
use shield::bundle::{MAX_BUNDLE_BYTES, verify};

fuzz_target!(|data: &[u8]| {
    let key = SigningKey::from_bytes(&[7; 32]);
    let public = key.verifying_key().to_bytes();
    let (manifest, file) = match data.iter().position(|&b| b == 0) {
        Some(at) => (&data[..at], &data[at + 1..]),
        None => (data, &[][..]),
    };
    let signature = key.sign(manifest).to_bytes();
    let files = |_: &str, max: u64| (file.len() as u64 <= max).then(|| file.to_vec());
    let Ok(bundle) = verify(manifest, &signature, &files, &public, None) else { return };
    assert!(bundle.lists.iter().map(|l| l.data.len() as u64).sum::<u64>() <= MAX_BUNDLE_BYTES);
    for list in &bundle.lists {
        assert!(list.data.len() as u64 <= list.kind.max_bytes());
    }
    assert!(verify(manifest, &signature, &files, &public, Some(bundle.version)).is_err());
});
