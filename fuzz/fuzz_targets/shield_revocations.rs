#![no_main]

use libfuzzer_sys::fuzz_target;
use shield::revocations::{contains, merge, parse};

fuzz_target!(|data: &[u8]| {
    let stored = std::str::from_utf8(data).unwrap_or_default();
    let new = parse(data).unwrap_or_default();
    let merged = merge(stored, &new);
    for hash in &new.sha256 {
        assert!(contains(&merged, hash));
    }
    for line in stored.lines() {
        let _ = contains(stored, line);
    }
    assert_eq!(merge(&merged, &new), merged);
});
