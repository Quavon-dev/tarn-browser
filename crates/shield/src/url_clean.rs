use url::Url;

const EXACT: &[&str] = &[
    "fbclid",
    "gclid",
    "gclsrc",
    "dclid",
    "gbraid",
    "wbraid",
    "msclkid",
    "yclid",
    "twclid",
    "ttclid",
    "li_fat_id",
    "igshid",
    "mc_cid",
    "mc_eid",
    "_hsenc",
    "_hsmi",
    "__hssc",
    "__hstc",
    "__hsfp",
    "hsCtaTracking",
    "mkt_tok",
    "oly_anon_id",
    "oly_enc_id",
    "vero_conv",
    "vero_id",
    "wickedid",
    "_openstat",
    "srsltid",
    "pk_campaign",
    "pk_kwd",
    "pk_keyword",
    "pk_source",
    "pk_medium",
    "pk_content",
    "pk_cid",
    "pk_vid",
];
const PREFIXES: &[&str] = &["utm_", "mtm_"];

fn is_tracking(key: &str) -> bool {
    EXACT.contains(&key) || PREFIXES.iter().any(|p| key.starts_with(p))
}

/// Removes known tracking parameters, leaving the other pairs byte-for-byte as they were.
/// Returns `None` when nothing changed.
pub fn strip_tracking_params(url: &Url) -> Option<Url> {
    let query = url.query()?;
    let pairs: Vec<&str> = query.split('&').collect();
    let kept: Vec<&str> = pairs
        .iter()
        .copied()
        .filter(|pair| {
            let key = url::form_urlencoded::parse(pair.as_bytes()).next().map(|(k, _)| k);
            !key.is_some_and(|k| is_tracking(&k))
        })
        .collect();
    if kept.len() == pairs.len() {
        return None;
    }
    let mut out = url.clone();
    out.set_query((!kept.is_empty()).then(|| kept.join("&")).as_deref());
    Some(out)
}

/// How many query parameters cleaning took out of `before`.
pub fn removed_params(before: &Url, after: &Url) -> i64 {
    before.query_pairs().count() as i64 - after.query_pairs().count() as i64
}

/// `url` without its tracking parameters, and how many it lost.
pub fn clean_link(url: &str) -> (String, i64) {
    let Some((before, after)) = Url::parse(url).ok().and_then(|u| Some((u.clone(), strip_tracking_params(&u)?))) else {
        return (url.to_owned(), 0);
    };
    let removed = removed_params(&before, &after);
    (after.to_string(), removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(s: &str) -> Option<String> {
        strip_tracking_params(&Url::parse(s).unwrap()).map(String::from)
    }

    #[test]
    fn strips_known_params_and_keeps_others() {
        assert_eq!(clean("https://a.test/p?id=7&utm_source=x&fbclid=abc#frag").as_deref(), Some("https://a.test/p?id=7#frag"));
        assert_eq!(clean("https://a.test/?utm_medium=x&gclid=1").as_deref(), Some("https://a.test/"));
        assert_eq!(clean("https://a.test/?q=utm_source"), None);
        assert_eq!(clean("https://a.test/?print&q=a%20b&utm_source=x&sig=A%2B").as_deref(), Some("https://a.test/?print&q=a%20b&sig=A%2B"));
        assert_eq!(clean("https://a.test/"), None);
        assert_eq!(clean("https://a.test/?pk__in=1,2&pk_campaign=x").as_deref(), Some("https://a.test/?pk__in=1,2"));
    }

    #[test]
    fn clean_links_lose_only_tracking_parameters() {
        assert_eq!(clean_link("https://a.test/p?utm_source=x&id=3&fbclid=y#top"), ("https://a.test/p?id=3#top".to_owned(), 2));
        assert_eq!(clean_link("https://a.test/p?gclid=1"), ("https://a.test/p".to_owned(), 1));
        assert_eq!(clean_link("https://a.test/p?id=3"), ("https://a.test/p?id=3".to_owned(), 0));
        assert_eq!(clean_link("not a url"), ("not a url".to_owned(), 0));
    }

    #[test]
    fn removed_parameters_are_counted_one_by_one() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert_eq!(removed_params(&u("https://a.test/?utm_source=x&utm_medium=y&id=1"), &u("https://a.test/?id=1")), 2);
        assert_eq!(removed_params(&u("https://a.test/"), &u("https://a.test/")), 0);
    }
}
