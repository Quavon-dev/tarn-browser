//! Where extension packages and themes come from: GitHub releases and the signed
//! catalog. Every request goes through `follow`, to a fixed host list, after a click.

use serde_json::Value;
use url::Url;

pub const HOSTS: [&str; 5] = ["api.github.com", "github.com", "codeload.github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];
pub const USER_AGENT: &str = "Tarn-Extensions/1";
pub const MAX_REDIRECTS: usize = 3;
const MAX_API_BYTES: u64 = 1 << 20;

pub trait Fetch {
    fn get(&self, url: &str, max: u64) -> Result<Vec<u8>, FetchError>;
}

#[derive(Debug, PartialEq)]
pub enum FetchError {
    Status(u16),
    Refused(String),
    Failed(String),
}

impl FetchError {
    pub fn describe(&self) -> String {
        match self {
            FetchError::Status(403 | 429) => "GitHub's rate limit was reached. Try again later.".to_owned(),
            FetchError::Status(404) => "Not found on GitHub.".to_owned(),
            FetchError::Status(code) => format!("The server answered {code}."),
            FetchError::Refused(why) | FetchError::Failed(why) => why.clone(),
        }
    }
}

pub fn allowed(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|h| HOSTS.contains(&h))
}

pub fn hop(from: &Url, location: &str) -> Result<Url, FetchError> {
    let to = from.join(location).map_err(|_| FetchError::Refused("a bad redirect".into()))?;
    if !allowed(&to) {
        return Err(FetchError::Refused(format!("a redirect to {} is not allowed", to.host_str().unwrap_or("?"))));
    }
    Ok(to)
}

#[derive(Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

pub trait Hop {
    fn request(&self, url: &Url, max: u64) -> Result<Reply, FetchError>;
}

impl<H: Hop> Fetch for H {
    fn get(&self, url: &str, max: u64) -> Result<Vec<u8>, FetchError> {
        follow(self, url, max)
    }
}

pub const CONNECT_TIMEOUT_SECS: u64 = 10;
pub const TIMEOUT_SECS: u64 = 120;

pub fn follow(hop_once: &dyn Hop, url: &str, max: u64) -> Result<Vec<u8>, FetchError> {
    let mut url = Url::parse(url).map_err(|_| FetchError::Refused("not a valid address".into()))?;
    for _ in 0..=MAX_REDIRECTS {
        if !allowed(&url) {
            return Err(FetchError::Refused(format!("{} is not an allowed host", url.host_str().unwrap_or("?"))));
        }
        let reply = hop_once.request(&url, max)?;
        if (300..400).contains(&reply.status) {
            let location = reply.location.ok_or(FetchError::Status(reply.status))?;
            url = hop(&url, &location)?;
            continue;
        }
        if reply.status != 200 {
            return Err(FetchError::Status(reply.status));
        }
        if reply.body.len() as u64 > max {
            return Err(FetchError::Failed("The answer is too large.".into()));
        }
        return Ok(reply.body);
    }
    Err(FetchError::Refused("too many redirects".into()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Repo {
    pub owner: String,
    pub name: String,
    pub tag: Option<String>,
}

fn part(s: &str, max: usize, extra: &[u8]) -> bool {
    (1..=max).contains(&s.len()) && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
}

fn owner_ok(s: &str) -> bool {
    part(s, 39, b"-")
}

fn repo_ok(s: &str) -> bool {
    part(s, 100, b"._-")
}

fn tag_ok(s: &str) -> bool {
    part(s, 100, b"._+-")
}

/// `owner/repo[@tag]`, or an `https://github.com/owner/repo[/releases/tag/<tag>]` address.
pub fn parse_github(input: &str) -> Result<Repo, String> {
    let input = input.trim();
    let refuse = || Err("Enter owner/repository, optionally with @tag, or a github.com address.".to_owned());
    let (path, tag) = if input.contains("://") {
        let Ok(url) = Url::parse(input) else { return refuse() };
        if url.scheme() != "https" || url.host_str() != Some("github.com") || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
            return refuse();
        }
        let parts: Vec<&str> = url.path().trim_matches('/').split('/').collect();
        match parts.as_slice() {
            [owner, repo] => (format!("{owner}/{repo}"), None),
            [owner, repo, "releases", "tag", tag] => (format!("{owner}/{repo}"), Some(tag.to_string())),
            _ => return refuse(),
        }
    } else {
        match input.split_once('@') {
            Some((path, tag)) => (path.to_owned(), Some(tag.to_owned())),
            None => (input.to_owned(), None),
        }
    };
    let Some((owner, name)) = path.split_once('/') else { return refuse() };
    let name = name.strip_suffix(".git").unwrap_or(name);
    if !owner_ok(owner) || !repo_ok(name) || tag.as_deref().is_some_and(|t| !tag_ok(t)) {
        return refuse();
    }
    Ok(Repo { owner: owner.to_owned(), name: name.to_owned(), tag })
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub repo_id: u64,
    pub owner_id: u64,
    pub full_name: String,
    pub tag: String,
    /// `.zip` and `.crx` assets of the release, Chromium builds first; empty means the tag's
    /// source archive.
    pub assets: Vec<Asset>,
}

impl Resolved {
    pub fn source(&self) -> String {
        format!("github:{}:{}", self.repo_id, self.full_name)
    }

    pub fn source_archive(&self) -> String {
        format!("https://codeload.github.com/{}/zip/refs/tags/{}", self.full_name, enc(&self.tag))
    }
}

fn json(fetch: &dyn Fetch, url: &str) -> Result<Value, String> {
    let body = fetch.get(url, MAX_API_BYTES).map_err(|e| e.describe())?;
    serde_json::from_slice(&body).map_err(|_| "GitHub sent an answer Tarn cannot read.".to_owned())
}

/// The repository by its numeric id from here on, with the name GitHub returned.
fn repository(v: &Value) -> Result<(u64, u64, String), String> {
    let id = v["id"].as_u64();
    let owner_id = v["owner"]["id"].as_u64();
    let full_name = v["full_name"].as_str().filter(|n| n.split_once('/').is_some_and(|(o, r)| owner_ok(o) && repo_ok(r)));
    match (id, owner_id, full_name) {
        (Some(id), Some(owner), Some(name)) => Ok((id, owner, name.to_owned())),
        _ => Err("GitHub sent an answer Tarn cannot read.".to_owned()),
    }
}

/// Which release files an install can use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wanted {
    /// `.zip` and `.crx` packages.
    Extension,
    /// The one file named `theme.css`.
    Theme,
}

pub const THEME_ASSET: &str = "theme.css";

impl Wanted {
    fn accepts(self, name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        match self {
            Wanted::Extension => lower.ends_with(".zip") || lower.ends_with(".crx"),
            Wanted::Theme => lower == THEME_ASSET,
        }
    }
}

fn release(fetch: &dyn Fetch, repo_id: u64, full_name: &str, tag: Option<&str>, wanted: Wanted) -> Result<(String, Vec<Asset>), String> {
    let url = match tag {
        Some(tag) => format!("https://api.github.com/repositories/{repo_id}/releases/tags/{}", enc(tag)),
        None => format!("https://api.github.com/repositories/{repo_id}/releases/latest"),
    };
    let release = match fetch.get(&url, MAX_API_BYTES) {
        Ok(body) => serde_json::from_slice::<Value>(&body).map_err(|_| "GitHub sent an answer Tarn cannot read.".to_owned())?,
        Err(FetchError::Status(404)) => match tag {
            Some(tag) => return Ok((tag.to_owned(), Vec::new())),
            None => return Err("This repository has no release. Name a tag with @tag.".to_owned()),
        },
        Err(e) => return Err(e.describe()),
    };
    let tag = release["tag_name"].as_str().filter(|t| tag_ok(t)).ok_or("The release has no usable tag.")?.to_owned();
    let prefix = format!("https://github.com/{full_name}/releases/download/");
    let mut assets: Vec<Asset> = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some(Asset { name: a["name"].as_str()?.to_owned(), url: a["browser_download_url"].as_str()?.to_owned() }))
        .filter(|a| wanted.accepts(&a.name) && a.url.starts_with(&prefix) && Url::parse(&a.url).is_ok_and(|u| allowed(&u)))
        .collect();
    let rank = |a: &Asset| {
        let lower = a.name.to_ascii_lowercase();
        u8::from(!(lower.contains("chromium") || lower.contains("chrome")))
    };
    assets.sort_by_key(rank);
    Ok((tag, assets))
}

pub fn resolve(fetch: &dyn Fetch, repo: &Repo) -> Result<Resolved, String> {
    resolve_for(fetch, repo, Wanted::Extension)
}

pub fn resolve_for(fetch: &dyn Fetch, repo: &Repo, wanted: Wanted) -> Result<Resolved, String> {
    let v = json(fetch, &format!("https://api.github.com/repos/{}/{}", enc(&repo.owner), enc(&repo.name)))?;
    let (repo_id, owner_id, full_name) = repository(&v)?;
    let (tag, assets) = release(fetch, repo_id, &full_name, repo.tag.as_deref(), wanted)?;
    Ok(Resolved { repo_id, owner_id, full_name, tag, assets })
}

/// The newest release of an installed repository, asked for by its id: a deleted repository
/// is an error, a renamed or transferred one comes back with its new name and owner id.
pub fn latest(fetch: &dyn Fetch, repo_id: u64) -> Result<Resolved, String> {
    latest_for(fetch, repo_id, Wanted::Extension)
}

pub fn latest_for(fetch: &dyn Fetch, repo_id: u64, wanted: Wanted) -> Result<Resolved, String> {
    let v = json(fetch, &format!("https://api.github.com/repositories/{repo_id}"))?;
    let (id, owner_id, full_name) = repository(&v)?;
    if id != repo_id {
        return Err("GitHub returned another repository.".to_owned());
    }
    let (tag, assets) = release(fetch, repo_id, &full_name, None, wanted)?;
    Ok(Resolved { repo_id, owner_id, full_name, tag, assets })
}

/// What an update changes about where a package comes from: a renamed repository, a new owner,
/// or new contents under the same version. `shown` bounds names for display.
pub fn provenance(previous: (&str, u64, &str, &str), next: (&str, u64, &str, &str), shown: fn(&str) -> String) -> Vec<String> {
    let ((old_source, old_owner, old_version, old_hash), (new_source, new_owner, new_version, new_hash)) = (previous, next);
    let mut changes = Vec::new();
    if let (Some((_, old)), Some((_, new))) = (github_source(old_source), github_source(new_source))
        && old != new
    {
        changes.push(format!("The repository is now named {} (was {}).", shown(new), shown(old)));
    }
    if old_owner != 0 && new_owner != old_owner {
        changes.push("The repository now belongs to another GitHub account.".to_owned());
    }
    if new_version == old_version && new_hash != old_hash {
        changes.push("The package changed although its version did not.".to_owned());
    }
    changes
}

/// `(index, signature)` of the official extension catalog.
pub fn catalog(fetch: &dyn Fetch) -> Result<(Vec<u8>, Vec<u8>), String> {
    catalog_files(fetch, &shield::catalog::EXTENSIONS)
}

/// `(index, signature)` of an official catalog.
pub fn catalog_files(fetch: &dyn Fetch, catalog: &shield::catalog::Catalog) -> Result<(Vec<u8>, Vec<u8>), String> {
    let index = fetch.get(catalog.index_url, shield::catalog::MAX_INDEX_BYTES as u64).map_err(|e| e.describe())?;
    let signature = fetch.get(catalog.signature_url, 1024).map_err(|e| e.describe())?;
    Ok((index, signature))
}

pub use crate::theme_library::github_source;

#[cfg(test)]
pub mod testing {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use serde_json::Value;

    use crate::github::{Fetch, FetchError};

    #[derive(Default)]
    pub struct Fake {
        pub answers: HashMap<String, Result<Vec<u8>, FetchError>>,
        pub asked: RefCell<Vec<String>>,
    }

    impl Fetch for Fake {
        fn get(&self, url: &str, max: u64) -> Result<Vec<u8>, FetchError> {
            self.asked.borrow_mut().push(url.to_owned());
            match self.answers.get(url) {
                Some(Ok(body)) if body.len() as u64 > max => Err(FetchError::Failed("too large".into())),
                Some(Ok(body)) => Ok(body.clone()),
                Some(Err(FetchError::Status(s))) => Err(FetchError::Status(*s)),
                _ => Err(FetchError::Status(404)),
            }
        }
    }

    impl Fake {
        pub fn with(mut self, url: &str, body: Value) -> Self {
            self.answers.insert(url.to_owned(), Ok(body.to_string().into_bytes()));
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::testing::Fake;
    use super::*;

    fn repo_json(id: u64, owner: u64, name: &str) -> Value {
        serde_json::json!({"id": id, "full_name": name, "owner": {"id": owner}})
    }

    #[test]
    fn github_input_grammar() {
        assert_eq!(parse_github("uBlockOrigin/uBOL-home").unwrap(), Repo { owner: "uBlockOrigin".into(), name: "uBOL-home".into(), tag: None });
        assert_eq!(parse_github("a/b@v1.2.3").unwrap().tag.as_deref(), Some("v1.2.3"));
        assert_eq!(parse_github("https://github.com/a/b.git").unwrap().name, "b");
        assert_eq!(parse_github("https://github.com/a/b/releases/tag/v2").unwrap().tag.as_deref(), Some("v2"));
        for bad in [
            "a/b/../../users/x",
            "a/..",
            "a/.",
            "a/b@..",
            "https://gitlab.com/a/b",
            "https://github.com.evil.test/a/b",
            "https://user:pw@github.com/a/b",
            "https://github.com:8443/a/b",
            "http://github.com/a/b",
            "a b/c",
            "a/b@t/x",
            "a",
            &format!("{}/b", "o".repeat(40)),
        ] {
            assert!(parse_github(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn picks_chromium_assets_of_the_repository_only() {
        let release = serde_json::json!({"tag_name": "v1", "assets": [
            {"name": "ext-firefox.zip", "browser_download_url": "https://github.com/A/b/releases/download/v1/ext-firefox.zip"},
            {"name": "ext-chromium.zip", "browser_download_url": "https://github.com/A/b/releases/download/v1/ext-chromium.zip"},
            {"name": "notes.txt", "browser_download_url": "https://github.com/A/b/releases/download/v1/notes.txt"},
            {"name": "evil.zip", "browser_download_url": "https://github.com/other/repo/releases/download/v1/evil.zip"},
            {"name": "lower.zip", "browser_download_url": "https://github.com/a/b/releases/download/v1/lower.zip"},
            {"name": "x.crx", "browser_download_url": "https://evil.test/A/b/releases/download/v1/x.crx"}
        ]});
        let fake = Fake::default()
            .with("https://api.github.com/repos/a/b", repo_json(7, 3, "A/b"))
            .with("https://api.github.com/repositories/7/releases/latest", release);
        let r = resolve(&fake, &parse_github("a/b").unwrap()).unwrap();
        assert_eq!((r.repo_id, r.owner_id, r.full_name.as_str(), r.tag.as_str()), (7, 3, "A/b", "v1"));
        assert_eq!(r.assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["ext-chromium.zip", "ext-firefox.zip"]);
        assert_eq!(r.source(), "github:7:A/b");
        assert!(fake.asked.borrow().iter().all(|u| u.starts_with("https://api.github.com/")));
    }

    #[test]
    fn a_tag_without_release_uses_the_source_archive_and_no_release_is_explained() {
        let fake = Fake::default().with("https://api.github.com/repos/a/b", repo_json(7, 3, "a/b"));
        let r = resolve(&fake, &parse_github("a/b@v2").unwrap()).unwrap();
        assert!(r.assets.is_empty());
        assert_eq!(r.source_archive(), "https://codeload.github.com/a/b/zip/refs/tags/v2");
        assert!(resolve(&fake, &parse_github("a/b").unwrap()).unwrap_err().contains("no release"));
        let mut limited = Fake::default();
        limited.answers.insert("https://api.github.com/repos/a/b".into(), Err(FetchError::Status(403)));
        assert!(resolve(&limited, &parse_github("a/b").unwrap()).unwrap_err().contains("rate limit"));
    }

    #[test]
    fn a_bad_full_name_or_an_oversized_answer_is_refused() {
        let fake = Fake::default().with("https://api.github.com/repos/a/b", repo_json(7, 3, "../x"));
        assert!(resolve(&fake, &parse_github("a/b").unwrap()).is_err());
        let mut big = Fake::default();
        big.answers.insert("https://api.github.com/repos/a/b".into(), Ok(vec![b' '; (MAX_API_BYTES + 1) as usize]));
        assert!(resolve(&big, &parse_github("a/b").unwrap()).is_err());
    }

    #[test]
    fn only_listed_https_hosts_are_allowed() {
        for ok in ["https://api.github.com/x", "https://objects.githubusercontent.com/y", "https://codeload.github.com/a"] {
            assert!(allowed(&Url::parse(ok).unwrap()), "{ok}");
        }
        for bad in ["http://github.com/", "https://evil.test/", "https://u@github.com/", "https://github.com:444/", "https://gist.github.com/"] {
            assert!(!allowed(&Url::parse(bad).unwrap()), "{bad}");
        }
        let from = Url::parse("https://github.com/a/b/releases/download/v1/x.zip").unwrap();
        assert!(hop(&from, "https://objects.githubusercontent.com/z").is_ok());
        assert!(hop(&from, "https://evil.test/z").is_err());
        assert!(hop(&from, "http://objects.githubusercontent.com/z").is_err());
    }

    struct Hops(RefCell<Vec<(String, Reply)>>);

    impl Hop for Hops {
        fn request(&self, url: &Url, _max: u64) -> Result<Reply, FetchError> {
            let mut left = self.0.borrow_mut();
            assert!(!left.is_empty(), "unexpected request to {url}");
            let (expected, reply) = left.remove(0);
            assert_eq!(url.as_str(), expected);
            Ok(reply)
        }
    }

    fn reply(status: u16, location: Option<&str>, body: &[u8]) -> Reply {
        Reply { status, location: location.map(str::to_owned), body: body.to_vec() }
    }

    #[test]
    fn follow_checks_every_redirect_and_the_size() {
        let a = "https://github.com/a/b/releases/download/v1/theme.css";
        let b = "https://objects.githubusercontent.com/x";
        let ok = Hops(RefCell::new(vec![(a.into(), reply(302, Some(b), b"")), (b.into(), reply(200, None, b"css"))]));
        assert_eq!(ok.get(a, 3).unwrap(), b"css");
        let evil = Hops(RefCell::new(vec![(a.into(), reply(301, Some("https://evil.test/x"), b""))]));
        assert!(matches!(evil.get(a, 10), Err(FetchError::Refused(_))));
        let plain = Hops(RefCell::new(vec![(a.into(), reply(307, Some("http://github.com/x"), b""))]));
        assert!(matches!(plain.get(a, 10), Err(FetchError::Refused(_))));
        let big = Hops(RefCell::new(vec![(a.into(), reply(200, None, b"four"))]));
        assert!(matches!(big.get(a, 3), Err(FetchError::Failed(_))));
        let missing = Hops(RefCell::new(vec![(a.into(), reply(404, None, b""))]));
        assert_eq!(missing.get(a, 3), Err(FetchError::Status(404)));
        let nowhere = Hops(RefCell::new(vec![(a.into(), reply(302, None, b""))]));
        assert_eq!(nowhere.get(a, 3), Err(FetchError::Status(302)));
        let looping = Hops(RefCell::new((0..=MAX_REDIRECTS).map(|_| (a.to_owned(), reply(302, Some(a), b""))).collect()));
        assert!(matches!(looping.get(a, 3), Err(FetchError::Refused(m)) if m.contains("too many")));
        let none = Hops(RefCell::new(Vec::new()));
        for refused in ["https://evil.test/x", "http://github.com/x", "https://github.com:8443/x", "https://u:p@github.com/x", "not a url"] {
            assert!(matches!(none.get(refused, 3), Err(FetchError::Refused(_))), "{refused}");
        }
    }
}
