//! Extension installs every client shares: trust follows the install source, never the manifest;
//! packages come from GitHub, the official catalog or a file and unpack into read-only directories.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use shield::catalog::Entry;
use store::{InstalledExtension, Store};

use crate::ext_package::{self, MAX_ARCHIVE_BYTES};
use crate::github::{self, Fetch, Resolved};

pub mod install;

pub use engine::tarn_ext::{STORE_EVENT, STORE_URL, store_install};

pub const MAX_ICON_BYTES: u64 = 1 << 20;
pub const MAX_ICON_SIDE: u32 = 512;

/// The badge comes from where the extension was installed from, never from its manifest.
pub fn badge(source: &str) -> String {
    match source {
        "appex" => "App".to_owned(),
        "file" => "Unverified · file".to_owned(),
        s if s.starts_with("official:") => "Official".to_owned(),
        s => crate::theme_library::github_source(s).map_or_else(|| "Unverified".to_owned(), |(_, name)| format!("Unverified · {name}")),
    }
}

pub fn may_use_all_sites(source: &str) -> bool {
    source == "appex" || source.starts_with("official:")
}

/// The stored global "Allow extensions on every site" switch, the same setting on every client.
pub const EVERY_SITE_SETTING: &str = "extensions_every_site";

/// Whether the switch is on: it is unless the user turned it off.
pub fn every_site_setting(stored: Option<&str>) -> bool {
    stored != Some("off")
}

/// Whether an install gets every site it lists: its own switch or the global one, trusted sources only.
pub fn every_site(own: bool, global: bool, source: &str) -> bool {
    (own || global) && may_use_all_sites(source)
}

/// Turns an install off whose files changed or were withdrawn: "every site" goes off and its site
/// answers are removed.
pub fn turn_off(store: &Store, row: &InstalledExtension) {
    store.put_extension(&InstalledExtension { enabled: false, all_sites: false, ..row.clone() }).ok();
    store.retain_extension_hosts(row.profile_id, &row.install_id, |_| false).ok();
}

fn version_parts(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

/// Dotted numeric versions; anything else compares as zero parts.
pub fn older(new: &str, old: &str) -> bool {
    let (mut a, mut b) = (version_parts(new), version_parts(old));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a < b
}

/// Whether a match pattern covers `host`: `<all_urls>`, `*`, `*.domain` or the exact host, in
/// any case and with or without a trailing dot.
pub fn pattern_covers(pattern: &str, host: &str) -> bool {
    if pattern == "<all_urls>" {
        return true;
    }
    let Some(rest) = pattern.split_once("://").map(|(_, rest)| rest) else { return false };
    let plain = |h: &str| h.trim_end_matches('.').to_ascii_lowercase();
    let (pattern_host, host) = (plain(rest.split('/').next().unwrap_or_default()), plain(host));
    let (pattern_host, host) = (pattern_host.as_str(), host.as_str());
    match pattern_host.strip_prefix("*.") {
        _ if pattern_host == "*" => true,
        Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
        None => host == pattern_host,
    }
}

/// Whether an update asks for a site pattern the confirmed version did not.
pub fn grew(prev: &InstalledExtension, hosts: &[String], optional: &[String]) -> bool {
    let before: HashSet<&str> = prev.permissions.lines().collect();
    hosts.iter().chain(optional).any(|h| !before.contains(h.as_str()))
}

/// The PNG named in a manifest, checked from its header before anything decodes it: at most 1 MB
/// and 512 × 512, inside the extension's directory.
pub fn png_icon(dir: &Path, rel: &str) -> Option<Vec<u8>> {
    let path = dir.join(rel);
    let real = path.canonicalize().ok()?;
    if !real.starts_with(dir.canonicalize().ok()?) || std::fs::metadata(&real).ok()?.len() > MAX_ICON_BYTES {
        return None;
    }
    let bytes = std::fs::read(&real).ok()?;
    let header = bytes.get(..24)?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        return None;
    }
    let side = |at: usize| u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    (side(16) > 0 && side(16) <= MAX_ICON_SIDE && side(20) > 0 && side(20) <= MAX_ICON_SIDE).then_some(bytes)
}

#[derive(Clone)]
pub enum Install {
    Github(String),
    /// A repository and the release asset the user picked.
    GithubAsset(String, String),
    /// An installed repository, its release tag and the asset installed from it.
    GithubUpdate {
        repo_id: u64,
        tag: String,
        asset: String,
    },
    File(PathBuf),
    Official(Entry),
}

impl Install {
    pub fn needs_network(&self) -> bool {
        !matches!(self, Install::File(_))
    }
}

#[derive(Clone, Debug)]
pub struct Unpacked {
    pub owner_id: i64,
    pub asset: String,
    pub release_tag: String,
    pub dir: PathBuf,
    pub archive_sha256: String,
    pub tree: String,
    pub source: String,
    pub official: Option<Entry>,
}

pub enum Step {
    Unpacked(Box<Unpacked>),
    /// The release has several packages; the user picks one.
    Choose(String, Vec<github::Asset>),
    /// The installed release is still the newest.
    Current,
}

/// The installed package's asset in a newer release, by name; the first one when the name is gone
/// and the release has only one, or the source archive when it has none.
pub fn update_url(resolved: &Resolved, asset: &str) -> Option<(String, String, bool)> {
    match resolved.assets.iter().find(|a| a.name == asset) {
        Some(a) => Some((a.url.clone(), a.name.clone(), false)),
        None if resolved.assets.len() == 1 => Some((resolved.assets[0].url.clone(), resolved.assets[0].name.clone(), false)),
        None if resolved.assets.is_empty() => Some((resolved.source_archive(), String::new(), true)),
        None => None,
    }
}

pub fn download(fetch: &dyn Fetch, url: &str) -> Result<Vec<u8>, String> {
    fetch.get(url, MAX_ARCHIVE_BYTES).map_err(|e| e.describe())
}

enum Fetched {
    Github(Resolved),
    File,
    Official(Entry),
}

/// Downloads or reads the package, unpacks it into `dir` and hashes the tree. Blocks; run it off
/// the main thread.
pub fn fetch_and_unpack(fetch: &dyn Fetch, what: Install, dir: PathBuf, revoked: &BTreeSet<(String, String)>) -> Result<Step, String> {
    let mut asset = String::new();
    let (bytes, wrapped, fetched) = match what {
        Install::Github(input) => {
            let repo = github::parse_github(&input)?;
            let resolved = github::resolve(fetch, &repo)?;
            if resolved.assets.len() > 1 {
                return Ok(Step::Choose(input, resolved.assets));
            }
            let (url, wrapped) = match resolved.assets.first() {
                Some(a) => {
                    asset.clone_from(&a.name);
                    (a.url.clone(), false)
                }
                None => (resolved.source_archive(), true),
            };
            (download(fetch, &url)?, wrapped, Fetched::Github(resolved))
        }
        Install::GithubAsset(input, url) => {
            let resolved = github::resolve(fetch, &github::parse_github(&input)?)?;
            let Some(a) = resolved.assets.iter().find(|a| a.url == url) else {
                return Err("That package is no longer part of the release.".to_owned());
            };
            asset.clone_from(&a.name);
            (download(fetch, &url)?, false, Fetched::Github(resolved))
        }
        Install::GithubUpdate { repo_id, tag, asset: installed } => {
            let resolved = github::latest(fetch, repo_id)?;
            if !tag.is_empty() && resolved.tag == tag {
                return Ok(Step::Current);
            }
            let (url, name, wrapped) =
                update_url(&resolved, &installed).ok_or("The new release no longer has the package you installed; install it again to pick one.")?;
            asset = name;
            (download(fetch, &url)?, wrapped, Fetched::Github(resolved))
        }
        Install::File(path) => {
            use std::io::Read;
            let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(MAX_ARCHIVE_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
                return Err("The file is larger than 64 MB.".to_owned());
            }
            (bytes, false, Fetched::File)
        }
        Install::Official(entry) => {
            if revoked.contains(&(entry.id.clone(), entry.sha256.clone())) {
                return Err("This package was withdrawn from the official catalog.".to_owned());
            }
            let bytes = download(fetch, &entry.url)?;
            if bytes.len() as u64 != entry.size || shield::bundle::sha256_hex(&bytes) != entry.sha256 {
                return Err("The package does not match the catalog.".to_owned());
            }
            (bytes, false, Fetched::Official(entry))
        }
    };
    let archive_sha256 = shield::bundle::sha256_hex(&bytes);
    ext_package::unpack(&bytes, &dir, wrapped)?;
    let tree = ext_package::tree_sha256(&dir).map_err(|e| e.to_string())?;
    let (source, official, owner_id, release_tag) = match fetched {
        Fetched::Github(r) => (r.source(), None, r.owner_id as i64, r.tag.clone()),
        Fetched::File => ("file".to_owned(), None, 0, String::new()),
        Fetched::Official(entry) => (format!("official:{}", entry.id), Some(entry), 0, String::new()),
    };
    Ok(Step::Unpacked(Box::new(Unpacked { owner_id, asset, release_tag, dir, archive_sha256, tree, source, official })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badges_follow_the_source() {
        assert_eq!(badge("appex"), "App");
        assert_eq!(badge("official:ublock"), "Official");
        assert_eq!(badge("github:7:a/b"), "Unverified · a/b");
        assert_eq!(badge("file"), "Unverified · file");
        assert!(may_use_all_sites("appex") && may_use_all_sites("official:x"));
        assert!(!may_use_all_sites("github:7:a/b") && !may_use_all_sites("file"));
        assert!(every_site(false, true, "appex") && every_site(true, false, "official:x") && !every_site(false, false, "appex"));
        assert!(!every_site(true, true, "github:7:a/b") && !every_site(true, true, "file"));
    }

    #[test]
    fn kept_host_grants_follow_the_patterns_still_asked_for() {
        assert!(pattern_covers("<all_urls>", "a.com"));
        assert!(pattern_covers("*://*/*", "a.com"));
        assert!(pattern_covers("https://*.a.com/*", "a.com") && pattern_covers("https://*.a.com/*", "x.a.com"));
        assert!(!pattern_covers("*://*.nota.com/*", "a.com"));
        assert!(!pattern_covers("*://b.com/*", "a.com"));
        assert!(pattern_covers("*://*.Shop.Test/*", "www.shop.test.") && pattern_covers("https://a.test./*", "A.Test"));
    }

    #[test]
    fn an_update_takes_the_build_the_user_installed() {
        let asset = |name: &str| github::Asset { name: name.into(), url: format!("https://github.com/a/b/releases/download/v2/{name}") };
        let release = |assets: Vec<github::Asset>| Resolved { repo_id: 1, owner_id: 1, full_name: "a/b".into(), tag: "v2".into(), assets };
        let two = release(vec![asset("x-chromium.zip"), asset("x-safari.zip")]);
        assert_eq!(update_url(&two, "x-safari.zip").unwrap().1, "x-safari.zip");
        assert!(update_url(&two, "x-old.zip").is_none(), "an unknown name is not guessed among several");
        assert_eq!(update_url(&release(vec![asset("y.zip")]), "x.zip").unwrap().1, "y.zip");
        assert!(update_url(&release(Vec::new()), "").unwrap().2, "no asset means the source archive");
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(older("1.9", "1.10"));
        assert!(!older("1.10", "1.9"));
        assert!(!older("2.0", "2"));
        assert!(older("1.0.0", "1.0.1"));
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        [b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".as_slice(), &w.to_be_bytes(), &h.to_be_bytes(), &[8, 6, 0, 0, 0]].concat()
    }

    #[test]
    fn icons_are_small_pngs_inside_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ok.png"), png(128, 128)).unwrap();
        std::fs::write(dir.path().join("big.png"), png(20_000, 20_000)).unwrap();
        std::fs::write(dir.path().join("icon.svg"), b"<svg/>").unwrap();
        std::fs::write(dir.path().join("icon.tiff"), b"II*\0").unwrap();
        assert!(png_icon(dir.path(), "ok.png").is_some());
        for bad in ["big.png", "icon.svg", "icon.tiff", "../ok.png", "missing.png"] {
            assert!(png_icon(dir.path(), bad).is_none(), "{bad}");
        }
    }
}
