use crate::command::{MAX_HOST, host_tail};
use crate::tabs::TabKey;
use crate::text::bounded_url;

pub fn text(raw: &str) -> Option<String> {
    let max = engine::view_policy::MAX_HOVERED_LINK;
    let raw = &raw[..raw.char_indices().nth(max).map_or(raw.len(), |(at, _)| at)];
    let cut = raw.encode_utf16().count() >= max;
    if cut && raw.find("://").is_some_and(|at| !raw[at + 3..].contains(['/', '?', '#', '\\'])) {
        return None;
    }
    let url = url::Url::parse(raw).ok()?;
    match url.scheme() {
        "javascript" => return Some(bounded_url(&url.as_str().replace("%20", " "))),
        "blob" => return Some(text(&url.as_str()["blob:".len()..]).map_or_else(|| bounded_url(url.as_str()), |inner| format!("blob:{inner}"))),
        _ => {}
    }
    let host = match url.host() {
        Some(url::Host::Domain(domain)) => host_tail(domain, MAX_HOST),
        Some(ip) => ip.to_string(),
        None => return Some(bounded_url(url.as_str())),
    };
    Some(format!("{}://{host}{}", url.scheme(), bounded_url(&url[url::Position::AfterHost..])))
}

pub fn split(text: &str) -> (&str, &str) {
    let scheme = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    let Some(start) = text.find("://").filter(|at| scheme(text[..*at].trim_start_matches("blob:"))).map(|at| at + 3) else { return ("", text) };
    let end = text[start..].find(['/', '?', '#']).map_or(text.len(), |at| start + at);
    text.split_at(end)
}

#[derive(Default, Debug)]
pub struct Hovered(Option<(TabKey, String)>);

impl Hovered {
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
        assert_eq!(text("http://localhost:8080/x?y").as_deref(), Some("http://localhost:8080/x?y"));
    }

    #[test]
    fn the_real_domain_always_shows() {
        assert_eq!(text("https://bank.example.com@evil.test/login").as_deref(), Some("https://evil.test/login"));
        assert_eq!(text("blob:https://bank.example.com@evil.test/x").as_deref(), Some("blob:https://evil.test/x"));
        assert_eq!(text("http://[1111:2222:3333:4444:5555:6666:7777:8888]/").as_deref(), Some("http://[1111:2222:3333:4444:5555:6666:7777:8888]/"));
        assert!(text(&format!("blob:https://bank.example.com.{}.evil.test/id", "a".repeat(200))).unwrap().contains(".evil.test/id"));
        assert_eq!(split("https://evil.test:8080/a?b"), ("https://evil.test:8080", "/a?b"));
        assert_eq!(split("javascript:void(0)"), ("", "javascript:void(0)"));
        assert_eq!(split("data:text/html,https://x.test/"), ("", "data:text/html,https://x.test/"));
        assert_eq!(split("javascript:location='https://x.test'"), ("", "javascript:location='https://x.test'"));
        assert_eq!(split("blob:https://a.test/id"), ("blob:https://a.test", "/id"));
        assert_eq!(text("blob:https://a.test/id?x#y").as_deref(), Some("blob:https://a.test/id?x#y"));
        assert_eq!(text("blob:abc").as_deref(), Some("blob:abc"));
    }

    #[test]
    fn a_link_cut_inside_its_authority_shows_nothing() {
        let cut = |link: String| link.chars().take(engine::view_policy::MAX_HOVERED_LINK).collect::<String>();
        assert_eq!(text(&cut(format!("https://{}.bank.test@evil.test/", "a".repeat(4078)))), None);
        assert_eq!(text(&format!("https://{}.bank.test@evil.test/", "a".repeat(4078))), None);
        assert_eq!(text(&cut(format!("blob:https://{}.bank.test@evil.test/", "a".repeat(4078)))), None);
        assert!(text(&cut(format!("https://evil.test/{}", "p".repeat(5000)))).is_some_and(|t| t.starts_with("https://evil.test/")));
        let long = format!("https://bank.example.com.{}.evil.test/{}", "a".repeat(200), "p".repeat(300));
        let shown = text(&long).unwrap();
        assert!(shown.starts_with("https://…") && shown.contains(".evil.test/"), "{shown}");
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
