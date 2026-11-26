use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use block2::{DynBlock, RcBlock};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message};
use std::ptr::NonNull;

use objc2_foundation::{NSArray, NSDate, NSError, NSProcessInfo, NSSet, NSString, NSUUID};
use objc2_web_kit::{
    WKContentRuleList, WKContentRuleListStore, WKContentWorld, WKUserContentController, WKUserScript, WKUserScriptInjectionTime, WKWebExtensionController,
    WKWebsiteDataRecord, WKWebsiteDataStore, WKWebsiteDataTypeDiskCache, WKWebsiteDataTypeFetchCache, WKWebsiteDataTypeMemoryCache,
};
use shield::{ShieldLevel, rules};

use crate::Event;
use crate::cookies::Cookie;
use crate::cookies_webkit::WebKitCookies;
use crate::delegates::DownloadDelegate;
use crate::extensions::{self, Enabled};

/// One profile's WebKit storage plus two content controllers per Shield level: one with the ad
/// and tracker rules, one without for sites where the user paused them.
///
/// Controllers are shared by every web view of the profile at that level, so a rule list that
/// finishes compiling after a view was created still applies to it.
pub struct DataStore {
    pub(crate) store: Retained<WKWebsiteDataStore>,
    pub(crate) downloads: Retained<DownloadDelegate>,
    /// `[level][filtered]`.
    controllers: [[Retained<WKUserContentController>; 2]; 3],
    /// Installed rule lists by group, and the controllers they go into.
    rule_lists: RefCell<HashMap<String, (FilterLists, Attach)>>,
    page_script: RefCell<Option<String>>,
    /// The user's per-host style sheets as one client-world script (`site_styles_script`).
    site_styles: RefCell<Option<String>>,
    /// WebKit's fraudulent-website warning (Safe Browsing through Apple) in views created from now on.
    safe_browsing: Cell<bool>,
    id: Option<Retained<NSUUID>>,
    extensions: RefCell<Option<extensions::Profile>>,
    /// Per level, the built-in rules only, for extension pages and workers.
    extension_rules: [Retained<WKUserContentController>; 3],
    extension_level: Cell<ShieldLevel>,
    passkeys: Cell<bool>,
    /// Per level, whether `tarn.builtin.*` (private network, HTTPS upgrade) is attached.
    builtin: [Builtin; 3],
    /// Per level, that list once compiled, for Tarn extension pages.
    builtin_lists: [BuiltinList; 3],
    /// Tarn extensions' content scripts, in the controllers of every level but Lockdown.
    tarn: Rc<crate::tarn_webkit::Worlds>,
    pub(crate) cert_exceptions: crate::cert::Exceptions,
}

pub(crate) type BuiltinList = Rc<RefCell<Option<Retained<WKContentRuleList>>>>;

/// Whether a level's built-in rule list is attached; views hold their first load until it is.
#[derive(Clone, Default)]
pub(crate) struct Builtin(Rc<RefCell<BuiltinState>>);

type BuiltinJob = Box<dyn FnOnce(Result<(), &str>)>;

enum BuiltinState {
    Waiting(Vec<BuiltinJob>),
    Ready,
    Failed(String),
}

impl Default for BuiltinState {
    fn default() -> Self {
        BuiltinState::Waiting(Vec::new())
    }
}

impl Builtin {
    /// For a popup, whose opener loaded only once the list was attached.
    pub(crate) fn ready() -> Self {
        Builtin(Rc::new(RefCell::new(BuiltinState::Ready)))
    }

    /// Runs `job` once the list is attached, at once if it already is; with the error if it failed.
    pub(crate) fn then(&self, job: impl FnOnce(Result<(), &str>) + 'static) {
        let failed = match &mut *self.0.borrow_mut() {
            BuiltinState::Waiting(jobs) => {
                jobs.push(Box::new(job));
                return;
            }
            BuiltinState::Ready => None,
            BuiltinState::Failed(error) => Some(error.clone()),
        };
        job(failed.as_deref().map_or(Ok(()), Err));
    }

    fn settle(&self, result: Result<(), String>) {
        let state = match &result {
            Ok(()) => BuiltinState::Ready,
            Err(error) => BuiltinState::Failed(error.clone()),
        };
        let previous = std::mem::replace(&mut *self.0.borrow_mut(), state);
        if let BuiltinState::Waiting(jobs) = previous {
            for job in jobs {
                job(result.as_ref().map(|_| ()).map_err(String::as_str));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attach {
    Always,
    Unpaused,
    OutsideLockdown,
}

impl Attach {
    fn applies(self, level: ShieldLevel, filtered: bool) -> bool {
        match self {
            Attach::Always => true,
            Attach::Unpaused => filtered,
            Attach::OutsideLockdown => level != ShieldLevel::Lockdown,
        }
    }
}

impl DataStore {
    /// Returns `None` off the main thread or for a malformed UUID string. `extensions` names the
    /// profile's own extension storage, so profiles sharing website data keep theirs apart.
    /// `downloads` receives this profile's download events.
    pub fn persistent(uuid: &str, extensions: &str, downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let id = NSUUID::from_string(&NSString::from_str(uuid))?;
        let extensions = NSUUID::from_string(&NSString::from_str(extensions))?;
        // SAFETY: called on the main thread (marker); `id` is a valid NSUUID.
        let store = unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&id, mtm) };
        Some(Self::new(mtm, store, Some(extensions), downloads))
    }

    /// Returns `None` off the main thread.
    pub fn ephemeral(downloads: Rc<dyn Fn(Event)>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        // SAFETY: called on the main thread (marker).
        Some(Self::new(mtm, unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) }, None, downloads))
    }

    fn new(mtm: MainThreadMarker, store: Retained<WKWebsiteDataStore>, id: Option<Retained<NSUUID>>, downloads: Rc<dyn Fn(Event)>) -> Self {
        let controllers = ShieldLevel::ALL.map(|level| {
            [false, true].map(|filtered| {
                // SAFETY: plain allocation on the main thread.
                let controller = unsafe { WKUserContentController::new(mtm) };
                install_scripts(mtm, &controller, level, filtered, None, None, false);
                controller
            })
        });
        let downloads = DownloadDelegate::new(mtm, downloads);
        let extension_levels = ShieldLevel::ALL.into_iter().filter(|l| *l != ShieldLevel::Lockdown);
        let tarn = crate::tarn_webkit::Worlds::new(mtm, extension_levels.flat_map(|l| controllers[l as usize].iter().cloned()).collect());
        let this = DataStore {
            store,
            downloads,
            controllers,
            rule_lists: RefCell::default(),
            page_script: RefCell::new(None),
            site_styles: RefCell::new(None),
            safe_browsing: Cell::new(true),
            id,
            extensions: RefCell::new(None),
            // SAFETY: plain allocation on the main thread.
            extension_rules: ShieldLevel::ALL.map(|_| unsafe { WKUserContentController::new(mtm) }),
            extension_level: Cell::new(ShieldLevel::Standard),
            passkeys: Cell::new(false),
            builtin: Default::default(),
            builtin_lists: Default::default(),
            tarn,
            cert_exceptions: crate::cert::Exceptions::default(),
        };
        this.compile_builtin_rules(mtm);
        this
    }

    /// The persistent store's UUID; `None` for an ephemeral store.
    #[cfg(feature = "chromium")]
    pub(crate) fn identifier(&self) -> Option<String> {
        // SAFETY: main-thread read of a property of the live store.
        unsafe { self.store.identifier() }.map(|id| id.UUIDString().to_string())
    }

    pub(crate) fn builtin(&self, level: ShieldLevel) -> Builtin {
        self.builtin[level as usize].clone()
    }

    pub(crate) fn builtin_list(&self, level: ShieldLevel) -> BuiltinList {
        self.builtin_lists[level as usize].clone()
    }

    pub(crate) fn controller(&self, level: ShieldLevel, filtered: bool) -> &WKUserContentController {
        &self.controllers[level as usize][usize::from(filtered)]
    }

    /// The profile's certificate exceptions; views read them on each server-trust failure.
    pub fn cert_exceptions(&self) -> crate::cert::Exceptions {
        self.cert_exceptions.clone()
    }

    pub fn set_safe_browsing(&self, on: bool) {
        self.safe_browsing.set(on);
    }

    /// New downloads go into a Downloads subfolder by file type.
    pub fn set_sort_downloads(&self, on: bool) {
        self.downloads.ivars().sort.set(on);
    }

    /// `id` is from `Event::DownloadProgress`; the data to continue from stays in memory.
    pub fn pause_download(&self, id: u64) {
        self.downloads.pause(id);
    }

    pub fn resume_download(&self, id: u64) {
        self.downloads.resume(id, &self.store);
    }

    /// Stops download `id`, saving or paused, and removes its part; `Event::DownloadCancelled` follows.
    pub fn cancel_download(&self, id: u64) {
        self.downloads.cancel(id);
    }

    /// For quit: quarantines the parts of saving downloads and removes the others with theirs.
    pub fn quit_downloads(&self) {
        self.downloads.quit();
    }

    pub(crate) fn safe_browsing(&self) -> bool {
        self.safe_browsing.get()
    }

    fn each_controller(&self) -> impl Iterator<Item = (ShieldLevel, bool, &WKUserContentController)> {
        ShieldLevel::ALL.into_iter().flat_map(move |level| [false, true].map(|filtered| (level, filtered, self.controller(level, filtered))))
    }

    /// Sets the page-world script of filtered views (the YouTube script), replacing the previous
    /// one; open pages get it on their next load.
    pub fn set_page_script(&self, source: Option<&str>) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        if self.page_script.borrow().as_deref() == source {
            return;
        }
        *self.page_script.borrow_mut() = source.map(str::to_owned);
        self.reinstall_scripts(mtm);
    }

    /// Sets the style sheets by host pattern (`page_scripts::site_styles_script`), replacing the previous
    /// ones; open pages get them on their next load, Chromium views when they are created. True when they changed.
    pub fn set_site_styles(&self, styles: &[(String, String)]) -> bool {
        let Some(mtm) = MainThreadMarker::new() else { return false };
        let script = crate::page_scripts::site_styles_script(styles);
        if *self.site_styles.borrow() == script {
            return false;
        }
        *self.site_styles.borrow_mut() = script;
        self.reinstall_scripts(mtm);
        true
    }

    #[cfg(feature = "chromium")]
    pub(crate) fn site_styles(&self) -> Option<String> {
        self.site_styles.borrow().clone()
    }

    fn reinstall_scripts(&self, mtm: MainThreadMarker) {
        let (page_script, site_styles) = (self.page_script.borrow(), self.site_styles.borrow());
        for (level, filtered, controller) in self.each_controller() {
            install_scripts(mtm, controller, level, filtered, page_script.as_deref(), site_styles.as_deref(), self.passkeys.get());
        }
        self.tarn.reinstalled();
    }

    /// Runs `enabled` in this profile's web views created from now on, with storage kept per
    /// profile (in memory for an ephemeral store), and never on `blocked` hosts. The controller
    /// is created on first use.
    /// Extension pages and workers get the built-in rules of `level`; a new level rebuilds the
    /// controller, since WebKit copies its configuration.
    pub fn set_extensions(&self, enabled: &[Enabled], blocked: &[String], level: ShieldLevel) {
        let mut slot = self.extensions.borrow_mut();
        if self.extension_level.replace(level) != level {
            slot.take();
        }
        let Some(mtm) = MainThreadMarker::new() else { return };
        if slot.is_some() || !enabled.is_empty() {
            let rules = &self.extension_rules[level as usize];
            slot.get_or_insert_with(|| extensions::Profile::new(mtm, self.id.as_deref(), &self.store, rules)).set_enabled(enabled, blocked);
        }
        drop(slot);
        let bitwarden = enabled.iter().any(Enabled::privileged);
        if self.passkeys.replace(bitwarden) != bitwarden {
            self.reinstall_scripts(mtm);
        }
    }

    /// Deletes what the extension `key` stored in this profile and unloads it.
    pub fn remove_extension(&self, key: &str, done: impl FnOnce() + 'static) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let level = self.extension_level.get();
        let rules = &self.extension_rules[level as usize];
        self.extensions.borrow_mut().get_or_insert_with(|| extensions::Profile::new(mtm, self.id.as_deref(), &self.store, rules)).remove(key, done);
    }

    /// Keys of the extensions that actually loaded, which may be fewer than were enabled.
    pub fn running_extensions(&self) -> Vec<String> {
        self.extensions.borrow().as_ref().map(extensions::Profile::running).unwrap_or_default()
    }

    pub fn open_extension_popup(&self, extension: &extensions::Extension) {
        if let Some(profile) = &*self.extensions.borrow() {
            profile.open_popup(extension);
        }
    }

    /// Unloads this profile's extensions and removes what they stored, in memory or on disk; they
    /// run again after the next `set_extensions`. `done` runs once WebKit finished.
    pub fn remove_extension_data(&self, done: impl FnOnce() + 'static) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let running = self.extensions.borrow_mut().as_mut().map(|profile| {
            extensions::forget_session(&profile.seed, None);
            profile.set_enabled(&[], &[]);
            profile.controller.clone()
        });
        if self.passkeys.replace(false) {
            self.reinstall_scripts(mtm);
        }
        match running.or_else(|| self.id.as_deref().map(|id| extensions::persistent_controller(mtm, id))) {
            Some(controller) => extensions::remove_data(controller, done),
            None => done(),
        }
    }

    /// Removes what extensions stored under `uuid` for a profile whose extensions do not run, then
    /// the folder WebKit keeps for it, whose `State.plist` would still name each extension.
    pub fn wipe_extensions(uuid: &str) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(id) = NSUUID::from_string(&NSString::from_str(uuid)) else { return };
        let folders = extension_folders(&id.UUIDString().to_string());
        extensions::remove_data(extensions::persistent_controller(mtm, &id), move || {
            for folder in &folders {
                std::fs::remove_dir_all(folder).ok();
            }
        });
    }

    pub(crate) fn extension_controller(&self) -> Option<Retained<WKWebExtensionController>> {
        self.extensions.borrow().as_ref().map(|p| p.controller.clone())
    }

    /// Writes cookies from another browser into this profile's WebKit jar. `done` gets how many
    /// were written and how many were refused by `cookies::admit_imported` or not storable with
    /// every attribute intact. The cookie bridge carries them to Chromium on the site's next load.
    pub fn import_cookies(&self, cookies: Vec<Cookie>, now: i64, done: impl FnOnce(usize, usize) + 'static) {
        let jar = Rc::new(WebKitCookies::new(self));
        jar.clone().read(move |existing| {
            let (valid, refused) = crate::cookies::admit_imported(cookies, &existing, now);
            if valid.is_empty() {
                return done(0, refused);
            }
            let state = Rc::new(Cell::new((valid.len(), 0, refused)));
            let done = Rc::new(RefCell::new(Some(done)));
            for cookie in &valid {
                let (state, done) = (state.clone(), done.clone());
                jar.set(cookie, move |written| {
                    let (pending, ok, refused) = state.get();
                    let next = (pending - 1, ok + usize::from(written), refused + usize::from(!written));
                    state.set(next);
                    if next.0 == 0
                        && let Some(done) = done.take()
                    {
                        done(next.1, next.2);
                    }
                });
            }
        });
    }

    /// Every cookie in this profile's WebKit jar.
    pub fn cookies(&self, done: impl FnOnce(Vec<Cookie>) + 'static) {
        WebKitCookies::new(self).read(done);
    }

    /// Empties a profile's persistent store without building a full `DataStore` for it.
    pub fn wipe_persistent(uuid: &str) {
        #[cfg(feature = "chromium")]
        crate::chromium::cookies::suspend(uuid, None);
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(id) = NSUUID::from_string(&NSString::from_str(uuid)) else { return };
        // SAFETY: main thread; `id` is a valid NSUUID.
        let store = unsafe { WKWebsiteDataStore::dataStoreForIdentifier(&id, mtm) };
        // SAFETY: main thread.
        remove_all(store, unsafe { WKWebsiteDataStore::allWebsiteDataTypes(mtm) }, || {});
    }

    /// Deletes the store folder WebKit keeps for `uuid`; `done` learns whether it is gone. WebKit
    /// refuses while a web view or a download still uses the store.
    pub fn delete_persistent(uuid: &str, done: impl FnOnce(bool) + 'static) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(id) = NSUUID::from_string(&NSString::from_str(uuid)) else { return done(true) };
        let done = RefCell::new(Some(done));
        let block = RcBlock::new(move |error: *mut NSError| {
            if let Some(done) = done.take() {
                done(error.is_null());
            }
        });
        // SAFETY: main thread; `id` is a valid NSUUID and WebKit copies the block.
        unsafe { WKWebsiteDataStore::removeDataStoreForIdentifier_completionHandler(&id, &block, mtm) };
    }

    pub fn remove_all_data(&self, done: impl FnOnce() + 'static) {
        #[cfg(feature = "chromium")]
        if let Some(store) = self.identifier() {
            crate::chromium::cookies::suspend(&store, None);
        }
        let Some(mtm) = MainThreadMarker::new() else { return };
        // SAFETY: main thread.
        remove_all(self.store.clone(), unsafe { WKWebsiteDataStore::allWebsiteDataTypes(mtm) }, done);
    }

    pub fn clear_cache(&self) {
        // SAFETY: WebKit's exported constant strings.
        let types = unsafe { NSSet::from_slice(&[WKWebsiteDataTypeDiskCache, WKWebsiteDataTypeMemoryCache, WKWebsiteDataTypeFetchCache]) };
        remove_all(self.store.clone(), types, || {});
    }

    /// Removes every data type WebKit keeps for `host`'s site; `done` gets the number of sites cleared.
    pub fn clear_site_data(&self, host: &str, done: impl FnOnce(usize) + 'static) {
        #[cfg(feature = "chromium")]
        if let Some(store) = self.identifier() {
            crate::chromium::cookies::suspend(&store, Some(host));
        }
        let Some(mtm) = MainThreadMarker::new() else { return };
        // SAFETY: main thread.
        let types = unsafe { WKWebsiteDataStore::allWebsiteDataTypes(mtm) };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let store = self.store.clone();
        let done = RefCell::new(Some(done));
        let remove_types = types.clone();
        let handler = RcBlock::new(move |records: NonNull<NSArray<WKWebsiteDataRecord>>| {
            // SAFETY: WebKit passes a valid array for the duration of the call.
            let records = unsafe { records.as_ref() };
            let matching: Vec<Retained<WKWebsiteDataRecord>> = records
                .iter()
                // SAFETY: reading a property of a live record on the main thread.
                .filter(|r| site_matches(&host, &unsafe { r.displayName() }.to_string()))
                .collect();
            let count = matching.len();
            let Some(done) = done.take() else { return };
            let finished = RefCell::new(Some(done));
            let complete = RcBlock::new(move || {
                if let Some(done) = finished.take() {
                    done(count);
                }
            });
            // SAFETY: main thread; arguments are valid and WebKit copies the block.
            unsafe {
                store.removeDataOfTypes_forDataRecords_completionHandler(&remove_types, &NSArray::from_retained_slice(&matching), &complete);
            }
        });
        // SAFETY: main thread; WebKit copies the block.
        unsafe { self.store.fetchDataRecordsOfTypes_completionHandler(&types, &handler) };
    }

    /// Replaces the rule lists of `group` on every level, or removes them with `None`; views of
    /// this profile pick them up without a reload. A `pausable` group stays off in the controllers
    /// for paused sites.
    pub fn set_rule_lists(&self, group: &str, lists: Option<&FilterLists>, pausable: bool) {
        self.attach_rule_lists(group, lists, if pausable { Attach::Unpaused } else { Attach::Always });
    }

    /// `set_rule_lists` for a Tarn extension's network rules, which never act in Lockdown-level views.
    pub fn set_extension_rule_lists(&self, group: &str, lists: Option<&FilterLists>) {
        self.attach_rule_lists(group, lists, Attach::OutsideLockdown);
    }

    fn attach_rule_lists(&self, group: &str, lists: Option<&FilterLists>, attach: Attach) {
        let unchanged = match (lists, self.rule_lists.borrow().get(group)) {
            (Some(new), Some((old, was))) => Rc::ptr_eq(&new.0, &old.0) && *was == attach,
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return;
        }
        let previous = match lists {
            Some(lists) => self.rule_lists.borrow_mut().insert(group.to_owned(), (lists.clone(), attach)),
            None => self.rule_lists.borrow_mut().remove(group),
        };
        for (level, filtered, controller) in self.each_controller() {
            for list in previous.iter().flat_map(|(p, _)| p.0.iter()) {
                // SAFETY: main thread (the controllers were created there and are not Send).
                unsafe { controller.removeContentRuleList(list) };
            }
            for list in lists.into_iter().filter(|_| attach.applies(level, filtered)).flat_map(|l| l.0.iter()) {
                // SAFETY: as above.
                unsafe { controller.addContentRuleList(list) };
            }
        }
    }

    fn compile_builtin_rules(&self, mtm: MainThreadMarker) {
        // SAFETY: main thread; the default store exists for apps with a container or home dir.
        let Some(rule_store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else {
            for builtin in &self.builtin {
                builtin.settle(Err("WebKit has no content rule list store".into()));
            }
            return;
        };
        for level in ShieldLevel::ALL {
            let json = NSString::from_str(&rules::builtin_rules(&level.policy()));
            let id = NSString::from_str(&format!("tarn.builtin.{}", level.as_str()));
            let controllers: Vec<_> = self.controllers[level as usize].iter().chain([&self.extension_rules[level as usize]]).cloned().collect();
            let builtin = self.builtin(level);
            let kept = self.builtin_list(level);
            let done = RcBlock::new(move |list: *mut WKContentRuleList, error: *mut NSError| {
                // SAFETY: WebKit passes either a valid rule list or null, retained for the call.
                match unsafe { list.as_ref() } {
                    Some(list) => {
                        *kept.borrow_mut() = Some(list.retain());
                        for controller in &controllers {
                            // SAFETY: completion runs on the main thread, where the controllers live.
                            unsafe { controller.addContentRuleList(list) };
                        }
                        builtin.settle(Ok(()));
                        crate::startup::mark("built-in rules attached");
                    }
                    None => {
                        // SAFETY: WebKit passes either a valid error or null.
                        let message = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_default();
                        eprintln!("tarn: rule list {} failed to compile: {message}", level.as_str());
                        builtin.settle(Err(message));
                        BUILTIN_RULES_FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
            // SAFETY: all arguments are non-null and valid; the block is copied by WebKit.
            unsafe {
                rule_store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(Some(&id), Some(&json), Some(&done));
            }
        }
    }
}

static BUILTIN_RULES_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether a built-in rule list (private-network block, HTTPS upgrade) failed to compile, which
/// leaves those protections off; the shell says so instead of failing silently.
pub fn builtin_rules_failed() -> bool {
    BUILTIN_RULES_FAILED.load(std::sync::atomic::Ordering::Relaxed)
}

/// WebKit keeps extension storage beside website data: in `~/Library/WebKit` inside an app
/// container, in `~/Library/WebKit/<bundle id or process name>` outside one.
pub(crate) fn extension_folders(uuid: &str) -> Vec<PathBuf> {
    let Some(base) = std::env::home_dir().map(|home| home.join("Library/WebKit")) else { return Vec::new() };
    let name = crate::bundle_identifier().unwrap_or_else(|| NSProcessInfo::processInfo().processName().to_string());
    [base.clone(), base.join(name)].into_iter().map(|dir| dir.join("WebExtensions").join(uuid)).collect()
}

/// WebKit names a record after its registrable domain, so a host matches that name or a subdomain of it.
pub(crate) use crate::page::site_matches;

/// The completion block keeps `store` alive until WebKit is done with it.
fn remove_all(store: Retained<WKWebsiteDataStore>, types: Retained<NSSet<NSString>>, done: impl FnOnce() + 'static) {
    let done = RefCell::new(Some(done));
    let target = store.clone();
    let handler = RcBlock::new(move || {
        let _ = &store;
        if let Some(done) = done.take() {
            done();
        }
    });
    // SAFETY: main thread (callers hold a MainThreadMarker or a main-thread DataStore); the
    // type set and date are valid and WebKit copies the block.
    unsafe { target.removeDataOfTypes_modifiedSince_completionHandler(&types, &NSDate::distantPast(), &handler) };
}

const FILTER_PREFIX: &str = "tarn.lists.";

/// A controller's whole script set, rebuilt in one place: WebKit can only remove all user scripts
/// at once, and the referrer and passkey scripts must survive every change of the YouTube script.
fn install_scripts(
    mtm: MainThreadMarker,
    controller: &WKUserContentController,
    level: ShieldLevel,
    filtered: bool,
    page_script: Option<&str>,
    site_styles: Option<&str>,
    passkeys: bool,
) {
    // SAFETY: main thread (the controllers were created there and are not Send).
    unsafe { controller.removeAllUserScripts() };
    if level.policy().trim_referrer {
        // SAFETY: as above; the client world is out of page scripts' reach.
        unsafe { controller.addUserScript(&user_script(mtm, TRIM_REFERRER, &WKContentWorld::defaultClientWorld(mtm), false)) };
    }
    // Bitwarden does not run under Lockdown, so neither does its passkey check.
    if passkeys && level != ShieldLevel::Lockdown {
        // SAFETY: as above; the script has to change what the page itself sees.
        unsafe { controller.addUserScript(&user_script(mtm, PASSKEYS, &WKContentWorld::pageWorld(mtm), false)) };
    }
    if level.policy().javascript {
        // SAFETY: as above; the page must see the patched visibility. The script holds no bridge to the shell.
        unsafe { controller.addUserScript(&user_script(mtm, ALWAYS_VISIBLE, &WKContentWorld::pageWorld(mtm), false)) };
    }
    if let Some(source) = page_script.filter(|_| filtered && level.policy().javascript) {
        // SAFETY: as above. The page world is where the script must patch the page's own JSON
        // handling; it is Tarn's compiled-in script and holds no secret or bridge to the shell.
        unsafe { controller.addUserScript(&user_script(mtm, source, &WKContentWorld::pageWorld(mtm), false)) };
    }
    if let Some(source) = site_styles.filter(|_| level != ShieldLevel::Lockdown) {
        // SAFETY: as above; the client world keeps the other hosts' style sheets from the page.
        unsafe { controller.addUserScript(&user_script(mtm, source, &WKContentWorld::defaultClientWorld(mtm), true)) };
    }
}

/// A `<meta name=referrer>` inserted before the page's own markup sets the document's policy and
/// keeps it after removal, so pages cannot see the marker. A policy the page declares later still wins.
const TRIM_REFERRER: &str = "(() => { const m = document.createElement('meta'); m.name = 'referrer'; \
    m.content = 'same-origin'; document.documentElement.prepend(m); m.remove(); })();";

/// Without Apple's browser entitlement WebKit says passkeys are unavailable, so sites never offer
/// them, and it refuses a conditional request at once, which ends Bitwarden's race against it before
/// the user picks a passkey. So where `navigator.credentials.get` has been replaced again, by
/// Bitwarden's page script or the page, passkeys are said to be available and a conditional request
/// WebKit refuses stays pending until the page aborts it through its signal, as Safari's own conditional UI does.
const PASSKEYS: &str = "(() => { const P = window.PublicKeyCredential; const C = navigator.credentials; if (!P || !C) return; \
    const native = f => Function.prototype.toString.call(f).includes('[native code]'); if (!native(C.get)) return; \
    const nativeGet = C.get; C.get = { get(options) { const asked = nativeGet.call(C, options); \
    if (!options || options.mediation !== 'conditional') return asked; \
    return asked.catch(e => { if (!e || e.name !== 'NotAllowedError' || !ours() || !options.signal) throw e; \
    const s = options.signal; return new Promise((_, reject) => { if (s.aborted) reject(s.reason); \
    else s.addEventListener('abort', () => reject(s.reason), { once: true }); }); }); } }.get; \
    const mine = C.get; const ours = () => { const g = navigator.credentials.get; return g !== mine && !native(g); }; \
    for (const name of ['isConditionalMediationAvailable', 'isUserVerifyingPlatformAuthenticatorAvailable']) { \
    const asked = P[name]; if (typeof asked !== 'function') continue; \
    Object.defineProperty(P, name, { configurable: true, writable: true, value: { async [name]() { \
    for (let i = 0; i < 40 && !ours() && (i < 4 || document.readyState === 'loading'); i++) await new Promise(r => setTimeout(r, 50)); \
    return ours() ? true : asked.call(P); } }[name] }); } })();";

use crate::page_scripts::ALWAYS_VISIBLE;

fn user_script(mtm: MainThreadMarker, source: &str, world: &WKContentWorld, main_frame_only: bool) -> Retained<WKUserScript> {
    // SAFETY: main-thread creation of a fresh script.
    unsafe {
        WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            WKUserScript::alloc(mtm),
            &NSString::from_str(source),
            WKUserScriptInjectionTime::AtDocumentStart,
            main_frame_only,
            world,
        )
    }
}

type FilterDone = Box<dyn FnOnce(Result<FilterLists, String>)>;
type RuleListHandler = DynBlock<dyn Fn(*mut WKContentRuleList, *mut NSError)>;

/// Compiled rule lists of one group, stored by WebKit under `tarn.lists.<group>.<key>.<index>`.
#[derive(Clone)]
pub struct FilterLists(Rc<Vec<Retained<WKContentRuleList>>>);

impl FilterLists {
    /// Finds lists compiled in an earlier run; fails if any of the `count` lists is missing.
    pub fn look_up(group: &str, key: &str, count: usize, done: impl FnOnce(Result<FilterLists, String>) + 'static) {
        let ids: Vec<_> = (0..count).map(|i| NSString::from_str(&format!("{FILTER_PREFIX}{group}.{key}.{i}"))).collect();
        gather(count, done, move |store, i, handler| {
            // SAFETY: main thread; arguments are valid and WebKit copies the block.
            unsafe { store.lookUpContentRuleListForIdentifier_completionHandler(Some(&ids[i]), Some(handler)) };
        });
    }

    /// Deletes every stored list of `group`.
    pub fn remove(group: &str) {
        remove_filter_lists_except(&format!("{FILTER_PREFIX}{group}."), None);
    }

    /// Compiles `chunks` and, once all succeed, deletes the group's stored lists of every other key.
    pub fn compile(group: &str, key: &str, chunks: Vec<String>, done: impl FnOnce(Result<FilterLists, String>) + 'static) {
        let ids: Vec<_> = (0..chunks.len()).map(|i| NSString::from_str(&format!("{FILTER_PREFIX}{group}.{key}.{i}"))).collect();
        let (prefix, keep) = (format!("{FILTER_PREFIX}{group}."), format!("{FILTER_PREFIX}{group}.{key}."));
        let done = move |result: Result<FilterLists, String>| {
            if result.is_ok() {
                remove_filter_lists_except(&prefix, Some(&keep));
            }
            done(result);
        };
        gather(chunks.len(), done, move |store, i, handler| {
            let json = NSString::from_str(&chunks[i]);
            // SAFETY: main thread; arguments are valid and WebKit copies the block.
            unsafe {
                store.compileContentRuleListForIdentifier_encodedContentRuleList_completionHandler(Some(&ids[i]), Some(&json), Some(handler));
            }
        });
    }
}

struct Gathering {
    lists: Vec<Option<Retained<WKContentRuleList>>>,
    remaining: usize,
    done: Option<FilterDone>,
}

/// Starts `count` WebKit rule-list requests and calls `done` once with all lists or the first error.
fn gather(count: usize, done: impl FnOnce(Result<FilterLists, String>) + 'static, start: impl Fn(&WKContentRuleListStore, usize, &RuleListHandler)) {
    let Some(mtm) = MainThreadMarker::new() else {
        return done(Err("not on the main thread".into()));
    };
    // SAFETY: main thread; the default store exists for apps with a container or home dir.
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else {
        return done(Err("no rule list store".into()));
    };
    if count == 0 {
        return done(Ok(FilterLists(Rc::new(Vec::new()))));
    }
    let state = Rc::new(RefCell::new(Gathering { lists: vec![None; count], remaining: count, done: Some(Box::new(done)) }));
    for i in 0..count {
        let state = state.clone();
        let handler = RcBlock::new(move |list: *mut WKContentRuleList, error: *mut NSError| {
            let mut s = state.borrow_mut();
            // SAFETY: WebKit passes a valid rule list or null, alive for the call.
            match unsafe { Retained::retain(list) } {
                Some(list) => s.lists[i] = Some(list),
                None => {
                    // SAFETY: WebKit passes a valid error or null.
                    let message = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string());
                    if let Some(done) = s.done.take() {
                        drop(s);
                        return done(Err(message.unwrap_or_else(|| "rule list not found".into())));
                    }
                    return;
                }
            }
            s.remaining -= 1;
            if s.remaining == 0
                && let Some(done) = s.done.take()
            {
                let lists = s.lists.drain(..).flatten().collect();
                drop(s);
                done(Ok(FilterLists(Rc::new(lists))));
            }
        });
        start(&store, i, &handler);
    }
}

fn remove_filter_lists_except(prefix: &str, keep: Option<&str>) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    // SAFETY: main thread.
    let Some(store) = (unsafe { WKContentRuleListStore::defaultStore(mtm) }) else { return };
    let (prefix, keep) = (prefix.to_owned(), keep.map(str::to_owned));
    let remover = store.clone();
    let handler = RcBlock::new(move |ids: *mut NSArray<NSString>| {
        // SAFETY: WebKit passes a valid array or null, alive for the call.
        let Some(ids) = (unsafe { ids.as_ref() }) else { return };
        let ignore = RcBlock::new(|_: *mut NSError| {});
        for id in ids.iter() {
            let name = id.to_string();
            if name.starts_with(&prefix) && !keep.as_ref().is_some_and(|k| name.starts_with(k)) {
                // SAFETY: main thread (WebKit calls back there); arguments are valid.
                unsafe { remover.removeContentRuleListForIdentifier_completionHandler(Some(&id), Some(&ignore)) };
            }
        }
    });
    // SAFETY: main thread; the block is copied by WebKit.
    unsafe { store.getAvailableContentRuleListIdentifiers(Some(&handler)) };
}

#[cfg(test)]
mod tests {
    use super::{ALWAYS_VISIBLE, Attach, Builtin, ShieldLevel, site_matches};

    /// Runs the script in Node's `vm` against a minimal document per host:
    /// `cargo test -p engine always_visible -- --ignored`.
    #[test]
    #[ignore = "needs node"]
    fn always_visible_hides_leaving_only_on_netflix() {
        let check = r#"
            const vm = require('vm');
            const run = host => {
                class Document extends EventTarget {}
                Object.defineProperty(Document.prototype, 'hidden', { configurable: true, get: () => true });
                Object.defineProperty(Document.prototype, 'visibilityState', { configurable: true, get: () => 'hidden' });
                Document.prototype.hasFocus = () => false;
                // One target stands in for both, as an event on the document passes the window first.
                const document = new Document(), window = document, seen = [];
                const ctx = { window, document, Document, location: { hostname: host } };
                vm.runInNewContext(process.argv[1], ctx);
                document.addEventListener('visibilitychange', () => seen.push('visibility'));
                window.addEventListener('blur', () => seen.push('blur'));
                document.dispatchEvent(new Event('visibilitychange'));
                window.dispatchEvent(new Event('blur'));
                return [document.hidden, document.visibilityState, document.hasFocus(), seen.join()].join(' ');
            };
            console.log(['www.netflix.com', 'netflix.com', 'notnetflix.com', 'netflix.com.example'].map(run).join('|'));
        "#;
        let output = std::process::Command::new("node").args(["-e", check, ALWAYS_VISIBLE]).output().expect("node");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "false visible true |false visible true |true hidden false visibility,blur|true hidden false visibility,blur",
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn extension_rule_lists_stay_out_of_lockdown() {
        for (level, filtered) in ShieldLevel::ALL.into_iter().flat_map(|l| [(l, false), (l, true)]) {
            assert!(Attach::Always.applies(level, filtered));
            assert_eq!(Attach::Unpaused.applies(level, filtered), filtered);
            assert_eq!(Attach::OutsideLockdown.applies(level, filtered), level != ShieldLevel::Lockdown);
        }
    }

    #[test]
    fn loads_wait_for_the_builtin_list_and_fail_closed() {
        let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let record = |builtin: &Builtin, name: &'static str| {
            let seen = seen.clone();
            builtin.then(move |r| seen.borrow_mut().push((name, r.map_err(str::to_owned))));
        };
        let ready = Builtin::default();
        record(&ready, "early");
        assert!(seen.borrow().is_empty());
        ready.settle(Ok(()));
        record(&ready, "late");
        let failed = Builtin::default();
        record(&failed, "held");
        failed.settle(Err("bad".into()));
        record(&failed, "after");
        let err = Err("bad".to_owned());
        assert_eq!(*seen.borrow(), [("early", Ok(())), ("late", Ok(())), ("held", err.clone()), ("after", err)]);
    }

    #[test]
    fn site_data_matches_host_and_subdomains_only() {
        assert!(site_matches("example.com", "example.com"));
        assert!(site_matches("www.example.com", "Example.com"));
        assert!(!site_matches("badexample.com", "example.com"));
        assert!(!site_matches("example.com", "www.example.com"));
        assert!(!site_matches("example.com", ""));
    }
}
