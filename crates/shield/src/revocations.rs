//! Themes withdrawn as unsafe, delivered in the signed filter bundle (threat model S29). The list
//! names each theme file by its sha256; Tarn keeps every hash it has seen, so an older bundle
//! cannot bring a withdrawn theme back.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The bundle category of the list (a valid `bundle` category: lowercase letters only); older
/// builds verify and ignore it.
pub const CATEGORY: &str = "themes";
/// The list's file name in the bundle.
pub const FILE: &str = "theme-revocations.json";
const MAX_HASHES: usize = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revocations {
    pub sha256: Vec<String>,
}

fn is_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn parse(bytes: &[u8]) -> Option<Revocations> {
    let r: Revocations = serde_json::from_slice(bytes).ok()?;
    (r.sha256.len() <= MAX_HASHES && r.sha256.iter().all(|h| is_hash(h))).then_some(r)
}

/// `stored` (one hash per line) with `new` added; never drops a hash.
pub fn merge(stored: &str, new: &Revocations) -> String {
    let mut all: BTreeSet<&str> = stored.lines().filter(|h| is_hash(h)).collect();
    all.extend(new.sha256.iter().map(String::as_str));
    all.into_iter().collect::<Vec<_>>().join("\n")
}

/// Whether `sha256` is in the stored list; anything but a hash (such as the empty one of the
/// user's own themes) never is.
pub fn contains(stored: &str, sha256: &str) -> bool {
    is_hash(sha256) && stored.lines().any(|h| h == sha256)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_checked_merged_and_kept() {
        let a = "a".repeat(64);
        let b = "0".repeat(63) + "1";
        let r = parse(format!("{{\"sha256\":[\"{a}\"]}}").as_bytes()).unwrap();
        assert!(parse(b"{\"sha256\":[\"ABC\"]}").is_none());
        assert!(parse(format!("{{\"sha256\":[\"{}\"]}}", "A".repeat(64)).as_bytes()).is_none(), "lower case only");
        assert!(parse(b"{\"sha256\":[],\"extra\":1}").is_none());
        let stored = merge(&b, &r);
        assert!(contains(&stored, &a) && contains(&stored, &b));
        assert_eq!(merge(&stored, &Revocations::default()), stored, "an empty or older list removes nothing");
        assert!(!contains(&stored, ""));
        assert!(!contains("garbage\n", "garbage"), "only hashes are kept");
        assert_eq!(merge("garbage", &Revocations::default()), "");
    }
}
