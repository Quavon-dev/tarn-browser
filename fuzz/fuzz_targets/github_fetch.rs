#![no_main]

use browser_core::github::{Fetch, FetchError, hop, parse_github};
use browser_core::theme_fetch::{Release, fetch_theme};
use libfuzzer_sys::fuzz_target;
use url::Url;

struct Fake<'a> {
    repo: &'a [u8],
    release: &'a [u8],
    file: &'a [u8],
}

impl Fetch for Fake<'_> {
    fn get(&self, url: &str, max: u64) -> Result<Vec<u8>, FetchError> {
        let body = if !url.starts_with("https://api.github.com/") {
            self.file
        } else if url.contains("/releases/") {
            self.release
        } else {
            self.repo
        };
        if body.len() as u64 > max {
            return Err(FetchError::Failed("too large".into()));
        }
        Ok(body.to_vec())
    }
}

fuzz_target!(|data: &[u8]| {
    let mut parts = data.splitn(4, |&b| b == 0);
    let input = std::str::from_utf8(parts.next().unwrap_or_default()).unwrap_or_default();
    let fake = Fake { repo: parts.next().unwrap_or_default(), release: parts.next().unwrap_or_default(), file: parts.next().unwrap_or_default() };
    if let Ok(repo) = parse_github(input) {
        assert!(!repo.owner.is_empty() && !repo.name.is_empty() && !repo.owner.contains('/') && !repo.name.contains('/'));
    }
    let base = Url::parse("https://api.github.com/repos/a/b").expect("a valid base");
    if let Ok(to) = hop(&base, input) {
        assert_eq!(to.scheme(), "https");
    }
    if let Ok(download) = fetch_theme(&fake, Release::Named(input)) {
        assert_eq!(download.sha256.len(), 64);
    }
    let _ = fetch_theme(&fake, Release::Newest(1));
});
