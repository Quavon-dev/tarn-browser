pub const MAX_TEXT_BYTES: usize = 48 * 1024;
const MAX_TITLE_BYTES: usize = 512;
const MAX_URL_BYTES: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageContext {
    pub url: String,
    pub title: String,
    pub text: String,
}

impl PageContext {
    pub fn new(url: &str, title: &str, text: &str) -> Self {
        Self { url: clean(&strip_url(url), MAX_URL_BYTES), title: clean(title, MAX_TITLE_BYTES).replace('\n', " "), text: clean(text, MAX_TEXT_BYTES) }
    }
}

fn strip_url(raw: &str) -> String {
    let Ok(mut url) = url::Url::parse(raw) else {
        return String::new();
    };
    if !matches!(url.scheme(), "http" | "https") {
        return format!("{}:", url.scheme());
    }
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.into()
}

pub(crate) fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{115F}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{3164}' | '\u{FEFF}' | '\u{FFA0}' | '\u{E0000}'..='\u{E007F}')
}

fn clean(s: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_bytes));
    for c in s.chars().map(|c| if matches!(c, '\u{2028}' | '\u{2029}') { '\n' } else { c }) {
        if (c.is_control() && c != '\n' && c != '\t') || is_bidi(c) {
            continue;
        }
        if out.len() + c.len_utf8() > max_bytes {
            break;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_keep_their_path_and_none_is_unbounded() {
        assert_eq!(PageContext::new("data:text/html,<p>secret</p>", "", "").url, "data:");
        assert_eq!(PageContext::new("file:///Users/me/notes.txt", "", "").url, "file:");
        let long = format!("https://a.test/{}", "x".repeat(10_000));
        assert!(PageContext::new(&long, "", "").url.len() <= MAX_URL_BYTES);
    }

    #[test]
    fn caps_text_at_char_boundary() {
        let text = "é".repeat(MAX_TEXT_BYTES);
        let ctx = PageContext::new("https://a.test/", "t", &text);
        assert!(ctx.text.len() <= MAX_TEXT_BYTES);
        assert_eq!(ctx.text.len(), MAX_TEXT_BYTES);
        assert!(ctx.text.chars().all(|c| c == 'é'));
    }

    #[test]
    fn strips_query_fragment_and_credentials() {
        let ctx = PageContext::new("https://user:pw@a.test/p?token=s3cret#frag", "", "");
        assert_eq!(ctx.url, "https://a.test/p");
        assert_eq!(PageContext::new("not a url", "", "").url, "");
    }

    #[test]
    fn drops_control_and_bidi() {
        let ctx = PageContext::new("https://a.test/", "a\u{202E}b\nc", "x\u{0007}y\u{2066}z\n\tok\u{200F}");
        assert_eq!(ctx.title, "ab c");
        assert_eq!(ctx.text, "xyz\n\tok");
        let hidden = PageContext::new("https://a.test/", "t", "a\u{3164}b\u{E0041}c\u{2028}d");
        assert_eq!(hidden.text, "abc\nd");
    }
}
