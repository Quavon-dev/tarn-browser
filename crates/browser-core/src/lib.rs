#![forbid(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use engine::Event;
use shield::ShieldLevel;
use shield::matcher::Matcher;
use store::{Id, Profile, Store, WebEngine};
use url::Url;

pub mod cleanup;
pub mod closed;
pub mod command;
#[cfg(target_os = "macos")]
pub mod container;
pub mod data;
pub mod effects;
pub mod find;
pub mod fog;
pub mod github;
pub mod import;
pub mod permissions;
pub mod prompts;
pub mod routing;
pub mod schedule;
pub mod shield_card;
pub mod site;
pub mod snapshot;
pub mod split;
#[cfg(any(target_os = "macos", windows))]
pub mod staleness;
pub mod suggest;
pub mod tabs;
pub mod text;
pub mod theme;
pub mod theme_fetch;
pub mod theme_library;
pub mod windows;

pub use data::WebsiteData;

#[cfg(test)]
pub(crate) type TestCore = Core<data::fake::Memory>;
pub use site::{Overrides, Sets, SiteRules, effective_level};

pub const LICENCE: &str = "GNU GPL v3 or later";
pub const SOURCE_URL: &str = env!("CARGO_PKG_REPOSITORY");
pub const LICENCE_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/blob/main/LICENSE");

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Where Tarn keeps its data: `TARN_DATA_DIR`, else `base`, else the platform's default. iOS and
/// Android have none, so their host passes `base`.
pub fn data_dir(base: Option<&Path>) -> Option<PathBuf> {
    let platform = if cfg!(windows) { "LOCALAPPDATA" } else { "XDG_DATA_HOME" };
    resolve_data_dir(std::env::var_os("TARN_DATA_DIR"), base, std::env::home_dir(), std::env::var_os(platform))
}

/// `xdg` is `XDG_DATA_HOME` on Linux and `LOCALAPPDATA` on Windows.
#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
fn resolve_data_dir(tarn: Option<OsString>, base: Option<&Path>, home: Option<PathBuf>, xdg: Option<OsString>) -> Option<PathBuf> {
    if let Some(dir) = tarn {
        return Some(dir.into());
    }
    if let Some(base) = base {
        return Some(base.to_owned());
    }
    #[cfg(target_os = "macos")]
    return Some(home.unwrap_or_default().join("Library/Application Support/Tarn"));
    #[cfg(target_os = "linux")]
    return xdg.map(PathBuf::from).filter(|d| d.is_absolute()).or_else(|| Some(home?.join(".local/share"))).map(|d| d.join("tarn"));
    #[cfg(windows)]
    return xdg.map(PathBuf::from).filter(|d| d.is_absolute()).map(|d| d.join("Tarn"));
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    None
}

pub fn shared_key(id: Id) -> String {
    format!("shared_store.{id}")
}

/// A profile created with "Share data" uses the other profile's website data store.
pub fn share_store(disk: &Store, profile: &mut Profile) {
    if let Some(uuid) = disk.setting(&shared_key(profile.id)).ok().flatten().filter(|u| !u.is_empty()) {
        profile.data_store_uuid = uuid;
    }
}

pub struct ProfileRuntime<D> {
    pub profile: Profile,
    pub data: D,
    pub overrides: Overrides,
    pub rules: SiteRules,
    pub own_store: String,
}

/// App-level state shared by every window. Fields change through `Cell`/`RefCell`, so callers
/// hold a borrow only for the statement that needs it and never across a call into a window.
pub struct Core<D: WebsiteData> {
    pub disk: Store,
    memory: Store,
    /// Why the profile data is not what the user expects: the database could not be opened, or
    /// the sandboxed app's data stayed in its container.
    pub storage_warning: Option<String>,
    pub profiles: RefCell<HashMap<(bool, Id), ProfileRuntime<D>>>,
    /// Data stores of deleted or wiped profiles, kept alive until the engine has removed them.
    pub retired_stores: RefCell<Vec<D>>,
    /// Downloads started before a profile was wiped are not recorded in it again.
    pub wiped_at: RefCell<HashMap<(bool, Id), i64>>,
    pub rule_lists: RefCell<HashMap<&'static str, D::RuleLists>>,
    /// Domain sets of the switched-on categories, shared with every view's `SiteFor`.
    pub sets: Rc<RefCell<Sets>>,
    pub youtube_script: RefCell<String>,
    /// Both groups of the installed bundle, for Chromium's request filter.
    pub matcher: RefCell<Matcher>,
    /// The profile whose choices Chromium's request filter reads, once it opened a Chromium view.
    pub chromium_profile: Cell<Option<Id>>,
    /// The `default_web_engine` setting.
    pub default_web_engine: Cell<WebEngine>,
    #[cfg(any(target_os = "macos", windows))]
    pub updates: Cell<staleness::Updates>,
    /// The result of the last `check_chromium`.
    #[cfg(any(target_os = "macos", windows))]
    pub chromium_staleness: Cell<staleness::Staleness>,
    /// The last judged time and the monotonic instant it was judged at.
    #[cfg(any(target_os = "macos", windows))]
    judged: Cell<Option<(i64, std::time::Instant)>>,
    /// Profiles of the private store that each hold one window's burner tab.
    pub burners: RefCell<HashSet<Id>>,
}

impl<D: WebsiteData> Core<D> {
    /// `chromium` says whether this build can run Chromium; without it the default stays WebKit.
    pub fn open(dir: &Path, chromium: bool) -> Self {
        let (disk, storage_warning) = match Store::open(dir) {
            Ok(store) => (store, None),
            Err(e) => (Store::open_in_memory().expect("in-memory store"), Some(format!("Profile data could not be opened ({e}); nothing will be saved."))),
        };
        if disk.profiles().map(|p| p.is_empty()).unwrap_or(false) {
            let uuid = uuid::Uuid::new_v4().to_string();
            if let Ok(p) = disk.create_profile("Personal", &uuid, false) {
                disk.create_space(p.id, "Home", "").ok();
            }
        }
        #[cfg(any(target_os = "macos", windows))]
        let updates = staleness::Updates::at_launch(
            now(),
            disk.setting(staleness::FEED_VERIFIED).ok().flatten().and_then(|v| v.parse().ok()),
            disk.setting(staleness::CRITICAL_FOUND).ok().flatten().and_then(|v| staleness::stored_critical(&v, env!("CARGO_PKG_VERSION"))),
        );
        let default_web_engine =
            disk.setting(site::DEFAULT_WEB_ENGINE).ok().flatten().and_then(|v| WebEngine::parse(&v)).filter(|_| chromium).unwrap_or(WebEngine::WebKit);
        Core {
            disk,
            memory: Store::open_in_memory().expect("in-memory store"),
            storage_warning,
            profiles: RefCell::default(),
            retired_stores: RefCell::default(),
            wiped_at: RefCell::default(),
            rule_lists: RefCell::default(),
            sets: Rc::default(),
            youtube_script: RefCell::new(shield::youtube::script(&shield::youtube::Config::default())),
            matcher: RefCell::default(),
            chromium_profile: Cell::new(None),
            default_web_engine: Cell::new(default_web_engine),
            #[cfg(any(target_os = "macos", windows))]
            updates: Cell::new(updates),
            #[cfg(any(target_os = "macos", windows))]
            chromium_staleness: Cell::new(staleness::Staleness::Unavailable(staleness::Reason::FeedNeverVerified)),
            #[cfg(any(target_os = "macos", windows))]
            judged: Cell::new(None),
            burners: RefCell::default(),
        }
    }

    pub fn store(&self, private: bool) -> &Store {
        if private { &self.memory } else { &self.disk }
    }

    /// Whether the profile keeps history, site icons and downloads; a burner keeps nothing.
    pub fn keeps_records(&self, private: bool, profile_id: Id) -> bool {
        !(private && self.burners.borrow().contains(&profile_id))
    }

    /// The Shield level for `url` under an opened profile.
    pub fn level(&self, private: bool, profile_id: Id, url: Option<&Url>) -> Option<ShieldLevel> {
        self.profiles.borrow().get(&(private, profile_id)).map(|rt| effective_level(&rt.profile, &rt.overrides, url))
    }

    /// An address the user opens (typed, completed or from another app) without the tracking
    /// parameters its Shield level strips; links from pages are left alone.
    pub fn clean_user_url(&self, profile: Option<(bool, Id)>, url: Url) -> Url {
        let level = profile.and_then(|(private, id)| self.level(private, id, Some(&url))).unwrap_or_default();
        if level.policy().strip_tracking_params { shield::url_clean::strip_tracking_params(&url).unwrap_or(url) } else { url }
    }

    /// Allows, blocks or (with `None`) forgets a remembered permission; a pop-up rule also reaches
    /// the views of the profile if it is open.
    pub fn set_permission(&self, private: bool, profile_id: Id, origin: &str, kind: &str, allow: Option<bool>) -> Result<(), store::Error> {
        permissions::set(self.store(private), profile_id, origin, kind, allow)?;
        if kind == site::POPUPS
            && let Some(rt) = self.profiles.borrow().get(&(private, profile_id))
        {
            let mut popups = rt.rules.popups.borrow_mut();
            match allow {
                None => {
                    popups.remove(origin);
                }
                Some(allow) => {
                    popups.insert(origin.to_owned(), if allow { engine::nav_policy::PopupRule::Allow } else { engine::nav_policy::PopupRule::Block });
                }
            }
        }
        Ok(())
    }

    /// As `clean_user_url`, and counts the parameters taken out on the profile's Shield card.
    pub fn clean_counted_url(&self, profile: Option<(bool, Id)>, url: Url) -> Url {
        let clean = self.clean_user_url(profile, url.clone());
        let removed = shield::url_clean::removed_params(&url, &clean);
        if let Some((private, id)) = profile.filter(|(private, id)| removed > 0 && self.keeps_records(*private, *id)) {
            self.store(private).count(id, store::Counter::TrackingParams, removed, site::site_host(&clean)).ok();
        }
        clean
    }

    /// Whether an opened profile has its own level for `host`.
    pub fn has_override(&self, private: bool, profile_id: Id, host: &str) -> bool {
        self.profiles.borrow().get(&(private, profile_id)).is_some_and(|rt| rt.overrides.borrow().contains_key(host.trim_end_matches('.')))
    }

    /// The engine for a tab of an opened profile asking for `tab` and showing `url`.
    pub fn web_engine(&self, private: bool, profile_id: Id, tab: Option<WebEngine>, url: Option<&Url>) -> Option<WebEngine> {
        let profiles = self.profiles.borrow();
        let rt = profiles.get(&(private, profile_id))?;
        Some(site::web_engine(&rt.profile, &rt.overrides, tab, self.default_web_engine.get(), url))
    }

    /// True when the runtime was created by this call.
    pub fn open_profile(&self, private: bool, profile_id: Id, downloads: impl FnOnce() -> Rc<dyn Fn(Event)>) -> Option<bool> {
        if self.profiles.borrow().contains_key(&(private, profile_id)) {
            return Some(false);
        }
        let store = self.store(private);
        let mut profile = store.profile(profile_id).ok()?;
        let own_store = profile.data_store_uuid.clone();
        if !private {
            share_store(store, &mut profile);
        }
        let overrides = store.site_overrides(profile_id).unwrap_or_default().into_iter().collect();
        let rules = SiteRules::load(store, profile_id);
        // Lockdown keeps website data in memory only and empties the profile's persistent store,
        // so nothing a Lockdown session stored survives a quit.
        let data = if private {
            D::ephemeral(downloads())?
        } else if profile.shield_level == ShieldLevel::Lockdown {
            D::wipe_persistent(&profile.data_store_uuid);
            D::ephemeral(downloads())?
        } else {
            D::persistent(&profile.data_store_uuid, &own_store, downloads())?
        };
        let runtime = ProfileRuntime { profile, data, overrides: Rc::new(RefCell::new(overrides)), rules, own_store };
        self.profiles.borrow_mut().insert((private, profile_id), runtime);
        Some(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_store_replaces_the_uuid_only_when_set() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_profile("A", "uuid-a", false).unwrap();
        let mut b = store.create_profile("B", "uuid-b", false).unwrap();
        share_store(&store, &mut b);
        assert_eq!(b.data_store_uuid, "uuid-b");
        store.set_setting(&shared_key(b.id), &a.data_store_uuid).unwrap();
        share_store(&store, &mut b);
        assert_eq!(b.data_store_uuid, "uuid-a");
        let mut c = store.create_profile("C", "uuid-c", false).unwrap();
        store.set_setting(&shared_key(c.id), "").unwrap();
        share_store(&store, &mut c);
        assert_eq!(c.data_store_uuid, "uuid-c");
    }

    #[test]
    fn a_fresh_database_gets_one_profile_with_one_space() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        assert!(core.storage_warning.is_none());
        let profiles = core.disk.profiles().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(core.disk.spaces(profiles[0].id).unwrap().len(), 1);
        drop(core);
        assert_eq!(TestCore::open(dir.path(), false).disk.profiles().unwrap().len(), 1, "a reopened database is not seeded again");
        assert!(TestCore::open(dir.path(), false).store(true).profiles().unwrap().is_empty(), "the private store starts empty");
    }

    #[test]
    fn data_dir_prefers_the_override_then_the_host_then_the_platform() {
        let resolve = |tarn: Option<&str>, base: Option<&str>, home: Option<&str>, xdg: Option<&str>| {
            resolve_data_dir(tarn.map(OsString::from), base.map(Path::new), home.map(PathBuf::from), xdg.map(OsString::from))
        };
        assert_eq!(resolve(Some("/t"), Some("/b"), Some("/h"), None), Some("/t".into()));
        assert_eq!(resolve(None, Some("/b"), Some("/h"), None), Some("/b".into()));
        let default = resolve(None, None, Some("/h"), Some("/x"));
        if cfg!(target_os = "macos") {
            assert_eq!(default, Some("/h/Library/Application Support/Tarn".into()));
        } else if cfg!(target_os = "linux") {
            assert_eq!(default, Some("/x/tarn".into()));
            assert_eq!(resolve(None, None, Some("/h"), Some("rel")), Some("/h/.local/share/tarn".into()), "a relative XDG_DATA_HOME is ignored");
            assert_eq!(resolve(None, None, Some("/h"), None), Some("/h/.local/share/tarn".into()));
            assert_eq!(resolve(None, None, None, None), None);
        } else if cfg!(windows) {
            assert_eq!(resolve(None, None, Some("C:\\h"), Some("C:\\l")), Some(PathBuf::from("C:\\l").join("Tarn")));
            assert_eq!(resolve(None, None, Some("C:\\h"), Some("rel")), None, "a relative LOCALAPPDATA is ignored");
        } else {
            assert_eq!(default, None, "the host passes the directory");
        }
    }

    fn downloads() -> Rc<dyn Fn(Event)> {
        Rc::new(|_| {})
    }

    #[test]
    fn a_changed_popup_permission_reaches_the_open_profile() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let profile = core.disk.profiles().unwrap().remove(0);
        core.open_profile(false, profile.id, downloads);
        let rule = |core: &TestCore| core.profiles.borrow()[&(false, profile.id)].rules.popups.borrow().get("https://a.test").copied();
        core.set_permission(false, profile.id, "https://a.test", site::POPUPS, Some(true)).unwrap();
        assert_eq!(rule(&core), Some(engine::nav_policy::PopupRule::Allow));
        core.set_permission(false, profile.id, "https://a.test", site::POPUPS, None).unwrap();
        assert_eq!(rule(&core), None);
        assert!(core.disk.permissions(profile.id).unwrap().is_empty());
    }

    #[test]
    fn typed_addresses_lose_tracking_parameters_and_count_them() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let profile = core.disk.profiles().unwrap().remove(0);
        core.open_profile(false, profile.id, downloads);
        let url = Url::parse("https://shop.test/item?id=4&utm_source=x&fbclid=y").unwrap();
        let clean = core.clean_counted_url(Some((false, profile.id)), url);
        assert_eq!(clean.as_str(), "https://shop.test/item?id=4");
        assert_eq!(core.disk.shield_stats(profile.id).unwrap().tracking_params, 2);
        core.clean_counted_url(Some((false, profile.id)), clean);
        assert_eq!(core.disk.shield_stats(profile.id).unwrap().tracking_params, 2, "nothing removed, nothing counted");
    }

    #[test]
    fn profiles_open_once_with_the_data_their_level_allows() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let normal = core.disk.profiles().unwrap().remove(0);
        assert_eq!(core.open_profile(false, normal.id, downloads), Some(true));
        assert_eq!(core.open_profile(false, normal.id, || panic!("an open profile is reused")), Some(false));
        assert_eq!(core.profiles.borrow()[&(false, normal.id)].data.persistent.as_deref(), Some(normal.data_store_uuid.as_str()));
        let locked = core.disk.create_profile("L", "uuid-l", false).unwrap();
        core.disk.set_profile_shield_level(locked.id, ShieldLevel::Lockdown).unwrap();
        core.open_profile(false, locked.id, downloads).unwrap();
        assert!(core.profiles.borrow()[&(false, locked.id)].data.persistent.is_none(), "Lockdown keeps data in memory");
        assert!(data::fake::WIPED.with(|w| w.borrow().contains(&"uuid-l".to_owned())), "and empties its persistent store");
        let private = core.store(true).create_profile("P", "uuid-p", true).unwrap();
        core.open_profile(true, private.id, downloads).unwrap();
        assert!(core.profiles.borrow()[&(true, private.id)].data.persistent.is_none());
        assert_eq!(core.open_profile(true, normal.id + 100, downloads), None, "an unknown profile");
    }

    #[test]
    fn a_shared_profile_opens_the_other_store_and_keeps_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), false);
        let a = core.disk.profiles().unwrap().remove(0);
        let b = core.disk.create_profile("B", "uuid-b", false).unwrap();
        core.disk.set_setting(&shared_key(b.id), &a.data_store_uuid).unwrap();
        core.open_profile(false, b.id, downloads).unwrap();
        let profiles = core.profiles.borrow();
        let rt = &profiles[&(false, b.id)];
        assert_eq!(rt.data.persistent.as_deref(), Some(a.data_store_uuid.as_str()));
        assert_eq!(rt.own_store, "uuid-b");
    }

    #[test]
    fn the_default_engine_is_webkit_unless_the_build_has_chromium() {
        let dir = tempfile::tempdir().unwrap();
        TestCore::open(dir.path(), false).disk.set_setting(site::DEFAULT_WEB_ENGINE, "chromium").unwrap();
        assert_eq!(TestCore::open(dir.path(), false).default_web_engine.get(), WebEngine::WebKit);
        assert_eq!(TestCore::open(dir.path(), true).default_web_engine.get(), WebEngine::Chromium);
    }

    #[test]
    fn levels_and_engines_follow_the_opened_profile() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), true);
        core.default_web_engine.set(WebEngine::Chromium);
        let p = core.disk.profiles().unwrap().remove(0).id;
        let url = Url::parse("https://locked.test./").unwrap();
        assert_eq!(core.level(false, p, Some(&url)), None, "not opened yet");
        assert_eq!(core.web_engine(false, p, None, Some(&url)), None);
        core.disk.set_site_override(p, "locked.test", Some(ShieldLevel::Lockdown)).unwrap();
        core.open_profile(false, p, downloads).unwrap();
        assert_eq!(core.level(false, p, Some(&url)), Some(ShieldLevel::Lockdown));
        assert!(core.has_override(false, p, "locked.test.") && !core.has_override(false, p, "other.test"));
        assert_eq!(core.web_engine(false, p, Some(WebEngine::Chromium), Some(&url)), Some(WebEngine::WebKit), "Lockdown runs in WebKit");
        assert_eq!(core.web_engine(false, p, None, Url::parse("https://a.test/").ok().as_ref()), Some(WebEngine::Chromium));
    }

    #[test]
    fn user_addresses_lose_tracking_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let core = TestCore::open(dir.path(), true);
        let p = core.disk.profiles().unwrap().remove(0).id;
        core.open_profile(false, p, downloads).unwrap();
        let url = Url::parse("https://a.test/p?id=7&utm_source=x").unwrap();
        assert_eq!(core.clean_user_url(Some((false, p)), url.clone()).as_str(), "https://a.test/p?id=7");
        assert_eq!(core.clean_user_url(None, url).as_str(), "https://a.test/p?id=7");
    }
}
