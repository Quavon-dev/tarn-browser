#![no_main]

use libfuzzer_sys::fuzz_target;
use shield::matcher::Matcher;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let mut parts = text.splitn(4, '\0');
    let list = parts.next().unwrap_or_default();
    let url = parts.next().unwrap_or("https://ads.example/x.js");
    let source = parts.next().unwrap_or("https://a.example/");
    let kind = parts.next().unwrap_or("script");
    let matcher = Matcher::new(&[list], &[list], &[list]);
    let _ = matcher.blocks(url, source, kind);
    let _ = matcher.only(true, false, true).blocks(url, source, kind);
    let _ = matcher.hide_css(source);
});
