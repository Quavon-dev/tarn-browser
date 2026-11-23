#![no_main]

use libfuzzer_sys::fuzz_target;
use shield::lists::{HideSheets, MAX_RULES_PER_LIST, compile, compile_cosmetic, valid_zap, without_invalid_domain_entries, zap_css};

fn check_chunks(chunks: &[String]) {
    for chunk in chunks {
        let rules: serde_json::Value = serde_json::from_str(chunk).expect("a chunk is JSON");
        assert!(rules.as_array().expect("a chunk is an array").len() <= MAX_RULES_PER_LIST);
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    check_chunks(&compile(&[text], &[text]).chunks);
    let cleaned = without_invalid_domain_entries(text);
    assert_eq!(without_invalid_domain_entries(&cleaned), cleaned);
    check_chunks(&compile_cosmetic(&[text]).chunks);
    let sheets = HideSheets::new(&[text]);
    let _ = sheets.css("a.example.com");
    for line in text.lines() {
        let _ = sheets.css(line);
        if valid_zap(line) {
            assert!(zap_css([line]).is_some());
        }
    }
});
