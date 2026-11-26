//! Certificate errors and the exceptions a user confirmed for them (threat model S7): one host, port
//! and leaf certificate each. The engine's own trust evaluation runs first; an exception only
//! answers its failure, and never one for a revoked certificate.

use std::collections::HashSet;
use std::sync::{Arc, PoisonError, RwLock};

use sha2::{Digest, Sha256};
use url::Url;

/// Certificates kept from one failed load; more are dropped, as is a larger certificate.
pub const MAX_CHAIN: usize = 10;
pub const MAX_CERT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Issued by an authority the system does not trust, or self-signed.
    Untrusted,
    /// Expired or not valid yet.
    Dates,
    /// Issued for another name than the site's.
    NameMismatch,
    Revoked,
    /// An error Chromium never lets through: invalid, pinned or known to intercept connections.
    Refused,
    Invalid,
}

impl Problem {
    pub fn allows_exception(self) -> bool {
        !matches!(self, Problem::Revoked | Problem::Refused)
    }

    /// From the `OSStatus` of Security's trust evaluation.
    pub fn from_sec_trust(status: i64) -> Problem {
        match status {
            -67818 | -67819 => Problem::Dates,
            -67820 => Problem::Revoked,
            -67602 => Problem::NameMismatch,
            -67843 | -67653 | -67808 | -67809 | -67810 => Problem::Untrusted,
            _ => Problem::Invalid,
        }
    }

    /// From a Chromium net error, `None` when it is not a certificate error. Only the errors Chrome
    /// itself lets a user override can get an exception; invalid, pinned or intercepted ones cannot.
    pub fn from_chromium(code: i64) -> Option<Problem> {
        Some(match code {
            -200 => Problem::NameMismatch,
            -201 => Problem::Dates,
            -202 => Problem::Untrusted,
            -206 => Problem::Revoked,
            -219..=-203 | -150 => Problem::Refused,
            _ => return None,
        })
    }

    /// From GLib's `GTlsCertificateFlags`.
    pub fn from_gtls(flags: u32) -> Problem {
        if flags & 0x10 != 0 {
            Problem::Revoked
        } else if flags & 0x02 != 0 {
            Problem::NameMismatch
        } else if flags & 0x01 != 0 {
            Problem::Untrusted
        } else if flags & 0x0c != 0 {
            Problem::Dates
        } else {
            Problem::Invalid
        }
    }

    /// From Android's `SslError.getPrimaryError()`.
    pub fn from_android(primary: i64) -> Problem {
        match primary {
            0 | 1 | 4 => Problem::Dates,
            2 => Problem::NameMismatch,
            3 => Problem::Untrusted,
            _ => Problem::Invalid,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Problem::Untrusted => "untrusted",
            Problem::Dates => "dates",
            Problem::NameMismatch => "name",
            Problem::Revoked => "revoked",
            Problem::Refused => "refused",
            Problem::Invalid => "invalid",
        }
    }

    pub fn parse(s: &str) -> Problem {
        [Problem::Untrusted, Problem::Dates, Problem::NameMismatch, Problem::Revoked, Problem::Refused]
            .into_iter()
            .find(|p| p.as_str() == s)
            .unwrap_or(Problem::Invalid)
    }
}

/// Keeps at most `MAX_CHAIN` certificates of at most `MAX_CERT_BYTES` each, leaf first. A leaf
/// that is empty or too large leaves no chain, so an issuer can never stand in for it.
pub fn bounded_chain(chain: impl IntoIterator<Item = Vec<u8>>) -> Vec<Vec<u8>> {
    let fits = |c: &Vec<u8>| !c.is_empty() && c.len() <= MAX_CERT_BYTES;
    let mut chain = chain.into_iter();
    let Some(leaf) = chain.next().filter(fits) else { return Vec::new() };
    std::iter::once(leaf).chain(chain.filter(fits)).take(MAX_CHAIN).collect()
}

/// Lowercase hex SHA-256 of a certificate's DER bytes.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02x}")).collect()
}

/// The host and port an exception is kept for; the scheme's default port when the URL has none.
pub fn host_port(url: &Url) -> Option<(String, u16)> {
    Some((url.host_str()?.to_ascii_lowercase(), url.port_or_known_default()?))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Exception {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

impl Exception {
    /// The prop and event form every client passes to its web views.
    pub fn list_to_json(list: &[Exception]) -> serde_json::Value {
        list.iter().map(|e| serde_json::json!({ "host": e.host, "port": e.port, "fingerprint": e.fingerprint })).collect()
    }

    /// Reads `list_to_json`'s form; an entry with a bad port or fingerprint is dropped.
    pub fn list_from_json(value: &serde_json::Value) -> Vec<Exception> {
        let field = |e: &serde_json::Value, name: &str| e.get(name).and_then(serde_json::Value::as_str).map(str::to_ascii_lowercase);
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| {
                let port = u16::try_from(e.get("port")?.as_u64()?).ok()?;
                let fingerprint = field(e, "fingerprint").filter(|f| f.len() == 64 && f.bytes().all(|b| b.is_ascii_hexdigit()))?;
                Some(Exception { host: field(e, "host")?, port, fingerprint })
            })
            .collect()
    }
}

/// One profile's exceptions, shared by its views; a change applies to the next load.
#[derive(Clone, Default)]
pub struct Exceptions(Arc<RwLock<Lists>>);

#[derive(Default)]
struct Lists {
    now: HashSet<Exception>,
    /// Every host and port that had an exception since launch: an engine may keep a connection
    /// that accepted the certificate after its exception is gone.
    seen: HashSet<(String, u16)>,
}

impl Exceptions {
    pub fn set(&self, list: impl IntoIterator<Item = Exception>) {
        let mut lists = self.0.write().unwrap_or_else(PoisonError::into_inner);
        lists.now = list.into_iter().collect();
        let seen: Vec<_> = lists.now.iter().map(|e| (e.host.clone(), e.port)).collect();
        lists.seen.extend(seen);
    }

    /// Whether any certificate is trusted for `host:port`, before the costlier fingerprint check.
    pub fn has(&self, host: &str, port: u16) -> bool {
        let host = host.to_ascii_lowercase();
        self.0.read().unwrap_or_else(PoisonError::into_inner).now.iter().any(|e| e.host == host && e.port == port)
    }

    /// Whether an https `url` may load under an exception, now or through a connection kept from
    /// before its removal, so it is shown as not secure until Tarn quits.
    pub fn covers(&self, url: &Url) -> bool {
        host_port(url).filter(|_| url.scheme() == "https").is_some_and(|key| self.0.read().unwrap_or_else(PoisonError::into_inner).seen.contains(&key))
    }

    /// Whether the user confirmed exactly this leaf certificate for `host:port`.
    pub fn allows(&self, host: &str, port: u16, leaf: &[u8], problem: Problem) -> bool {
        if !problem.allows_exception() || leaf.is_empty() || leaf.len() > MAX_CERT_BYTES {
            return false;
        }
        let wanted = Exception { host: host.to_ascii_lowercase(), port, fingerprint: fingerprint(leaf) };
        self.0.read().unwrap_or_else(PoisonError::into_inner).now.contains(&wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exception_holds_for_its_host_port_and_certificate_only() {
        let leaf = b"leaf".to_vec();
        let ex = Exceptions::default();
        ex.set([Exception { host: "192.168.1.2".into(), port: 8006, fingerprint: fingerprint(&leaf) }]);
        assert!(ex.allows("192.168.1.2", 8006, &leaf, Problem::Untrusted));
        assert!(ex.allows("192.168.1.2", 8006, &leaf, Problem::NameMismatch));
        assert!(!ex.allows("192.168.1.2", 8006, &leaf, Problem::Revoked));
        assert!(!ex.allows("192.168.1.2", 8006, &leaf, Problem::Refused));
        assert!(ex.has("192.168.1.2", 8006) && !ex.has("192.168.1.2", 443));
        assert!(!ex.allows("192.168.1.2", 443, &leaf, Problem::Untrusted));
        assert!(!ex.allows("192.168.1.3", 8006, &leaf, Problem::Untrusted));
        assert!(!ex.allows("192.168.1.2", 8006, b"other", Problem::Untrusted));
        assert!(!ex.allows("192.168.1.2", 8006, &[], Problem::Untrusted));
        assert!(ex.covers(&Url::parse("https://192.168.1.2:8006/x").unwrap()));
        assert!(!ex.covers(&Url::parse("http://192.168.1.2:8006/").unwrap()));
        assert!(!ex.covers(&Url::parse("https://192.168.1.2/").unwrap()));
        ex.set([]);
        assert!(!ex.allows("192.168.1.2", 8006, &leaf, Problem::Untrusted));
        assert!(!ex.has("192.168.1.2", 8006));
        assert!(ex.covers(&Url::parse("https://192.168.1.2:8006/").unwrap()), "a removed exception still shows as not secure until quit");
    }

    #[test]
    fn exceptions_round_trip_through_json_and_bad_entries_drop() {
        let good = Exception { host: "pve.local".into(), port: 8006, fingerprint: "ab".repeat(32) };
        assert_eq!(Exception::list_from_json(&Exception::list_to_json(std::slice::from_ref(&good))), [good]);
        let bad = serde_json::json!([
            { "host": "a", "port": 70000, "fingerprint": "ab".repeat(32) },
            { "host": "a", "port": 1, "fingerprint": "zz" },
            { "port": 1, "fingerprint": "ab".repeat(32) },
        ]);
        assert!(Exception::list_from_json(&bad).is_empty());
    }

    #[test]
    fn engine_codes_map_to_problems() {
        assert_eq!(Problem::from_chromium(-202), Some(Problem::Untrusted));
        assert_eq!(Problem::from_chromium(-206), Some(Problem::Revoked));
        assert_eq!(Problem::from_chromium(-105), None);
        assert_eq!(Problem::from_chromium(-217), Some(Problem::Refused));
        assert_eq!(Problem::from_chromium(-207), Some(Problem::Refused));
        assert_eq!(Problem::from_chromium(-150), Some(Problem::Refused));
        assert_eq!(Problem::from_sec_trust(-67818), Problem::Dates);
        assert_eq!(Problem::from_gtls(0x10 | 0x01), Problem::Revoked);
        assert_eq!(Problem::from_gtls(0x01), Problem::Untrusted);
        assert_eq!(Problem::from_android(2), Problem::NameMismatch);
        for p in [Problem::Untrusted, Problem::Dates, Problem::NameMismatch, Problem::Revoked, Problem::Refused, Problem::Invalid] {
            assert_eq!(Problem::parse(p.as_str()), p);
        }
    }

    #[test]
    fn a_chain_is_bounded_and_ports_default() {
        let chain = bounded_chain((0..20).map(|_| vec![1u8]).chain([vec![0; MAX_CERT_BYTES + 1]]));
        assert_eq!(chain.len(), MAX_CHAIN);
        assert!(bounded_chain([vec![0; MAX_CERT_BYTES + 1], vec![2]]).is_empty(), "an oversized leaf leaves no chain");
        assert!(bounded_chain([Vec::new(), vec![2]]).is_empty(), "an empty leaf leaves no chain");
        assert_eq!(bounded_chain([vec![1], vec![0; MAX_CERT_BYTES + 1], vec![3]]), [vec![1], vec![3]]);
        assert_eq!(host_port(&Url::parse("https://Pve.Local/").unwrap()), Some(("pve.local".into(), 443)));
        assert_eq!(host_port(&Url::parse("https://192.168.1.2:8006/#x").unwrap()), Some(("192.168.1.2".into(), 8006)));
    }
}
