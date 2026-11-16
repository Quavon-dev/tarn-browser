#![no_main]

use libfuzzer_sys::fuzz_target;
use shield::youtube::{parse, script};

fuzz_target!(|data: &[u8]| {
    if let Some(config) = parse(data) {
        assert!(!config.keys.is_empty() && config.keys.len() <= 32);
        assert!(config.keys.iter().all(|k| (1..=40).contains(&k.len()) && k.bytes().all(|b| b.is_ascii_alphabetic())));
        assert!(!script(&config).contains("__TARN_KEYS__"));
    }
});
