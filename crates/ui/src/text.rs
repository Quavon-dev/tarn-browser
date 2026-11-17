pub use browser_core::text::{bounded_origin, bounded_title, bounded_url, is_bidi};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_controls_bidi_and_bounds_length() {
        assert_eq!(bounded_title("  Hello\n\t world\u{202E}gpj.exe "), "Hello worldgpj.exe");
        assert_eq!(bounded_title(""), "");
        let long = bounded_title(&"a".repeat(500));
        assert_eq!(long.chars().count(), browser_core::text::MAX_TITLE_CHARS);
        assert!(long.ends_with('…'));
    }
}
