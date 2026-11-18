use crate::tabs::TabKey;
use crate::text::bounded_url;

/// What the status bubble shows for a link, `javascript:` links included. The page's href is
/// parsed first, so hosts show as punycode and odd characters percent-encoded (S14).
pub fn text(raw: &str) -> Option<String> {
    const MAX_RAW: usize = 4096;
    let end = raw.char_indices().nth(MAX_RAW).map_or(raw.len(), |(at, _)| at);
    let url = url::Url::parse(&raw[..end]).ok()?;
    Some(bounded_url(&if url.scheme() == "javascript" { url.as_str().replace("%20", " ") } else { url.as_str().to_owned() }))
}

/// The link under the pointer and the tab whose page reported it.
#[derive(Default, Debug)]
pub struct Hovered(Option<(TabKey, String)>);

impl Hovered {
    /// `raw` from `key`'s page, `None` once the pointer left its link; a page the pointer left
    /// clears only its own link. Whether the shown link changed.
    pub fn set(&mut self, key: TabKey, raw: Option<&str>) -> bool {
        let link = raw.and_then(text).map(|text| (key, text));
        if link.is_none() && self.0.as_ref().is_some_and(|(k, _)| *k != key) || link == self.0 {
            return false;
        }
        self.0 = link;
        true
    }

    pub fn clear(&mut self) {
        self.0 = None;
    }

    pub fn get(&self) -> Option<(TabKey, &str)> {
        self.0.as_ref().map(|(key, text)| (*key, text.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_the_parsed_link() {
        assert_eq!(text("https://Bücher.example/a b").as_deref(), Some("https://xn--bcher-kva.example/a%20b"));
        assert_eq!(text("javascript:void(0)").as_deref(), Some("javascript:void(0)"));
        assert_eq!(text("javascript:alert('hello%20world')").as_deref(), Some("javascript:alert('hello world')"));
        assert_eq!(text("javascript:alert('\u{202E}x')").as_deref(), Some("javascript:alert('%E2%80%AEx')"));
        assert_eq!(text(""), None);
        assert_eq!(text("not a url"), None);
        assert_eq!(text(&format!("data:text/plain,{}", "a".repeat(100_000))).map(|s| s.chars().count()), Some(199));
    }

    #[test]
    fn a_page_the_pointer_left_clears_only_its_own_link() {
        let (a, b) = (TabKey { private: false, id: 1 }, TabKey { private: false, id: 2 });
        let mut hovered = Hovered::default();
        assert!(hovered.set(a, Some("https://a.test/")));
        assert!(!hovered.set(a, Some("https://a.test/")), "the same link again changes nothing");
        assert!(!hovered.set(b, None));
        assert_eq!(hovered.get(), Some((a, "https://a.test/")));
        assert!(hovered.set(a, Some("not a link")));
        assert_eq!(hovered.get(), None);
    }
}
