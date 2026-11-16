#![no_main]

use libfuzzer_sys::fuzz_target;
use shield::url_clean::clean_link;

fuzz_target!(|data: &[u8]| {
    let Ok(url) = std::str::from_utf8(data) else { return };
    let (cleaned, removed) = clean_link(url);
    assert!(removed >= 0);
    if cleaned != url {
        assert!(removed > 0);
        assert_eq!(clean_link(&cleaned), (cleaned.clone(), 0));
    }
});
