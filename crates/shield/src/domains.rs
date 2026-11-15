//! Domain sets checked on every navigation: sorted, de-duplicated 12-byte prefixes of SHA-256 over
//! each domain. A list author who wants a harmless-looking entry to collide with one of a million
//! popular hosts needs about 2^76 hashes, so a set cannot hide whom it blocks.

use std::net::IpAddr;

use sha2::{Digest, Sha256};
use url::Host;

pub const ENTRY_BYTES: usize = 12;
pub const MAX_ENTRIES: usize = 1 << 20;

/// Registrable domains no set may block, so one bad upstream list cannot take away sign-in, mail,
/// search, the filter-list update itself or the system's own services (S20).
pub const PROTECTED: &[&str] = &[
    "apple.com",
    "icloud.com",
    "github.com",
    "githubusercontent.com",
    "google.com",
    "gmail.com",
    "microsoft.com",
    "microsoftonline.com",
    "live.com",
    "outlook.com",
    "office.com",
    "yahoo.com",
    "proton.me",
    "protonmail.com",
    "fastmail.com",
    "duckduckgo.com",
    "bing.com",
    "wikipedia.org",
    "mozilla.org",
    "paypal.com",
    "amazon.com",
    "anthropic.com",
    "claude.ai",
];

pub fn hash(domain: &str) -> [u8; ENTRY_BYTES] {
    let digest = Sha256::digest(domain.as_bytes());
    let mut entry = [0; ENTRY_BYTES];
    entry.copy_from_slice(&digest[..ENTRY_BYTES]);
    entry
}

pub fn protected(host: &str) -> bool {
    PROTECTED.iter().any(|p| host == *p || host.strip_suffix(p).is_some_and(|rest| rest.ends_with('.')))
}

/// A list line's domain in the form the app looks up, or `None` for anything that must not be a
/// set entry: not a hostname or IPv4 address, a single label, a public suffix, or protected.
pub fn entry(raw: &str) -> Option<String> {
    let domain = match Host::parse(raw.trim().trim_end_matches('.')).ok()? {
        Host::Domain(d) => d,
        Host::Ipv4(ip) => return Some(ip.to_string()),
        Host::Ipv6(_) => return None,
    };
    let valid = domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_')));
    (valid && psl::suffix_str(&domain) != Some(&domain) && !protected(&domain)).then_some(domain)
}

/// The host and each parent domain above its public suffix; an IP address only as itself.
fn candidates(host: &str) -> Vec<&str> {
    if host.parse::<IpAddr>().is_ok() {
        return vec![host];
    }
    let Some(suffix) = psl::suffix_str(host) else { return Vec::new() };
    let mut out = Vec::new();
    let mut current = host;
    while current.len() > suffix.len() && current.parse::<IpAddr>().is_err() {
        out.push(current);
        let Some((_, parent)) = current.split_once('.') else { break };
        current = parent;
    }
    out
}

pub struct DomainSet(Vec<[u8; ENTRY_BYTES]>);

impl DomainSet {
    /// Accepts only whole, strictly ascending entries within the size cap.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if !bytes.len().is_multiple_of(ENTRY_BYTES) || bytes.len() / ENTRY_BYTES > MAX_ENTRIES {
            return None;
        }
        let entries = bytes.as_chunks::<ENTRY_BYTES>().0.to_vec();
        entries.windows(2).all(|w| w[0] < w[1]).then_some(DomainSet(entries))
    }

    /// The file format for `domains`, which must already be `entry`-normalised.
    pub fn encode<'a>(domains: impl IntoIterator<Item = &'a str>) -> Vec<u8> {
        let mut entries: Vec<_> = domains.into_iter().map(hash).collect();
        entries.sort_unstable();
        entries.dedup();
        entries.concat()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `host` as `url::Url::host_str` gives it: lowercase, punycode, IPv6 in brackets.
    pub fn contains(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        !protected(&host) && candidates(&host).into_iter().any(|c| self.0.binary_search(&hash(c)).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(domains: &[&str]) -> DomainSet {
        DomainSet::from_bytes(&DomainSet::encode(domains.iter().copied())).unwrap()
    }

    #[test]
    fn matches_host_and_subdomains_only() {
        let s = set(&["evil.example", "203.0.113.7"]);
        for host in ["evil.example", "a.b.evil.example", "EVIL.example", "evil.example.", "203.0.113.7"] {
            assert!(s.contains(host), "{host}");
        }
        for host in ["notevil.example", "example", "evil.example.org", "203.0.113.8", "7.203.0.113.7", "[::1]", ""] {
            assert!(!s.contains(host), "{host}");
        }
    }

    #[test]
    fn never_looks_up_public_suffixes_or_protected_hosts() {
        let wide = DomainSet::from_bytes(&DomainSet::encode(["com", "co.uk", "github.io", "google.com", "localhost"])).unwrap();
        for host in ["example.com", "example.co.uk", "user.github.io", "accounts.google.com", "google.com", "localhost"] {
            assert!(!wide.contains(host), "{host}");
        }
    }

    #[test]
    fn entries_are_normalised_and_filtered() {
        assert_eq!(entry("Evil.Example."), Some("evil.example".into()));
        assert_eq!(entry("bücher.example"), Some("xn--bcher-kva.example".into()));
        assert_eq!(entry("0x7f.1"), Some("127.0.0.1".into()));
        assert_eq!(entry("my_host.example"), Some("my_host.example".into()));
        for bad in ["com", "co.uk", "github.io", "localhost", "mail.google.com", "google.com", "a b.example", "[::1]", "", "*.example.com", "evil.example/path"]
        {
            assert_eq!(entry(bad), None, "{bad}");
        }
    }

    #[test]
    fn rejects_malformed_sets() {
        let a = hash("a.example");
        let b = hash("b.example");
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        assert!(DomainSet::from_bytes(&[lo, hi].concat()).is_some());
        assert!(DomainSet::from_bytes(&[hi, lo].concat()).is_none());
        assert!(DomainSet::from_bytes(&[lo, lo].concat()).is_none());
        assert!(DomainSet::from_bytes(&[0; ENTRY_BYTES + 1]).is_none());
        assert!(DomainSet::from_bytes(&[]).is_some_and(|s| s.is_empty()));
    }
}
