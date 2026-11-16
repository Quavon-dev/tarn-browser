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
use shield::catalog::{self, Index};
use store::{Id, InstalledExtension};
use ui::controls::{icon_button, tooltip};
use ui::icon::Icon;
use ui::theme::Theme;

use crate::browser::Browser;
use crate::ext_package;
use crate::ext_sources::{self, Net};
use crate::prefs::Pref;
use crate::shared::AppCore;
pub use browser_core::extensions::Install;
pub use browser_core::extensions::install::UNREVIEWED;
use browser_core::extensions::install::{self, CATALOG_SEEN_SETTING, Slot};
use browser_core::extensions::{Step, Unpacked, badge, every_site, fetch_and_unpack, grew, may_use_all_sites, png_icon};

pub use browser_core::theme_fetch::DOWNLOADS_SETTING;
const NOTICE_SETTING: &str = "extension_all_sites_notice";
const LOADING_CATALOG: &str = "Loading the official catalog…";

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

pub use browser_core::theme_fetch::shown;

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
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

/// A package awaiting consent: a full extension WebKit runs, or one Tarn's own runtime runs.
enum Candidate {
    Classic(Extension),
    Tarn(Box<browser_core::tarn_ext::Package>),
}

impl Candidate {
    fn name(&self) -> String {
        match self {
            Candidate::Classic(e) => e.name(),
            Candidate::Tarn(p) => p.name.clone(),
        }
    }

    fn version(&self) -> String {
        match self {
            Candidate::Classic(e) => e.version(),
            Candidate::Tarn(p) => p.manifest.version.clone(),
        }
    }

    fn api_permissions(&self) -> (Vec<String>, Vec<String>) {
        match self {
            Candidate::Classic(e) => e.api_permissions(),
            Candidate::Tarn(p) => (p.manifest.consent().0, Vec::new()),
        }
    }

    fn host_patterns(&self) -> (Vec<String>, Vec<String>) {
        match self {
            Candidate::Classic(e) => e.host_patterns(),
            Candidate::Tarn(p) => (p.manifest.consent().1, Vec::new()),
        }
    }

    fn has_active_tab(&self) -> bool {
        match self {
            Candidate::Classic(e) => e.has_active_tab(),
            Candidate::Tarn(p) => p.manifest.active_tab,
        }
    }
}

struct Pending {
    slot: Slot,
    row: InstalledExtension,
    extension: Candidate,
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
    pub(crate) fn mark_verified(&mut self, path: &str) {
        self.verified.insert(path.to_owned());
    }

    pub(crate) fn forget_running(&mut self, private: bool, profile_id: Id) {
        self.running.remove(&(private, profile_id));
    }

    pub fn consent(&self) -> Option<Consent> {
        let p = self.pending.as_ref()?;
        let ((apis, optional_apis), (hosts, optional_hosts)) = (p.extension.api_permissions(), p.extension.host_patterns());
        let mut changes = Vec::new();
        if let Some(prev) = &p.slot.previous {
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
    permission_lines(extension.api_permissions(), extension.host_patterns())
}

fn permission_lines((apis, optional_apis): (Vec<String>, Vec<String>), (hosts, optional_hosts): (Vec<String>, Vec<String>)) -> String {
    apis.into_iter().chain(optional_apis).chain(hosts).chain(optional_hosts).collect::<Vec<_>>().join("\n")
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
        install::merge_revoked(&core.disk, index);
    }
    if let Ok(paths) = core.disk.extension_paths() {
        let keep: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        cx.background_executor().spawn(async move { ext_package::sweep(&root(), &keep) }).detach();
    }
    verify(cx);
}

fn cached_catalog() -> Option<Index> {
    install::cached_catalog(&crate::browser::data_dir().join("catalog"), &catalog_key())
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
    let revoked = install::revoked(&core.disk);
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

pub(crate) fn turn_off_row(core: &AppCore, profile_id: Id, row: &InstalledExtension, why: &str) {
    turn_off(core, profile_id, row, why);
}

fn turn_off(core: &AppCore, profile_id: Id, row: &InstalledExtension, why: &str) {
    browser_core::extensions::turn_off(&core.disk, row);
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
            if browser_core::tarn_ext::is_tarn_dir(Path::new(&row.path)) {
                return crate::tarn_ext::load(profile_id, row, cx);
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
pub(crate) fn refresh(profile_id: Id, cx: &mut App) {
    let core = crate::shared::core(cx);
    let changed = core.apply_extensions(false, profile_id);
    cx.defer(move |cx| {
        crate::tarn_ext::reconcile(profile_id, cx);
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

/// The window that started an install is gone: removes what it unpacked, frees the shared busy
/// state unless a newer install holds it, and lets another window carry on with queued updates.
fn release_install(core: &crate::shared::AppCore, generation: u64, leftover: Option<PathBuf>, cx: &mut gpui::AsyncApp) {
    if let Some(dir) = leftover {
        ext_package::remove_tree(&dir).ok();
    }
    let mut ext = core.extensions.borrow_mut();
    if ext.generation == generation {
        ext.busy = None;
    }
    drop(ext);
    cx.update(|cx| {
        let mut resumed = false;
        crate::shared::each(cx, |b, _, cx| {
            if !std::mem::replace(&mut resumed, true) {
                b.next_update(cx);
            }
        });
        cx.refresh_windows();
    })
    .ok();
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
        let tarn = self.core.tarn.borrow();
        let mut out: Vec<Row> = rows
            .iter()
            .map(|r| {
                let loaded = if r.source == "appex" { ext.apps.iter().find(|a| a.path.to_string_lossy() == r.path) } else { ext.unpacked.get(&r.path) };
                Row {
                    install_id: Some(r.install_id.clone()),
                    app_path: None,
                    name: shown(
                        &loaded
                            .map(|e| format!("{} {}", e.name(), e.version()))
                            .or_else(|| tarn.loaded(id, &r.install_id).map(|(n, v)| format!("{n} {v}")))
                            .unwrap_or_else(|| r.name.clone()),
                    ),
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
        cx.defer(move |cx| crate::tarn_ext::reconcile(profile_id, cx));
        self.restart_profile_views(false, profile_id, None, false, window, cx);
        cx.notify();
    }

    /// Deletes the install's storage first, then its rows and host decisions, then its files.
    pub fn remove_extension(&mut self, install_id: &str, cx: &mut Context<Self>) {
        let Some(row) = self.row_of(install_id) else { return };
        let key = if row.source == "appex" { row.path.clone() } else { row.install_id.clone() };
        let tarn = row.source != "appex" && browser_core::tarn_ext::is_tarn_dir(Path::new(&row.path));
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
        match self.core.profiles.borrow().get(&(false, row.profile_id)).filter(|_| !tarn) {
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
            cx.background_executor().spawn(async move { p.slot.discard() }).detach();
        }
        cx.notify();
    }

    /// One install at a time; the download, unpacking and hashing run off the main thread.
    pub fn install_extension(&mut self, what: Install, update_of: Option<String>, cx: &mut Context<Self>) {
        let Some(profile_id) = self.current_profile_id().filter(|_| self.extensions_allowed()) else { return };
        let fail = |b: &mut Browser, why: String, cx: &mut Context<Browser>| {
            b.core.extensions.borrow_mut().notices.push(why);
            cx.notify();
        };
        if what.needs_network() && !self.extension_downloads() {
            return fail(self, "Extension downloads are off in Privacy settings.".into(), cx);
        }
        let slot = match install::slot(&self.core.disk, profile_id, &root(), update_of.as_deref(), || uuid::Uuid::new_v4().simple().to_string()) {
            Ok(slot) => slot,
            Err(why) => return fail(self, why, cx),
        };
        let generation = {
            let mut ext = self.core.extensions.borrow_mut();
            if ext.busy.is_some() || ext.pending.is_some() {
                return;
            }
            ext.busy = Some(if what.needs_network() { "Downloading…".into() } else { "Unpacking…".into() });
            ext.generation += 1;
            ext.generation
        };
        let (revoked, dir) = (install::revoked(&self.core.disk), slot.dir.clone());
        let work = cx.background_executor().spawn(async move { fetch_and_unpack(&Net::new(), what, dir, &revoked) });
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let result = work.await;
            if this.upgrade().is_none() {
                let leftover = match result {
                    Ok(Step::Unpacked(u)) => Some(slot.new_install.unwrap_or(u.dir)),
                    _ => slot.new_install,
                };
                return release_install(&core, generation, leftover, cx);
            }
            this.update(cx, |b, cx| b.unpacked(slot, generation, result, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn unpacked(&mut self, slot: Slot, generation: u64, result: Result<Step, String>, cx: &mut Context<Self>) {
        let current = self.core.extensions.borrow().generation == generation;
        let unpacked = match result {
            Ok(Step::Current) => {
                if let Some(dir) = &slot.new_install {
                    ext_package::remove_tree(dir).ok();
                }
                if current {
                    let mut ext = self.core.extensions.borrow_mut();
                    ext.busy = None;
                    ext.notices.push(format!("{}: this extension is up to date.", slot.previous.as_ref().map_or_else(String::new, |p| shown(&p.name))));
                    drop(ext);
                    self.next_update(cx);
                }
                return cx.notify();
            }
            Ok(Step::Choose(input, assets)) => {
                if let Some(dir) = &slot.new_install {
                    ext_package::remove_tree(dir).ok();
                }
                if current {
                    let mut ext = self.core.extensions.borrow_mut();
                    ext.busy = None;
                    ext.choice = Some((input, assets));
                }
                return cx.notify();
            }
            Ok(Step::Unpacked(u)) if current => *u,
            Ok(Step::Unpacked(_)) => {
                slot.discard();
                return;
            }
            Err(why) => {
                if let Some(dir) = &slot.new_install {
                    ext_package::remove_tree(dir).ok();
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
        let (core, leftover) = (self.core.clone(), slot.new_install.clone().unwrap_or_else(|| dir.clone()));
        if browser_core::tarn_ext::is_tarn_dir(&dir) {
            let seed = self.core.disk.profile(slot.profile_id).map(|p| p.data_store_uuid).unwrap_or_default();
            let (id, locales) = (slot.install_id.clone(), crate::tarn_ext::ui_locales());
            let work = cx.background_executor().spawn(async move {
                browser_core::tarn_ext::load(&dir, "", &id, &seed, &locales).map(Box::new).map_err(|e| match e {
                    browser_core::tarn_ext::LoadError::Refused(why) => why,
                    browser_core::tarn_ext::LoadError::Changed => "The extension changed while it was read.".to_owned(),
                })
            });
            cx.spawn(async move |this, cx| {
                let loaded = work.await.map(Candidate::Tarn);
                let consent = this.update(cx, |b, cx| b.loaded_for_consent(slot, generation, unpacked, loaded, cx));
                if consent.is_err() {
                    release_install(&core, generation, Some(leftover), cx);
                }
            })
            .detach();
            return;
        }
        Extension::unpacked(&dir, &slot.install_id.clone(), move |loaded| {
            let loaded = loaded.map(Candidate::Classic);
            let consent = this.update(&mut async_cx, |b, cx| b.loaded_for_consent(slot, generation, unpacked, loaded, cx));
            if consent.is_err() {
                release_install(&core, generation, Some(leftover), &mut async_cx);
            }
        });
    }

    fn loaded_for_consent(&mut self, slot: Slot, generation: u64, u: Unpacked, loaded: Result<Candidate, String>, cx: &mut Context<Self>) {
        if self.core.extensions.borrow().generation != generation {
            slot.discard();
            return;
        }
        let discard = |why: Option<String>, b: &mut Browser, cx: &mut Context<Browser>| {
            slot.discard();
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
        let ((apis, optional), (hosts, optional_hosts)) = (extension.api_permissions(), extension.host_patterns());
        let all_hosts: Vec<String> = hosts.into_iter().chain(optional_hosts).collect();
        if let Err(why) = install::vet(&u, slot.previous.as_ref(), &extension.version(), &apis, &optional, &all_hosts) {
            discard(Some(why), self, cx);
            return cx.notify();
        }
        let permissions = permission_lines(extension.api_permissions(), extension.host_patterns());
        let row = u.row(slot.profile_id, &slot.install_id, extension.name(), extension.version(), permissions);
        let mut ext = self.core.extensions.borrow_mut();
        ext.busy = None;
        ext.pending = Some(Pending { slot, row, extension });
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
        if hash.as_deref() != Some(p.row.sha256.as_str()) {
            p.slot.discard();
            self.core.extensions.borrow_mut().notices.push("The extension changed while you were deciding and was not installed.".into());
            self.next_update(cx);
            return cx.notify();
        }
        let profile_id = p.slot.profile_id;
        let hosts_before = self.core.disk.extension_hosts(profile_id, &p.row.install_id).unwrap_or_default();
        let row = match &p.slot.previous {
            Some(prev) => {
                let (hosts, optional) = p.extension.host_patterns();
                install::carry_over(&self.core.disk, p.row.clone(), prev, &hosts, &optional)
            }
            None => p.row.clone(),
        };
        if self.core.disk.put_extension(&row).is_err() {
            p.slot.discard();
            self.next_update(cx);
            return cx.notify();
        }
        let tarn = match p.extension {
            Candidate::Classic(extension) => {
                let mut ext = self.core.extensions.borrow_mut();
                ext.icons.insert(extension.key.clone(), icon_of(&extension));
                ext.unpacked.insert(row.path.clone(), extension);
                ext.verified.insert(row.path.clone());
                false
            }
            Candidate::Tarn(package) => {
                if row.enabled {
                    crate::tarn_ext::add(&self.core, row.clone(), *package);
                }
                self.core.extensions.borrow_mut().verified.insert(row.path.clone());
                true
            }
        };
        refresh(profile_id, cx);
        let loaded = tarn || self.core.extensions.borrow().running.get(&(false, profile_id)).is_some_and(|r| r.contains(&row.install_id));
        if let Some(prev) = p.slot.previous.as_ref().filter(|_| row.enabled && !loaded) {
            // The new version did not start: the old one runs again and the new files go.
            self.core.disk.put_extension(prev).ok();
            for (host, allow) in &hosts_before {
                self.core.disk.set_extension_host(profile_id, &prev.install_id, host, *allow).ok();
            }
            ext_package::remove_tree(Path::new(&row.path)).ok();
            let mut ext = self.core.extensions.borrow_mut();
            ext.unpacked.remove(&row.path);
            ext.verified.remove(&row.path);
            ext.notices.push(format!("{} could not start after the update; the previous version runs again.", shown(&prev.name)));
            drop(ext);
            refresh(profile_id, cx);
        } else if let Some(prev) = &p.slot.previous {
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
        let updates: Vec<(Install, String)> = rows.into_iter().filter_map(|row| Some((install::update_of(&row, catalog.as_ref())?, row.install_id))).collect();
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
        let dir = crate::browser::data_dir().join("catalog");
        let work = cx.background_executor().spawn(async move { install::fetch_catalog(&Net::new(), &dir, last, now(), &catalog_key()) });
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
                        install::catalog_fetched(&b.core.disk, &index);
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
    let tarn = crate::tarn_ext::buttons(b, cx);
    div().flex().items_center().gap(px(2.)).children(tarn).children(shown_buttons.into_iter().map(|(ix, key, name, icon)| {
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
    fn a_tarn_package_asks_consent_for_what_its_manifest_names() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "manifest_version": 3, "name": "Helper", "version": "1.2",
            "browser_specific_settings": {"tarn": {"runtime": 1}},
            "permissions": ["storage", "activeTab"],
            "host_permissions": ["*://*.example.com/*"],
        });
        std::fs::write(dir.path().join("manifest.json"), manifest.to_string()).unwrap();
        assert!(browser_core::tarn_ext::is_tarn_dir(dir.path()));
        let candidate = Candidate::Tarn(Box::new(browser_core::tarn_ext::load(dir.path(), "", "aa", "SEED", &[]).unwrap()));
        assert_eq!((candidate.name(), candidate.version(), candidate.has_active_tab()), ("Helper".to_owned(), "1.2".to_owned(), true));
        assert_eq!(candidate.api_permissions(), (vec!["storage".to_owned(), "activeTab".to_owned()], Vec::new()));
        assert_eq!(permission_lines(candidate.api_permissions(), candidate.host_patterns()), "storage\nactiveTab\n*://*.example.com/*");
    }

    #[test]
    fn extension_text_is_bounded_and_inert() {
        let name = format!("<b>{}</b>\u{202E}gnp.exe\n", "x".repeat(10_000));
        let out = shown(&name);
        assert!(out.chars().count() < 200);
        assert!(!out.chars().any(|c| c.is_control() || ui::text::is_bidi(c)));
    }
}
