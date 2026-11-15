use store::{Id, SpaceRule};
use url::Url;

/// The space a rule sends a new tab of `url` to. An exact host beats `*.host`, which covers the
/// host and its subdomains, and a longer `*.` rule beats a shorter one. Nothing leaves a private space.
pub fn route(rules: &[SpaceRule], url: &Url, from_private: bool) -> Option<Id> {
    if from_private || !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.trim_end_matches('.');
    rules
        .iter()
        .filter_map(|r| match r.host_pattern.strip_prefix("*.") {
            Some(base) => (host == base || host.strip_suffix(base).is_some_and(|sub| sub.ends_with('.'))).then_some((base.len(), r.space_id)),
            None => (host == r.host_pattern).then_some((usize::MAX, r.space_id)),
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, space_id)| space_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(list: &[(&str, Id)]) -> Vec<SpaceRule> {
        (0..).zip(list).map(|(id, (p, s))| SpaceRule { id, host_pattern: (*p).to_owned(), space_id: *s }).collect()
    }

    fn at(rules: &[SpaceRule], url: &str) -> Option<Id> {
        route(rules, &Url::parse(url).unwrap(), false)
    }

    #[test]
    fn exact_host_matches_only_itself() {
        let r = rules(&[("github.com", 7)]);
        assert_eq!(at(&r, "https://GitHub.com./x"), Some(7));
        assert_eq!(at(&r, "http://github.com:8080/"), Some(7));
        assert_eq!(at(&r, "https://gist.github.com/"), None);
    }

    #[test]
    fn wildcard_matches_host_and_subdomains_only() {
        let r = rules(&[("*.example.com", 3)]);
        assert_eq!(at(&r, "https://example.com/"), Some(3));
        assert_eq!(at(&r, "https://a.b.example.com/"), Some(3));
        assert_eq!(at(&r, "https://notexample.com/"), None);
        assert_eq!(at(&r, "https://example.com.evil.test/"), None);
    }

    #[test]
    fn exact_beats_wildcard_and_longer_wildcard_beats_shorter() {
        let r = rules(&[("*.example.com", 1), ("*.mail.example.com", 2), ("login.mail.example.com", 3)]);
        assert_eq!(at(&r, "https://inbox.mail.example.com/"), Some(2));
        assert_eq!(at(&r, "https://login.mail.example.com/"), Some(3));
        assert_eq!(at(&r, "https://www.example.com/"), Some(1));
    }

    #[test]
    fn no_match_without_rule_host_or_web_scheme() {
        let r = rules(&[("example.com", 1)]);
        assert_eq!(at(&r, "https://other.test/"), None);
        assert_eq!(at(&r, "file:///etc/hosts"), None);
        assert_eq!(at(&r, "ftp://example.com/"), None);
        assert_eq!(at(&[], "https://example.com/"), None);
    }

    #[test]
    fn private_source_is_never_routed() {
        let r = rules(&[("example.com", 1)]);
        assert_eq!(route(&r, &Url::parse("https://example.com/").unwrap(), true), None);
    }
}
