//! Installing, updating and removing extensions, the same on every client: where an install
//! unpacks, the checks before consent, what an update keeps, and the signed catalog. The transport
//! is the caller's `Fetch`; the decisions about what a Tarn extension may do stay in `tarn_ext`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use shield::catalog::{self, Entry, Index};
use store::{Id, InstalledExtension, Store};

use super::{Install, Step, Unpacked, badge, fetch_and_unpack, grew, may_use_all_sites, older, pattern_covers};
use crate::ext_package;
use crate::github::{Asset, Fetch, github_source};
use crate::tarn_ext::{self, LoadError};
use crate::theme_fetch::shown;

pub const MAX_INSTALLS: usize = 32;
pub const REVOKED_SETTING: &str = "extension_revoked";
pub const CATALOG_SEEN_SETTING: &str = "extension_catalog_version";
pub const UNREVIEWED: &str = "Nobody reviewed this extension's code. It can read and change the sites you allow and send what it reads anywhere.";
pub const STORE_LINK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);
pub const STORE_CONFIRM_DELAY: std::time::Duration = std::time::Duration::from_millis(700);
pub const STORE_REFUSED: &str = "Extensions are not installed in private spaces, burner tabs or Lockdown.";
pub const STORE_NO_CATALOG: &str = "The official catalog is not available in this build.";
pub const STORE_DOWNLOADS_OFF: &str = "Extension downloads are off in Privacy settings.";
pub const STORE_NEEDS_CATALOG: &str =
    "Tarn has no current official catalog. Load it with Show Official Extensions in Settings › Extensions, then open the link again.";
pub const STORE_BUSY: &str = "An extension install is already in progress.";
pub const STORE_CHANGED: &str = "The official catalog changed since you opened the link; open it again.";
pub const STORE_FETCH_NOTE: &str = "Tarn downloads it from GitHub only when you press Install.";
pub const STORE_UNKNOWN: &str = "This extension is not in Tarn's official catalog.";
pub const STORE_WITHDRAWN: &str = "This extension was withdrawn from the official catalog.";
pub const NOT_TARN: &str =
    "This is not a Tarn extension. Only extensions whose manifest sets browser_specific_settings.tarn.runtime to 1 run in this version of Tarn.";

/// Where one install or update unpacks.
#[derive(Clone, Debug)]
pub struct Slot {
    pub profile_id: Id,
    pub install_id: String,
    pub dir: PathBuf,
    /// The install's own directory, removed whole if a new install is not confirmed.
    pub new_install: Option<PathBuf>,
    pub previous: Option<InstalledExtension>,
}

impl Slot {
    fn leftover(&self) -> &Path {
        self.new_install.as_deref().unwrap_or(&self.dir)
    }

    pub fn discard(&self) {
        ext_package::remove_tree(self.leftover()).ok();
    }
}

pub fn catalog_key() -> [u8; 32] {
    catalog::EXTENSION_CATALOG_PUBLIC_KEY
}

pub fn catalog_configured() -> bool {
    catalog_key() != [0; 32]
}

pub fn revoked(store: &Store) -> BTreeSet<(String, String)> {
    catalog::parse_revoked(&store.setting(REVOKED_SETTING).ok().flatten().unwrap_or_default())
}

pub fn install_dir(root: &Path, profile_id: Id, install_id: &str) -> PathBuf {
    root.join(profile_id.to_string()).join(install_id)
}

/// Where an install's files are now: the app's data folder can move (iOS moves its container on
/// every update), so only the version folder's name is taken from the stored path.
pub fn installed_dir(root: &Path, row: &InstalledExtension) -> PathBuf {
    let version = Path::new(&row.path).file_name().map(PathBuf::from).unwrap_or_default();
    install_dir(root, row.profile_id, &row.install_id).join(version)
}

/// A new numbered directory for the install, or for an update of `update_of`.
pub fn slot(store: &Store, profile_id: Id, root: &Path, update_of: Option<&str>, new_id: impl FnOnce() -> String) -> Result<Slot, String> {
    let rows = store.extensions(profile_id).map_err(|e| e.to_string())?;
    let previous = update_of.and_then(|id| rows.iter().find(|r| r.install_id == id).cloned());
    if previous.is_none() && rows.len() >= MAX_INSTALLS {
        return Err(format!("A profile holds at most {MAX_INSTALLS} extensions."));
    }
    let install_id = previous.as_ref().map_or_else(new_id, |p| p.install_id.clone());
    let home = install_dir(root, profile_id, &install_id);
    let version = std::fs::read_dir(&home).into_iter().flatten().flatten().filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok()).max().unwrap_or(0) + 1;
    let dir = home.join(version.to_string());
    Ok(Slot { profile_id, install_id, dir, new_install: previous.is_none().then_some(home), previous })
}

/// What the consent sheet shows before anything is stored.
#[derive(Clone, Debug)]
pub struct Offer {
    pub unpacked: Box<Unpacked>,
    pub name: String,
    pub version: String,
    pub description: String,
    pub apis: Vec<String>,
    pub sites: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum Fetched {
    Offer(Offer),
    Choose(String, Vec<Asset>),
    Current,
}

/// Downloads or reads the package, unpacks it into the slot and loads it as a Tarn extension.
/// Blocks; run it off the main thread. Nothing is left on disk when it fails.
pub fn fetch(net: &dyn Fetch, what: Install, slot: &Slot, revoked: &BTreeSet<(String, String)>, seed: &str, locales: &[String]) -> Result<Fetched, String> {
    let step = fetch_and_unpack(net, what, slot.dir.clone(), revoked);
    let unpacked = match step {
        Ok(Step::Unpacked(u)) => *u,
        Ok(Step::Choose(input, assets)) => {
            slot.discard();
            return Ok(Fetched::Choose(input, assets));
        }
        Ok(Step::Current) => {
            slot.discard();
            return Ok(Fetched::Current);
        }
        Err(why) => {
            slot.discard();
            return Err(shown(&why));
        }
    };
    let refuse = |why: String| {
        slot.discard();
        Err(shown(&why))
    };
    if !tarn_ext::is_tarn_dir(&unpacked.dir) {
        slot.discard();
        return Err(NOT_TARN.to_owned());
    }
    let package = match tarn_ext::load(&unpacked.dir, Some(&unpacked.tree), &slot.install_id, seed, locales) {
        Ok(p) => p,
        Err(LoadError::Changed) => return refuse("The extension changed while it was unpacked.".to_owned()),
        Err(LoadError::Refused(why)) => return refuse(why),
    };
    let (apis, sites) = package.manifest.consent();
    Ok(Fetched::Offer(Offer {
        unpacked: Box::new(unpacked),
        name: shown(&package.name),
        version: shown(&package.manifest.version),
        description: shown(&package.description),
        apis,
        sites,
    }))
}

/// Waiting for the user's consent.
#[derive(Clone, Debug)]
pub struct Pending {
    pub slot: Slot,
    pub row: InstalledExtension,
    pub description: String,
    pub apis: Vec<String>,
    pub sites: Vec<String>,
}

impl Pending {
    pub fn title(&self) -> String {
        let verb = if self.slot.previous.is_some() { "Update" } else { "Install" };
        format!("{verb} \u{201C}{}\u{201D} {}?", self.row.name, self.row.version)
    }

    pub fn badge(&self) -> String {
        badge(&self.row.source)
    }

    pub fn unverified(&self) -> bool {
        !may_use_all_sites(&self.row.source)
    }

    /// The lines of the consent sheet: what it may use and where.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.description.is_empty() {
            out.push(self.description.clone());
        }
        out.push(format!("Source: {}", self.badge()));
        if !self.apis.is_empty() {
            out.push(format!("Uses: {}", self.apis.join(", ")));
        }
        out.push(if self.sites.is_empty() { "Asks for no sites.".to_owned() } else { format!("Wants to read and change: {}", self.sites.join(", ")) });
        out.push(if self.unverified() {
            "It asks you on every site before it runs there.".to_owned()
        } else {
            "It asks you on every site before it runs there, unless you let it use every site it lists.".to_owned()
        });
        if self.unverified() {
            out.push(UNREVIEWED.to_owned());
        }
        out
    }
}

/// Refuses a package that asks for more than its catalog entry lists, or that is not newer than
/// the installed version.
pub fn vet(
    u: &Unpacked,
    previous: Option<&InstalledExtension>,
    version: &str,
    apis: &[String],
    optional_apis: &[String],
    hosts: &[String],
) -> Result<(), String> {
    if let Some(entry) = &u.official
        && !catalog::covers(entry, apis, optional_apis, hosts)
    {
        return Err("The package asks for more than its catalog entry lists.".to_owned());
    }
    let Some(prev) = previous else { return Ok(()) };
    if older(version, &prev.version) {
        return Err(format!("{}: The offered version is older than the installed one.", shown(&prev.name)));
    }
    if version == prev.version && u.archive_sha256 == prev.archive_sha256 {
        return Err(format!("{}: This extension is up to date.", shown(&prev.name)));
    }
    Ok(())
}

impl Unpacked {
    /// The row stored once the user agrees, turned on and asking on every site.
    pub fn row(&self, profile_id: Id, install_id: &str, name: String, version: String, permissions: String) -> InstalledExtension {
        InstalledExtension {
            profile_id,
            install_id: install_id.to_owned(),
            source: self.source.clone(),
            path: self.dir.to_string_lossy().into_owned(),
            name,
            version,
            sha256: self.tree.clone(),
            archive_sha256: self.archive_sha256.clone(),
            permissions,
            enabled: true,
            all_sites: false,
            owner_id: self.owner_id,
            asset: self.asset.clone(),
            release_tag: self.release_tag.clone(),
        }
    }
}

/// An update keeps its on switch, keeps "every site" unless it asks for more sites, and keeps
/// only the site answers its patterns still cover.
pub fn carry_over(store: &Store, row: InstalledExtension, prev: &InstalledExtension, hosts: &[String], optional_hosts: &[String]) -> InstalledExtension {
    let all_sites = prev.all_sites && !grew(prev, hosts, optional_hosts) && may_use_all_sites(&row.source);
    let asked: Vec<&String> = hosts.iter().chain(optional_hosts).collect();
    store.retain_extension_hosts(row.profile_id, &row.install_id, |host| asked.iter().any(|pattern| pattern_covers(pattern, host))).ok();
    InstalledExtension { all_sites, enabled: prev.enabled, ..row }
}

/// Checks an offer against its catalog entry and the installed version.
pub fn pending(slot: Slot, offer: Offer) -> Result<Pending, String> {
    if let Err(why) = vet(&offer.unpacked, slot.previous.as_ref(), &offer.version, &offer.apis, &[], &offer.sites) {
        slot.discard();
        return Err(why);
    }
    let permissions = offer.apis.iter().chain(&offer.sites).cloned().collect::<Vec<_>>().join("\n");
    let row = offer.unpacked.row(slot.profile_id, &slot.install_id, offer.name, offer.version, permissions);
    Ok(Pending { slot, row, description: offer.description, apis: offer.apis, sites: offer.sites })
}

/// The tree must still hash as it did when it was offered; then the row is stored, an update keeps
/// the switches and the site answers still covered, and the old version's files go.
pub fn confirm(store: &Store, p: &Pending) -> Result<(), String> {
    if ext_package::tree_sha256(&p.slot.dir).ok().as_deref() != Some(p.row.sha256.as_str()) {
        p.slot.discard();
        return Err("The extension changed while you were deciding and was not installed.".to_owned());
    }
    let row = match &p.slot.previous {
        Some(prev) => carry_over(store, p.row.clone(), prev, &p.sites, &[]),
        None => p.row.clone(),
    };
    if let Err(e) = store.put_extension(&row) {
        p.slot.discard();
        return Err(e.to_string());
    }
    if let Some(prev) = &p.slot.previous
        && let Some(root) = p.slot.dir.parent().and_then(Path::parent).and_then(Path::parent)
    {
        ext_package::remove_tree(&installed_dir(root, prev)).ok();
    }
    Ok(())
}

/// Removes the install, its site answers, its storage and its files.
pub fn remove(store: &Store, root: &Path, row: &InstalledExtension) -> Result<(), String> {
    store.delete_extension(row.profile_id, &row.install_id).map_err(|e| e.to_string())?;
    ext_package::remove_tree(&install_dir(root, row.profile_id, &row.install_id)).ok();
    Ok(())
}

pub fn set_enabled(store: &Store, row: &InstalledExtension, on: bool) -> Result<(), String> {
    store.put_extension(&InstalledExtension { enabled: on, ..row.clone() }).map_err(|e| e.to_string())
}

pub fn set_all_sites(store: &Store, row: &InstalledExtension, on: bool) -> Result<(), String> {
    if on && !may_use_all_sites(&row.source) {
        return Err("Only App and Official extensions may use every site they list.".to_owned());
    }
    store.put_extension(&InstalledExtension { all_sites: on, ..row.clone() }).map_err(|e| e.to_string())
}

/// What an update check of `row` installs: its GitHub repository's newest release, or the
/// catalog's newer version; `None` for files.
pub fn update_of(row: &InstalledExtension, catalog: Option<&Index>) -> Option<Install> {
    if let Some((repo_id, _)) = github_source(&row.source) {
        return Some(Install::GithubUpdate { repo_id, tag: row.release_tag.clone(), asset: row.asset.clone() });
    }
    let entry = row.source.strip_prefix("official:").and_then(|id| catalog?.extensions.iter().find(|e| e.id == id))?;
    older(&row.version, &entry.version).then(|| Install::Official(entry.clone()))
}

/// The catalog's rows for the profile: id, name, description, version and whether it is installed.
pub fn catalog_rows(index: &Index, installed: &[InstalledExtension]) -> Vec<(String, String, String, String, bool)> {
    index
        .extensions
        .iter()
        .map(|e| (e.id.clone(), shown(&e.name), shown(&e.description), shown(&e.version), installed.iter().any(|r| r.source == format!("official:{}", e.id))))
        .collect()
}

/// Fetches, verifies and caches the signed catalog. Blocks.
pub fn fetch_catalog(net: &dyn Fetch, dir: &Path, last: Option<u64>, now: u64, key: &[u8; 32]) -> Result<Index, String> {
    let (index, sig) = crate::github::catalog(net)?;
    let parsed = catalog::verify(&index, &sig, key).map_err(|e| e.to_string())?;
    catalog::check_fetched(&parsed, last, now).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(dir.join("index.json"), &index))
        .and_then(|()| std::fs::write(dir.join("index.sig"), &sig))
        .map_err(|e| e.to_string())?;
    Ok(parsed)
}

/// The index last fetched, verified again.
pub fn cached_catalog(dir: &Path, key: &[u8; 32]) -> Option<Index> {
    if *key == [0; 32] {
        return None;
    }
    let index = std::fs::read(dir.join("index.json")).ok()?;
    let sig = std::fs::read(dir.join("index.sig")).ok()?;
    catalog::verify(&index, &sig, key).ok()
}

pub fn merge_revoked(store: &Store, index: &Index) {
    let stored = store.setting(REVOKED_SETTING).ok().flatten().unwrap_or_default();
    store.set_setting(REVOKED_SETTING, &catalog::merge_revoked(&stored, index)).ok();
}

pub fn catalog_fetched(store: &Store, index: &Index) {
    store.set_setting(CATALOG_SEEN_SETTING, &index.version.to_string()).ok();
    merge_revoked(store, index);
}

#[derive(Clone, Copy, Debug)]
pub struct StoreContext {
    pub allowed: bool,
    pub configured: bool,
    pub downloads: bool,
    pub busy: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct StoreCatalog<'a> {
    pub index: Option<&'a Index>,
    pub now: u64,
    pub revoked: &'a BTreeSet<(String, String)>,
    pub installed: &'a [InstalledExtension],
}

pub fn store_step(id: &str, context: StoreContext, catalog: StoreCatalog) -> Result<Box<Entry>, String> {
    let refuse = |why: &str| Err(why.to_owned());
    if !context.allowed {
        return refuse(STORE_REFUSED);
    }
    if !context.configured {
        return refuse(STORE_NO_CATALOG);
    }
    if !context.downloads {
        return refuse(STORE_DOWNLOADS_OFF);
    }
    if context.busy {
        return refuse(STORE_BUSY);
    }
    let Some(index) = catalog.index.filter(|c| !catalog::is_old(c, catalog.now)) else { return refuse(STORE_NEEDS_CATALOG) };
    let Some(entry) = index.extensions.iter().find(|e| e.id == id) else { return refuse(STORE_UNKNOWN) };
    let listed = |rid: &str, sha: &str| rid == entry.id && sha == entry.sha256;
    if catalog.revoked.iter().any(|(rid, sha)| listed(rid, sha)) || index.revoked.iter().any(|r| listed(&r.id, &r.sha256)) {
        return refuse(STORE_WITHDRAWN);
    }
    if catalog.installed.iter().any(|r| r.source.strip_prefix("official:") == Some(id)) {
        return Err(format!("{} is already installed.", shown(&entry.name)));
    }
    Ok(Box::new(entry.clone()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreToken {
    seq: u64,
    id: String,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StoreOffer {
    pub tab: crate::tabs::TabKey,
    pub entry: Entry,
    pub token: StoreToken,
    shown: std::time::Instant,
}

#[derive(Clone, Debug, Default)]
pub struct StoreSheet {
    seq: u64,
    offer: Option<StoreOffer>,
}

impl StoreSheet {
    pub fn offer(&mut self, tab: crate::tabs::TabKey, id: &str, context: StoreContext, catalog: StoreCatalog, now: std::time::Instant) -> Result<(), String> {
        let entry = *store_step(id, context, catalog)?;
        self.seq += 1;
        let token = StoreToken { seq: self.seq, id: entry.id.clone(), sha256: entry.sha256.clone() };
        self.offer = Some(StoreOffer { tab, entry, token, shown: now });
        Ok(())
    }

    pub fn shown(&self) -> Option<&StoreOffer> {
        self.offer.as_ref()
    }

    pub fn clear(&mut self) {
        self.offer = None;
    }

    pub fn take(&mut self, token: &StoreToken, now: std::time::Instant) -> Option<StoreOffer> {
        let ready = self.offer.as_ref().is_some_and(|o| o.token == *token && now.saturating_duration_since(o.shown) >= STORE_CONFIRM_DELAY);
        if ready { self.offer.take() } else { None }
    }
}

pub fn store_confirm(offered: &Entry, context: StoreContext, catalog: StoreCatalog) -> Result<Box<Entry>, String> {
    let entry = store_step(&offered.id, context, catalog)?;
    if *entry != *offered {
        return Err(STORE_CHANGED.to_owned());
    }
    Ok(entry)
}

pub fn store_sheet(entry: &Entry) -> (String, Vec<String>) {
    let list = |items: &[String]| items.iter().map(|i| shown(i)).collect::<Vec<_>>().join(", ");
    let mut lines = Vec::new();
    if !entry.description.is_empty() {
        lines.push(shown(&entry.description));
    }
    lines.push(format!("Source: {}", badge(&format!("official:{}", entry.id))));
    if !entry.permissions.is_empty() {
        lines.push(format!("Uses: {}", list(&entry.permissions)));
    }
    if !entry.optional_permissions.is_empty() {
        lines.push(format!("May ask for: {}", list(&entry.optional_permissions)));
    }
    lines.push(if entry.hosts.is_empty() { "Asks for no sites.".to_owned() } else { format!("Wants to read and change: {}", list(&entry.hosts)) });
    lines.push("It asks you on every site before it runs there, unless you let it use every site it lists.".to_owned());
    lines.push(STORE_FETCH_NOTE.to_owned());
    (format!("Install \u{201C}{}\u{201D} {}?", shown(&entry.name), shown(&entry.version)), lines)
}

pub fn store_confirmed(confirmed: &str, source: &str) -> bool {
    source.strip_prefix("official:") == Some(confirmed)
}

/// The title of the per-site prompt every client shows.
pub fn access_title(name: &str, badge: &str, host: &str) -> String {
    format!("The extension \u{201C}{}\u{201D} ({}) wants to read and change {}", shown(name), shown(badge), shown(host))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::FetchError;

    const TARN: &str = r#"{"manifest_version":3,"name":"Helper","version":"1.0","browser_specific_settings":{"tarn":{"runtime":1}},
        "permissions":["storage"],"content_scripts":[{"matches":["*://*.shop.test/*"],"js":["c.js"]}]}"#;

    fn crc32(bytes: &[u8]) -> u32 {
        !bytes.iter().fold(!0u32, |crc, b| (0..8).fold(crc ^ u32::from(*b), |c, _| if c & 1 == 1 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 }))
    }

    fn zip(manifest: &str) -> Vec<u8> {
        let files: [(&str, &[u8]); 2] = [("manifest.json", manifest.as_bytes()), ("c.js", b"document.documentElement.dataset.ext = 'on';")];
        let (mut out, mut central) = (Vec::new(), Vec::new());
        for (name, data) in files {
            let (crc, size, offset) = (crc32(data), data.len() as u32, out.len() as u32);
            let common = [
                &20u16.to_le_bytes()[..],
                &0u16.to_le_bytes(),
                &0u16.to_le_bytes(),
                &0u32.to_le_bytes(),
                &crc.to_le_bytes(),
                &size.to_le_bytes(),
                &size.to_le_bytes(),
                &(name.len() as u16).to_le_bytes(),
                &0u16.to_le_bytes(),
            ]
            .concat();
            out.extend([&0x0403_4b50u32.to_le_bytes()[..], &common, name.as_bytes(), data].concat());
            central.extend(
                [&0x0201_4b50u32.to_le_bytes()[..], &20u16.to_le_bytes(), &common, &[0u8; 6], &0u32.to_le_bytes(), &offset.to_le_bytes(), name.as_bytes()]
                    .concat(),
            );
        }
        let (at, len) = (out.len() as u32, central.len() as u32);
        out.extend(central);
        out.extend(
            [
                &0x0605_4b50u32.to_le_bytes()[..],
                &[0u8; 4],
                &2u16.to_le_bytes(),
                &2u16.to_le_bytes(),
                &len.to_le_bytes(),
                &at.to_le_bytes(),
                &0u16.to_le_bytes(),
            ]
            .concat(),
        );
        out
    }

    #[test]
    fn a_file_install_offers_consent_then_stores_the_row() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let profile = store.create_profile("P", "2fda0aba-1c3e-4e0b-9c51-4f7f1b2d3e4a", false).unwrap().id;
        let file = root.path().join("helper.zip");
        std::fs::write(&file, zip(TARN)).unwrap();
        let slot = slot(&store, profile, &root.path().join("Extensions"), None, || "0123456789abcdef0123456789abcdef".to_owned()).unwrap();
        assert_eq!(slot.install_id.len(), 32);
        let Fetched::Offer(offer) = fetch(&NoNet, Install::File(file), &slot, &BTreeSet::new(), "seed", &[]).unwrap() else { panic!("no offer") };
        assert_eq!(
            (offer.name.as_str(), offer.apis.as_slice(), offer.sites.as_slice()),
            ("Helper", ["storage".to_owned()].as_slice(), ["*://*.shop.test/*".to_owned()].as_slice())
        );
        let p = pending(slot.clone(), offer).unwrap();
        assert!(p.unverified() && p.lines().iter().any(|l| l == UNREVIEWED));
        assert_eq!(p.badge(), "Unverified · file");
        assert!(store.extensions(profile).unwrap().is_empty(), "nothing is stored before consent");
        confirm(&store, &p).unwrap();
        let rows = store.extensions(profile).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].enabled && !rows[0].all_sites && rows[0].source == "file");
        assert!(set_all_sites(&store, &rows[0], true).is_err(), "a file never gets every site");
        remove(&store, &root.path().join("Extensions"), &rows[0]).unwrap();
        assert!(store.extensions(profile).unwrap().is_empty());
        assert!(!slot.dir.exists());
    }

    #[test]
    fn a_classic_extension_is_refused_and_leaves_nothing() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let profile = store.create_profile("P", "2fda0aba-1c3e-4e0b-9c51-4f7f1b2d3e4a", false).unwrap().id;
        let file = root.path().join("classic.zip");
        std::fs::write(&file, zip(r#"{"manifest_version":3,"name":"Classic","version":"1"}"#)).unwrap();
        let slot = slot(&store, profile, &root.path().join("Extensions"), None, || "0123456789abcdef0123456789abcdef".to_owned()).unwrap();
        let err = fetch(&NoNet, Install::File(file), &slot, &BTreeSet::new(), "seed", &[]).unwrap_err();
        assert_eq!(err, NOT_TARN);
        assert!(!slot.new_install.unwrap().exists());
    }

    #[test]
    fn a_tree_changed_before_consent_is_not_installed() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(root.path()).unwrap();
        let profile = store.create_profile("P", "2fda0aba-1c3e-4e0b-9c51-4f7f1b2d3e4a", false).unwrap().id;
        let file = root.path().join("helper.zip");
        std::fs::write(&file, zip(TARN)).unwrap();
        let slot = slot(&store, profile, &root.path().join("Extensions"), None, || "0123456789abcdef0123456789abcdef".to_owned()).unwrap();
        let Fetched::Offer(offer) = fetch(&NoNet, Install::File(file), &slot, &BTreeSet::new(), "seed", &[]).unwrap() else { panic!("no offer") };
        let p = pending(slot.clone(), offer).unwrap();
        let script = slot.dir.join("c.js");
        crate::ext_package::set_mode(&script, 0o600).unwrap();
        std::fs::write(&script, "steal()").unwrap();
        assert!(confirm(&store, &p).is_err());
        assert!(store.extensions(profile).unwrap().is_empty());
    }

    #[test]
    fn an_install_is_found_under_the_root_after_the_container_moved() {
        let row = InstalledExtension { profile_id: 3, install_id: "ab".into(), path: "/old/container/Extensions/3/ab/2".into(), ..Default::default() };
        assert_eq!(installed_dir(Path::new("/new/Extensions"), &row), Path::new("/new/Extensions/3/ab/2"));
    }

    #[test]
    fn the_prompt_names_extension_source_and_host() {
        assert_eq!(access_title("Helper", "Official", "shop.test"), "The extension \u{201C}Helper\u{201D} (Official) wants to read and change shop.test");
    }

    fn store_index(now: u64) -> Index {
        serde_json::from_value(serde_json::json!({
            "kind": "tarn-extensions", "version": 3, "created": now,
            "extensions": [
                {"id": "helper", "name": "Helper", "description": "", "version": "1.0", "url": "https://x.test/h.zip", "sha256": "a".repeat(64), "size": 1, "permissions": [], "hosts": []},
                {"id": "gone", "name": "Gone", "description": "", "version": "1.0", "url": "https://x.test/g.zip", "sha256": "b".repeat(64), "size": 1, "permissions": [], "hosts": []},
                {"id": "pulled", "name": "Pulled", "description": "", "version": "1.0", "url": "https://x.test/p.zip", "sha256": "c".repeat(64), "size": 1, "permissions": [], "hosts": []}
            ],
            "revoked": [{"id": "gone", "sha256": "b".repeat(64)}]
        }))
        .unwrap()
    }

    #[test]
    fn a_store_link_offers_only_a_fresh_verified_catalog_entry_and_never_fetches() {
        let now = 1_900_000_000;
        let (index, old) = (store_index(now), store_index(now - 31 * 24 * 3600));
        let ok = StoreContext { allowed: true, configured: true, downloads: true, busy: false };
        let none = BTreeSet::new();
        let fresh = StoreCatalog { index: Some(&index), now, revoked: &none, installed: &[] };
        let refused = |why: &str| Err(why.to_owned());
        assert_eq!(store_step("helper", ok, fresh).unwrap().id, "helper");
        assert_eq!(store_step("helper", StoreContext { allowed: false, ..ok }, fresh), refused(STORE_REFUSED));
        assert_eq!(store_step("helper", StoreContext { configured: false, ..ok }, fresh), refused(STORE_NO_CATALOG));
        assert_eq!(store_step("helper", StoreContext { downloads: false, ..ok }, fresh), refused(STORE_DOWNLOADS_OFF));
        assert_eq!(store_step("helper", StoreContext { busy: true, ..ok }, fresh), refused(STORE_BUSY));
        assert_eq!(store_step("helper", ok, StoreCatalog { index: None, ..fresh }), refused(STORE_NEEDS_CATALOG), "no catalog: a notice, never a fetch");
        assert_eq!(store_step("helper", ok, StoreCatalog { index: Some(&old), ..fresh }), refused(STORE_NEEDS_CATALOG), "a list over 30 days old is not used");
        assert_eq!(store_step("missing", ok, fresh), refused(STORE_UNKNOWN));
        assert_eq!(store_step("gone", ok, fresh), refused(STORE_WITHDRAWN));
        let stored: BTreeSet<_> = [("pulled".to_owned(), "c".repeat(64))].into();
        assert_eq!(store_step("pulled", ok, StoreCatalog { revoked: &stored, ..fresh }), refused(STORE_WITHDRAWN));
        let other_build: BTreeSet<_> = [("pulled".to_owned(), "d".repeat(64))].into();
        assert!(store_step("pulled", ok, StoreCatalog { revoked: &other_build, ..fresh }).is_ok());
        let installed = [InstalledExtension { source: "official:helper".into(), ..Default::default() }];
        assert_eq!(store_step("helper", ok, StoreCatalog { installed: &installed, ..fresh }), refused("Helper is already installed."));
        assert!(store_step("pulled", ok, StoreCatalog { installed: &installed, ..fresh }).is_ok());
    }

    #[test]
    fn the_store_confirmation_checks_everything_again_and_holds_to_the_shown_entry() {
        let now = 1_900_000_000;
        let index = store_index(now);
        let none = BTreeSet::new();
        let ok = StoreContext { allowed: true, configured: true, downloads: true, busy: false };
        let fresh = StoreCatalog { index: Some(&index), now, revoked: &none, installed: &[] };
        let offered = *store_step("helper", ok, fresh).unwrap();
        assert_eq!(*store_confirm(&offered, ok, fresh).unwrap(), offered);
        assert_eq!(store_confirm(&offered, StoreContext { allowed: false, ..ok }, fresh), Err(STORE_REFUSED.to_owned()), "a private space or Lockdown by now");
        assert_eq!(store_confirm(&offered, StoreContext { busy: true, ..ok }, fresh), Err(STORE_BUSY.to_owned()));
        let stale = StoreCatalog { now: now + 31 * 24 * 3600, ..fresh };
        assert_eq!(store_confirm(&offered, ok, stale), Err(STORE_NEEDS_CATALOG.to_owned()));
        let grown = Entry { hosts: vec!["*://*.bank.test/*".into()], ..offered.clone() };
        assert_eq!(store_confirm(&grown, ok, fresh), Err(STORE_CHANGED.to_owned()), "what was shown is what installs");
    }

    #[test]
    fn a_new_store_link_replaces_an_unanswered_confirmation_and_only_a_running_install_blocks() {
        let now = 1_900_000_000;
        let index = store_index(now);
        let none = BTreeSet::new();
        let ok = StoreContext { allowed: true, configured: true, downloads: true, busy: false };
        let catalog = StoreCatalog { index: Some(&index), now, revoked: &none, installed: &[] };
        let (first, second) = (crate::tabs::TabKey { private: false, id: 1 }, crate::tabs::TabKey { private: false, id: 2 });
        let at = std::time::Instant::now();
        let mut sheet = StoreSheet::default();
        sheet.offer(first, "helper", ok, catalog, at).unwrap();
        sheet.offer(second, "pulled", ok, catalog, at).unwrap();
        assert_eq!(sheet.shown().map(|o| (o.tab, o.entry.id.as_str())), Some((second, "pulled")), "the unanswered one is replaced");
        assert_eq!(sheet.offer(first, "helper", StoreContext { busy: true, ..ok }, catalog, at), Err(STORE_BUSY.to_owned()));
        assert_eq!(sheet.offer(first, "missing", ok, catalog, at), Err(STORE_UNKNOWN.to_owned()));
        assert_eq!(sheet.shown().map(|o| o.entry.id.as_str()), Some("pulled"), "a refused link leaves the confirmation shown");
        sheet.clear();
        assert!(sheet.shown().is_none(), "closing the pane drops it");
    }

    #[test]
    fn install_confirms_only_the_sheet_it_was_drawn_for_and_not_at_once() {
        let now = 1_900_000_000;
        let index = store_index(now);
        let none = BTreeSet::new();
        let ok = StoreContext { allowed: true, configured: true, downloads: true, busy: false };
        let catalog = StoreCatalog { index: Some(&index), now, revoked: &none, installed: &[] };
        let tab = crate::tabs::TabKey { private: false, id: 1 };
        let (at, ms) = (std::time::Instant::now(), std::time::Duration::from_millis);
        let mut sheet = StoreSheet::default();
        sheet.offer(tab, "helper", ok, catalog, at).unwrap();
        let a = sheet.shown().unwrap().token.clone();
        sheet.offer(tab, "pulled", ok, catalog, at + ms(10_000)).unwrap();
        let b = sheet.shown().unwrap().token.clone();
        assert_ne!(a, b);
        assert!(sheet.take(&a, at + ms(20_000)).is_none(), "a click aimed at the replaced sheet never confirms the new one");
        assert!(sheet.take(&b, at + ms(10_000) + STORE_CONFIRM_DELAY - ms(1)).is_none(), "too soon after it appeared");
        assert_eq!(sheet.shown().map(|o| o.entry.id.as_str()), Some("pulled"), "refused clicks leave the sheet");
        let taken = sheet.take(&b, at + ms(10_000) + STORE_CONFIRM_DELAY).unwrap();
        assert_eq!(taken.entry.id, "pulled");
        assert!(sheet.shown().is_none() && sheet.take(&b, at + ms(30_000)).is_none(), "taken once");
        sheet.offer(tab, "helper", ok, catalog, at + ms(40_000)).unwrap();
        let again = sheet.shown().unwrap().token.clone();
        assert!(again != a && sheet.take(&a, at + ms(50_000)).is_none(), "the same entry offered again gets a new token");
    }

    #[test]
    fn the_store_sheet_shows_only_the_signed_entry() {
        let index = store_index(1_900_000_000);
        let entry = Entry {
            optional_permissions: vec!["tabs".into()],
            hosts: vec!["*://*.shop.test/*".into()],
            permissions: vec!["storage".into()],
            ..index.extensions[0].clone()
        };
        let (title, lines) = store_sheet(&entry);
        assert_eq!(title, "Install \u{201C}Helper\u{201D} 1.0?");
        assert!(
            lines.contains(&"Source: Official".to_owned()) && lines.contains(&"Uses: storage".to_owned()) && lines.contains(&"May ask for: tabs".to_owned())
        );
        assert!(lines.contains(&"Wants to read and change: *://*.shop.test/*".to_owned()) && lines.last().map(String::as_str) == Some(STORE_FETCH_NOTE));
        assert!(store_confirmed("helper", "official:helper") && !store_confirmed("helper", "official:other") && !store_confirmed("helper", "github:1"));
    }

    struct NoNet;

    impl Fetch for NoNet {
        fn get(&self, _: &str, _: u64) -> Result<Vec<u8>, FetchError> {
            Err(FetchError::Failed("offline".into()))
        }
    }
}
