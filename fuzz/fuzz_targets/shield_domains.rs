#![no_main]

use libfuzzer_sys::fuzz_target;
use shield::domains::{DomainSet, entry};
use shield::lists::MAX_RULES_PER_LIST;
use shield::rules::domain_rules;

fuzz_target!(|data: &[u8]| {
    if let Some(set) = DomainSet::from_bytes(data) {
        let _ = set.contains("a.example.com");
    }
    let Ok(text) = std::str::from_utf8(data) else { return };
    for chunk in domain_rules(text) {
        let rules: serde_json::Value = serde_json::from_str(&chunk).expect("a chunk is JSON");
        assert!(rules.as_array().expect("a chunk is an array").len() <= MAX_RULES_PER_LIST);
    }
    let entries: Vec<String> = text.lines().filter_map(entry).collect();
    let set = DomainSet::from_bytes(&DomainSet::encode(entries.iter().map(String::as_str))).expect("encoded entries load");
    for line in text.lines() {
        let _ = set.contains(line);
    }
});
