//! Tarn's own extension runtime, the same on every engine and client. The shells only render its
//! state, forward what the engine reports and carry out the `router::Out` actions.

pub mod grants;
pub mod i18n;
pub mod manifest;
pub mod patterns;
pub mod resources;
pub mod router;
pub mod runtime;

use std::path::{Path, PathBuf};

use engine::tarn_ext::{FrameNav, Script, World, WorldScripts};
use serde_json::{Value, json};
use shield::ShieldLevel;
use shield::dnr::{Rules, Scope};

pub use grants::{Access, Session};
pub use manifest::{Manifest, is_tarn};
pub use resources::{Resolver, origin_id};

const MAX_MANIFEST: u64 = 1 << 20;
const MAX_SCRIPT_BYTES: u64 = 4 << 20;
const MAX_INJECTED_BYTES: u64 = 16 << 20;

pub fn is_tarn_dir(dir: &Path) -> bool {
    read_manifest(dir).is_ok_and(|m| is_tarn(&m))
}

fn read_manifest(dir: &Path) -> Result<Value, String> {
    let bytes = resources::read_inside(dir, "manifest.json", MAX_MANIFEST).ok_or("manifest.json is missing or too large")?;
    serde_json::from_slice(&bytes).map_err(|_| "manifest.json is not valid JSON".to_owned())
}

#[derive(Debug, PartialEq)]
pub enum LoadError {
    /// The tree no longer hashes as recorded: the install goes off and its site grants are dropped.
    Changed,
    Refused(String),
}

pub struct Package {
    pub install_id: String,
    pub origin_id: String,
    pub world: World,
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub name: String,
    pub description: String,
    pub locale: String,
    pub rules: Rules,
    pub scripts: WorldScripts,
    raw: Value,
    messages: i18n::Messages,
}

/// Loads the install at `dir` for the profile whose data-store seed is `seed`. `expected` is the
/// tree hash recorded at install (empty before the first check); nothing runs if it differs.
pub fn load(dir: &Path, expected: &str, install_id: &str, seed: &str, ui_locales: &[String]) -> Result<Package, LoadError> {
    let tree = crate::ext_package::tree_sha256(dir).map_err(|e| LoadError::Refused(e.to_string()))?;
    if !expected.is_empty() && tree != expected {
        return Err(LoadError::Changed);
    }
    parse(dir, install_id, seed, ui_locales).map_err(LoadError::Refused)
}

fn parse(dir: &Path, install_id: &str, seed: &str, ui_locales: &[String]) -> Result<Package, String> {
    let raw = read_manifest(dir)?;
    let manifest = manifest::parse(&raw)?;
    let world = World::for_install(install_id).ok_or("the install id is invalid")?;
    let (locale, messages) = i18n::load(dir, manifest.default_locale.as_deref(), ui_locales)?;
    let mut budget = MAX_INJECTED_BYTES;
    let mut read = |path: &str| -> Result<String, String> {
        let bytes = resources::read_inside(dir, path, MAX_SCRIPT_BYTES.min(budget))
            .ok_or_else(|| format!("This Tarn extension cannot be installed: \"{path}\" is missing or too large."))?;
        budget -= bytes.len() as u64;
        String::from_utf8(bytes).map_err(|_| format!("This Tarn extension cannot be installed: \"{path}\" is not UTF-8."))
    };
    let mut scripts = Vec::new();
    for c in &manifest.content_scripts {
        let js = c.js.iter().map(|p| read(p)).collect::<Result<_, _>>()?;
        let css = c.css.iter().map(|p| read(p)).collect::<Result<_, _>>()?;
        scripts.push(Script { run_at: c.run_at, all_frames: c.all_frames, js, css });
    }
    let mut rule_files = Vec::new();
    for path in &manifest.rule_files {
        let bytes = resources::read_inside(dir, path, shield::dnr::MAX_FILE_BYTES as u64)
            .ok_or_else(|| format!("This Tarn extension cannot be installed: the rule file \"{path}\" is missing or larger than 4 MiB."))?;
        rule_files.push(bytes);
    }
    let rules = shield::dnr::compile(&rule_files.iter().map(Vec::as_slice).collect::<Vec<_>>())
        .map_err(|e| format!("This Tarn extension cannot be installed: {e}."))?;
    for path in manifest.action.iter().flat_map(|a| &a.popup).chain(manifest.background.iter().map(|b| &b.script)) {
        if !dir.join(path).is_file() {
            return Err(format!("This Tarn extension cannot be installed: \"{path}\" is missing."));
        }
    }
    let origin_id = origin_id(seed, install_id);
    let name = i18n::localize(&manifest.name, &messages);
    let description = i18n::localize(&manifest.description, &messages);
    let mut package = Package {
        install_id: install_id.to_owned(),
        origin_id,
        world: world.clone(),
        dir: dir.to_owned(),
        manifest,
        name,
        description,
        locale,
        rules,
        scripts: WorldScripts { world, bootstrap: String::new(), scripts },
        raw,
        messages,
    };
    package.scripts.bootstrap = package.bootstrap(false);
    Ok(package)
}

impl Package {
    /// The shim with this install's configuration, for its content-script world (`page` false) or
    /// its own pages.
    pub fn bootstrap(&self, page: bool) -> String {
        let mut messages: serde_json::Map<String, Value> = self.messages.iter().map(|(k, v)| (k.clone(), Value::from(v.as_str()))).collect();
        messages.insert("@@extension_id".into(), self.origin_id.clone().into());
        messages.insert("@@ui_locale".into(), self.locale.clone().into());
        messages.insert("@@bidi_dir".into(), "ltr".into());
        engine::tarn_ext::bootstrap(&json!({
            "id": self.origin_id,
            "base": resources::base_url(&self.origin_id),
            "manifest": self.raw.to_string(),
            "messages": messages,
            "locale": self.locale,
            "storage": self.manifest.storage,
            "page": page,
        }))
    }

    pub fn url(&self, path: &str) -> Option<url::Url> {
        url::Url::parse(&resources::base_url(&self.origin_id)).ok()?.join(&manifest::package_path(path)?).ok()
    }

    pub fn popup_url(&self) -> Option<url::Url> {
        self.url(self.manifest.action.as_ref()?.popup.as_deref()?)
    }

    pub fn background_url(&self) -> Option<url::Url> {
        self.manifest.background.as_ref().and_then(|_| self.url(resources::BACKGROUND_PAGE))
    }

    pub fn serve(&self, resolver: &Resolver) {
        resolver.serve(&self.origin_id, &self.dir, self.manifest.background.as_ref().map(|b| (b.script.clone(), b.module)));
    }

    /// The toolbar icon closest to `size` pixels, checked as a small PNG inside the package.
    pub fn icon_png(&self, size: u32) -> Option<Vec<u8>> {
        let icons = self.manifest.action.as_ref().map(|a| &a.icons).filter(|i| !i.is_empty()).unwrap_or(&self.manifest.icons);
        crate::extensions::png_icon(&self.dir, manifest::select_icon(icons, size)?)
    }
}

/// Where a frame is: extensions run only in a persistent, non-burner space of a profile not at
/// Lockdown, in a view not at Lockdown.
#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub private: bool,
    pub burner: bool,
    pub profile_level: ShieldLevel,
    pub view_level: ShieldLevel,
}

impl Context {
    pub fn runs_extensions(&self) -> bool {
        !self.private && !self.burner && self.profile_level != ShieldLevel::Lockdown && self.view_level != ShieldLevel::Lockdown
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct FrameDecision {
    pub run: Vec<usize>,
    /// A host to raise the site prompt for, once per session (`Session::ask`).
    pub ask: Option<String>,
}

/// The engine's `Gate` answer for one frame of `package`. Only http and https documents qualify;
/// a script runs only where the extension holds a grant for the frame's host.
pub fn frame_decision(package: &Package, context: &Context, frame: &FrameNav, access: impl Fn(&str) -> Access) -> FrameDecision {
    let web = |u: &url::Url| matches!(u.scheme(), "http" | "https");
    let mut out = FrameDecision::default();
    if !context.runs_extensions() || !web(frame.url) || !web(frame.top) {
        return out;
    }
    let Some(host) = frame.url.host_str() else { return out };
    for (ix, script) in package.manifest.content_scripts.iter().enumerate() {
        let wanted = (frame.main_frame || script.all_frames)
            && script.matches.iter().any(|p| p.matches(frame.url))
            && !script.exclude.iter().any(|p| p.matches(frame.url));
        if !wanted {
            continue;
        }
        match access(host) {
            Access::Granted => out.run.push(ix),
            Access::Ask if frame.main_frame => out.ask = Some(host.trim_end_matches('.').to_ascii_lowercase()),
            Access::Ask | Access::Denied => {}
        }
    }
    out
}

/// The install's content rule list for views of `context`, `None` where none applies.
pub fn rule_list(package: &Package, context: &Context, scope: &Scope) -> Option<String> {
    if !context.runs_extensions() {
        return None;
    }
    package.rules.content_rules(scope)
}

#[cfg(test)]
mod tests;
