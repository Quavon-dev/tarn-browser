//! Filter-list bundles from GitHub Releases (S20): download, verify, cache, convert, install.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use engine::FilterLists;
use futures::channel::oneshot;
use gpui::{App, AsyncApp, Task};
use shield::bundle::{self, BundleError, FILTER_BUNDLE_URL, Kind, MANIFEST, MAX_MANIFEST_BYTES, Manifest, SIGNATURE, VerifiedBundle};
use shield::cache::{self, State, Stored};
use shield::groups::{GROUPS, Inputs, hex, public_key};
use shield::{lists, rules, youtube};
use ureq::tls::{RootCerts, TlsConfig};

use crate::blocking::Sets;
use crate::shared::AppCore;

pub const SETTING: &str = "filter_updates";
const UPDATE_INTERVAL: u64 = 86_400;
/// Settings warns once the installed lists were signed longer ago than this.
const OUT_OF_DATE: u64 = 14 * 86_400;
const TICK: Duration = Duration::from_secs(600);
pub const YOUTUBE_GROUP: &str = "youtube";

#[derive(Default, Clone)]
pub struct Status {
    pub updates: bool,
    /// `created` of the installed bundle.
    pub installed: Option<String>,
    /// `version` of the installed bundle, its signing time.
    pub version: Option<u64>,
    pub error: Option<String>,
    /// Rules or domains per category, as counted when the bundle was prepared.
    pub counts: BTreeMap<String, u64>,
}

pub fn configured() -> bool {
    public_key() != [0; 32] && (FILTER_BUNDLE_URL.starts_with("https://") || cfg!(debug_assertions))
}

impl Status {
    pub fn out_of_date(&self) -> bool {
        self.version.is_some_and(|v| v.saturating_add(OUT_OF_DATE) < now())
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn describe(e: &BundleError) -> String {
    match e {
        BundleError::NoPublicKey => "no signing key in this build".into(),
        BundleError::BadSignature => "signature check failed".into(),
        BundleError::Rollback { .. } => "offered lists are older than the installed ones".into(),
        BundleError::Stale(_) => "offered lists are out of date".into(),
        BundleError::Missing(name) => format!("{name} could not be downloaded"),
        other => format!("invalid bundle ({other:?})"),
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(3)
        .proxy(None)
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .build(),
        )
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .into()
}

fn get(agent: &ureq::Agent, url: &str, max: u64) -> Result<Vec<u8>, String> {
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    response.body_mut().with_config().limit(max).read_to_vec().map_err(|e| e.to_string())
}

pub struct Update {
    manifest: Vec<u8>,
    signature: Vec<u8>,
    bundle: VerifiedBundle,
}

/// `Ok(None)` when the published bundle is the installed one. The unsigned manifest is read first
/// only to skip a download that would be refused; list files are fetched after the signature checks
/// out, all of them, so the download does not reveal which categories are switched on.
fn download(base: &str, public_key: &[u8; 32], last: Option<u64>, now: u64) -> Result<Option<Update>, String> {
    let agent = agent();
    let manifest = get(&agent, &format!("{base}{MANIFEST}"), MAX_MANIFEST_BYTES as u64)?;
    if let Ok(unsigned) = serde_json::from_slice::<Manifest>(&manifest) {
        if Some(unsigned.version) == last {
            return Ok(None);
        }
        cache::check_age(unsigned.version, now).map_err(|e| describe(&e))?;
    }
    let signature = get(&agent, &format!("{base}{SIGNATURE}"), 64)?;
    let failure = RefCell::new(None);
    let fetch = |name: &str, size: u64| get(&agent, &format!("{base}{name}"), size.saturating_add(1)).map_err(|e| *failure.borrow_mut() = Some(e)).ok();
    let bundle = match bundle::verify(&manifest, &signature, &fetch, public_key, last) {
        Err(BundleError::Rollback { version, last }) if version == last => return Ok(None),
        Err(e @ BundleError::Missing(_)) => return Err(failure.take().unwrap_or_else(|| describe(&e))),
        result => result.map_err(|e| describe(&e))?,
    };
    cache::check_age(bundle.version, now).map_err(|e| describe(&e))?;
    Ok(Some(Update { manifest, signature, bundle }))
}

fn counts(bundle: &VerifiedBundle) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for list in bundle.lists.iter().filter(|l| matches!(l.kind, Kind::Filters | Kind::Set)) {
        *counts.entry(list.category.clone()).or_default() += list.count;
    }
    counts
}

async fn webkit(cx: &mut AsyncApp, run: impl FnOnce(Box<dyn FnOnce(Result<FilterLists, String>)>)) -> Result<FilterLists, String> {
    let (tx, rx) = oneshot::channel();
    cx.update(|_| run(Box::new(move |r| drop(tx.send(r))))).map_err(|_| "app quit".to_string())?;
    rx.await.unwrap_or_else(|_| Err("cancelled".into()))
}

/// The group's lists from WebKit's store when it still has them under `key`, else converted and
/// compiled; returns them with their chunk count.
async fn obtain(
    cx: &mut AsyncApp,
    group: &'static str,
    key: String,
    stored: Option<Stored>,
    convert: impl FnOnce() -> Vec<String> + Send + 'static,
) -> Result<(FilterLists, usize), String> {
    if let Some(chunks) = stored.filter(|s| s.fingerprint == key).map(|s| s.chunks) {
        let k = key.clone();
        if let Ok(lists) = webkit(cx, move |done| FilterLists::look_up(group, &k, chunks, done)).await {
            return Ok((lists, chunks));
        }
    }
    let chunks = cx.background_executor().spawn(async move { convert() }).await;
    let count = chunks.len();
    let lists = webkit(cx, move |done| FilterLists::compile(group, &key, chunks, done)).await?;
    Ok((lists, count))
}

fn set_error(core: &AppCore, cx: &mut AsyncApp, error: Option<String>) {
    core.filters.borrow_mut().error = error;
    cx.update(|cx| cx.refresh_windows()).ok();
}

/// Installs every group of `bundle` and its sets. Groups that fail keep their previous lists; the
/// first failure is returned after the others are installed.
async fn install(core: &AppCore, state: &mut State, bundle: VerifiedBundle, cx: &mut AsyncApp) -> Result<(), String> {
    let categories = core.set_categories();
    let version = bundle.version;
    let (inputs, keys, sets, config, counts, created, revocations) = cx
        .background_executor()
        .spawn(async move {
            let inputs = Inputs::new(&bundle);
            let keys: Vec<String> = GROUPS.iter().map(|g| inputs.key(g)).collect();
            let config = bundle.of("youtube", Kind::Config).next().and_then(|l| youtube::parse(&l.data)).unwrap_or_default();
            let revocations = bundle.of(shield::revocations::CATEGORY, Kind::Config).next().and_then(|l| shield::revocations::parse(&l.data));
            (Arc::new(inputs), keys, Sets::from_bundle(&bundle, &categories), config, counts(&bundle), bundle.created, revocations)
        })
        .await;
    core.set_youtube_config(&config);
    if revocations.is_some_and(|list| crate::themes::merge_revocations(core, &list)) {
        cx.update(crate::themes::changed).ok();
    }
    {
        let mut filters = core.filters.borrow_mut();
        filters.counts = counts;
        filters.installed = Some(created.clone());
    }
    cx.update(|cx| crate::blocking::set_sets(sets, cx)).ok();
    let mut first_error = None;
    for (group, key) in GROUPS.into_iter().zip(keys) {
        let inputs = inputs.clone();
        let stored = state.compiled.get(group).cloned();
        match obtain(cx, group, key.clone(), stored, move || inputs.convert(group)).await {
            Ok((lists, chunks)) => {
                state.compiled.insert(group.into(), Stored { fingerprint: key, chunks });
                core.set_rule_lists(group, lists);
            }
            Err(e) => {
                first_error.get_or_insert(format!("{group}: {e}"));
            }
        }
    }
    #[cfg(any(feature = "chromium", windows))]
    {
        let matcher = cx
            .background_executor()
            .spawn(async move { shield::matcher::Matcher::new(&Inputs::refs(&inputs.ads), &Inputs::refs(&inputs.trackers), &Inputs::refs(&inputs.annoyances)) })
            .await;
        *core.matcher.borrow_mut() = matcher;
        core.sync_shield();
    }
    state.created = created;
    let mut filters = core.filters.borrow_mut();
    if first_error.is_none() {
        filters.version = Some(version);
    } else {
        filters.version.get_or_insert(version);
    }
    first_error.map_or(Ok(()), Err)
}

/// Compiles Tarn's own YouTube rules, which need no bundle.
async fn install_builtin(core: &AppCore, state: &mut State, cx: &mut AsyncApp) {
    let json = rules::youtube_rules();
    let key = hex(lists::fingerprint(&[&json]));
    if let Ok((lists, chunks)) = obtain(cx, YOUTUBE_GROUP, key.clone(), state.compiled.get(YOUTUBE_GROUP).cloned(), move || vec![json]).await {
        state.compiled.insert(YOUTUBE_GROUP.into(), Stored { fingerprint: key, chunks });
        core.set_rule_lists(YOUTUBE_GROUP, lists);
        engine::startup::mark("youtube rules attached");
    }
}

/// Installs the cached bundle after verifying it again, then checks for updates once a day while
/// updates are on. A failure keeps the installed lists and the version floor in `state.json`.
pub fn spawn(dir: PathBuf, core: Rc<AppCore>, cx: &mut App) -> Task<()> {
    cx.spawn(async move |cx| {
        let mut state = cache::load_state(&dir);
        install_builtin(&core, &mut state, cx).await;
        cache::save_state(&dir, &state).ok();
        if !configured() {
            return;
        }
        match load_cached(&core, &dir, &mut state, cx).await {
            Ok(()) => engine::startup::mark("filter lists attached"),
            Err(e) if state.version.is_some() => set_error(&core, cx, Some(e)),
            Err(_) => {}
        }
        loop {
            let (updates, installed) = {
                let filters = core.filters.borrow();
                (filters.updates, filters.installed.is_some())
            };
            if updates && now().saturating_sub(state.checked) >= UPDATE_INTERVAL {
                state.checked = now();
                match update(&core, &dir, &mut state, installed, cx).await {
                    Ok(()) => {
                        if core.filters.borrow().installed.is_some() {
                            set_error(&core, cx, None);
                        }
                    }
                    Err(e) => set_error(&core, cx, Some(e)),
                }
                cache::save_state(&dir, &state).ok();
            }
            cx.background_executor().timer(TICK).await;
        }
    })
}

async fn read_cached(dir: &Path, cx: &AsyncApp) -> Result<VerifiedBundle, String> {
    let dir = dir.to_owned();
    cx.background_executor().spawn(async move { cache::read(&dir, &public_key()).map_err(|e| describe(&e)) }).await
}

async fn load_cached(core: &AppCore, dir: &Path, state: &mut State, cx: &mut AsyncApp) -> Result<(), String> {
    let bundle = read_cached(dir, cx).await?;
    state.version = state.version.max(Some(bundle.version));
    let result = install(core, state, bundle, cx).await;
    cache::save_state(dir, state).ok();
    result
}

/// Re-reads the cached bundle's domain sets after a category switch changed; the bundle is
/// verified again, so a modified cache is never used.
pub fn reload_sets(core: Rc<AppCore>, cx: &mut App) {
    if !configured() {
        return;
    }
    let dir = crate::browser::data_dir().join("filters");
    cx.spawn(async move |cx| {
        let categories = core.set_categories();
        let Ok(bundle) = read_cached(&dir, cx).await else { return };
        let sets = cx.background_executor().spawn(async move { Sets::from_bundle(&bundle, &categories) }).await;
        cx.update(|cx| crate::blocking::set_sets(sets, cx)).ok();
    })
    .detach();
}

/// The new bundle replaces the cached one only after every group compiled.
/// With nothing installed the published copy of the floor version is accepted again, so a lost
/// cache can be repaired; older versions stay refused.
async fn update(core: &AppCore, dir: &Path, state: &mut State, installed: bool, cx: &mut AsyncApp) -> Result<(), String> {
    let last = if installed { state.version } else { state.version.map(|v| v.saturating_sub(1)) };
    let fetched = cx.background_executor().spawn(async move { download(FILTER_BUNDLE_URL, &public_key(), last, now()) }).await?;
    let Some(Update { manifest, signature, bundle }) = fetched else { return Ok(()) };
    let version = bundle.version;
    let lists = bundle.lists.clone();
    let created = bundle.created.clone();
    install(core, state, bundle, cx).await?;
    let write_dir = dir.to_owned();
    cx.background_executor()
        .spawn(async move { cache::write(&write_dir, &manifest, &signature, &VerifiedBundle { version, created, lists }) })
        .await
        .map_err(|e| e.to_string())?;
    state.version = Some(version);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shield::Category;
    use shield::bundle::VerifiedList;
    use shield::domains::DomainSet;

    fn list(category: &str, kind: Kind, data: &[u8]) -> VerifiedList {
        VerifiedList { name: format!("{category}.x"), category: category.into(), kind, count: 2, data: data.to_vec() }
    }

    fn bundle() -> VerifiedBundle {
        let set = DomainSet::encode(["evil.example"]);
        VerifiedBundle {
            version: 1,
            created: String::new(),
            lists: vec![
                list("ads", Kind::Filters, b"||ads.example^\n@@||ok-ad.example^\n##.banner\n"),
                list("trackers", Kind::Filters, b"||tracker.example^\n@@||ok-tracker.example^\n"),
                list("annoyances", Kind::Filters, b"||consent.example^\n##.cookie-banner\n"),
                list("malware", Kind::Domains, b"evil.example\n"),
                list("malware", Kind::Set, &set),
                list("adult", Kind::Set, &set),
                list("later", Kind::Set, b"not a set"),
            ],
        }
    }

    #[test]
    fn lists_signed_over_two_weeks_ago_are_out_of_date() {
        let status = |age: u64| Status { version: Some(now() - age), ..Status::default() };
        assert!(!Status::default().out_of_date());
        assert!(!status(OUT_OF_DATE - 60).out_of_date());
        assert!(status(OUT_OF_DATE + 60).out_of_date());
    }

    #[test]
    fn only_switched_on_sets_load_and_counts_add_up() {
        let b = bundle();
        let loaded = Sets::from_bundle(&b, &[Category::Malware]);
        assert!(loaded.malware.as_ref().is_some_and(|s| s.contains("evil.example")) && loaded.adult.is_none() && loaded.popups.is_none());
        assert_eq!(
            counts(&b),
            BTreeMap::from([
                ("ads".into(), 2),
                ("adult".into(), 2),
                ("annoyances".into(), 2),
                ("later".into(), 2),
                ("malware".into(), 2),
                ("trackers".into(), 2)
            ])
        );
    }
}
