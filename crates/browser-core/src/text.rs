const MAX_ORIGIN_CHARS: usize = 80;

/// Origins keep their end, where the registrable domain is, when shortened (S5).
pub fn bounded_origin(raw: &str) -> String {
    let clean: Vec<char> = raw.chars().filter(|c| !c.is_control() && !is_bidi(*c) && !c.is_whitespace()).collect();
    if clean.len() <= MAX_ORIGIN_CHARS {
        return clean.into_iter().collect();
    }
    std::iter::once('…').chain(clean[clean.len() - (MAX_ORIGIN_CHARS - 1)..].iter().copied()).collect()
}

pub const MAX_TITLE_CHARS: usize = 120;

/// Page titles are attacker-controlled: drop control and bidi characters, collapse whitespace
/// and cap the length before the shell renders them (S14).
pub fn bounded_title(raw: &str) -> String {
    let mut out = String::new();
    for word in
        raw.split(|c: char| c.is_whitespace() || c.is_control()).map(|w| w.chars().filter(|c| !is_bidi(*c)).collect::<String>()).filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&word);
    }
    if out.chars().count() > MAX_TITLE_CHARS {
        out = out.chars().take(MAX_TITLE_CHARS - 1).collect::<String>() + "…";
    }
    out
}

pub fn is_bidi(c: char) -> bool {
    // Kept in sync with engine::downloads and ai::page_context, which cannot depend on `browser-core`.
    matches!(c, '\u{061C}' | '\u{115F}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}' | '\u{E0000}'..='\u{E007F}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invisible_and_bidi_characters() {
        for c in ['\u{061C}', '\u{180E}', '\u{200E}', '\u{2028}', '\u{2029}', '\u{202E}', '\u{2060}', '\u{2067}', '\u{FEFF}', '\u{3164}', '\u{E0041}'] {
            assert!(is_bidi(c), "{:?}", c);
        }
        assert!(!is_bidi('a') && !is_bidi('é'));
    }

    #[test]
    fn origins_keep_their_end() {
        let long = format!("https://accounts.example.com.{}.evil.test", "a".repeat(120));
        let shown = bounded_origin(&long);
        assert!(shown.starts_with('…') && shown.ends_with(".evil.test"));
        assert_eq!(shown.chars().count(), MAX_ORIGIN_CHARS);
        assert_eq!(bounded_origin("https://a.test"), "https://a.test");
    }
}
