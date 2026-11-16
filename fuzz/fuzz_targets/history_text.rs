#![no_main]

use libfuzzer_sys::fuzz_target;
use store::{MAX_HISTORY_TEXT, clip_history_text};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let clipped = clip_history_text(text);
    assert!(clipped.len() <= MAX_HISTORY_TEXT);
    assert!(text.starts_with(clipped));
    if text.len() <= MAX_HISTORY_TEXT {
        assert_eq!(clipped, text);
    } else {
        assert!(MAX_HISTORY_TEXT - clipped.len() < 4);
    }
});
