#![no_main]

use browser_core::theme::{MAX_THEME_BYTES, Source, parse, parse_as_written, to_css};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&which, rest)) = data.split_first() else { return };
    let Ok(css) = std::str::from_utf8(rest) else { return };
    let source = [Source::Yours, Source::BuiltIn, Source::Official, Source::Unverified][usize::from(which % 4)];
    let parsed = if which & 4 == 0 { parse(css, source) } else { parse_as_written(css, source) };
    let Ok(parsed) = parsed else {
        assert!(css.len() > MAX_THEME_BYTES);
        return;
    };
    let _ = parse_as_written(&to_css(&parsed), source);
});
