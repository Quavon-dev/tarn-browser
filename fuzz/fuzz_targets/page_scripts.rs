#![no_main]

use engine::page_scripts::{FIND_MATCHES, MAX_THEME_SHEETS, find_snippets, script_string, theme_sheets};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let snippets = find_snippets(text);
    assert!(snippets.len() <= FIND_MATCHES);
    assert!(snippets.iter().all(|s| s.chars().count() <= 320 && !s.contains('\n')));
    if let Some(literal) = script_string(text) {
        assert!(!literal[1..literal.len() - 1].contains(['"', '\\', '\n', '\r']));
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        let sheets = theme_sheets(&value);
        assert!(sheets.iter().map(|(_, css)| css.len()).sum::<usize>() <= MAX_THEME_SHEETS);
    }
});
