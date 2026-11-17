use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;

use block2::{DynBlock, RcBlock};
use core_foundation::url::CFURL;
use dispatch2::MainThreadBound;
use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBackingStoreType, NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypeString, NSPopoverBehavior, NSWindow,
    NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    NSArray, NSBundle, NSDate, NSDictionary, NSError, NSNotification, NSNumber, NSObject, NSPoint, NSRect, NSRectEdge, NSSet, NSSize, NSString, NSTimer, NSURL,
    NSURLRequest, NSUUID,
};
use objc2_web_kit::{
    WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKUserContentController, WKWebExtension, WKWebExtensionAction, WKWebExtensionContext,
    WKWebExtensionContextPermissionStatus, WKWebExtensionController, WKWebExtensionControllerConfiguration, WKWebExtensionControllerDelegate,
    WKWebExtensionDataRecord, WKWebExtensionMatchPattern, WKWebExtensionMessagePort, WKWebExtensionPermissionClipboardWrite, WKWebExtensionTab,
    WKWebExtensionTabConfiguration, WKWebExtensionWindow, WKWebExtensionWindowConfiguration, WKWebExtensionWindowState, WKWebExtensionWindowType, WKWebView,
};
use security_framework::os::macos::code_signing::{Flags, SecRequirement, SecStaticCode};
use sha2::{Digest, Sha256};

use crate::biometrics;
use crate::nav_policy::{self, Decision, Source};

const APPLICATIONS: &str = "/Applications";
const CLIPBOARD_READ: &str = "clipboardRead";
const SAFARI_EXTENSION_POINT: &str = "com.apple.Safari.web-extension";
/// Safari's scheme for extension pages; Bitwarden's inline menu refuses to load frames from any
/// extension scheme it does not know, `webkit-extension` included.
const SCHEME: &str = "safari-web-extension";
const POPOUT_SIZE: (f64, f64) = (380.0, 630.0);
const POPOUT_MIN_SIDE: f64 = 200.0;
const POPOUT_MAX_SIDE: f64 = 1000.0;
const MAX_POPOUTS: usize = 4;
const BITWARDEN_APP: &str = "com.8bit.bitwarden";
/// Native messages, the user key among them, go only to Bitwarden's own app extension.
const BITWARDEN_EXTENSION: &str = "com.bitwarden.desktop.safari";
const BITWARDEN_SIGNATURE: &str = r#"identifier "com.bitwarden.desktop.safari" and anchor apple generic and (certificate leaf[subject.OU] = "LTZ2PFU5D6" or certificate leaf[field.1.2.840.113635.100.6.1.9] and entitlement["com.apple.developer.team-identifier"] = "LTZ2PFU5D6")"#;
const MAX_PORTS: usize = 4;
/// API permissions an unpacked extension never gets, whatever its manifest asks for.
const UNPACKED_NEVER: [&str; 2] = ["nativeMessaging", CLIPBOARD_READ];
/// How long a host answered Once stays granted.
const ONCE_SECONDS: f64 = 3600.0;
/// How long a "sleep" message waits before replying; the Safari build of Bitwarden loops on it to
/// check its vault timeout, so an immediate reply would spin.
const SLEEP_SECONDS: f64 = 5.0;

/// A Safari web extension bundled with an app the user installed, or one unpacked from a
/// package into a directory the shell owns.
#[derive(Clone)]
pub struct Extension {
    pub path: PathBuf,
    /// Names the storage: the install id of an unpacked extension, the bundle path of an app's.
    pub key: String,
    web: Retained<WKWebExtension>,
    bitwarden: bool,
    unpacked: bool,
}

/// One extension to run, with the host decisions the user stored for it.
pub struct Enabled {
    pub extension: Extension,
    pub all_sites: bool,
    /// API permissions the user confirmed, optional ones included: a run-time request may get these.
    pub confirmed: Vec<String>,
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
    /// The user turned Bitwarden's privileges on; they also need Bitwarden's own signed app extension.
    pub bitwarden: bool,
}

impl Enabled {
    pub(crate) fn privileged(&self) -> bool {
        self.bitwarden && self.extension.bitwarden
    }
}

impl Extension {
    /// Loads the extension in `dir`. Whatever its manifest claims, it gets none of the
    /// privileges kept for Bitwarden's app extension. `done` runs on the main thread.
    pub fn unpacked(dir: &Path, install_id: &str, done: impl FnOnce(Result<Extension, String>) + 'static) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let (path, key) = (dir.to_path_buf(), install_id.to_owned());
        let done = RefCell::new(Some(done));
        let handler = RcBlock::new(move |web: *mut WKWebExtension, error: *mut NSError| {
            // SAFETY: WebKit passes a valid extension or null, and a valid error or null.
            let (web, error) = unsafe { (Retained::retain(web), error.as_ref()) };
            let result = match web {
                Some(web) => Ok(Extension { path: path.clone(), key: key.clone(), web, bitwarden: false, unpacked: true }),
                None => Err(error.map_or_else(|| "the extension could not be loaded".to_owned(), |e| e.localizedDescription().to_string())),
            };
            if let Some(done) = done.borrow_mut().take() {
                done(result);
            }
        });
        let url = NSURL::fileURLWithPath(&NSString::from_str(&dir.to_string_lossy()));
        // SAFETY: main thread; a valid file URL; WebKit copies the block.
        unsafe { WKWebExtension::extensionWithResourceBaseURL_completionHandler(&url, &handler, mtm) };
    }

    pub fn is_unpacked(&self) -> bool {
        self.unpacked
    }

    /// API permissions WebKit parsed, required then optional.
    pub fn api_permissions(&self) -> (Vec<String>, Vec<String>) {
        // SAFETY: main-thread reads of a loaded extension.
        let (mut required, mut optional): (Vec<String>, Vec<String>) = unsafe {
            (self.web.requestedPermissions().iter().map(|p| p.to_string()).collect(), self.web.optionalPermissions().iter().map(|p| p.to_string()).collect())
        };
        if reads_clipboard(&self.web) {
            required.push(CLIPBOARD_READ.to_owned());
        }
        required.sort();
        optional.sort();
        (required, optional)
    }

    /// Host patterns WebKit parsed, content-script matches included, then the optional ones.
    pub fn host_patterns(&self) -> (Vec<String>, Vec<String>) {
        // SAFETY: as above.
        let (mut required, mut optional): (Vec<String>, Vec<String>) = unsafe {
            (
                self.web.allRequestedMatchPatterns().iter().map(|p| p.string().to_string()).collect(),
                self.web.optionalPermissionMatchPatterns().iter().map(|p| p.string().to_string()).collect(),
            )
        };
        required.sort();
        optional.sort();
        (required, optional)
    }

    pub fn has_active_tab(&self) -> bool {
        self.api_permissions().0.iter().any(|p| p == "activeTab")
    }

    /// The manifest's own icon paths, largest size first.
    pub fn manifest_icons(&self) -> Vec<String> {
        // SAFETY: main-thread read of a loaded extension's parsed manifest.
        let manifest = unsafe { self.web.manifest() };
        let Some(icons) = manifest.objectForKey(&NSString::from_str("icons")).and_then(|i| i.downcast::<NSDictionary>().ok()) else {
            return Vec::new();
        };
        let mut sized: Vec<(u32, String)> = icons
            .allKeys()
            .iter()
            .filter_map(|k| {
                let size = k.downcast_ref::<NSString>()?.to_string().parse().ok()?;
                let path = icons.objectForKey(&k)?.downcast::<NSString>().ok()?.to_string();
                Some((size, path))
            })
            .collect();
        sized.sort_by_key(|(size, _)| std::cmp::Reverse(*size));
        sized.into_iter().map(|(_, p)| p).collect()
    }

    pub fn name(&self) -> String {
        // SAFETY: main-thread read of a loaded extension.
        unsafe { self.web.displayName() }.map(|s| s.to_string()).unwrap_or_else(|| self.path.display().to_string())
    }

    pub fn version(&self) -> String {
        // SAFETY: as above.
        unsafe { self.web.version().or_else(|| self.web.displayVersion()) }.map(|s| s.to_string()).unwrap_or_default()
    }

    /// API permissions and the sites it asks to read and change, optional ones included, sorted.
    pub fn permissions(&self) -> Vec<String> {
        let ((apis, optional_apis), (hosts, optional_hosts)) = (self.api_permissions(), self.host_patterns());
        let mut list: Vec<String> = apis.into_iter().chain(optional_apis).chain(hosts).chain(optional_hosts).collect();
        list.sort();
        list.dedup();
        list
    }

    /// Only for app extensions: `iconForSize` decodes whatever file WebKit picks.
    pub fn icon_png(&self, points: f64) -> Option<Vec<u8>> {
        if self.unpacked {
            return None;
        }
        // SAFETY: main-thread reads of a loaded extension and the image it returns.
        unsafe {
            let image = self.web.iconForSize(NSSize::new(points, points))?;
            let tiff = image.TIFFRepresentation()?;
            let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
            let png = rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?;
            Some(png.to_vec())
        }
    }
}

/// WebKit does not support `clipboardRead`, so it is missing from `requestedPermissions`; the
/// clipboard native message is answered only for extensions whose manifest asks for it.
fn reads_clipboard(web: &WKWebExtension) -> bool {
    // SAFETY: main-thread read of a loaded extension's parsed manifest.
    let permissions = unsafe { web.manifest() }.objectForKey(&NSString::from_str("permissions"));
    permissions
        .and_then(|p| p.downcast::<NSArray>().ok())
        .is_some_and(|list| list.iter().any(|p| p.downcast_ref::<NSString>().is_some_and(|p| p.to_string() == CLIPBOARD_READ)))
}

type Pending = Rc<RefCell<(usize, Vec<Extension>, Option<Box<dyn FnOnce(Vec<Extension>)>>)>>;

/// Loads the Safari web extensions of the apps in /Applications; `done` runs on the main thread.
pub fn discover(done: impl FnOnce(Vec<Extension>) + 'static) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let bundles: Vec<(PathBuf, Retained<NSBundle>)> = std::fs::read_dir(APPLICATIONS)
        .into_iter()
        .flatten()
        .flatten()
        .map(|app| app.path().join("Contents/PlugIns"))
        .flat_map(|plugins| std::fs::read_dir(plugins).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "appex"))
        .filter_map(|path| safari_extension(&path).map(|bundle| (path, bundle)))
        .collect();
    let pending: Pending = Rc::new(RefCell::new((bundles.len(), Vec::new(), Some(Box::new(done)))));
    let finish = |pending: &Pending| {
        let mut state = pending.borrow_mut();
        if state.0 == 0
            && let Some(done) = state.2.take()
        {
            let found = std::mem::take(&mut state.1);
            drop(state);
            done(found);
        }
    };
    finish(&pending);
    for (path, bundle) in bundles {
        let pending = pending.clone();
        let bitwarden = bundle.bundleIdentifier().is_some_and(|id| id.to_string() == BITWARDEN_EXTENSION) && signed_by_bitwarden(&path);
        let handler = RcBlock::new(move |web: *mut WKWebExtension, _: *mut NSError| {
            // SAFETY: WebKit passes a valid extension or null.
            let web = unsafe { Retained::retain(web) };
            {
                let mut state = pending.borrow_mut();
                state.0 -= 1;
                if let Some(web) = web {
                    state.1.push(Extension { key: path.display().to_string(), path: path.clone(), web, bitwarden, unpacked: false });
                }
            }
            finish(&pending);
        });
        // SAFETY: main thread; a valid bundle; WebKit copies the block.
        unsafe { WKWebExtension::extensionWithAppExtensionBundle_completionHandler(&bundle, &handler, mtm) };
    }
}

fn signed_by_bitwarden(path: &Path) -> bool {
    let Ok(requirement) = BITWARDEN_SIGNATURE.parse::<SecRequirement>() else { return false };
    CFURL::from_path(path, true)
        .and_then(|url| SecStaticCode::from_path(&url, Flags::NONE).ok())
        .is_some_and(|code| code.check_validity(Flags::NONE, &requirement).is_ok())
}

fn safari_extension(path: &Path) -> Option<Retained<NSBundle>> {
    let bundle = NSBundle::bundleWithPath(&NSString::from_str(&path.to_string_lossy()))?;
    let point = bundle
        .objectForInfoDictionaryKey(&NSString::from_str("NSExtension"))
        .and_then(|e| e.downcast::<NSDictionary>().ok())
        .and_then(|e| e.objectForKey(&NSString::from_str("NSExtensionPointIdentifier")))
        .and_then(|p| p.downcast::<NSString>().ok());
    point.is_some_and(|p| p.to_string() == SAFARI_EXTENSION_POINT).then_some(bundle)
}

/// A stable identifier per profile and extension that sites cannot guess: it names the
/// extension's storage and its `safari-web-extension://` origin, which pages can see.
fn context_id(seed: &str, key: &str) -> String {
    let hash = Sha256::digest(format!("{seed}\n{key}").as_bytes());
    let hex: String = hash[..16].iter().map(|b| format!("{b:02X}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// A controller for the persistent extension storage named `id`, with nothing loaded.
pub(crate) fn persistent_controller(mtm: MainThreadMarker, id: &NSUUID) -> Retained<WKWebExtensionController> {
    // SAFETY: main-thread creation of fresh objects.
    unsafe {
        let config = WKWebExtensionControllerConfiguration::configurationWithIdentifier(id, mtm);
        WKWebExtensionController::initWithConfiguration(WKWebExtensionController::alloc(mtm), &config)
    }
}

/// Removes every kind of data stored through `controller`, for loaded and unloaded extensions
/// alike; `done` runs on the main thread once WebKit finished.
pub(crate) fn remove_data(controller: Retained<WKWebExtensionController>, done: impl FnOnce() + 'static) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    // SAFETY: main thread.
    let types = unsafe { WKWebExtensionController::allExtensionDataTypes(mtm) };
    let done = Rc::new(RefCell::new(Some(done)));
    let (remover, remove_types) = (controller.clone(), types.clone());
    let fetched = RcBlock::new(move |records: NonNull<NSArray<WKWebExtensionDataRecord>>| {
        let (done, kept) = (done.clone(), remover.clone());
        let finished = RcBlock::new(move || {
            let _ = &kept;
            if let Some(done) = done.take() {
                done();
            }
        });
        // SAFETY: main thread; WebKit passes a valid array for the call and copies the block.
        unsafe { remover.removeDataOfTypes_fromDataRecords_completionHandler(&remove_types, records.as_ref(), &finished) };
    });
    // SAFETY: main thread; WebKit copies the block, which keeps the controller alive until it runs.
    unsafe { controller.fetchDataRecordsOfTypes_completionHandler(&types, &fetched) };
}

/// One profile's extension controller and the extensions loaded into it.
pub(crate) struct Profile {
    pub(crate) controller: Retained<WKWebExtensionController>,
    _delegate: Retained<ControllerDelegate>,
    contexts: HashMap<String, Retained<WKWebExtensionContext>>,
    pub(crate) seed: String,
}

impl Drop for Profile {
    fn drop(&mut self) {
        for context in self.contexts.values() {
            release(context);
        }
    }
}

impl Profile {
    /// `id` names the persistent extension storage; without one it lives in memory.
    /// `rules` carries the profile's private-network and HTTPS rule lists, which WebKit applies to
    /// background pages, service workers and popups.
    pub(crate) fn new(mtm: MainThreadMarker, id: Option<&NSUUID>, store: &objc2_web_kit::WKWebsiteDataStore, rules: &WKUserContentController) -> Self {
        // SAFETY: main-thread configuration of fresh objects; the delegate is retained by `Profile`.
        unsafe {
            WKWebExtensionMatchPattern::registerCustomURLScheme(&NSString::from_str(SCHEME), mtm);
            let config = match id {
                Some(id) => WKWebExtensionControllerConfiguration::configurationWithIdentifier(id, mtm),
                None => WKWebExtensionControllerConfiguration::nonPersistentConfiguration(mtm),
            };
            let web = config.webViewConfiguration();
            web.setUserContentController(rules);
            if let Some(name) = crate::webview::safari_application_name() {
                web.setApplicationNameForUserAgent(Some(&name));
            }
            config.setWebViewConfiguration(Some(&web));
            config.setDefaultWebsiteDataStore(Some(store));
            let controller = WKWebExtensionController::initWithConfiguration(WKWebExtensionController::alloc(mtm), &config);
            let delegate: Retained<ControllerDelegate> = msg_send![ControllerDelegate::alloc(mtm), init];
            controller.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            let seed = id.map_or_else(|| NSUUID::new().UUIDString().to_string(), |id| id.UUIDString().to_string());
            Profile { controller, _delegate: delegate, contexts: HashMap::new(), seed }
        }
    }

    /// Loads `enabled` and unloads the rest, then sets every context's host access from what
    /// the user stored: requested patterns only with `all_sites`, otherwise the hosts answered
    /// Always (or Once in this session), and never a `blocked` (Lockdown) host. WebKit keeps
    /// nothing for a context id across loads; grants are rewritten all the same.
    pub(crate) fn set_enabled(&mut self, enabled: &[Enabled], blocked: &[String]) {
        self.contexts.retain(|key, context| {
            // An update loads the new version under the same key, so its storage stays.
            // SAFETY: main-thread read of a loaded context.
            let web = unsafe { context.webExtension() };
            let stays =
                enabled.iter().any(|e| e.extension.key == *key && std::ptr::eq(&*e.extension.web, &*web) && e.privileged() == speaks_native(context).is_some());
            if !stays {
                release(context);
                // SAFETY: main thread; the context belongs to this controller.
                unsafe { self.controller.unloadExtensionContext_error(context) }.ok();
            }
            stays
        });
        for enabled in enabled {
            let extension = &enabled.extension;
            if self.contexts.contains_key(&extension.key) {
                continue;
            }
            let id = NSString::from_str(&context_id(&self.seed, &extension.key));
            // SAFETY: main-thread setup of a fresh context before it is loaded.
            unsafe {
                let context = WKWebExtensionContext::contextForExtension(&extension.web);
                context.setUniqueIdentifier(&id);
                if let Some(base) = NSURL::URLWithString(&NSString::from_str(&format!("{SCHEME}://{id}/"))) {
                    context.setBaseURL(&base);
                }
                let private = !self.controller.configuration().isPersistent();
                context.setHasAccessToPrivateData(private);
                for permission in extension.web.requestedPermissions().iter() {
                    if extension.unpacked && UNPACKED_NEVER.contains(&permission.to_string().as_str()) {
                        continue;
                    }
                    context.setPermissionStatus_forPermission(WKWebExtensionContextPermissionStatus::GrantedExplicitly, &permission);
                }
                if self.controller.loadExtensionContext_error(&context).is_err() {
                    continue;
                }
                context.loadBackgroundContentWithCompletionHandler(&RcBlock::new(|_: *mut NSError| {}));
                if enabled.privileged() {
                    NATIVE.with_borrow_mut(|native| native.push((context.clone(), extension.path.clone())));
                }
                LOADED.with_borrow_mut(|loaded| {
                    loaded.push(Loaded {
                        context: context.clone(),
                        seed: self.seed.clone(),
                        key: extension.key.clone(),
                        unpacked: extension.unpacked,
                        confirmed: Vec::new(),
                        all_sites: false,
                    })
                });
                self.contexts.insert(extension.key.clone(), context);
            }
        }
        let Some(mtm) = MainThreadMarker::new() else { return };
        let pattern = |host: &str| {
            if !plain_host(host) {
                return None;
            }
            // SAFETY: main thread; the pattern string is built from a host name.
            unsafe { WKWebExtensionMatchPattern::matchPatternWithString(&NSString::from_str(&format!("*://{host}/*")), mtm) }
        };
        let denied: Vec<_> = blocked.iter().flat_map(|host| [host.clone(), format!("{host}.")]).filter_map(|h| pattern(&h)).collect();
        let now = std::time::Instant::now();
        ONCE.with_borrow_mut(|once| once.retain(|g| g.until > now && g.view.load().is_some()));
        for Enabled { extension, all_sites, confirmed, allowed, denied: refused, .. } in enabled {
            let Some(context) = self.contexts.get(&extension.key) else { continue };
            LOADED.with_borrow_mut(|loaded| {
                if let Some(l) = loaded.iter_mut().find(|l| std::ptr::eq(&*l.context, &**context)) {
                    l.confirmed = confirmed.clone();
                    l.all_sites = *all_sites;
                }
            });
            let once: Vec<(String, f64)> = ONCE.with_borrow(|once| {
                once.iter()
                    .filter(|g| g.seed == self.seed && g.key == extension.key)
                    .map(|g| (g.host.clone(), g.until.duration_since(now).as_secs_f64()))
                    .collect()
            });
            // SAFETY: main thread; replaces the loaded context's host grants with the current set.
            unsafe {
                context.setGrantedPermissionMatchPatterns(&NSDictionary::new());
                context.setDeniedPermissionMatchPatterns(&NSDictionary::new());
                let grant =
                    |p: &WKWebExtensionMatchPattern| context.setPermissionStatus_forMatchPattern(WKWebExtensionContextPermissionStatus::GrantedExplicitly, p);
                if *all_sites {
                    extension.web.allRequestedMatchPatterns().iter().for_each(|p| grant(&p));
                }
                allowed.iter().filter_map(|h| pattern(h)).for_each(|p| grant(&p));
                for (host, seconds) in once {
                    if let Some(p) = pattern(&host) {
                        let until = NSDate::dateWithTimeIntervalSinceNow(seconds);
                        context.setPermissionStatus_forMatchPattern_expirationDate(WKWebExtensionContextPermissionStatus::GrantedExplicitly, &p, Some(&until));
                    }
                }
                for p in refused.iter().filter_map(|h| pattern(h)).chain(denied.iter().cloned()) {
                    context.setPermissionStatus_forMatchPattern(WKWebExtensionContextPermissionStatus::DeniedExplicitly, &p);
                }
            }
        }
    }

    /// Opens the popup of a toolbar click; only this lets an unpacked extension show one.
    pub(crate) fn open_popup(&self, extension: &Extension) {
        let Some(context) = self.contexts.get(&extension.key) else { return };
        let tab = clicked_tab(&self.controller);
        CLICKED.with_borrow_mut(|clicked| *clicked = Some((context.clone(), std::time::Instant::now())));
        // SAFETY: main thread; WebKit calls the delegate to present the popup.
        unsafe { context.performActionForTab(tab.as_deref().map(ProtocolObject::from_ref)) };
    }

    /// Keys of the extensions that loaded into this profile's controller.
    pub(crate) fn running(&self) -> Vec<String> {
        self.contexts.keys().cloned().collect()
    }

    /// Removes what the extension `key` stored, unloads it and deletes WebKit's folder for its
    /// context, which also covers a context that could not be loaded. `done` runs afterwards.
    pub(crate) fn remove(&mut self, key: &str, done: impl FnOnce() + 'static) {
        forget_session(&self.seed, Some(key));
        let id = context_id(&self.seed, key);
        let folders: Vec<PathBuf> = crate::data_store::extension_folders(&self.seed).into_iter().map(|f| f.join(&id)).collect();
        let finish = move || {
            for folder in &folders {
                std::fs::remove_dir_all(folder).ok();
            }
            done();
        };
        let Some(context) = self.contexts.remove(key) else { return finish() };
        release(&context);
        let Some(mtm) = MainThreadMarker::new() else { return };
        // SAFETY: main thread.
        let types = unsafe { WKWebExtensionController::allExtensionDataTypes(mtm) };
        let finish: Rc<RefCell<Option<Finish>>> = Rc::new(RefCell::new(Some(Box::new(finish))));
        let (controller, unload, remove_types) = (self.controller.clone(), context.clone(), types.clone());
        let fetched = RcBlock::new(move |record: *mut WKWebExtensionDataRecord| {
            let (kept, unload, finish) = (controller.clone(), unload.clone(), finish.clone());
            let removed = RcBlock::new(move || {
                // SAFETY: main thread; the context belongs to this controller.
                unsafe { kept.unloadExtensionContext_error(&unload) }.ok();
                if let Some(finish) = finish.borrow_mut().take() {
                    finish();
                }
            });
            // SAFETY: WebKit passes a valid record or null.
            match unsafe { record.as_ref() } {
                // SAFETY: main thread; WebKit copies the block.
                Some(record) => unsafe {
                    controller.removeDataOfTypes_fromDataRecords_completionHandler(&remove_types, &NSArray::from_slice(&[record]), &removed)
                },
                None => removed.call(()),
            }
        });
        // SAFETY: main thread; the context is loaded into this controller; WebKit copies the block.
        unsafe { self.controller.fetchDataRecordOfTypes_forExtensionContext_completionHandler(&types, &context, &fetched) };
    }
}

pub(crate) struct TabState {
    view: Weak<WKWebView>,
    controller: Retained<WKWebExtensionController>,
    window: Weak<Window>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnExtensionTab"]
    #[ivars = TabState]
    pub(crate) struct Tab;

    unsafe impl NSObjectProtocol for Tab {}

    unsafe impl WKWebExtensionTab for Tab {
        #[unsafe(method_id(webViewForWebExtensionContext:))]
        fn web_view(&self, _context: &WKWebExtensionContext) -> Option<Retained<WKWebView>> {
            self.ivars().view.load()
        }

        #[unsafe(method_id(windowForWebExtensionContext:))]
        fn window(&self, _context: &WKWebExtensionContext) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            self.ivars().window.load().map(ProtocolObject::from_retained)
        }

        #[unsafe(method(isSelectedForWebExtensionContext:))]
        fn is_selected(&self, _context: &WKWebExtensionContext) -> bool {
            self.ivars().window.load().is_some_and(|w| w.is_popout()) || active_tab(&self.ivars().controller).is_some_and(|t| std::ptr::eq(&*t, self))
        }

        /// WebKit would load any URL into any tab; only an extension with access to
        /// the tab's page may, and only where a link from that page could go.
        #[unsafe(method(loadURL:forWebExtensionContext:completionHandler:))]
        fn load_url(&self, url: &NSURL, context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            let target = url.absoluteString().and_then(|u| url::Url::parse(&u.to_string()).ok());
            let allowed = target.is_some_and(|u| nav_policy::decide(&u, Source::Link) == Decision::Allow);
            // SAFETY: main thread; a live view.
            self.control(context, done, |view| allowed && unsafe { view.loadRequest(&NSURLRequest::requestWithURL(url)) }.is_some());
        }

        #[unsafe(method(reloadFromOrigin:forWebExtensionContext:completionHandler:))]
        fn reload(&self, _origin: bool, context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            // SAFETY: main thread; a live view.
            self.control(context, done, |view| unsafe { view.reload() }.is_some());
        }

        #[unsafe(method(goBackForWebExtensionContext:completionHandler:))]
        fn go_back(&self, context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            // SAFETY: main thread; a live view.
            self.control(context, done, |view| unsafe { view.goBack() }.is_some());
        }

        #[unsafe(method(goForwardForWebExtensionContext:completionHandler:))]
        fn go_forward(&self, context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            // SAFETY: main thread; a live view.
            self.control(context, done, |view| unsafe { view.goForward() }.is_some());
        }

        #[unsafe(method(setZoomFactor:forWebExtensionContext:completionHandler:))]
        fn set_zoom(&self, zoom: f64, context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            self.control(context, done, |view| {
                let ok = zoom.is_finite() && (0.3..=5.0).contains(&zoom);
                if ok {
                    // SAFETY: main thread; a live view.
                    unsafe { view.setPageZoom(zoom) };
                }
                ok
            });
        }
    }
);

fn respond(done: &DynBlock<dyn Fn(*mut NSError)>, ok: bool) {
    let error = (!ok).then(|| NSError::new(1, &NSString::from_str("TarnExtensionTab")));
    done.call((error.as_ref().map_or(std::ptr::null_mut(), |e| Retained::as_ptr(e).cast_mut()),));
}

impl Tab {
    /// Runs `act` on the tab's view only when `context` may access the page it shows.
    fn control(&self, context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>, act: impl FnOnce(&WKWebView) -> bool) {
        let view = self.ivars().view.load();
        // SAFETY: main-thread reads of a live view and a loaded context.
        let allowed = view
            .as_ref()
            .and_then(|v| unsafe { v.URL() })
            .is_some_and(|url| unsafe { context.hasAccessToURL_inTab(&url, Some(ProtocolObject::from_ref(self))) });
        respond(done, allowed && view.is_some_and(|v| act(&v)));
    }

    /// Tells the view's extension controller, if it has one, about a new tab.
    pub(crate) fn track(view: &WKWebView) -> Option<Retained<Tab>> {
        // SAFETY: main-thread reads of a live web view's configuration.
        let controller = unsafe { view.configuration().webExtensionController() }?;
        let tab = Tab::new(view, controller, &window(view.mtm()));
        // SAFETY: main thread; WebKit holds the tab weakly and `WebView` keeps it alive.
        unsafe { tab.ivars().controller.didOpenTab(ProtocolObject::from_ref(&*tab)) };
        Some(tab)
    }

    fn new(view: &WKWebView, controller: Retained<WKWebExtensionController>, window: &Window) -> Retained<Tab> {
        let tab = Tab::alloc(view.mtm()).set_ivars(TabState { view: Weak::new(view), controller, window: Weak::new(window) });
        // SAFETY: NSObject's designated initialiser on a freshly allocated instance.
        let tab: Retained<Tab> = unsafe { msg_send![super(tab), init] };
        window.ivars().tabs.borrow_mut().push(Weak::from_retained(&tab));
        tab
    }

    pub(crate) fn activate(&self) {
        let window = window(self.mtm());
        let controller = &self.ivars().controller;
        let previous = window.ivars().active.borrow().load().filter(|t| std::ptr::eq(&*t.ivars().controller, &**controller));
        if previous.as_deref().is_some_and(|p| std::ptr::eq(p, self)) {
            return;
        }
        *window.ivars().active.borrow_mut() = Weak::new(self);
        // SAFETY: main thread; both tabs are alive.
        unsafe { self.ivars().controller.didActivateTab_previousActiveTab(ProtocolObject::from_ref(self), previous.as_deref().map(ProtocolObject::from_ref)) };
    }

    pub(crate) fn close(&self) {
        if let Some(view) = self.ivars().view.load() {
            end_once(&view);
        }
        window(self.mtm()).ivars().tabs.borrow_mut().retain(|t| t.load().is_some_and(|t| !std::ptr::eq(&*t, self)));
        // SAFETY: main thread; the tab is still alive.
        unsafe { self.ivars().controller.didCloseTab_windowIsClosing(ProtocolObject::from_ref(self), false) };
    }
}

#[derive(Default)]
pub(crate) struct WindowState {
    tabs: RefCell<Vec<Weak<Tab>>>,
    active: RefCell<Weak<Tab>>,
    popout: RefCell<Option<Popout>>,
}

struct Popout {
    panel: Retained<NSWindow>,
    view: Retained<WKWebView>,
    tab: Retained<Tab>,
    context: Retained<WKWebExtensionContext>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnExtensionWindow"]
    #[ivars = WindowState]
    pub(crate) struct Window;

    unsafe impl NSObjectProtocol for Window {}

    unsafe impl WKWebExtensionWindow for Window {
        #[unsafe(method_id(tabsForWebExtensionContext:))]
        fn tabs(&self, context: &WKWebExtensionContext) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionTab>>> {
            let tabs: Vec<_> = self.tabs_of(context).into_iter().map(ProtocolObject::from_retained).collect();
            NSArray::from_retained_slice(&tabs)
        }

        #[unsafe(method_id(activeTabForWebExtensionContext:))]
        fn active(&self, context: &WKWebExtensionContext) -> Option<Retained<ProtocolObject<dyn WKWebExtensionTab>>> {
            let tab = if self.is_popout() {
                self.popout_of(context).map(|(_, tab)| tab)
            } else {
                // SAFETY: main-thread read of a loaded context.
                unsafe { context.webExtensionController() }.and_then(|c| active_tab(&c))
            };
            tab.map(ProtocolObject::from_retained)
        }

        #[unsafe(method(windowTypeForWebExtensionContext:))]
        fn window_type(&self, _context: &WKWebExtensionContext) -> WKWebExtensionWindowType {
            if self.is_popout() { WKWebExtensionWindowType::Popup } else { WKWebExtensionWindowType::Normal }
        }

        #[unsafe(method(windowStateForWebExtensionContext:))]
        fn window_state(&self, _context: &WKWebExtensionContext) -> WKWebExtensionWindowState {
            WKWebExtensionWindowState::Normal
        }

        #[unsafe(method(isPrivateForWebExtensionContext:))]
        fn is_private(&self, context: &WKWebExtensionContext) -> bool {
            // SAFETY: main-thread reads of a loaded context.
            unsafe { context.webExtensionController() }.is_some_and(|c| unsafe { !c.configuration().isPersistent() })
        }

        /// Extensions place their popouts relative to this, and reject a window without one.
        #[unsafe(method(frameForWebExtensionContext:))]
        fn frame(&self, context: &WKWebExtensionContext) -> NSRect {
            if self.is_popout() {
                return self.popout_of(context).map_or(NSRect::ZERO, |(panel, _)| panel.frame());
            }
            // SAFETY: main-thread read of a loaded context.
            let controller = unsafe { context.webExtensionController() };
            let view = controller.and_then(|c| active_tab(&c)).and_then(|t| t.ivars().view.load());
            view.and_then(|v| v.window()).map_or(NSRect::ZERO, |w| w.frame())
        }

        #[unsafe(method(focusForWebExtensionContext:completionHandler:))]
        fn focus(&self, context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            if let Some((panel, _)) = self.popout_of(context) {
                panel.makeKeyAndOrderFront(None);
            }
            done.call((std::ptr::null_mut(),));
        }

        /// Extensions may close their own popouts, never the shell window.
        #[unsafe(method(closeForWebExtensionContext:completionHandler:))]
        fn close(&self, context: &WKWebExtensionContext, done: &DynBlock<dyn Fn(*mut NSError)>) {
            let error = match self.popout_of(context) {
                Some((panel, _)) => {
                    panel.close();
                    None
                }
                None => Some(NSError::new(1, &NSString::from_str("TarnExtensionWindow"))),
            };
            done.call((error.as_ref().map_or(std::ptr::null_mut(), |e| Retained::as_ptr(e).cast_mut()),));
        }
    }

    unsafe impl NSWindowDelegate for Window {
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, _notification: &NSNotification) {
            let this = self.retain();
            let Some(Popout { panel, view, tab, context }) = self.ivars().popout.borrow_mut().take() else { return };
            panel.setDelegate(None);
            POPOUTS.with_borrow_mut(|list| list.retain(|w| !std::ptr::eq(&**w, &*this)));
            // SAFETY: main thread; the tab and window are alive until `tab` and `this` drop.
            unsafe {
                context.didCloseTab_windowIsClosing(ProtocolObject::from_ref(&*tab), true);
                context.didCloseWindow(ProtocolObject::from_ref(&*this));
            }
            // AppKit is still inside `close` for this window, so release it with the event's pool.
            let _ = Retained::autorelease_ptr(panel);
            let _ = Retained::autorelease_ptr(view);
        }
    }

    unsafe impl WKNavigationDelegate for Window {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        fn decide_policy(&self, _web_view: &WKWebView, action: &WKNavigationAction, handler: &DynBlock<dyn Fn(WKNavigationActionPolicy)>) {
            // SAFETY: WebKit hands us a live navigation action on the main thread.
            let (main, url) = unsafe { (action.targetFrame().is_none_or(|f| f.isMainFrame()), action.request().URL().and_then(|u| u.absoluteString())) };
            // SAFETY: main-thread read of a loaded context.
            let host = self.ivars().popout.borrow().as_ref().and_then(|p| unsafe { p.context.baseURL() }.host());
            let allow = !main || url.zip(host).is_some_and(|(url, host)| own_page(&url.to_string(), &host.to_string()));
            handler.call((if allow { WKNavigationActionPolicy::Allow } else { WKNavigationActionPolicy::Cancel },));
        }
    }
);

impl Window {
    fn is_popout(&self) -> bool {
        self.ivars().popout.borrow().is_some()
    }

    fn popout_of(&self, context: &WKWebExtensionContext) -> Option<(Retained<NSWindow>, Retained<Tab>)> {
        let popout = self.ivars().popout.borrow();
        popout.as_ref().filter(|p| std::ptr::eq(&*p.context, context)).map(|p| (p.panel.clone(), p.tab.clone()))
    }

    fn tabs_of(&self, context: &WKWebExtensionContext) -> Vec<Retained<Tab>> {
        // SAFETY: main-thread read of a loaded context.
        let Some(controller) = (unsafe { context.webExtensionController() }) else { return Vec::new() };
        if self.is_popout() && self.popout_of(context).is_none() {
            return Vec::new();
        }
        let tabs = self.ivars().tabs.borrow().iter().filter_map(Weak::load).collect::<Vec<_>>();
        tabs.into_iter().filter(|t| std::ptr::eq(&*t.ivars().controller, &*controller)).collect()
    }
}

thread_local! {
    static WINDOW: RefCell<Option<Retained<Window>>> = const { RefCell::new(None) };
    static POPOUTS: RefCell<Vec<Retained<Window>>> = const { RefCell::new(Vec::new()) };
}

/// The shell has one window, shared by every profile; each extension sees only its own profile's tabs.
fn window(mtm: MainThreadMarker) -> Retained<Window> {
    WINDOW.with_borrow_mut(|w| {
        w.get_or_insert_with(|| {
            let window = Window::alloc(mtm).set_ivars(WindowState::default());
            // SAFETY: NSObject's designated initialiser on a freshly allocated instance.
            unsafe { msg_send![super(window), init] }
        })
        .clone()
    })
}

/// The tab the shell activated last, only if it belongs to `controller` and is on screen: a
/// toolbar click must never hand `activeTab` to a page in another window.
fn clicked_tab(controller: &WKWebExtensionController) -> Option<Retained<Tab>> {
    let active = window(MainThreadMarker::new()?).ivars().active.borrow().load()?;
    let view = active.ivars().view.load()?;
    let shown = view.window().is_some() && !view.isHiddenOrHasHiddenAncestor();
    (std::ptr::eq(&*active.ivars().controller, controller) && shown).then_some(active)
}

/// The tab shown last among this controller's tabs that is still on screen.
fn active_tab(controller: &WKWebExtensionController) -> Option<Retained<Tab>> {
    let window = window(MainThreadMarker::new()?);
    let on_screen = |t: &Tab| t.ivars().view.load().is_some_and(|v| v.window().is_some() && !v.isHiddenOrHasHiddenAncestor());
    let active = window.ivars().active.borrow().load();
    let ours = |t: &Retained<Tab>| std::ptr::eq(&*t.ivars().controller, controller);
    active.filter(|t| ours(t) && on_screen(t)).or_else(|| {
        let tabs = window.ivars().tabs.borrow().iter().filter_map(Weak::load).collect::<Vec<_>>();
        tabs.into_iter().rev().find(|t| ours(t) && on_screen(t))
    })
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "TarnExtensionControllerDelegate"]
    pub(crate) struct ControllerDelegate;

    unsafe impl NSObjectProtocol for ControllerDelegate {}

    unsafe impl WKWebExtensionControllerDelegate for ControllerDelegate {
        #[unsafe(method_id(webExtensionController:openWindowsForExtensionContext:))]
        fn open_windows(
            &self,
            _controller: &WKWebExtensionController,
            context: &WKWebExtensionContext,
        ) -> Retained<NSArray<ProtocolObject<dyn WKWebExtensionWindow>>> {
            let popouts = POPOUTS.with_borrow(|list| {
                let ours = |w: &&Retained<Window>| w.ivars().popout.borrow().as_ref().is_some_and(|p| std::ptr::eq(&*p.context, context));
                list.iter().filter(ours).cloned().collect::<Vec<_>>()
            });
            let windows: Vec<_> = std::iter::once(window(self.mtm())).chain(popouts).map(ProtocolObject::from_retained).collect();
            NSArray::from_retained_slice(&windows)
        }

        #[unsafe(method(webExtensionController:openNewWindowUsingConfiguration:forExtensionContext:completionHandler:))]
        fn open_new_window(
            &self,
            controller: &WKWebExtensionController,
            configuration: &WKWebExtensionWindowConfiguration,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionWindow>, *mut NSError)>,
        ) {
            // SAFETY: main-thread reads of a live configuration.
            let (url, frame) = unsafe { (configuration.tabURLs().firstObject(), configuration.frame()) };
            let Some((window, tab)) = url.and_then(|url| open_popout(controller, &url, frame, context)) else {
                let error = NSError::new(1, &NSString::from_str("TarnExtensionWindow"));
                done.call((std::ptr::null_mut(), Retained::as_ptr(&error).cast_mut()));
                return;
            };
            let opened = ProtocolObject::<dyn WKWebExtensionWindow>::from_ref(&*window);
            done.call((opened as *const _ as *mut _, std::ptr::null_mut()));
            // SAFETY: main thread; `POPOUTS` keeps the window and its tab alive until it closes.
            unsafe {
                context.didOpenWindow(opened);
                context.didOpenTab(ProtocolObject::from_ref(&*tab));
            }
        }

        /// `tabs.create`: the extension's own page opens as a pop-out window, a web page as a tab of
        /// the browser, at the pop-up rate, which the shell passes through its navigation policy. A web
        /// page's tab is not handed back, so the call resolves without one.
        #[unsafe(method(webExtensionController:openNewTabUsingConfiguration:forExtensionContext:completionHandler:))]
        fn open_new_tab(
            &self,
            controller: &WKWebExtensionController,
            configuration: &WKWebExtensionTabConfiguration,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut ProtocolObject<dyn WKWebExtensionTab>, *mut NSError)>,
        ) {
            // SAFETY: main-thread reads of a live configuration.
            let (url, active) = unsafe { (configuration.url(), configuration.shouldBeActive()) };
            let centred = NSRect::new(NSPoint::new(f64::NAN, f64::NAN), NSSize::new(0.0, 0.0));
            if let Some((window, tab)) = url.as_deref().and_then(|url| open_popout(controller, url, centred, context)) {
                let opened = ProtocolObject::<dyn WKWebExtensionTab>::from_ref(&*tab);
                done.call((opened as *const _ as *mut _, std::ptr::null_mut()));
                // SAFETY: main thread; `POPOUTS` keeps the window and its tab alive until it closes.
                unsafe {
                    context.didOpenWindow(ProtocolObject::from_ref(&*window));
                    context.didOpenTab(opened);
                }
                return;
            }
            let page =
                url.and_then(|u| u.absoluteString()).and_then(|u| url::Url::parse(&u.to_string()).ok()).filter(|u| matches!(u.scheme(), "http" | "https"));
            let allowed =
                page.is_some() && trusted(context) && TABS_OPENED.with(|recent| crate::delegates::within_burst(recent, crate::delegates::POPUP_BURST));
            if let (Some(page), Some((seed, _)), true) = (page, loaded_key(context), allowed) {
                let handler = OPEN_TAB.with_borrow(|h| h.clone());
                if let Some(handler) = handler {
                    handler(seed, page, active);
                }
            }
            done.call((std::ptr::null_mut(), std::ptr::null_mut()));
        }

        #[unsafe(method(webExtensionController:connectUsingMessagePort:forExtensionContext:completionHandler:))]
        fn connect_port(
            &self,
            _controller: &WKWebExtensionController,
            port: &WKWebExtensionMessagePort,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let error = (!answer_port(port, context)).then(|| NSError::new(1, &NSString::from_str("TarnExtensionPort")));
            done.call((error.as_ref().map_or(std::ptr::null_mut(), |e| Retained::as_ptr(e).cast_mut()),));
        }

        #[unsafe(method_id(webExtensionController:focusedWindowForExtensionContext:))]
        fn focused_window(
            &self,
            _controller: &WKWebExtensionController,
            _context: &WKWebExtensionContext,
        ) -> Option<Retained<ProtocolObject<dyn WKWebExtensionWindow>>> {
            Some(ProtocolObject::from_retained(window(self.mtm())))
        }

        #[unsafe(method(webExtensionController:presentPopupForAction:forExtensionContext:completionHandler:))]
        fn present_popup(
            &self,
            controller: &WKWebExtensionController,
            action: &WKWebExtensionAction,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let clicked =
                CLICKED.with_borrow_mut(Option::take).is_some_and(|(c, at)| std::ptr::eq(&*c, context) && at.elapsed() < std::time::Duration::from_secs(2));
            let error = (!((clicked || trusted(context)) && present(controller, action))).then(|| NSError::new(1, &NSString::from_str("TarnExtensionPopup")));
            done.call((error.as_ref().map_or(std::ptr::null_mut(), |e| Retained::as_ptr(e).cast_mut()),));
        }

        /// A run-time request gets only API permissions the user confirmed at install, and
        /// nothing at all if it asks for one more; the rest needs a new install prompt.
        #[unsafe(method(webExtensionController:promptForPermissions:inTab:forExtensionContext:completionHandler:))]
        fn prompt_permissions(
            &self,
            _controller: &WKWebExtensionController,
            permissions: &NSSet<NSString>,
            _tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(NonNull<NSSet<NSString>>, *mut NSDate)>,
        ) {
            let asked: Vec<String> = permissions.iter().map(|p| p.to_string()).collect();
            let granted = LOADED.with_borrow(|loaded| {
                loaded
                    .iter()
                    .find(|l| std::ptr::eq(&*l.context, context))
                    .is_some_and(|l| asked.iter().all(|p| l.confirmed.contains(p) && !(l.unpacked && UNPACKED_NEVER.contains(&p.as_str()))))
            });
            let answer = if granted { permissions.retain() } else { NSSet::new() };
            if !granted {
                mark(context);
            }
            done.call((NonNull::from(&*answer), std::ptr::null_mut()));
        }

        /// WebKit asks when an extension meets a page it requested and was not granted. The
        /// request is answered at once, with the listed URLs when every site is allowed and empty
        /// otherwise, so nothing waits on the user; an Allow grants the host afterwards and reloads
        /// the tab.
        #[unsafe(method(webExtensionController:promptForPermissionToAccessURLs:inTab:forExtensionContext:completionHandler:))]
        fn prompt_urls(
            &self,
            _controller: &WKWebExtensionController,
            urls: &NSSet<NSURL>,
            tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(NonNull<NSSet<NSURL>>, *mut NSDate)>,
        ) {
            // With every site allowed, the sites the extension lists need no answer from the user;
            // a Lockdown host stays denied and an unlisted one is asked for as usual.
            let granted = |u: &NSURL| every_site(context) && listed(context, u);
            let open: Vec<_> = urls.iter().filter(|u| granted(u)).collect();
            done.call((NonNull::from(&*NSSet::from_retained_slice(&open)), std::ptr::null_mut()));
            let hosts: std::collections::BTreeSet<String> = urls
                .iter()
                .filter(|u| !granted(u))
                .filter_map(|u| u.absoluteString().and_then(|u| url::Url::parse(&u.to_string()).ok()))
                .filter(|u| matches!(u.scheme(), "http" | "https"))
                .filter_map(|u| u.host_str().map(str::to_owned))
                .collect();
            if let Some(host) = hosts.into_iter().next() {
                ask_host(context, tab, host);
            }
        }

        /// A wildcard cannot be answered with a narrower pattern (WebKit throws): the
        /// answer is empty and the host of the tab is asked for instead.
        #[unsafe(method(webExtensionController:promptForPermissionMatchPatterns:inTab:forExtensionContext:completionHandler:))]
        fn prompt_patterns(
            &self,
            _controller: &WKWebExtensionController,
            patterns: &NSSet<WKWebExtensionMatchPattern>,
            tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>,
            context: &WKWebExtensionContext,
            done: &DynBlock<dyn Fn(NonNull<NSSet<WKWebExtensionMatchPattern>>, *mut NSDate)>,
        ) {
            done.call((NonNull::from(&*NSSet::new()), std::ptr::null_mut()));
            let view = tab.and_then(|t| AsRef::<AnyObject>::as_ref(t).downcast_ref::<Tab>()).and_then(|t| t.ivars().view.load());
            // SAFETY: main-thread read of a live view.
            let page = view.and_then(|v| unsafe { v.URL() });
            // With every site allowed a listed page is granted already; there is nothing to ask.
            if every_site(context) && page.as_deref().is_some_and(|url| listed(context, url)) {
                return;
            }
            // Only the tab's site, and only if the extension asked for a pattern that covers it.
            // SAFETY: main-thread reads of live patterns.
            let asked = page.as_deref().is_some_and(|url| patterns.iter().any(|p| unsafe { p.matchesURL(Some(url)) }));
            let host = page.filter(|_| asked).and_then(|u| u.absoluteString()).and_then(|u| url::Url::parse(&u.to_string()).ok());
            if let Some(host) = host.filter(|u| matches!(u.scheme(), "http" | "https")).and_then(|u| u.host_str().map(str::to_owned)) {
                ask_host(context, tab, host);
            }
        }

        #[unsafe(method(webExtensionController:sendMessage:toApplicationWithIdentifier:forExtensionContext:replyHandler:))]
        fn send_message(
            &self,
            controller: &WKWebExtensionController,
            message: &AnyObject,
            _application: Option<&NSString>,
            context: &WKWebExtensionContext,
            reply: &DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>,
        ) {
            native_message(controller, message, context, reply);
        }
    }
);

/// Only the extension's own pages: a web page there would bypass the shell's navigation policy and Shield.
fn own_page(url: &str, host: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| u.scheme() == SCHEME && u.host_str().is_some_and(|h| h.eq_ignore_ascii_case(host)))
}

fn close_popouts(context: &WKWebExtensionContext) {
    let panels: Vec<_> = POPOUTS.with_borrow(|list| {
        let ours = |w: &Retained<Window>| w.ivars().popout.borrow().as_ref().filter(|p| std::ptr::eq(&*p.context, context)).map(|p| p.panel.clone());
        list.iter().filter_map(ours).collect()
    });
    for panel in panels {
        panel.close();
    }
}

fn open_popout(
    controller: &WKWebExtensionController,
    url: &NSURL,
    frame: NSRect,
    context: &WKWebExtensionContext,
) -> Option<(Retained<Window>, Retained<Tab>)> {
    let mtm = MainThreadMarker::new()?;
    // SAFETY: main-thread reads of a loaded context.
    let (base, web_config, title) = unsafe { (context.baseURL(), context.webViewConfiguration()?, context.webExtension().displayName()) };
    let own = url.absoluteString().zip(base.host()).is_some_and(|(url, host)| own_page(&url.to_string(), &host.to_string()));
    let open =
        POPOUTS.with_borrow(|list| list.iter().filter(|w| w.ivars().popout.borrow().as_ref().is_some_and(|p| std::ptr::eq(&*p.context, context))).count());
    if !own || open >= MAX_POPOUTS || !trusted(context) {
        return None;
    }
    let length = |value: f64, fallback: f64| if value.is_finite() && value > 0.0 { value.clamp(POPOUT_MIN_SIDE, POPOUT_MAX_SIDE) } else { fallback };
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(length(frame.size.width, POPOUT_SIZE.0), length(frame.size.height, POPOUT_SIZE.1)));
    // SAFETY: main-thread creation and configuration of fresh AppKit and WebKit objects. The panel is
    // not released on close because `Popout` owns it.
    unsafe {
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;
        let panel = NSWindow::initWithContentRect_styleMask_backing_defer(NSWindow::alloc(mtm), rect, style, NSBackingStoreType::Buffered, false);
        panel.setReleasedWhenClosed(false);
        if let Some(title) = title {
            panel.setTitle(&title);
        }
        let view = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), rect, &web_config);
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        panel.setContentView(Some(&view));
        let window = Window::alloc(mtm).set_ivars(WindowState::default());
        let window: Retained<Window> = msg_send![super(window), init];
        let tab = Tab::new(&view, controller.retain(), &window);
        panel.setDelegate(Some(ProtocolObject::from_ref(&*window)));
        view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*window)));
        view.loadRequest(&NSURLRequest::requestWithURL(url));
        if frame.origin.x.is_finite() && frame.origin.y.is_finite() {
            panel.setFrameOrigin(frame.origin);
        } else {
            panel.center();
        }
        match panel.screen() {
            Some(screen) => panel.setFrame_display(panel.constrainFrameRect_toScreen(panel.frame(), Some(&screen)), false),
            None => panel.center(),
        }
        panel.makeKeyAndOrderFront(None);
        *window.ivars().popout.borrow_mut() = Some(Popout { panel, view: view.clone(), tab: tab.clone(), context: context.retain() });
        POPOUTS.with_borrow_mut(|list| list.push(window.clone()));
        Some((window, tab))
    }
}

/// Shows the popup under the toolbar, at the top right of the active page; false without one on screen.
fn present(controller: &WKWebExtensionController, action: &WKWebExtensionAction) -> bool {
    let Some(view) = active_tab(controller).and_then(|t| t.ivars().view.load()) else { return false };
    // SAFETY: main thread; the anchor view is on screen (checked by `active_tab`), as AppKit requires.
    unsafe {
        let Some(popover) = action.popupPopover() else { return false };
        popover.setBehavior(NSPopoverBehavior::Transient);
        let bounds = view.bounds();
        let flipped = view.isFlipped();
        let x = (bounds.size.width - 40.0).max(0.0);
        let y = if flipped { 0.0 } else { bounds.size.height - 1.0 };
        let edge = if flipped { NSRectEdge::MinY } else { NSRectEdge::MaxY };
        popover.showRelativeToRect_ofView_preferredEdge(NSRect::new(NSPoint::new(x, y), NSSize::new(1.0, 1.0)), &view, edge);
    }
    true
}

/// An extension sending `sleep` in a loop would otherwise pile up timers and held replies. Counted
/// per extension context, so one extension or profile cannot use up another's budget and make its
/// sleeps return at once.
const MAX_PENDING_SLEEPS: usize = 16;

thread_local! {
    /// By the context's unique identifier, which a later context never reuses by accident.
    static PENDING_SLEEPS: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

/// Counts a sleep for `context` if it is under its cap.
fn start_sleep(context: &str) -> bool {
    PENDING_SLEEPS.with_borrow_mut(|pending| {
        let count = pending.entry(context.to_owned()).or_insert(0);
        let room = *count < MAX_PENDING_SLEEPS;
        *count += usize::from(room);
        room
    })
}

fn end_sleep(context: &str) {
    PENDING_SLEEPS.with_borrow_mut(|pending| {
        if let Some(count) = pending.get_mut(context) {
            *count -= 1;
            if *count == 0 {
                pending.remove(context);
            }
        }
    });
}

type OpenPort = (Retained<WKWebExtensionMessagePort>, Retained<WKWebExtensionContext>);

struct Loaded {
    context: Retained<WKWebExtensionContext>,
    seed: String,
    key: String,
    unpacked: bool,
    confirmed: Vec<String>,
    all_sites: bool,
}

struct OnceGrant {
    seed: String,
    key: String,
    host: String,
    until: std::time::Instant,
    /// Once ends with the tab it was answered in.
    view: Weak<WKWebView>,
}

/// What the user answered for a host, to be stored for the extension `key` of the profile whose
/// extension storage is `seed`.
pub struct HostDecision {
    pub seed: String,
    pub key: String,
    pub host: String,
    pub allow: bool,
}

type DecisionHandler = Rc<dyn Fn(HostDecision)>;
type OpenTabHandler = Rc<dyn Fn(String, url::Url, bool)>;
type Finish = Box<dyn FnOnce()>;

thread_local! {
    static LOADED: RefCell<Vec<Loaded>> = const { RefCell::new(Vec::new()) };
    static ONCE: RefCell<Vec<OnceGrant>> = const { RefCell::new(Vec::new()) };
    /// Extensions whose request was denied without asking: (seed, key), shown on their button.
    static MARKED: RefCell<std::collections::HashSet<(String, String)>> = RefCell::new(std::collections::HashSet::new());
    /// One prompt per extension and site in a session: (seed, key, host).
    static PROMPTED: RefCell<std::collections::HashSet<(String, String, String)>> = RefCell::new(std::collections::HashSet::new());
    /// The context of the last toolbar click, honoured for a popup for a moment only.
    static CLICKED: RefCell<Option<(Retained<WKWebExtensionContext>, std::time::Instant)>> = const { RefCell::new(None) };
    static DECISIONS: RefCell<Option<DecisionHandler>> = const { RefCell::new(None) };
    static OPEN_TAB: RefCell<Option<OpenTabHandler>> = const { RefCell::new(None) };
    static TABS_OPENED: RefCell<std::collections::VecDeque<std::time::Instant>> = const { RefCell::new(std::collections::VecDeque::new()) };
}

fn mark(context: &WKWebExtensionContext) {
    if let Some(key) = loaded_key(context) {
        MARKED.with_borrow_mut(|m| m.insert(key));
    }
}

/// Whether a request of the extension `key` in the profile `seed` was denied without a prompt
/// since its toolbar button was last clicked.
pub fn denied_request(seed: &str, key: &str) -> bool {
    MARKED.with_borrow(|m| m.iter().any(|(s, k)| s.eq_ignore_ascii_case(seed) && k == key))
}

pub fn clear_denied_request(seed: &str, key: &str) {
    MARKED.with_borrow_mut(|m| m.retain(|(s, k)| !(s.eq_ignore_ascii_case(seed) && k == key)));
}

/// Revokes the Once grants answered in `view`, which is closing.
fn end_once(view: &WKWebView) {
    let ended: Vec<OnceGrant> = ONCE.with_borrow_mut(|once| {
        let (ended, kept) = std::mem::take(once).into_iter().partition(|g| g.view.load().is_none_or(|v| std::ptr::eq(&*v, view)));
        *once = kept;
        ended
    });
    let Some(mtm) = MainThreadMarker::new() else { return };
    for g in ended {
        // SAFETY: main thread; the pattern string is built from a host name.
        let Some(pattern) = (unsafe { WKWebExtensionMatchPattern::matchPatternWithString(&NSString::from_str(&format!("*://{}/*", g.host)), mtm) }) else {
            continue;
        };
        LOADED.with_borrow(|loaded| {
            for l in loaded.iter().filter(|l| l.seed == g.seed && l.key == g.key) {
                // A host denied since the grant (Lockdown, refused) keeps its denial (S18).
                // SAFETY: main thread; the context is loaded.
                if unsafe { l.context.permissionStatusForMatchPattern(&pattern) } == WKWebExtensionContextPermissionStatus::DeniedExplicitly {
                    continue;
                }
                // WebKit asserts on any status but denied, unknown and granted; unknown makes it ask again.
                // SAFETY: main thread; the context is loaded.
                unsafe { l.context.setPermissionStatus_forMatchPattern(WKWebExtensionContextPermissionStatus::Unknown, &pattern) };
            }
        });
    }
}

/// Receives Always and Never answers to host prompts, for the shell to store.
pub fn on_host_decision(handler: impl Fn(HostDecision) + 'static) {
    DECISIONS.with_borrow_mut(|h| *h = Some(Rc::new(handler)));
}

/// Receives the web pages app extensions open with `tabs.create`: the seed of their profile's
/// store, the page and whether its window should come to the front.
pub fn on_open_tab(handler: impl Fn(String, url::Url, bool) + 'static) {
    OPEN_TAB.with_borrow_mut(|h| *h = Some(Rc::new(handler)));
}

/// App extensions; unpacked ones get no pop-out windows and no popup they open themselves.
fn trusted(context: &WKWebExtensionContext) -> bool {
    LOADED.with_borrow(|loaded| loaded.iter().any(|l| std::ptr::eq(&*l.context, context) && !l.unpacked))
}

fn every_site(context: &WKWebExtensionContext) -> bool {
    LOADED.with_borrow(|loaded| loaded.iter().any(|l| std::ptr::eq(&*l.context, context) && l.all_sites))
}

/// Whether the extension's manifest asks for `url` and the user has not denied it.
fn listed(context: &WKWebExtensionContext, url: &NSURL) -> bool {
    // SAFETY: main-thread reads of a loaded context and its extension's patterns.
    unsafe {
        context.permissionStatusForURL(url) != WKWebExtensionContextPermissionStatus::DeniedExplicitly
            && context.webExtension().allRequestedMatchPatterns().iter().any(|p| p.matchesURL(Some(url)))
    }
}

fn loaded_key(context: &WKWebExtensionContext) -> Option<(String, String)> {
    LOADED.with_borrow(|loaded| loaded.iter().find(|l| std::ptr::eq(&*l.context, context)).map(|l| (l.seed.clone(), l.key.clone())))
}

/// A host name or IP address that names one site: a `*` would turn a grant into a wildcard.
pub fn plain_host(host: &str) -> bool {
    !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b)) && !host.starts_with('.')
}

/// Forgets what this session answered for the extension `key` of the profile `seed` (all of the
/// profile's with `None`): Once grants and the asked-once marks go with Remove and data removal.
pub fn forget_session(seed: &str, key: Option<&str>) {
    let ours = |s: &str, k: &str| s == seed && key.is_none_or(|key| key == k);
    ONCE.with_borrow_mut(|once| once.retain(|g| !ours(&g.seed, &g.key)));
    PROMPTED.with_borrow_mut(|p| p.retain(|(s, k, _)| !ours(s, k)));
    MARKED.with_borrow_mut(|m| m.retain(|(s, k)| !ours(s, k)));
}

/// Applies an answer to the host prompt of `context` for `host`: `None` dismisses it.
fn decide(context: &WKWebExtensionContext, seed: &str, key: &str, host: &str, allow: Option<bool>, remember: bool, view: Option<Retained<WKWebView>>) {
    let Some(allow) = allow else {
        MARKED.with_borrow_mut(|m| m.insert((seed.to_owned(), key.to_owned())));
        return;
    };
    let Some(mtm) = MainThreadMarker::new() else { return };
    // SAFETY: main thread; the pattern string is built from a host name.
    let Some(pattern) = (unsafe { WKWebExtensionMatchPattern::matchPatternWithString(&NSString::from_str(&format!("*://{host}/*")), mtm) }) else {
        return;
    };
    // SAFETY: main-thread read of a loaded context.
    if unsafe { context.permissionStatusForMatchPattern(&pattern) } == WKWebExtensionContextPermissionStatus::DeniedExplicitly {
        return;
    }
    let status = if allow { WKWebExtensionContextPermissionStatus::GrantedExplicitly } else { WKWebExtensionContextPermissionStatus::DeniedExplicitly };
    let until = (allow && !remember).then(|| NSDate::dateWithTimeIntervalSinceNow(ONCE_SECONDS));
    // SAFETY: main thread; the context is loaded.
    unsafe { context.setPermissionStatus_forMatchPattern_expirationDate(status, &pattern, until.as_deref()) };
    if remember {
        if let Some(handler) = DECISIONS.with_borrow(|h| h.clone()) {
            handler(HostDecision { seed: seed.to_owned(), key: key.to_owned(), host: host.to_owned(), allow });
        }
    } else if allow && let Some(view) = &view {
        let until = std::time::Instant::now() + std::time::Duration::from_secs_f64(ONCE_SECONDS);
        let grant = OnceGrant { seed: seed.to_owned(), key: key.to_owned(), host: host.to_owned(), until, view: Weak::new(&**view) };
        ONCE.with_borrow_mut(|once| once.push(grant));
    }
    if allow && let Some(view) = view {
        // SAFETY: main thread; a live view.
        unsafe { view.reload() };
    }
}

/// Asks, tab-modally in the tab showing the page, whether `context` may read and change `host`;
/// once per extension and site in a session.
fn ask_host(context: &WKWebExtensionContext, tab: Option<&ProtocolObject<dyn WKWebExtensionTab>>, host: String) {
    let Some((seed, key)) = loaded_key(context).filter(|_| plain_host(&host)) else { return };
    // A Lockdown host or one answered Never is denied explicitly and never asked about.
    let url = NSURL::URLWithString(&NSString::from_str(&format!("https://{host}/")));
    // SAFETY: main-thread read of a loaded context.
    if url.is_none_or(|u| unsafe { context.permissionStatusForURL(&u) } == WKWebExtensionContextPermissionStatus::DeniedExplicitly) {
        return;
    }
    let view = tab.and_then(|t| AsRef::<AnyObject>::as_ref(t).downcast_ref::<Tab>()).and_then(|t| t.ivars().view.load());
    // SAFETY: main-thread read of a live view.
    let delegate = view.as_ref().and_then(|v| unsafe { v.UIDelegate() });
    let delegate = delegate.and_then(|d| AsRef::<AnyObject>::as_ref(&*d).downcast_ref::<crate::delegates::Delegate>().map(|d| d.retain()));
    let (Some(view), Some(delegate)) = (view, delegate) else {
        MARKED.with_borrow_mut(|m| m.insert((seed, key)));
        return;
    };
    if !PROMPTED.with_borrow_mut(|p| p.insert((seed.clone(), key.clone(), host.clone()))) {
        MARKED.with_borrow_mut(|m| m.insert((seed, key)));
        return;
    }
    // SAFETY: main-thread read of a loaded context.
    let name = unsafe { context.webExtension().displayName() };
    let name = name.map(|n| n.to_string()).unwrap_or_default();
    let (context, weak, prompt_key, prompt_host) = (context.retain(), Weak::new(&*view), key.clone(), host.clone());
    let reply = move |allow: Option<bool>, remember: bool| decide(&context, &seed, &key, &host, allow, remember, weak.load());
    delegate.extension_prompt(name, prompt_key, prompt_host, Box::new(reply));
}

thread_local! {
    static PORTS: RefCell<Vec<OpenPort>> = const { RefCell::new(Vec::new()) };
    static NATIVE: RefCell<Vec<(Retained<WKWebExtensionContext>, std::path::PathBuf)>> = const { RefCell::new(Vec::new()) };
}

fn release(context: &WKWebExtensionContext) {
    LOADED.with_borrow_mut(|loaded| loaded.retain(|l| !std::ptr::eq(&*l.context, context)));
    close_popouts(context);
    close_ports(context);
    NATIVE.with_borrow_mut(|native| native.retain(|(c, _)| !std::ptr::eq(&**c, context)));
}

/// The bundle of a context allowed to use Bitwarden's native port.
fn speaks_native(context: &WKWebExtensionContext) -> Option<std::path::PathBuf> {
    NATIVE.with_borrow(|native| native.iter().find(|(c, _)| std::ptr::eq(&**c, context)).map(|(_, p)| p.clone()))
}

/// Bitwarden's unlock page waits on this port for an app that is not there.
fn answer_port(port: &WKWebExtensionMessagePort, context: &WKWebExtensionContext) -> bool {
    // SAFETY: main-thread reads of a live port.
    let (application, closed) = unsafe { (port.applicationIdentifier(), port.isDisconnected()) };
    let open = PORTS.with_borrow(|ports| ports.iter().filter(|(_, c)| std::ptr::eq(&**c, context)).count());
    let Some(path) = speaks_native(context) else { return false };
    if closed || open >= MAX_PORTS || application.is_none_or(|a| a.to_string() != BITWARDEN_APP) {
        return false;
    }
    // SAFETY: main-thread read of a loaded context.
    let app = unsafe { context.webExtension().displayName() }.map_or_else(|| "Bitwarden".to_owned(), |n| n.to_string());
    PORTS.with_borrow_mut(|ports| ports.push((port.retain(), context.retain())));
    let weak = Weak::new(port);
    let on_disconnect = weak.clone();
    let message = RcBlock::new(move |message: *mut AnyObject, _: *mut NSError| {
        // SAFETY: WebKit passes a valid message or null.
        if let (Some(port), Some(message)) = (weak.load(), unsafe { message.as_ref() }) {
            answer(&port, message, &app, &path);
        }
    });
    let disconnect = RcBlock::new(move |_: *mut NSError| {
        let closed = on_disconnect.load();
        PORTS.with_borrow_mut(|ports| ports.retain(|(p, _)| closed.as_deref().is_none_or(|c| !std::ptr::eq(&**p, c))));
    });
    // SAFETY: main thread; the port copies both blocks.
    unsafe {
        port.setMessageHandler(Some(&message));
        port.setDisconnectHandler(Some(&disconnect));
    }
    true
}

fn close_ports(context: &WKWebExtensionContext) {
    let closing: Vec<_> = PORTS.with_borrow_mut(|ports| {
        let (closing, kept) = std::mem::take(ports).into_iter().partition(|(_, c)| std::ptr::eq(&**c, context));
        *ports = kept;
        closing
    });
    for (port, _) in closing {
        // SAFETY: main thread; the port is alive while `closing` holds it.
        unsafe { port.disconnect() };
    }
}

#[derive(Debug, PartialEq)]
enum Request {
    Status(Option<String>),
    Authenticate,
    Unlock(String),
}

/// Bitwarden's biometrics questions, which its Safari app extension answers.
fn route(command: &str, user: Option<String>) -> Option<Request> {
    match command {
        "getBiometricsStatus" => Some(Request::Status(None)),
        "getBiometricsStatusForUser" => Some(Request::Status(Some(user.unwrap_or_default()))),
        "authenticateWithBiometrics" => Some(Request::Authenticate),
        "unlockWithBiometricsForUser" => Some(Request::Unlock(user.unwrap_or_default())),
        _ => None,
    }
}

enum Answer {
    Status(i32),
    Done(bool, Option<String>),
}

fn answer(port: &WKWebExtensionMessagePort, message: &AnyObject, app: &str, path: &Path) {
    let Some((id, command, user)) = port_request(message) else { return };
    let Some(request) = route(&command, user) else { return };
    let later = MainThreadBound::new((Weak::new(port), id), port.mtm());
    match request {
        Request::Status(user) => biometrics::status(user, move |status| reply_later(later, Answer::Status(status))),
        Request::Authenticate => biometrics::evaluate(app, |ok| ok, move |ok| reply_later(later, Answer::Done(ok, None))),
        // The bundle is checked again, off the main thread, before the key leaves the keychain: it
        // was checked at discovery, and WebKit reads its scripts lazily.
        Request::Unlock(user) => {
            let (path, app) = (path.to_owned(), app.to_owned());
            std::thread::spawn(move || {
                let genuine = signed_by_bitwarden(&path);
                dispatch2::DispatchQueue::main().exec_async(move || {
                    if genuine {
                        biometrics::unlock(&user, &app, move |key| reply_later(later, Answer::Done(key.is_some(), key)));
                    } else {
                        reply_later(later, Answer::Done(false, None));
                    }
                });
            });
        }
    }
}

fn reply_later(later: MainThreadBound<(Weak<WKWebExtensionMessagePort>, Retained<AnyObject>)>, answer: Answer) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let (port, id) = later.into_inner(mtm);
    let Some(port) = port.load() else { return };
    let reply = match answer {
        Answer::Status(status) => port_reply(id, NSNumber::new_i32(status).into(), None),
        Answer::Done(ok, key) => port_reply(id, NSNumber::new_bool(ok).into(), key.as_deref()),
    };
    // SAFETY: main thread; the reply holds only property-list values.
    unsafe { port.sendMessage_completionHandler(Some(&reply), None) };
}

fn port_request(message: &AnyObject) -> Option<(Retained<AnyObject>, String, Option<String>)> {
    let request = message.downcast_ref::<NSDictionary>()?;
    let text = |key: &str| request.objectForKey(&NSString::from_str(key)).and_then(|v| v.downcast::<NSString>().ok()).map(|v| v.to_string());
    Some((request.objectForKey(&NSString::from_str("messageId"))?, text("command")?, text("userId")))
}

fn port_reply(id: Retained<AnyObject>, response: Retained<AnyObject>, user_key: Option<&str>) -> Retained<AnyObject> {
    let millis = NSDate::now().timeIntervalSince1970() * 1000.0;
    let mut keys = vec![NSString::from_str("messageId"), NSString::from_str("timestamp"), NSString::from_str("response")];
    let mut values: Vec<Retained<AnyObject>> = vec![id, NSNumber::new_f64(millis).into(), response];
    if let Some(key) = user_key {
        keys.push(NSString::from_str("userKeyB64"));
        values.push(NSString::from_str(key).into());
    }
    let keys: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let reply = NSDictionary::<NSString, AnyObject>::from_retained_objects(&keys, &values);
    NSDictionary::<NSString, AnyObject>::from_retained_objects(&[&*NSString::from_str("message")], &[reply.into()]).into()
}

/// The messages the Safari build of Bitwarden sends to its app; anything else gets no reply value.
fn native_message(
    controller: &WKWebExtensionController,
    message: &AnyObject,
    context: &WKWebExtensionContext,
    reply: &DynBlock<dyn Fn(*mut AnyObject, *mut NSError)>,
) {
    let field = |key: &str| -> Option<Retained<NSString>> {
        let dict = message.downcast_ref::<NSDictionary>()?;
        dict.objectForKey(&NSString::from_str(key))?.downcast::<NSString>().ok()
    };
    let none = || reply.call((std::ptr::null_mut(), std::ptr::null_mut()));
    if speaks_native(context).is_none() {
        return none();
    }
    // SAFETY: main-thread reads of a loaded context and an exported WebKit constant.
    let writes = unsafe { WKWebExtensionPermissionClipboardWrite.is_some_and(|p| context.hasPermission(p)) };
    // SAFETY: main-thread read of a loaded context.
    let reads = writes && reads_clipboard(&*unsafe { context.webExtension() });
    match field("command").map(|c| c.to_string()).as_deref() {
        Some("copyToClipboard") if writes => {
            if let Some(text) = field("data") {
                let pasteboard = NSPasteboard::generalPasteboard();
                pasteboard.clearContents();
                // SAFETY: an exported AppKit constant.
                pasteboard.setString_forType(&text, unsafe { NSPasteboardTypeString });
            }
            none();
        }
        Some("readFromClipboard") if reads => {
            // SAFETY: an exported AppKit constant.
            let text = NSPasteboard::generalPasteboard().stringForType(unsafe { NSPasteboardTypeString });
            let text = text.as_ref().map_or(std::ptr::null_mut(), |t| Retained::as_ptr(t) as *mut AnyObject);
            reply.call((text, std::ptr::null_mut()));
        }
        // SAFETY: main-thread read of a loaded context.
        Some("sleep") if start_sleep(&unsafe { context.uniqueIdentifier() }.to_string()) => {
            // SAFETY: as above.
            let id = unsafe { context.uniqueIdentifier() }.to_string();
            let reply = reply.copy();
            let fire = RcBlock::new(move |_: std::ptr::NonNull<NSTimer>| {
                end_sleep(&id);
                reply.call((std::ptr::null_mut(), std::ptr::null_mut()));
            });
            // SAFETY: main thread; the run loop retains the timer and the timer copies the block.
            unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(SLEEP_SECONDS, false, &fire) };
        }
        Some("showPopover") => {
            let tab = active_tab(controller);
            // SAFETY: main thread; WebKit calls the delegate to present the popup.
            unsafe { context.performActionForTab(tab.as_deref().map(ProtocolObject::from_ref)) };
            none();
        }
        _ => none(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Request, context_id, own_page, port_reply, port_request, route};
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2_foundation::{NSDictionary, NSNumber, NSString};

    fn request(command: &str, id: Option<i32>) -> Retained<AnyObject> {
        let mut keys = vec![NSString::from_str("command")];
        let mut values: Vec<Retained<AnyObject>> = vec![NSString::from_str(command).into()];
        if let Some(id) = id {
            keys.push(NSString::from_str("messageId"));
            values.push(NSNumber::new_i32(id).into());
        }
        let keys: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
        NSDictionary::<NSString, AnyObject>::from_retained_objects(&keys, &values).into()
    }

    fn field(message: &NSDictionary, key: &str) -> Option<Retained<AnyObject>> {
        message.objectForKey(&NSString::from_str(key))
    }

    #[test]
    fn native_port_answers_only_bitwardens_biometrics_questions() {
        let user = || Some("u1".to_owned());
        assert_eq!(route("getBiometricsStatus", user()), Some(Request::Status(None)));
        assert_eq!(route("getBiometricsStatusForUser", user()), Some(Request::Status(Some("u1".into()))));
        assert_eq!(route("getBiometricsStatusForUser", None), Some(Request::Status(Some(String::new()))));
        assert_eq!(route("authenticateWithBiometrics", user()), Some(Request::Authenticate));
        assert_eq!(route("unlockWithBiometricsForUser", user()), Some(Request::Unlock("u1".into())));
        assert_eq!(route("setupEncryption", user()), None);
        assert_eq!(route("canEnableBiometricUnlock", user()), None);
    }

    #[test]
    fn bitwardens_signature_requirement_parses() {
        assert!(super::BITWARDEN_SIGNATURE.parse::<security_framework::os::macos::code_signing::SecRequirement>().is_ok());
    }

    #[test]
    fn native_port_requests_and_replies_keep_bitwardens_shape() {
        let (id, command, user) = port_request(&request("unlockWithBiometricsForUser", Some(5))).unwrap();
        assert_eq!((command.as_str(), user), ("unlockWithBiometricsForUser", None));
        assert!(port_request(&request("getBiometricsStatus", None)).is_none());

        let reply = port_reply(id, NSNumber::new_bool(true).into(), Some("a2V5"));
        let message = field(reply.downcast_ref::<NSDictionary>().unwrap(), "message").unwrap();
        let message = message.downcast_ref::<NSDictionary>().unwrap();
        let number = |key: &str| field(message, key).unwrap().downcast::<NSNumber>().unwrap();
        assert_eq!(number("messageId").intValue(), 5);
        assert!(number("response").boolValue());
        assert!(number("timestamp").doubleValue() > 1.7e12);
        assert_eq!(field(message, "userKeyB64").unwrap().downcast::<NSString>().unwrap().to_string(), "a2V5");

        let (id, ..) = port_request(&request("getBiometricsStatus", Some(6))).unwrap();
        let reply = port_reply(id, NSNumber::new_i32(2).into(), None);
        let message = field(reply.downcast_ref::<NSDictionary>().unwrap(), "message").unwrap();
        assert!(field(message.downcast_ref::<NSDictionary>().unwrap(), "userKeyB64").is_none());
    }

    #[test]
    fn popouts_load_only_the_extensions_own_pages() {
        let host = "9F8E6AB3-9B11-649E-3CD1-56EA0BB00266";
        assert!(own_page("safari-web-extension://9F8E6AB3-9B11-649E-3CD1-56EA0BB00266/popup/index.html", host));
        assert!(own_page("safari-web-extension://9f8e6ab3-9b11-649e-3cd1-56ea0bb00266/popup/index.html#/lock", host));
        assert!(!own_page("safari-web-extension://00000000-0000-0000-0000-000000000000/popup/index.html", host));
        assert!(!own_page("webkit-extension://9F8E6AB3-9B11-649E-3CD1-56EA0BB00266/popup/index.html", host));
        assert!(!own_page("https://9F8E6AB3-9B11-649E-3CD1-56EA0BB00266/", host));
        assert!(!own_page("https://example.com/", host));
        assert!(!own_page("not a url", host));
    }

    #[test]
    fn grants_are_only_ever_for_one_plain_host() {
        use super::plain_host;
        assert!(plain_host("example.com") && plain_host("127.0.0.1") && plain_host("[::1]") && plain_host("xn--bcher-kva.de"));
        for bad in ["", "*", "*.com", "a.com/x", ".com", "a b", "a.com*"] {
            assert!(!plain_host(bad), "{bad}");
        }
    }

    #[test]
    fn sleep_budget_is_per_context() {
        use super::{MAX_PENDING_SLEEPS, end_sleep, start_sleep};
        assert!((0..MAX_PENDING_SLEEPS).all(|_| start_sleep("a")));
        assert!(!start_sleep("a"), "a full context waits instead of answering at once");
        assert!(start_sleep("b"), "another context keeps its own budget");
        end_sleep("a");
        assert!(start_sleep("a"));
        for _ in 0..MAX_PENDING_SLEEPS {
            end_sleep("a");
        }
        end_sleep("b");
        end_sleep("never-started");
        assert!(super::PENDING_SLEEPS.with_borrow(|p| p.is_empty()));
    }

    #[test]
    fn context_ids_are_stable_per_profile_and_extension() {
        let a = context_id("seed-a", "/Applications/X.app/Contents/PlugIns/x.appex");
        assert_eq!(a, context_id("seed-a", "/Applications/X.app/Contents/PlugIns/x.appex"));
        assert_ne!(a, context_id("seed-b", "/Applications/X.app/Contents/PlugIns/x.appex"));
        assert_ne!(a, context_id("seed-a", "/Applications/Y.app/Contents/PlugIns/y.appex"));
        assert_eq!(a.len(), 36);
        assert!(a.chars().all(|c| c == '-' || c.is_ascii_hexdigit()));
    }
}
