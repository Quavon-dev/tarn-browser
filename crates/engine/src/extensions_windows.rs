//! Web extensions run in WebKit only; on Windows none load, and the shell's extension surfaces
//! stay empty.

use std::path::{Path, PathBuf};

const UNSUPPORTED: &str = "Extensions are not available on Windows yet.";

#[derive(Clone)]
pub struct Extension {
    pub path: PathBuf,
    pub key: String,
}

pub struct Enabled {
    pub extension: Extension,
    pub all_sites: bool,
    pub confirmed: Vec<String>,
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
    pub bitwarden: bool,
}

impl Extension {
    pub fn unpacked(_dir: &Path, _install_id: &str, done: impl FnOnce(Result<Extension, String>) + 'static) {
        done(Err(UNSUPPORTED.to_owned()));
    }

    pub fn is_unpacked(&self) -> bool {
        true
    }

    pub fn api_permissions(&self) -> (Vec<String>, Vec<String>) {
        (Vec::new(), Vec::new())
    }

    pub fn host_patterns(&self) -> (Vec<String>, Vec<String>) {
        (Vec::new(), Vec::new())
    }

    pub fn has_active_tab(&self) -> bool {
        false
    }

    pub fn manifest_icons(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn name(&self) -> String {
        self.path.display().to_string()
    }

    pub fn version(&self) -> String {
        String::new()
    }

    pub fn permissions(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn icon_png(&self, _points: f64) -> Option<Vec<u8>> {
        None
    }
}

pub fn discover(done: impl FnOnce(Vec<Extension>) + 'static) {
    done(Vec::new());
}

pub struct HostDecision {
    pub seed: String,
    pub key: String,
    pub host: String,
    pub allow: bool,
}

pub fn denied_request(_seed: &str, _key: &str) -> bool {
    false
}

pub fn clear_denied_request(_seed: &str, _key: &str) {}

pub fn on_host_decision(_handler: impl Fn(HostDecision) + 'static) {}

pub fn on_open_tab(_handler: impl Fn(String, url::Url, bool) + 'static) {}

/// A host name or IP address that names one site: a `*` would turn a grant into a wildcard.
pub fn plain_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b)) && !host.starts_with('.')
}

pub fn forget_session(_seed: &str, _key: Option<&str>) {}
