pub use browser_core::text::{bounded_origin, bounded_title, is_bidi};

/// A URL shown in full up to `MAX_URL_CHARS`; a longer one keeps its start and its end, where an
/// app link's risky arguments often sit, with an ellipsis between.
pub fn bounded_url(raw: &str) -> String {
    const MAX_URL_CHARS: usize = 200;
    let clean: Vec<char> = raw.chars().filter(|c| !c.is_control() && !is_bidi(*c)).collect();
    if clean.len() <= MAX_URL_CHARS {
        return clean.into_iter().collect();
    }
    let half = (MAX_URL_CHARS - 1) / 2;
    clean[..half].iter().chain(std::iter::once(&'…')).chain(&clean[clean.len() - half..]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_urls_keep_their_start_and_end() {
        let url = format!("ms-officecmd:{}&tail=run", "a".repeat(400));
        let shown = bounded_url(&url);
        assert!(shown.starts_with("ms-officecmd:") && shown.ends_with("&tail=run") && shown.contains('…'));
        assert!(shown.chars().count() <= 200);
        assert_eq!(bounded_url("zoommtg://x\u{202E}y"), "zoommtg://xy");
    }

    #[test]
    fn strips_controls_bidi_and_bounds_length() {
        assert_eq!(bounded_title("  Hello\n\t world\u{202E}gpj.exe "), "Hello worldgpj.exe");
        assert_eq!(bounded_title(""), "");
        let long = bounded_title(&"a".repeat(500));
        assert_eq!(long.chars().count(), browser_core::text::MAX_TITLE_CHARS);
        assert!(long.ends_with('…'));
    }
}
