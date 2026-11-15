use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use engine::extensions::{Enabled, Extension, HostDecision};
use engine::nav_policy::{self, Decision, Source};
use gpui::{
    App, AsyncApp, Context, Image, ImageFormat, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window, div, img,
    prelude::FluentBuilder, px,
};
use shield::ShieldLevel;
use shield::catalog::{self, Entry, Index};
use store::{Id, InstalledExtension};
use ui::controls::{icon_button, tooltip};
use ui::icon::Icon;
use ui::theme::Theme;

use crate::browser::Browser;
use crate::ext_package::{self, MAX_ARCHIVE_BYTES};
use crate::ext_sources::{self, Fetch, Net};
use crate::prefs::Pref;
use crate::shared::AppCore;

pub use browser_core::theme_fetch::DOWNLOADS_SETTING;
const NOTICE_SETTING: &str = "extension_all_sites_notice";
const REVOKED_SETTING: &str = "extension_revoked";
const CATALOG_SEEN_SETTING: &str = "extension_catalog_version";
const LOADING_CATALOG: &str = "Loading the official catalog…";
const MAX_INSTALLS: usize = 32;
const MAX_ICON_BYTES: u64 = 1 << 20;
const MAX_ICON_SIDE: u32 = 512;
pub const UNREVIEWED: &str = "Nobody reviewed this extension's code. It can read and change the sites you allow and send what it reads anywhere.";

pub fn root() -> PathBuf {
    crate::browser::data_dir().join("Extensions")
}

/// The catalog key. Debug builds take `TARN_TEST_CATALOG_KEY` (64 hex digits) instead.
fn catalog_key() -> [u8; 32] {
    #[cfg(debug_assertions)]
    if let Some(key) = std::env::var("TARN_TEST_CATALOG_KEY").ok().and_then(|hex| {
        let bytes: Vec<u8> = (0..hex.len()).step_by(2).filter_map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect();
        bytes.try_into().ok()
    }) {
        return key;
    }
    catalog::EXTENSION_CATALOG_PUBLIC_KEY
}

pub fn catalog_configured() -> bool {
    catalog_key() != [0; 32]
}

/// The badge comes from where the extension was installed from, never from its manifest.
pub fn badge(source: &str) -> String {
    match source {
        "appex" => "App".to_owned(),
        "file" => "Unverified · file".to_owned(),
        s if s.starts_with("official:") => "Official".to_owned(),
        s => ext_sources::github_source(s).map_or_else(|| "Unverified".to_owned(), |(_, name)| format!("Unverified · {name}")),
    }
}

fn may_use_all_sites(source: &str) -> bool {
    source == "appex" || source.starts_with("official:")
}

/// Whether an install gets every site it lists: its own switch or the global one, trusted sources only.
fn every_site(own: bool, global: bool, source: &str) -> bool {
    (own || global) && may_use_all_sites(source)
}

pub use browser_core::theme_fetch::shown;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Dotted numeric versions; anything else compares as zero parts.
fn version_parts(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

fn older(new: &str, old: &str) -> bool {
    let (mut a, mut b) = (version_parts(new), version_parts(old));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a < b
}

/// The PNG an unpacked extension names in its manifest, checked from its header before anything
/// decodes it: at most 1 MB and 512 × 512, inside the extension's directory.
fn png_icon(dir: &Path, rel: &str) -> Option<Vec<u8>> {
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

fn icon_of(extension: &Extension) -> Option<Arc<Image>> {
    let png = if extension.is_unpacked() { extension.manifest_icons().iter().find_map(|rel| png_icon(&extension.path, rel)) } else { extension.icon_png(16.) };
    png.map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)))
}

/// What the consent prompt shows, read from WebKit's parse of the extension.
#[derive(Clone)]
pub struct Consent {
    pub name: String,
    pub version: String,
    pub badge: String,
    pub apis: Vec<String>,
    pub optional_apis: Vec<String>,
    pub hosts: Vec<String>,
    pub optional_hosts: Vec<String>,
    pub active_tab: bool,
    pub unreviewed: bool,
    /// For an update: what changed since the confirmed version.
    pub changes: Vec<String>,
}

struct Pending {
    profile_id: Id,
    row: InstalledExtension,
    tree: String,
    extension: Extension,
    /// The install's directory when it is new, removed on cancel; an update removes only its version.
    new_install: Option<PathBuf>,
    previous: Option<InstalledExtension>,
}

/// Extensions found at launch and installed per profile, with the ones running in each profile.
#[derive(Default)]
pub struct Extensions {
    apps: Vec<Extension>,
    discovered: bool,
    unpacked: HashMap<String, Extension>,
    /// Paths whose tree hash matched their row in this run.
    verified: HashSet<String>,
    checking: HashSet<String>,
    icons: HashMap<String, Option<Arc<Image>>>,
    running: HashMap<(bool, Id), Vec<String>>,
    pub notices: Vec<String>,
    pub busy: Option<String>,
    /// A repository whose release offers several packages, with them.
    pub choice: Option<(String, Vec<ext_sources::Asset>)>,
    generation: u64,
    pending: Option<Pending>,
    updates: Vec<(Install, String)>,
    /// App extensions turned back on by the user, whose changed bundle is accepted once.
    repin: HashSet<String>,
    /// Check for Updates runs once the catalog it asked for arrives.
    check_after_catalog: bool,
    catalog: Option<Index>,
}

pub struct Row {
    pub install_id: Option<String>,
    pub app_path: Option<String>,
    pub name: String,
    pub badge: String,
    pub permissions: String,
    pub enabled: bool,
    pub all_sites: Option<bool>,
}

pub struct CatalogRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub installed: bool,
}

impl Extensions {
    pub(crate) fn forget_running(&mut self, private: bool, profile_id: Id) {
        self.running.remove(&(private, profile_id));
    }

    pub fn consent(&self) -> Option<Consent> {
        let p = self.pending.as_ref()?;
        let ((apis, optional_apis), (hosts, optional_hosts)) = (p.extension.api_permissions(), p.extension.host_patterns());
        let mut changes = Vec::new();
        if let Some(prev) = &p.previous {
            let before: HashSet<&str> = prev.permissions.lines().collect();
            changes.extend(
                apis.iter()
                    .chain(&optional_apis)
                    .chain(&hosts)
                    .chain(&optional_hosts)
                    .filter(|x| !before.contains(x.as_str()))
                    .map(|x| format!("New: {}", shown(x))),
            );
            changes.extend(ext_sources::provenance(
                (&prev.source, prev.owner_id as u64, &prev.version, &prev.archive_sha256),
                (&p.row.source, p.row.owner_id as u64, &p.row.version, &p.row.archive_sha256),
                shown,
            ));
            if prev.all_sites && grew(prev, &hosts, &optional_hosts) {
                changes.push("Its \"Allow on every site\" switch turns off, since it asks for more sites.".to_owned());
            }
        }
        Some(Consent {
            name: shown(&p.extension.name()),
            version: shown(&p.extension.version()),
            badge: badge(&p.row.source),
            active_tab: p.extension.has_active_tab(),
            unreviewed: !may_use_all_sites(&p.row.source),
            apis,
            optional_apis,
            hosts,
            optional_hosts,
            changes,
        })
    }
}

fn confirmed(extension: &Extension) -> String {
    let ((apis, optional_apis), (hosts, optional_hosts)) = (extension.api_permissions(), extension.host_patterns());
    apis.into_iter().chain(optional_apis).chain(hosts).chain(optional_hosts).collect::<Vec<_>>().join("\n")
}

/// Whether a match pattern covers `host`: `<all_urls>`, `*`, `*.domain` or the exact host.
fn pattern_covers(pattern: &str, host: &str) -> bool {
    if pattern == "<all_urls>" {
        return true;
    }
    let Some(rest) = pattern.split_once("://").map(|(_, rest)| rest) else { return false };
    let pattern_host = rest.split('/').next().unwrap_or_default();
    match pattern_host.strip_prefix("*.") {
        _ if pattern_host == "*" => true,
        Some(domain) => host == domain || host.ends_with(&format!(".{domain}")),
        None => host == pattern_host,
    }
}

fn grew(prev: &InstalledExtension, hosts: &[String], optional: &[String]) -> bool {
    let before: HashSet<&str> = prev.permissions.lines().collect();
    hosts.iter().chain(optional).any(|h| !before.contains(h.as_str()))
}

/// Installs the extensions found at launch, sweeps leftovers and checks every enabled install.
pub(crate) fn discovered(found: Vec<Extension>, cx: &mut App) {
    let core = crate::shared::core(cx);
    {
        let mut extensions = core.extensions.borrow_mut();
        for e in &found {
            let icon = icon_of(e);
            extensions.icons.insert(e.key.clone(), icon);
        }
        extensions.apps = found;
        extensions.discovered = true;
        extensions.catalog = cached_catalog();
    }
    if let Some(index) = core.extensions.borrow().catalog.as_ref() {
        merge_revoked(&core, index);
    }
    if let Ok(paths) = core.disk.extension_paths() {
        let keep: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        cx.background_executor().spawn(async move { ext_package::sweep(&root(), &keep) }).detach();
    }
    verify(cx);
}

/// The index last fetched, verified again.
fn cached_catalog() -> Option<Index> {
    if !catalog_configured() {
        return None;
    }
    let dir = crate::browser::data_dir().join("catalog");
    let (index, sig) = (std::fs::read(dir.join("index.json")).ok()?, std::fs::read(dir.join("index.sig")).ok()?);
    catalog::verify(&index, &sig, &catalog_key()).ok()
}

fn merge_revoked(core: &AppCore, index: &Index) {
    let stored = core.disk.setting(REVOKED_SETTING).ok().flatten().unwrap_or_default();
    core.disk.set_setting(REVOKED_SETTING, &catalog::merge_revoked(&stored, index)).ok();
}

/// Opens a web page an app extension asked for with `tabs.create`, in a window showing its profile.
/// The tab is a shell load, so the public-to-private check (S10) is made here.
pub(crate) fn open_tab(seed: &str, url: &url::Url, front: bool, cx: &mut App) {
    if nav_policy::decide(url, Source::Link) != Decision::Allow || shield::rules::is_local(url, true) {
        return;
    }
    let mut pending = Some(url);
    for active_only in [true, false] {
        crate::shared::each(cx, |b, window, cx| {
            let own = b
                .model
                .current_space()
                .is_some_and(|s| !s.private && b.loaded(false, s.space.profile_id).is_some_and(|rt| rt.own_store.eq_ignore_ascii_case(seed)));
            if own
                && (window.is_window_active() || !active_only)
                && let Some(url) = pending.take()
            {
                b.new_tab(url, window, cx);
                if front {
                    window.activate_window();
                }
            }
        });
    }
}

/// Stores Always and Never answers of host prompts for the profile whose extension storage they came from.
pub(crate) fn store_host_decisions(core: std::rc::Rc<AppCore>) {
    engine::extensions::on_host_decision(move |d: HostDecision| {
        let profile = core.profiles.borrow().iter().find(|((private, _), rt)| !private && rt.own_store.eq_ignore_ascii_case(&d.seed)).map(|((_, id), _)| *id);
        let Some(id) = profile else { return };
        let rows = core.disk.extensions(id).unwrap_or_default();
        if let Some(row) = rows.iter().find(|r| r.install_id == d.key || r.path == d.key) {
            core.disk.set_extension_host(id, &row.install_id, &d.host, d.allow).ok();
        }
    });
}

/// Hashes every enabled install not checked in this run, off the main thread, and loads the
/// unpacked ones that match; a mismatch or a revoked package turns the install off.
pub(crate) fn verify(cx: &mut App) {
    let core = crate::shared::core(cx);
    let revoked = catalog::parse_revoked(&core.disk.setting(REVOKED_SETTING).ok().flatten().unwrap_or_default());
    let profiles = core.disk.profiles().unwrap_or_default();
    for profile in profiles.iter().filter(|p| !p.is_private) {
        for row in core.disk.extensions(profile.id).unwrap_or_default().into_iter().filter(|r| r.enabled) {
            {
                let ext = core.extensions.borrow();
                if ext.verified.contains(&row.path) || ext.checking.contains(&row.path) {
                    continue;
                }
            }
            if let Some(id) = row.source.strip_prefix("official:")
                && revoked.contains(&(id.to_owned(), row.archive_sha256.clone()))
            {
                turn_off(&core, profile.id, &row, "was withdrawn from the official catalog and was turned off.");
                refresh(profile.id, cx);
                continue;
            }
            core.extensions.borrow_mut().checking.insert(row.path.clone());
            let path = row.path.clone();
            let hashed = cx.background_executor().spawn(async move { ext_package::tree_sha256(Path::new(&path)).ok() });
            let profile_id = profile.id;
            cx.spawn(async move |cx: &mut AsyncApp| {
                let hash = hashed.await;
                cx.update(|cx| checked(profile_id, row, hash, cx)).ok();
            })
            .detach();
        }
    }
}

fn turn_off(core: &AppCore, profile_id: Id, row: &InstalledExtension, why: &str) {
    let off = InstalledExtension { enabled: false, all_sites: false, ..row.clone() };
    core.disk.put_extension(&off).ok();
    core.disk.retain_extension_hosts(profile_id, &row.install_id, |_| false).ok();
    if let Some(rt) = core.profiles.borrow().get(&(false, profile_id)) {
        let key = if row.source == "appex" { &row.path } else { &row.install_id };
        engine::extensions::forget_session(&rt.own_store.to_uppercase(), Some(key));
    }
    let name = if row.name.is_empty() { badge(&row.source) } else { shown(&row.name) };
    core.extensions.borrow_mut().notices.push(format!("{name} {why}"));
}

fn checked(profile_id: Id, row: InstalledExtension, hash: Option<String>, cx: &mut App) {
    let core = crate::shared::core(cx);
    core.extensions.borrow_mut().checking.remove(&row.path);
    // Removed or updated while it was hashed: the result is for a row that no longer exists.
    let current = core.disk.extensions(profile_id).unwrap_or_default().into_iter().find(|r| r.install_id == row.install_id);
    let Some(mut row) = current.filter(|r| r.path == row.path && r.sha256 == row.sha256 && r.enabled) else { return refresh(profile_id, cx) };
    let repinned = core.extensions.borrow_mut().repin.remove(&row.install_id);
    match hash {
        Some(hash) if row.sha256.is_empty() || row.sha256 == hash || repinned => {
            if row.sha256 != hash {
                // An app the user turned back on after it changed: what it asks for now is what
                // Settings lists, and every site needs a new yes. A row moved from the old
                // settings records the list it was turned on with.
                if (!row.sha256.is_empty() || row.permissions.is_empty())
                    && let Some(app) = core.extensions.borrow().apps.iter().find(|a| a.path.to_string_lossy() == row.path)
                {
                    row.permissions = confirmed(app);
                    row.all_sites &= row.sha256.is_empty();
                }
                row.sha256 = hash;
                core.disk.put_extension(&row).ok();
            }
            if row.source == "appex" {
                core.extensions.borrow_mut().verified.insert(row.path.clone());
                return refresh(profile_id, cx);
            }
            let path = row.path.clone();
            let async_cx = cx.to_async();
            Extension::unpacked(Path::new(&row.path.clone()), &row.install_id.clone(), move |loaded| {
                let _ = async_cx.update(|cx| {
                    let core = crate::shared::core(cx);
                    match loaded {
                        Ok(extension) => {
                            let mut ext = core.extensions.borrow_mut();
                            ext.icons.insert(extension.key.clone(), icon_of(&extension));
                            ext.unpacked.insert(path.clone(), extension);
                            ext.verified.insert(path);
                        }
                        Err(_) => turn_off(&core, profile_id, &row, "could not be loaded and was turned off."),
                    }
                    refresh(profile_id, cx);
                });
            });
        }
        _ => {
            turn_off(&core, profile_id, &row, "changed on disk and was turned off; its site permissions were removed.");
            refresh(profile_id, cx);
        }
    }
}

/// Applies the profile's extensions and restarts its views, in every window, if they changed.
/// The windows are updated once no `Browser` is leased, since callers may be inside one's update.
fn refresh(profile_id: Id, cx: &mut App) {
    let core = crate::shared::core(cx);
    let changed = core.apply_extensions(false, profile_id);
    cx.defer(move |cx| {
        crate::shared::each(cx, |b, window, cx| {
            if changed {
                b.restart_views_here(false, profile_id, None, false, window, cx);
            }
            if let Some(key) = b.model.active.filter(|key| !b.views.contains_key(key)) {
                b.activate(key, window, cx);
            }
            cx.notify();
        })
    });
}

impl AppCore {
    /// A profile with extensions turned on gets its first web views once they are checked and
    /// loaded, since a view created before would need a reload to run them.
    pub(crate) fn awaits_extensions(&self, private: bool, profile_id: Id) -> bool {
        if private {
            return false;
        }
        let rows = self.disk.extensions(profile_id).unwrap_or_default();
        let ext = self.extensions.borrow();
        rows.iter().any(|r| r.enabled) && (!ext.discovered || rows.iter().any(|r| r.enabled && ext.checking.contains(&r.path)))
    }

    fn enabled_extensions(&self, profile_id: Id) -> Vec<Enabled> {
        let ext = self.extensions.borrow();
        self.disk
            .extensions(profile_id)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.enabled && ext.verified.contains(&r.path))
            .filter_map(|r| {
                let extension = if r.source == "appex" {
                    ext.apps.iter().find(|a| a.path.to_string_lossy() == r.path).cloned()
                } else {
                    ext.unpacked.get(&r.path).cloned()
                }?;
                let hosts = self.disk.extension_hosts(profile_id, &r.install_id).unwrap_or_default();
                let (allowed, denied) = hosts.into_iter().partition::<Vec<_>, _>(|(_, allow)| *allow);
                Some(Enabled {
                    extension,
                    all_sites: every_site(r.all_sites, self.pref(Pref::ExtensionsEverySite), &r.source),
                    confirmed: r.permissions.lines().map(str::to_owned).collect(),
                    allowed: allowed.into_iter().map(|(h, _)| h).collect(),
                    denied: denied.into_iter().map(|(h, _)| h).collect(),
                })
            })
            .collect()
    }

    /// Private and Lockdown profiles never run extensions, and sites set to Lockdown are denied
    /// to them. Returns whether the extensions running in the profile changed.
    pub(crate) fn apply_extensions(&self, private: bool, profile_id: Id) -> bool {
        let profiles = self.profiles.borrow();
        let Some(rt) = profiles.get(&(private, profile_id)) else { return false };
        let level = rt.profile.shield_level;
        let enabled = if private || level == ShieldLevel::Lockdown { Vec::new() } else { self.enabled_extensions(profile_id) };
        let blocked: Vec<String> = rt.overrides.borrow().iter().filter(|(_, level)| **level == ShieldLevel::Lockdown).map(|(host, _)| host.clone()).collect();
        rt.data.set_extensions(&enabled, &blocked, level);
        let loaded = rt.data.running_extensions();
        let running: Vec<String> = enabled.into_iter().map(|e| e.extension.key).filter(|k| loaded.contains(k)).collect();
        self.extensions.borrow_mut().running.insert((private, profile_id), running.clone()).unwrap_or_default() != running
    }
}

enum Fetched {
    Github(ext_sources::Resolved),
    File,
    Official(Entry),
}

struct Unpacked {
    owner_id: i64,
    asset: String,
    release_tag: String,
    dir: PathBuf,
    archive_sha256: String,
    tree: String,
    source: String,
    official: Option<Entry>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    shield::bundle::sha256_hex(bytes)
}

/// Downloads or reads the package, unpacks it into `dir` and hashes the tree. Runs off the main thread.
enum Step {
    Unpacked(Box<Unpacked>),
    /// The release has several packages; the user picks one.
    Choose(String, Vec<ext_sources::Asset>),
    /// The installed release is still the newest.
    Current,
}

/// The installed package's asset in a newer release, by name; the first one when the name is gone
/// and the release has only one, or the source archive when it has none.
fn update_url(resolved: &ext_sources::Resolved, asset: &str) -> Option<(String, String, bool)> {
    match resolved.assets.iter().find(|a| a.name == asset) {
        Some(a) => Some((a.url.clone(), a.name.clone(), false)),
        None if resolved.assets.len() == 1 => Some((resolved.assets[0].url.clone(), resolved.assets[0].name.clone(), false)),
        None if resolved.assets.is_empty() => Some((resolved.source_archive(), String::new(), true)),
        None => None,
    }
}

fn fetch_and_unpack(what: Install, dir: PathBuf, revoked: &std::collections::BTreeSet<(String, String)>) -> Result<Step, String> {
    let net = Net::new();
    let fetch: &dyn Fetch = &net;
    let mut asset = String::new();
    let (bytes, wrapped, fetched) = match what {
        Install::Github(input) => {
            let repo = ext_sources::parse_github(&input)?;
            let resolved = ext_sources::resolve(fetch, &repo)?;
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
            (ext_sources::download(fetch, &url)?, wrapped, Fetched::Github(resolved))
        }
        Install::GithubAsset(input, url) => {
            let resolved = ext_sources::resolve(fetch, &ext_sources::parse_github(&input)?)?;
            // Only an asset of the release just resolved, never an address from elsewhere.
            let Some(a) = resolved.assets.iter().find(|a| a.url == url) else {
                return Err("That package is no longer part of the release.".to_owned());
            };
            asset.clone_from(&a.name);
            (ext_sources::download(fetch, &url)?, false, Fetched::Github(resolved))
        }
        Install::GithubUpdate { repo_id, tag, asset: installed } => {
            let resolved = ext_sources::latest(fetch, repo_id)?;
            if !tag.is_empty() && resolved.tag == tag {
                return Ok(Step::Current);
            }
            let (url, name, wrapped) =
                update_url(&resolved, &installed).ok_or("The new release no longer has the package you installed; install it again to pick one.")?;
            asset = name;
            (ext_sources::download(fetch, &url)?, wrapped, Fetched::Github(resolved))
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
            let bytes = ext_sources::download(fetch, &entry.url)?;
            if bytes.len() as u64 != entry.size || sha256_hex(&bytes) != entry.sha256 {
                return Err("The package does not match the catalog.".to_owned());
            }
            (bytes, false, Fetched::Official(entry))
        }
    };
    let archive_sha256 = sha256_hex(&bytes);
    ext_package::unpack(&bytes, &dir, wrapped)?;
    let tree = ext_package::tree_sha256(&dir).map_err(|e| e.to_string())?;
    let (source, official, owner_id, release_tag) = match fetched {
        Fetched::Github(r) => (r.source(), None, r.owner_id as i64, r.tag.clone()),
        Fetched::File => ("file".to_owned(), None, 0, String::new()),
        Fetched::Official(entry) => (format!("official:{}", entry.id), Some(entry), 0, String::new()),
    };
    Ok(Step::Unpacked(Box::new(Unpacked { owner_id, asset, release_tag, dir, archive_sha256, tree, source, official })))
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
    fn needs_network(&self) -> bool {
        !matches!(self, Install::File(_))
    }
}

/// The window that started an install is gone: removes what it unpacked and frees the shared
/// busy state, unless a newer install holds it.
fn release_install(core: &crate::shared::AppCore, generation: u64, leftover: Option<PathBuf>, cx: &mut gpui::AsyncApp) {
    if let Some(dir) = leftover {
        ext_package::remove_tree(&dir).ok();
    }
    let mut ext = core.extensions.borrow_mut();
    if ext.generation == generation {
        ext.busy = None;
    }
    drop(ext);
    cx.update(|cx| cx.refresh_windows()).ok();
}

impl Browser {
    pub fn extensions_allowed(&self) -> bool {
        self.model.current_space().is_some_and(|s| !s.private) && self.current_profile().is_some_and(|p| p.shield_level != ShieldLevel::Lockdown)
    }

    pub fn extension_downloads(&self) -> bool {
        self.core.disk.setting(DOWNLOADS_SETTING).ok().flatten().as_deref() != Some("off")
    }

    pub fn set_extension_downloads(&mut self, on: bool, cx: &mut Context<Self>) {
        self.core.disk.set_setting(DOWNLOADS_SETTING, if on { "on" } else { "off" }).ok();
        cx.notify();
    }

    fn current_profile_id(&self) -> Option<Id> {
        self.model.current_space().filter(|s| !s.private).map(|s| s.space.profile_id)
    }

    pub fn extension_rows(&self) -> Vec<Row> {
        let Some(id) = self.current_profile_id() else { return Vec::new() };
        let rows = self.core.disk.extensions(id).unwrap_or_default();
        let ext = self.core.extensions.borrow();
        let mut out: Vec<Row> = rows
            .iter()
            .map(|r| {
                let loaded = if r.source == "appex" { ext.apps.iter().find(|a| a.path.to_string_lossy() == r.path) } else { ext.unpacked.get(&r.path) };
                Row {
                    install_id: Some(r.install_id.clone()),
                    app_path: None,
                    name: shown(&loaded.map_or_else(|| r.name.clone(), |e| format!("{} {}", e.name(), e.version()))),
                    badge: badge(&r.source),
                    permissions: shown(&r.permissions.lines().collect::<Vec<_>>().join(", ")),
                    enabled: r.enabled,
                    all_sites: (may_use_all_sites(&r.source) && !self.pref(Pref::ExtensionsEverySite)).then_some(r.all_sites),
                }
            })
            .collect();
        for app in ext.apps.iter().filter(|a| !rows.iter().any(|r| r.path == a.path.to_string_lossy())) {
            out.push(Row {
                install_id: None,
                app_path: Some(app.path.to_string_lossy().into_owned()),
                name: shown(&format!("{} {}", app.name(), app.version())),
                badge: badge("appex"),
                permissions: shown(&app.permissions().join(", ")),
                enabled: false,
                all_sites: None,
            });
        }
        out
    }

    /// The badge of the install `key` (install id or bundle path) in the current profile.
    pub fn extension_badge(&self, key: &str) -> String {
        let rows = self.current_profile_id().map(|id| self.core.disk.extensions(id).unwrap_or_default()).unwrap_or_default();
        rows.iter().find(|r| r.install_id == key || r.path == key).map_or_else(|| badge("file"), |r| badge(&r.source))
    }

    pub fn all_sites_notice(&self) -> bool {
        self.core.disk.setting(NOTICE_SETTING).ok().flatten().is_some()
    }

    pub fn dismiss_all_sites_notice(&mut self, cx: &mut Context<Self>) {
        self.core.disk.delete_setting(NOTICE_SETTING).ok();
        cx.notify();
    }

    fn row_of(&self, install_id: &str) -> Option<InstalledExtension> {
        let id = self.current_profile_id()?;
        self.core.disk.extensions(id).ok()?.into_iter().find(|r| r.install_id == install_id)
    }

    /// Turning an app extension on for the first time records it; the list it shows is the consent.
    pub fn enable_app_extension(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current_profile_id() else { return };
        let Some(app) = self.core.extensions.borrow().apps.iter().find(|a| a.path.to_string_lossy() == path).cloned() else { return };
        let row = InstalledExtension {
            profile_id: id,
            install_id: uuid::Uuid::new_v4().simple().to_string(),
            source: "appex".into(),
            path,
            name: app.name(),
            version: app.version(),
            permissions: confirmed(&app),
            enabled: true,
            ..Default::default()
        };
        if self.core.disk.put_extension(&row).is_ok() {
            self.after_extension_change(id, window, cx);
        }
    }

    pub fn set_extension(&mut self, install_id: &str, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.row_of(install_id) else { return };
        if on {
            self.core.extensions.borrow_mut().verified.remove(&row.path);
        }
        // Turning an app extension back on after its app changed accepts the new bundle; an
        // unpacked one that changed stays refused until it is installed again.
        if on && row.source == "appex" && !row.enabled {
            self.core.extensions.borrow_mut().repin.insert(row.install_id.clone());
        }
        if self.core.disk.put_extension(&InstalledExtension { enabled: on, ..row.clone() }).is_ok() {
            self.after_extension_change(row.profile_id, window, cx);
        }
    }

    pub fn set_extension_all_sites(&mut self, install_id: &str, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.row_of(install_id).filter(|r| may_use_all_sites(&r.source)) else { return };
        if self.core.disk.put_extension(&InstalledExtension { all_sites: on, ..row.clone() }).is_ok() {
            self.after_extension_change(row.profile_id, window, cx);
        }
    }

    fn after_extension_change(&mut self, profile_id: Id, window: &mut Window, cx: &mut Context<Self>) {
        crate::extensions::verify(cx);
        self.core.apply_extensions(false, profile_id);
        self.restart_profile_views(false, profile_id, None, false, window, cx);
        cx.notify();
    }

    /// Deletes the install's storage first, then its rows and host decisions, then its files.
    pub fn remove_extension(&mut self, install_id: &str, cx: &mut Context<Self>) {
        let Some(row) = self.row_of(install_id) else { return };
        let key = if row.source == "appex" { row.path.clone() } else { row.install_id.clone() };
        let core = self.core.clone();
        let async_cx = cx.to_async();
        let done = move || {
            core.disk.delete_extension(row.profile_id, &row.install_id).ok();
            if row.source != "appex" {
                ext_package::remove_tree(&root().join(row.profile_id.to_string()).join(&row.install_id)).ok();
                let mut ext = core.extensions.borrow_mut();
                ext.unpacked.remove(&row.path);
                ext.verified.remove(&row.path);
            }
            // `done` runs at once when the extension is not loaded, inside this update.
            let profile_id = row.profile_id;
            async_cx.spawn(async move |cx| cx.update(|cx| refresh(profile_id, cx)).ok()).detach();
        };
        match self.core.profiles.borrow().get(&(false, row.profile_id)) {
            Some(rt) => rt.data.remove_extension(&key, done),
            None => done(),
        }
        cx.notify();
    }

    pub fn cancel_extension_install(&mut self, cx: &mut Context<Self>) {
        let mut ext = self.core.extensions.borrow_mut();
        ext.generation += 1;
        ext.busy = None;
        ext.choice = None;
        ext.updates.clear();
        if let Some(p) = ext.pending.take() {
            let dir = p.new_install.unwrap_or_else(|| PathBuf::from(&p.row.path));
            cx.background_executor().spawn(async move { ext_package::remove_tree(&dir) }).detach();
        }
        cx.notify();
    }

    /// One install at a time; the download, unpacking and hashing run off the main thread.
    pub fn install_extension(&mut self, what: Install, update_of: Option<String>, cx: &mut Context<Self>) {
        let Some(profile_id) = self.current_profile_id().filter(|_| self.extensions_allowed()) else { return };
        let rows = self.core.disk.extensions(profile_id).unwrap_or_default();
        let previous = update_of.and_then(|id| rows.iter().find(|r| r.install_id == id).cloned());
        let fail = |b: &mut Browser, why: String, cx: &mut Context<Browser>| {
            b.core.extensions.borrow_mut().notices.push(why);
            cx.notify();
        };
        if what.needs_network() && !self.extension_downloads() {
            return fail(self, "Extension downloads are off in Privacy settings.".into(), cx);
        }
        if previous.is_none() && rows.len() >= MAX_INSTALLS {
            return fail(self, format!("A profile holds at most {MAX_INSTALLS} extensions."), cx);
        }
        let generation = {
            let mut ext = self.core.extensions.borrow_mut();
            if ext.busy.is_some() || ext.pending.is_some() {
                return;
            }
            ext.busy = Some(if what.needs_network() { "Downloading…".into() } else { "Unpacking…".into() });
            ext.generation += 1;
            ext.generation
        };
        let install_id = previous.as_ref().map_or_else(|| uuid::Uuid::new_v4().simple().to_string(), |p| p.install_id.clone());
        let install_dir = root().join(profile_id.to_string()).join(&install_id);
        let version =
            std::fs::read_dir(&install_dir).into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok()).max().unwrap_or(0) + 1;
        let dir = install_dir.join(version.to_string());
        let new_install = previous.is_none().then(|| install_dir.clone());
        let revoked = catalog::parse_revoked(&self.core.disk.setting(REVOKED_SETTING).ok().flatten().unwrap_or_default());
        let work = cx.background_executor().spawn(async move { fetch_and_unpack(what, dir, &revoked) });
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let result = work.await;
            if this.upgrade().is_none() {
                let leftover = match result {
                    Ok(Step::Unpacked(u)) => Some(new_install.unwrap_or(u.dir)),
                    _ => new_install,
                };
                return release_install(&core, generation, leftover, cx);
            }
            this.update(cx, |b, cx| b.unpacked(profile_id, install_id, generation, result, new_install, previous, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn unpacked(
        &mut self,
        profile_id: Id,
        install_id: String,
        generation: u64,
        result: Result<Step, String>,
        new_install: Option<PathBuf>,
        previous: Option<InstalledExtension>,
        cx: &mut Context<Self>,
    ) {
        let current = self.core.extensions.borrow().generation == generation;
        let unpacked = match result {
            Ok(Step::Current) => {
                if let Some(dir) = new_install {
                    ext_package::remove_tree(&dir).ok();
                }
                if current {
                    let mut ext = self.core.extensions.borrow_mut();
                    ext.busy = None;
                    ext.notices.push(format!("{}: this extension is up to date.", previous.as_ref().map_or_else(String::new, |p| shown(&p.name))));
                    drop(ext);
                    self.next_update(cx);
                }
                return cx.notify();
            }
            Ok(Step::Choose(input, assets)) => {
                if let Some(dir) = new_install {
                    ext_package::remove_tree(&dir).ok();
                }
                if current {
                    let mut ext = self.core.extensions.borrow_mut();
                    ext.busy = None;
                    ext.choice = Some((input, assets));
                }
                return cx.notify();
            }
            Ok(Step::Unpacked(u)) if current => *u,
            Ok(Step::Unpacked(u)) => {
                ext_package::remove_tree(&new_install.unwrap_or(u.dir)).ok();
                return;
            }
            Err(why) => {
                if let Some(dir) = new_install {
                    ext_package::remove_tree(&dir).ok();
                }
                if current {
                    let mut ext = self.core.extensions.borrow_mut();
                    ext.busy = None;
                    ext.notices.push(why);
                    drop(ext);
                    self.next_update(cx);
                }
                cx.notify();
                return;
            }
        };
        let this = cx.entity().downgrade();
        let mut async_cx = cx.to_async();
        let dir = unpacked.dir.clone();
        let (core, leftover) = (self.core.clone(), new_install.clone().unwrap_or_else(|| dir.clone()));
        Extension::unpacked(&dir, &install_id.clone(), move |loaded| {
            let consent =
                this.update(&mut async_cx, |b, cx| b.loaded_for_consent(profile_id, install_id, generation, unpacked, loaded, new_install, previous, cx));
            if consent.is_err() {
                release_install(&core, generation, Some(leftover), &mut async_cx);
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn loaded_for_consent(
        &mut self,
        profile_id: Id,
        install_id: String,
        generation: u64,
        u: Unpacked,
        loaded: Result<Extension, String>,
        new_install: Option<PathBuf>,
        previous: Option<InstalledExtension>,
        cx: &mut Context<Self>,
    ) {
        if self.core.extensions.borrow().generation != generation {
            ext_package::remove_tree(new_install.as_deref().unwrap_or(&u.dir)).ok();
            return;
        }
        let discard = |why: Option<String>, b: &mut Browser, cx: &mut Context<Browser>| {
            ext_package::remove_tree(new_install.as_deref().unwrap_or(&u.dir)).ok();
            let mut ext = b.core.extensions.borrow_mut();
            ext.busy = None;
            ext.notices.extend(why);
            drop(ext);
            b.next_update(cx);
        };
        let extension = match loaded {
            Ok(e) => e,
            Err(why) => {
                discard(Some(shown(&why)), self, cx);
                return cx.notify();
            }
        };
        if let Some(entry) = &u.official {
            let ((apis, optional), (hosts, optional_hosts)) = (extension.api_permissions(), extension.host_patterns());
            let all_hosts: Vec<String> = hosts.into_iter().chain(optional_hosts).collect();
            if !catalog::covers(entry, &apis, &optional, &all_hosts) {
                discard(Some("The package asks for more than its catalog entry lists.".into()), self, cx);
                return cx.notify();
            }
        }
        if let Some(prev) = &previous {
            let refuse = if older(&extension.version(), &prev.version) {
                Some("The offered version is older than the installed one.")
            } else if extension.version() == prev.version && u.archive_sha256 == prev.archive_sha256 {
                Some("This extension is up to date.")
            } else {
                None
            };
            if let Some(why) = refuse {
                discard(Some(format!("{}: {why}", shown(&prev.name))), self, cx);
                return cx.notify();
            }
        }
        let row = InstalledExtension {
            profile_id,
            install_id,
            source: u.source,
            path: u.dir.to_string_lossy().into_owned(),
            name: extension.name(),
            version: extension.version(),
            sha256: u.tree.clone(),
            archive_sha256: u.archive_sha256,
            permissions: confirmed(&extension),
            enabled: true,
            all_sites: false,
            owner_id: u.owner_id,
            asset: u.asset,
            release_tag: u.release_tag,
        };
        let mut ext = self.core.extensions.borrow_mut();
        ext.busy = None;
        ext.pending = Some(Pending { profile_id, row, tree: u.tree, extension, new_install, previous });
        cx.notify();
    }

    /// The tree must still hash as it did before the prompt; then the row is written and the
    /// extension runs.
    pub fn confirm_extension_install(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.core.extensions.borrow().pending.as_ref().map(|p| p.row.path.clone()) else { return };
        let generation = self.core.extensions.borrow().generation;
        let hashed = cx.background_executor().spawn(async move { ext_package::tree_sha256(Path::new(&path)).ok() });
        cx.spawn(async move |this, cx| {
            let hash = hashed.await;
            this.update(cx, |b, cx| b.confirmed(generation, hash, cx)).ok();
        })
        .detach();
    }

    fn confirmed(&mut self, generation: u64, hash: Option<String>, cx: &mut Context<Self>) {
        let pending = {
            let mut ext = self.core.extensions.borrow_mut();
            if ext.generation != generation {
                return;
            }
            ext.pending.take()
        };
        let Some(p) = pending else { return };
        if hash.as_deref() != Some(p.tree.as_str()) {
            ext_package::remove_tree(p.new_install.as_deref().unwrap_or(Path::new(&p.row.path))).ok();
            self.core.extensions.borrow_mut().notices.push("The extension changed while you were deciding and was not installed.".into());
            self.next_update(cx);
            return cx.notify();
        }
        let mut row = p.row.clone();
        let hosts_before = self.core.disk.extension_hosts(p.profile_id, &row.install_id).unwrap_or_default();
        if let Some(prev) = &p.previous {
            let (hosts, optional) = p.extension.host_patterns();
            row.all_sites = prev.all_sites && !grew(prev, &hosts, &optional) && may_use_all_sites(&row.source);
            row.enabled = prev.enabled;
            let asked = hosts.iter().chain(&optional).cloned().collect::<Vec<_>>();
            self.core.disk.retain_extension_hosts(p.profile_id, &row.install_id, |host| asked.iter().any(|pattern| pattern_covers(pattern, host))).ok();
        }
        if self.core.disk.put_extension(&row).is_err() {
            ext_package::remove_tree(p.new_install.as_deref().unwrap_or(Path::new(&p.row.path))).ok();
            self.next_update(cx);
            return cx.notify();
        }
        {
            let mut ext = self.core.extensions.borrow_mut();
            ext.icons.insert(p.extension.key.clone(), icon_of(&p.extension));
            ext.unpacked.insert(row.path.clone(), p.extension.clone());
            ext.verified.insert(row.path.clone());
        }
        refresh(p.profile_id, cx);
        let loaded = self.core.extensions.borrow().running.get(&(false, p.profile_id)).is_some_and(|r| r.contains(&row.install_id));
        if let Some(prev) = p.previous.as_ref().filter(|_| row.enabled && !loaded) {
            // The new version did not start: the old one runs again and the new files go.
            self.core.disk.put_extension(prev).ok();
            for (host, allow) in &hosts_before {
                self.core.disk.set_extension_host(p.profile_id, &prev.install_id, host, *allow).ok();
            }
            ext_package::remove_tree(Path::new(&row.path)).ok();
            let mut ext = self.core.extensions.borrow_mut();
            ext.unpacked.remove(&row.path);
            ext.verified.remove(&row.path);
            ext.notices.push(format!("{} could not start after the update; the previous version runs again.", shown(&prev.name)));
            drop(ext);
            refresh(p.profile_id, cx);
        } else if let Some(prev) = &p.previous {
            self.core.extensions.borrow_mut().unpacked.remove(&prev.path);
            ext_package::remove_tree(Path::new(&prev.path)).ok();
        }
        self.next_update(cx);
        cx.notify();
    }

    /// Asks the network only on this click: the catalog for Official installs, GitHub per repository.
    pub fn check_extension_updates(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.current_profile_id() else { return };
        if !self.extension_downloads() {
            return;
        }
        let rows = self.core.disk.extensions(id).unwrap_or_default();
        let wants_catalog = catalog_configured() && rows.iter().any(|r| r.source.starts_with("official:"));
        if wants_catalog && !std::mem::take(&mut self.core.extensions.borrow_mut().check_after_catalog) {
            self.core.extensions.borrow_mut().check_after_catalog = true;
            return self.fetch_catalog(cx);
        }
        let catalog = self.core.extensions.borrow().catalog.clone();
        let updates: Vec<(Install, String)> = rows
            .into_iter()
            .filter_map(|row| {
                if let Some((repo_id, _)) = ext_sources::github_source(&row.source) {
                    return Some((Install::GithubUpdate { repo_id, tag: row.release_tag, asset: row.asset }, row.install_id));
                }
                let entry = row.source.strip_prefix("official:").and_then(|cid| catalog.as_ref()?.extensions.iter().find(|e| e.id == cid))?;
                older(&row.version, &entry.version).then(|| (Install::Official(entry.clone()), row.install_id))
            })
            .collect();
        if updates.is_empty() {
            self.core.extensions.borrow_mut().notices.push("Every extension is up to date.".into());
            return cx.notify();
        }
        self.core.extensions.borrow_mut().updates = updates;
        self.next_update(cx);
    }

    /// Checks the next queued update once nothing else is being installed or confirmed.
    fn next_update(&mut self, cx: &mut Context<Self>) {
        let next = {
            let mut ext = self.core.extensions.borrow_mut();
            (ext.busy.is_none() && ext.pending.is_none() && !ext.updates.is_empty()).then(|| ext.updates.remove(0))
        };
        if let Some((what, install_id)) = next {
            self.install_extension(what, Some(install_id), cx);
        }
    }

    /// The index read at launch or fetched since; never fetched here.
    pub fn catalog_rows(&self) -> (Vec<CatalogRow>, bool) {
        let Some(index) = self.core.extensions.borrow().catalog.clone() else { return (Vec::new(), false) };
        let installed: Vec<String> =
            self.current_profile_id().map(|id| self.core.disk.extensions(id).unwrap_or_default()).unwrap_or_default().into_iter().map(|r| r.source).collect();
        let rows = index
            .extensions
            .iter()
            .map(|e| CatalogRow {
                id: e.id.clone(),
                name: shown(&e.name),
                description: shown(&e.description),
                version: shown(&e.version),
                installed: installed.contains(&format!("official:{}", e.id)),
            })
            .collect();
        (rows, catalog::is_old(&index, now()))
    }

    pub fn fetch_catalog(&mut self, cx: &mut Context<Self>) {
        if !catalog_configured() || !self.extension_downloads() || self.core.extensions.borrow().busy.is_some() {
            self.core.extensions.borrow_mut().check_after_catalog = false;
            return;
        }
        self.core.extensions.borrow_mut().busy = Some(LOADING_CATALOG.into());
        let last = self.core.disk.setting(CATALOG_SEEN_SETTING).ok().flatten().and_then(|v| v.parse().ok());
        let work = cx.background_executor().spawn(async move {
            let (index, sig) = ext_sources::catalog(&Net::new())?;
            let parsed = catalog::verify(&index, &sig, &catalog_key()).map_err(|e| e.to_string())?;
            catalog::check_fetched(&parsed, last, now()).map_err(|e| e.to_string())?;
            let dir = crate::browser::data_dir().join("catalog");
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(dir.join("index.json"), &index))
                .and_then(|()| std::fs::write(dir.join("index.sig"), &sig))
                .map_err(|e| e.to_string())?;
            Ok::<_, String>(parsed)
        });
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let released = this.update(cx, |b, cx| {
                let mut ext = b.core.extensions.borrow_mut();
                if ext.busy.as_deref() == Some(LOADING_CATALOG) {
                    ext.busy = None;
                }
                drop(ext);
                match result {
                    Ok(index) => {
                        b.core.disk.set_setting(CATALOG_SEEN_SETTING, &index.version.to_string()).ok();
                        merge_revoked(&b.core, &index);
                        b.core.extensions.borrow_mut().catalog = Some(index);
                        crate::extensions::verify_revoked(b, cx);
                        if b.core.extensions.borrow().check_after_catalog {
                            b.check_extension_updates(cx);
                        }
                    }
                    Err(why) => {
                        let mut ext = b.core.extensions.borrow_mut();
                        ext.check_after_catalog = false;
                        ext.notices.push(why);
                    }
                }
                cx.notify();
            });
            if released.is_err() {
                let mut ext = core.extensions.borrow_mut();
                if ext.busy.as_deref() == Some(LOADING_CATALOG) {
                    ext.busy = None;
                    ext.check_after_catalog = false;
                }
                drop(ext);
                cx.update(|cx| cx.refresh_windows()).ok();
            }
        })
        .detach();
        cx.notify();
    }

    pub fn install_official(&mut self, id: &str, cx: &mut Context<Self>) {
        let fresh = self.core.extensions.borrow().catalog.as_ref().is_some_and(|c| !catalog::is_old(c, now()));
        if !fresh {
            self.core.extensions.borrow_mut().notices.push("Show official extensions again first: the list is more than 30 days old.".into());
            return cx.notify();
        }
        let entry = self.core.extensions.borrow().catalog.as_ref().and_then(|c| c.extensions.iter().find(|e| e.id == id).cloned());
        if let Some(entry) = entry {
            self.install_extension(Install::Official(entry), None, cx);
        }
    }

    pub fn choose_extension_asset(&mut self, ix: usize, cx: &mut Context<Self>) {
        let choice = self.core.extensions.borrow_mut().choice.take();
        if let Some((input, assets)) = choice
            && let Some(asset) = assets.get(ix)
        {
            self.install_extension(Install::GithubAsset(input, asset.url.clone()), None, cx);
        }
        cx.notify();
    }

    pub fn dismiss_extension_notices(&mut self, cx: &mut Context<Self>) {
        self.core.extensions.borrow_mut().notices.clear();
        cx.notify();
    }

    fn open_extension(&mut self, key: &str) {
        let Some(space) = self.model.current_space() else { return };
        let ext = self.core.extensions.borrow();
        let extension = ext.apps.iter().chain(ext.unpacked.values()).find(|e| e.key == key);
        if let (Some(extension), Some(rt)) = (extension, self.core.profiles.borrow().get(&(space.private, space.space.profile_id))) {
            rt.data.open_extension_popup(extension);
        }
    }
}

/// A newly read revocation turns matching official installs off at once.
fn verify_revoked(b: &mut Browser, cx: &mut Context<Browser>) {
    let mut ext = b.core.extensions.borrow_mut();
    ext.verified.clear();
    drop(ext);
    crate::extensions::verify(cx);
}

/// One toolbar button per extension running in the current profile; each opens its popup.
pub fn buttons(b: &Browser, cx: &mut Context<Browser>) -> impl IntoElement {
    let extensions = b.core.extensions.borrow();
    let running = b.model.current_space().and_then(|s| extensions.running.get(&(s.private, s.space.profile_id))).cloned().unwrap_or_default();
    let seed =
        b.model.current_space().and_then(|s| b.core.profiles.borrow().get(&(s.private, s.space.profile_id)).map(|rt| rt.own_store.clone())).unwrap_or_default();
    let shown_buttons: Vec<(usize, String, String, Option<Arc<Image>>)> = running
        .iter()
        .enumerate()
        .filter_map(|(ix, key)| {
            let e = extensions.apps.iter().chain(extensions.unpacked.values()).find(|e| &e.key == key)?;
            Some((ix, key.clone(), shown(&e.name()), extensions.icons.get(key).cloned().flatten()))
        })
        .collect();
    drop(extensions);
    div().flex().items_center().gap(px(2.)).children(shown_buttons.into_iter().map(|(ix, key, name, icon)| {
        let denied = engine::extensions::denied_request(&seed, &key);
        let seed = seed.clone();
        let name = if denied { format!("{name} \u{2014} a request was denied") } else { name };
        let button = match icon {
            Some(icon) => div()
                .id(("extension", ix))
                .w(px(28.))
                .h(px(28.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_row()))
                .cursor_pointer()
                .hover(|s| s.bg(Theme::hover()))
                .active(|s| s.bg(Theme::pressed()))
                .child(img(icon).w(px(16.)).h(px(16.))),
            None => icon_button(("extension", ix), Icon::Braces, true),
        };
        button
            .relative()
            .when(denied, |d| d.child(div().absolute().top(px(3.)).right(px(3.)).w(px(6.)).h(px(6.)).rounded_full().bg(Theme::warning())))
            .when(b.extension_clicked != Some(ix), |d| d.tooltip(tooltip(name, None)))
            .on_hover(cx.listener(move |b, hovered: &bool, _, cx| {
                if !*hovered && b.extension_clicked == Some(ix) {
                    b.extension_clicked = None;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |b, _, _, cx| {
                b.extension_clicked = Some(ix);
                engine::extensions::clear_denied_request(&seed, &key);
                b.open_extension(&key);
                cx.notify();
            }))
    }))
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
    }

    #[test]
    fn an_update_takes_the_build_the_user_installed() {
        let asset = |name: &str| ext_sources::Asset { name: name.into(), url: format!("https://github.com/a/b/releases/download/v2/{name}") };
        let release = |assets: Vec<ext_sources::Asset>| ext_sources::Resolved { repo_id: 1, owner_id: 1, full_name: "a/b".into(), tag: "v2".into(), assets };
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

    #[test]
    fn extension_text_is_bounded_and_inert() {
        let name = format!("<b>{}</b>\u{202E}gnp.exe\n", "x".repeat(10_000));
        let out = shown(&name);
        assert!(out.chars().count() < 200);
        assert!(!out.chars().any(|c| c.is_control() || ui::text::is_bidi(c)));
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
