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

/// A file name for a capture of the page: its title, else its host, as one safe path component.
pub fn capture_name(title: &str, url: Option<&url::Url>, ext: &str) -> String {
    let title = bounded_title(title);
    let base = if title.is_empty() { url.and_then(url::Url::host_str).unwrap_or("Page") } else { &title };
    format!("{}.{ext}", engine::downloads::sanitize_filename(&base.replace(['/', '\\'], "-")))
}

pub fn is_bidi(c: char) -> bool {
    // Kept in sync with engine::downloads and ai::page_context, which cannot depend on `browser-core`.
    matches!(c, '\u{061C}' | '\u{115F}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}' | '\u{E0000}'..='\u{E007F}')
}

pub fn title_or_host(title: &str, url: &str) -> String {
    let title = bounded_title(title);
    if !title.is_empty() {
        return title;
    }
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)).unwrap_or_else(|| bounded_title(url))
}

pub fn markdown_link(title: &str, url: &str) -> String {
    let title = bounded_title(title);
    let text = if title.is_empty() { url } else { &title };
    let mut escaped = String::new();
    for c in text.chars() {
        if matches!(c, '\\' | '[' | ']') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    format!("[{escaped}]({})", url.replace('(', "%28").replace(')', "%29").replace(' ', "%20"))
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
    fn an_untitled_page_is_named_by_its_host() {
        assert_eq!(title_or_host(" Inbox ", "https://mail.test/"), "Inbox");
        assert_eq!(title_or_host("", "https://Mail.Test/x"), "mail.test");
        assert_eq!(title_or_host("", "about:blank"), "about:blank");
    }

    #[test]
    fn invisible_and_bidi_characters() {
        for c in ['\u{061C}', '\u{180E}', '\u{200E}', '\u{2028}', '\u{2029}', '\u{202E}', '\u{2060}', '\u{2067}', '\u{FEFF}', '\u{3164}', '\u{E0041}'] {
            assert!(is_bidi(c), "{:?}", c);
        }
        assert!(!is_bidi('a') && !is_bidi('é'));
    }

    #[test]
    fn capture_names_are_one_safe_component() {
        let url = url::Url::parse("https://news.example/a").ok();
        assert_eq!(capture_name("Q3 / Q4 report", url.as_ref(), "pdf"), "Q3 - Q4 report.pdf");
        assert_eq!(capture_name("  \u{202E}", url.as_ref(), "png"), "news.example.png");
        assert_eq!(capture_name("", None, "png"), "Page.png");
        assert_eq!(capture_name("../../etc", None, "pdf"), "-..-etc.pdf");
        assert_eq!(capture_name("..", None, "pdf"), "download.pdf");
        assert_eq!(capture_name("a:b\nc", None, "png"), "a_b c.png");
    }

    #[test]
    fn origins_keep_their_end() {
        let long = format!("https://accounts.example.com.{}.evil.test", "a".repeat(120));
        let shown = bounded_origin(&long);
        assert!(shown.starts_with('…') && shown.ends_with(".evil.test"));
        assert_eq!(shown.chars().count(), MAX_ORIGIN_CHARS);
        assert_eq!(bounded_origin("https://a.test"), "https://a.test");
    }

    #[test]
    fn markdown_links_escape_brackets_and_parentheses() {
        assert_eq!(markdown_link("Rust [book]", "https://a.test/x"), r"[Rust \[book\]](https://a.test/x)");
        assert_eq!(markdown_link(r"a\]b", "https://a.test/f(1)"), r"[a\\\]b](https://a.test/f%281%29)");
        assert_eq!(markdown_link("", "https://a.test/"), "[https://a.test/](https://a.test/)");
        assert_eq!(markdown_link("Line\nbreak", "https://a.test/"), "[Line break](https://a.test/)");
    }
}
