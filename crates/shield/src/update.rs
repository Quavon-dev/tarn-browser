use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::bundle::{self, BundleError, FILTER_BUNDLE_URL, Kind, MANIFEST, MAX_MANIFEST_BYTES, Manifest, SIGNATURE, VerifiedBundle};
use crate::cache::{self, State};
use crate::groups::public_key;

pub const SETTING: &str = "filter_updates";
pub const UPDATE_INTERVAL: u64 = 86_400;
/// Settings warns once the installed lists were signed longer ago than this.
pub const OUT_OF_DATE: u64 = 14 * 86_400;
pub const TICK_SECS: u64 = 600;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
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

impl Status {
    pub fn out_of_date(&self) -> bool {
        self.version.is_some_and(|v| v.saturating_add(OUT_OF_DATE) < now())
    }

    pub fn installed(&mut self, bundle: &VerifiedBundle) {
        self.counts = counts(bundle);
        self.installed = Some(bundle.created.clone());
        self.version = Some(bundle.version);
    }

    pub fn checked(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) if self.installed.is_some() => self.error = None,
            Ok(()) => {}
            Err(e) => self.error = Some(e),
        }
    }

    /// The installed filter lists, their age and the last update error, in one line.
    pub fn summary(&self) -> String {
        if !configured() {
            return "Not set up in this build".to_owned();
        }
        let installed =
            self.installed.as_deref().map_or("No lists installed yet.".to_owned(), |c| format!("Lists from {} installed.", c.get(..10).unwrap_or(c)));
        let failed = self.error.as_ref().map(|e| format!(" Last update failed: {e}.")).unwrap_or_default();
        let stale = if self.out_of_date() { " Blocking lists are out of date." } else { "" };
        format!("{installed}{stale}{failed}")
    }
}

pub fn configured() -> bool {
    public_key() != [0; 32] && (FILTER_BUNDLE_URL.starts_with("https://") || cfg!(debug_assertions))
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn due(updates: bool, state: &State, now: u64) -> bool {
    updates && now.saturating_sub(state.checked) >= UPDATE_INTERVAL
}

/// With nothing installed the published copy of the floor version is accepted again, so a lost
/// cache can be repaired; older versions stay refused.
pub fn floor(state: &State, installed: bool) -> Option<u64> {
    if installed { state.version } else { state.version.map(|v| v.saturating_sub(1)) }
}

pub fn describe(e: &BundleError) -> String {
    match e {
        BundleError::NoPublicKey => "no signing key in this build".into(),
        BundleError::BadSignature => "signature check failed".into(),
        BundleError::Rollback { .. } => "offered lists are older than the installed ones".into(),
        BundleError::Stale(_) => "offered lists are out of date".into(),
        BundleError::Missing(name) => format!("{name} could not be downloaded"),
        other => format!("invalid bundle ({other:?})"),
    }
}

pub fn counts(bundle: &VerifiedBundle) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for list in bundle.lists.iter().filter(|l| matches!(l.kind, Kind::Filters | Kind::Set)) {
        *counts.entry(list.category.clone()).or_default() += list.count;
    }
    counts
}

pub struct Update {
    pub manifest: Vec<u8>,
    pub signature: Vec<u8>,
    pub bundle: VerifiedBundle,
}

/// `Ok(None)` when the published bundle is the installed one. The unsigned manifest is read first
/// only to skip a download that would be refused; list files are fetched after the signature checks
/// out, all of them, so the download does not reveal which categories are switched on.
pub fn download(
    get: impl Fn(&str, u64) -> Result<Vec<u8>, String>,
    base: &str,
    public_key: &[u8; 32],
    last: Option<u64>,
    now: u64,
) -> Result<Option<Update>, String> {
    let manifest = get(&format!("{base}{MANIFEST}"), MAX_MANIFEST_BYTES as u64)?;
    if let Ok(unsigned) = serde_json::from_slice::<Manifest>(&manifest) {
        if Some(unsigned.version) == last {
            return Ok(None);
        }
        cache::check_age(unsigned.version, now).map_err(|e| describe(&e))?;
    }
    let signature = get(&format!("{base}{SIGNATURE}"), 64)?;
    let failure = RefCell::new(None);
    let fetch = |name: &str, size: u64| get(&format!("{base}{name}"), size.saturating_add(1)).map_err(|e| *failure.borrow_mut() = Some(e)).ok();
    let bundle = match bundle::verify(&manifest, &signature, &fetch, public_key, last) {
        Err(BundleError::Rollback { version, last }) if version == last => return Ok(None),
        Err(e @ BundleError::Missing(_)) => return Err(failure.take().unwrap_or_else(|| describe(&e))),
        result => result.map_err(|e| describe(&e))?,
    };
    cache::check_age(bundle.version, now).map_err(|e| describe(&e))?;
    Ok(Some(Update { manifest, signature, bundle }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{FORMAT, ListEntry, VerifiedList, sha256_hex};
    use ed25519_dalek::{Signer, SigningKey};

    const BASE: &str = "https://lists.test/";
    const NOW: u64 = 100 * 86_400;
    const ADS: &[u8] = b"||ads.example^\n";
    const ADULT: &[u8] = b"adult.example\n";

    fn signed(version: u64) -> (Vec<u8>, Vec<u8>, [u8; 32]) {
        let key = SigningKey::from_bytes(&[9; 32]);
        let entry = |name: &str, category: &str, kind, data: &[u8]| ListEntry {
            name: name.into(),
            sha256: sha256_hex(data),
            size: data.len() as u64,
            category: category.into(),
            kind,
            count: 1,
        };
        let manifest = serde_json::to_vec(&Manifest {
            format: FORMAT,
            version,
            created: "2026-11-17T00:00:00Z".into(),
            lists: vec![entry("ads.txt", "ads", Kind::Filters, ADS), entry("adult.txt", "adult", Kind::Domains, ADULT)],
        })
        .unwrap();
        let signature = key.sign(&manifest).to_bytes().to_vec();
        (manifest, signature, key.verifying_key().to_bytes())
    }

    struct Server {
        files: BTreeMap<String, Vec<u8>>,
        asked: RefCell<Vec<String>>,
    }

    impl Server {
        fn new(version: u64) -> (Server, [u8; 32]) {
            let (manifest, signature, key) = signed(version);
            let files = BTreeMap::from([
                (format!("{BASE}{MANIFEST}"), manifest),
                (format!("{BASE}{SIGNATURE}"), signature),
                (format!("{BASE}ads.txt"), ADS.to_vec()),
                (format!("{BASE}adult.txt"), ADULT.to_vec()),
            ]);
            (Server { files, asked: RefCell::default() }, key)
        }

        fn get(&self, url: &str, max: u64) -> Result<Vec<u8>, String> {
            self.asked.borrow_mut().push(url.trim_start_matches(BASE).to_owned());
            let data = self.files.get(url).cloned().ok_or_else(|| format!("{url} answered 404"))?;
            if data.len() as u64 > max { Err("too large".into()) } else { Ok(data) }
        }

        fn run(&self, key: &[u8; 32], last: Option<u64>) -> Result<Option<Update>, String> {
            download(|url, max| self.get(url, max), BASE, key, last, NOW)
        }

        fn asked(&self) -> Vec<String> {
            self.asked.borrow().clone()
        }
    }

    #[test]
    fn a_new_bundle_is_verified_and_every_list_is_fetched_after_the_signature() {
        let (server, key) = Server::new(NOW - 60);
        let update = server.run(&key, Some(NOW - 86_400)).unwrap().unwrap();
        assert_eq!(update.bundle.version, NOW - 60);
        assert_eq!(update.bundle.lists.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(), ["ads.txt", "adult.txt"]);
        assert_eq!(server.asked(), [MANIFEST, SIGNATURE, "ads.txt", "adult.txt"]);
        assert_eq!(update.manifest, server.files[&format!("{BASE}{MANIFEST}")]);
    }

    #[test]
    fn the_installed_version_downloads_only_the_manifest() {
        let (server, key) = Server::new(NOW - 60);
        assert!(server.run(&key, Some(NOW - 60)).unwrap().is_none());
        assert_eq!(server.asked(), [MANIFEST]);
    }

    #[test]
    fn a_wrong_key_or_tampered_list_is_refused() {
        let (server, _) = Server::new(NOW - 60);
        assert_eq!(server.run(&SigningKey::from_bytes(&[3; 32]).verifying_key().to_bytes(), None).err().unwrap(), "signature check failed");
        assert_eq!(server.asked(), [MANIFEST, SIGNATURE]);
        let (mut server, key) = Server::new(NOW - 60);
        server.files.insert(format!("{BASE}ads.txt"), b"@@||ads.example^\n".to_vec());
        assert!(server.run(&key, None).is_err());
    }

    #[test]
    fn old_rolled_back_and_missing_bundles_are_refused() {
        let (server, key) = Server::new(NOW - cache::MAX_AGE_SECS - 1);
        assert_eq!(server.run(&key, None).err().unwrap(), "offered lists are out of date");
        assert_eq!(server.asked(), [MANIFEST]);
        let (server, key) = Server::new(NOW - 60);
        assert_eq!(server.run(&key, Some(NOW)).err().unwrap(), "offered lists are older than the installed ones");
        let (mut server, key) = Server::new(NOW - 60);
        server.files.remove(&format!("{BASE}adult.txt"));
        assert_eq!(server.run(&key, None).err().unwrap(), format!("{BASE}adult.txt answered 404"));
        let (server, _) = Server::new(NOW - 60);
        assert_eq!(server.run(&[0; 32], None).err().unwrap(), "no signing key in this build");
        assert_eq!(server.asked(), [MANIFEST, SIGNATURE]);
    }

    #[test]
    fn checks_are_due_once_a_day_while_switched_on() {
        let state = State { checked: NOW - UPDATE_INTERVAL, ..State::default() };
        assert!(due(true, &state, NOW));
        assert!(!due(false, &state, NOW));
        assert!(!due(true, &state, NOW - 1));
        assert!(due(true, &State::default(), NOW));
    }

    #[test]
    fn a_lost_cache_accepts_the_floor_version_again() {
        let state = State { version: Some(5), ..State::default() };
        assert_eq!(floor(&state, true), Some(5));
        assert_eq!(floor(&state, false), Some(4));
        assert_eq!(floor(&State::default(), false), None);
    }

    fn list(category: &str, kind: Kind) -> VerifiedList {
        VerifiedList { name: format!("{category}.x"), category: category.into(), kind, count: 2, data: Vec::new() }
    }

    #[test]
    fn counts_add_up_filters_and_sets_per_category() {
        let bundle = VerifiedBundle {
            version: 1,
            created: String::new(),
            lists: vec![
                list("ads", Kind::Filters),
                list("trackers", Kind::Filters),
                list("annoyances", Kind::Filters),
                list("malware", Kind::Domains),
                list("malware", Kind::Set),
                list("adult", Kind::Set),
                list("later", Kind::Set),
            ],
        };
        assert_eq!(
            counts(&bundle),
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

    #[test]
    fn status_tracks_installs_errors_and_age() {
        let status = |age: u64| Status { version: Some(now() - age), ..Status::default() };
        assert!(!Status::default().out_of_date());
        assert!(!status(OUT_OF_DATE - 60).out_of_date());
        assert!(status(OUT_OF_DATE + 60).out_of_date());
        let mut s = Status::default();
        s.checked(Err("offline".into()));
        assert_eq!(s.error.as_deref(), Some("offline"));
        s.checked(Ok(()));
        assert_eq!(s.error.as_deref(), Some("offline"));
        s.installed(&VerifiedBundle { version: now(), created: "2026-11-17T08:00:00Z".into(), lists: vec![list("ads", Kind::Filters)] });
        s.checked(Ok(()));
        assert_eq!(s.error, None);
        assert_eq!(s.counts.get("ads"), Some(&2));
        if configured() {
            assert_eq!(s.summary(), "Lists from 2026-11-17 installed.");
            s.error = Some("offline".into());
            s.version = Some(now() - OUT_OF_DATE - 60);
            assert_eq!(s.summary(), "Lists from 2026-11-17 installed. Blocking lists are out of date. Last update failed: offline.");
        }
    }
}
