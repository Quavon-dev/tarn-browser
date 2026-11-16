use url::Url;

/// A WebExtension match pattern limited to web pages: `<all_urls>` or `scheme://host/path` with
/// scheme `*`, `http` or `https`, host `*`, `*.domain` or a name, and a path glob.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    source: String,
    scheme: Option<&'static str>,
    path: String,
}

impl Pattern {
    pub fn parse(source: &str) -> Result<Pattern, String> {
        let bad = || format!("the match pattern \"{source}\" is not supported");
        if source == "<all_urls>" {
            return Ok(Pattern { source: source.to_owned(), scheme: None, path: "/*".into() });
        }
        let (scheme, rest) = source.split_once("://").ok_or_else(bad)?;
        let scheme = match scheme {
            "*" => None,
            "http" => Some("http"),
            "https" => Some("https"),
            _ => return Err(bad()),
        };
        let slash = rest.find('/').ok_or_else(bad)?;
        let (host, path) = rest.split_at(slash);
        let plain = |h: &str| !h.is_empty() && h.bytes().all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b)) && !h.starts_with('.');
        let v6 = |a: &str| a.contains(':') && a.bytes().all(|b| b.is_ascii_hexdigit() || b == b':');
        let ipv6 = |h: &str| h.strip_prefix('[').and_then(|h| h.strip_suffix(']')).is_some_and(v6);
        if !(host == "*" || host.strip_prefix("*.").is_some_and(plain) || plain(host) || ipv6(host)) {
            return Err(bad());
        }
        if path.len() > 2048 || path.bytes().any(|b| b.is_ascii_control() || b == b' ') {
            return Err(bad());
        }
        Ok(Pattern { source: source.to_owned(), scheme, path: path.to_owned() })
    }

    pub fn as_str(&self) -> &str {
        &self.source
    }

    pub fn matches(&self, url: &Url) -> bool {
        let scheme_ok = match self.scheme {
            None => matches!(url.scheme(), "http" | "https"),
            Some(s) => url.scheme() == s,
        };
        let path = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_owned(),
        };
        scheme_ok && url.host_str().is_some_and(|h| self.covers_host(h)) && glob(&self.path, &path)
    }

    /// Whether the pattern reaches `url`'s scheme and host, whatever its path.
    pub fn covers(&self, url: &Url) -> bool {
        self.scheme.is_none_or(|s| s == url.scheme()) && url.host_str().is_some_and(|h| self.covers_host(h))
    }

    pub fn covers_host(&self, host: &str) -> bool {
        crate::extensions::pattern_covers(&self.source, host)
    }
}

fn glob(pattern: &str, text: &str) -> bool {
    let (p, t) = (pattern.as_bytes(), text.as_bytes());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|b| *b == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn web_patterns_match_scheme_host_and_path() {
        let p = Pattern::parse("*://*.example.com/news/*").unwrap();
        assert!(p.matches(&url("https://example.com/news/a")));
        assert!(p.matches(&url("http://www.example.com/news/?q=1")));
        assert!(!p.matches(&url("https://notexample.com/news/a")));
        assert!(!p.matches(&url("https://example.com/other")));
        assert!(!p.matches(&url("ftp://example.com/news/a")));
        let all = Pattern::parse("<all_urls>").unwrap();
        assert!(all.matches(&url("https://a.test/")) && !all.matches(&url("file:///etc/passwd")) && !all.matches(&url("about:blank")));
        let exact = Pattern::parse("https://a.test/*.js").unwrap();
        assert!(exact.matches(&url("https://A.test./x/y.js")) && !exact.matches(&url("http://a.test/y.js")));
    }

    #[test]
    fn other_schemes_and_odd_hosts_are_refused() {
        for bad in [
            "file:///*",
            "chrome://*/*",
            "*://a.test",
            "*://*a.test/*",
            "*://a b/*",
            "tarn-extension://x/*",
            "*://a.*/*",
            "https:///x",
            "https://a.test:8080/*",
            "*://*.a.test:443/*",
            "http://[::1]:8080/*",
            "http://[a.test]/*",
            "http://a[1]/*",
        ] {
            assert!(Pattern::parse(bad).is_err(), "{bad}");
        }
        let v6 = Pattern::parse("http://[::1]/*").unwrap();
        assert!(v6.matches(&url("http://[::1]/x")) && v6.matches(&url("http://[::1]:8080/x")));
    }
}
