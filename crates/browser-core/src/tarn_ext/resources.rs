use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use engine::tarn_ext::{Resource, Resources, SCHEME};
use sha2::{Digest, Sha256};
use url::Url;

use super::manifest::package_path;

pub const PAGE_CSP: &str = "script-src 'self'; frame-src 'self'; object-src 'none'";
pub const BACKGROUND_PAGE: &str = "_tarn/background.html";
const MAX_RESOURCE_BYTES: u64 = 64 << 20;

/// First 32 hex digits of SHA-256(profile data-store seed ‖ install id): differs per profile and install.
pub fn origin_id(seed: &str, install_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(seed.to_ascii_lowercase().as_bytes());
    hash.update(install_id.as_bytes());
    hash.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

pub fn base_url(origin_id: &str) -> String {
    format!("{SCHEME}://{origin_id}/")
}

pub fn mime(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).as_deref() {
        Some("html" | "htm") => "text/html",
        Some("js" | "mjs") => "text/javascript",
        Some("css") => "text/css",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("txt") => "text/plain",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[derive(Clone, Debug)]
struct Served {
    dir: PathBuf,
    background: Option<(String, bool)>,
}

/// The `tarn-extension://` origins of one profile, each served only from its install's checked,
/// read-only directory. Clones share the table.
#[derive(Clone, Default)]
pub struct Resolver(Arc<RwLock<HashMap<String, Served>>>);

impl Resolver {
    /// Serves `dir` at `origin_id`, with the generated background page when `background` names
    /// its script and whether it is a module.
    pub fn serve(&self, origin_id: &str, dir: &Path, background: Option<(String, bool)>) {
        if let Ok(mut table) = self.0.write() {
            table.insert(origin_id.to_owned(), Served { dir: dir.to_owned(), background });
        }
    }

    pub fn forget(&self, origin_id: &str) {
        if let Ok(mut table) = self.0.write() {
            table.remove(origin_id);
        }
    }

    pub fn resolve(&self, url: &Url) -> Resource {
        let missing = |status: u16| Resource { status, mime: "text/plain", csp: Some(PAGE_CSP), body: Vec::new() };
        if url.scheme() != SCHEME {
            return missing(403);
        }
        let served = url.host_str().and_then(|h| self.0.read().ok()?.get(h).cloned());
        let Some(served) = served else { return missing(404) };
        let raw = url.path().trim_start_matches('/');
        let decoded = percent_decode(raw);
        let Some(path) = decoded.as_deref().and_then(package_path).filter(|p| p.len() == decoded.as_deref().map_or(0, str::len)) else {
            return missing(403);
        };
        if path == BACKGROUND_PAGE {
            let Some((script, module)) = &served.background else { return missing(404) };
            let kind = if *module { " type=\"module\"" } else { "" };
            let body = format!("<!doctype html><meta charset=\"utf-8\"><script src=\"/{script}\"{kind}></script>\n").into_bytes();
            return Resource { status: 200, mime: "text/html", csp: Some(PAGE_CSP), body };
        }
        match read_inside(&served.dir, &path, MAX_RESOURCE_BYTES) {
            Some(body) => Resource { status: 200, mime: mime(&path), csp: Some(PAGE_CSP), body },
            None => missing(404),
        }
    }

    pub fn handler(&self) -> Resources {
        let resolver = self.clone();
        Arc::new(move |url: &Url| resolver.resolve(url))
    }
}

fn percent_decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

pub(super) fn read_inside(dir: &Path, rel: &str, max: u64) -> Option<Vec<u8>> {
    let real = dir.join(rel).canonicalize().ok()?;
    let meta = std::fs::symlink_metadata(&real).ok()?;
    (real.starts_with(dir.canonicalize().ok()?) && meta.is_file() && meta.len() <= max).then(|| std::fs::read(&real).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_ids_differ_per_profile_and_install() {
        let a = origin_id("6B1C-UUID", "i1");
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_eq!(a, origin_id("6b1c-uuid", "i1"));
        assert_ne!(a, origin_id("6b1c-uuid", "i2"));
        assert_ne!(a, origin_id("other", "i1"));
    }

    #[test]
    fn serves_only_files_inside_the_install() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("ext");
        std::fs::create_dir_all(dir.join("ui")).unwrap();
        std::fs::write(dir.join("ui/popup.html"), b"<p>hi</p>").unwrap();
        std::fs::write(root.path().join("secret.txt"), b"no").unwrap();
        let resolver = Resolver::default();
        resolver.serve("abc", &dir, Some(("bg.js".into(), true)));
        let get = |u: &str| resolver.resolve(&Url::parse(u).unwrap());
        let page = get("tarn-extension://abc/ui/popup.html");
        assert_eq!((page.status, page.mime, page.csp), (200, "text/html", Some(PAGE_CSP)));
        assert_eq!(page.body, b"<p>hi</p>");
        for bad in [
            "tarn-extension://abc/../secret.txt",
            "tarn-extension://abc/ui/%2e%2e/%2e%2e/secret.txt",
            "tarn-extension://abc/ui%2f..%2f..%2fsecret.txt",
            "tarn-extension://abc/%5c..%5csecret.txt",
        ] {
            assert_ne!(get(bad).status, 200, "{bad}");
        }
        assert_eq!(get("tarn-extension://other/ui/popup.html").status, 404);
        assert_eq!(get("https://abc/ui/popup.html").status, 403);
        let bg = get("tarn-extension://abc/_tarn/background.html");
        assert!(String::from_utf8(bg.body).unwrap().contains("<script src=\"/bg.js\" type=\"module\">"));
        assert_eq!(mime("a.MJS"), "text/javascript");
        assert_eq!(mime("a.exe"), "application/octet-stream");
    }
}
