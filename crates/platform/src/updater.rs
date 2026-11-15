//! Updates. macOS uses Sparkle 2, loaded from the app bundle only when its Info.plist names a
//! feed and a public key, so unbundled builds and tests never load it. Windows release builds
//! check a signed JSON feed themselves (`windows`).

/// Whether `feed` is an https URL without credentials and `key` is Sparkle's base64 EdDSA public
/// key; anything else leaves update checks off.
pub fn configured(feed: Option<&str>, key: Option<&str>) -> bool {
    let feed = feed
        .and_then(|feed| url::Url::parse(feed).ok())
        .is_some_and(|url| url.scheme() == "https" && url.host_str().is_some() && url.username().is_empty() && url.password().is_none());
    let key = key
        .map(str::as_bytes)
        .is_some_and(|key| key.len() == 44 && key[43] == b'=' && key[..43].iter().all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/'));
    feed && key
}

/// Whether an appcast whose items carry `versions` (`sparkle:version`) lists `running` or a later
/// version, so a replayed appcast from before this release does not count as a current check.
/// Versions compare as dot-separated numbers; anything else only matches itself.
pub fn lists_current<'a>(versions: impl IntoIterator<Item = &'a str>, running: &str) -> bool {
    let numeric = |version: &str| {
        let mut parts = version.split('.').map(|part| part.parse::<u64>().ok()).collect::<Option<Vec<_>>>()?;
        while parts.last() == Some(&0) {
            parts.pop();
        }
        Some(parts)
    };
    let running_parts = numeric(running);
    versions.into_iter().any(|version| version == running || numeric(version).zip(running_parts.as_ref()).is_some_and(|(version, running)| &version >= running))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    FeedVerified,
    CriticalFound,
    /// Windows: the installer of this version is downloaded and verified.
    UpdateReady(String),
    /// Windows: a check asked for found nothing newer.
    UpToDate,
    /// Windows: a check asked for, or its download, failed.
    CheckFailed,
}

#[cfg(target_os = "macos")]
pub use mac::Updater;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Updater;

#[cfg(target_os = "macos")]
mod mac {
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{NSArray, NSBundle, NSError, NSObject, NSString};

    use super::Report;

    struct Ivars {
        report: Box<dyn Fn(Report)>,
        feed: Retained<NSString>,
        version: String,
    }

    /// `SPUAppcastSigningValidationStatusSucceeded` in Sparkle 2.10.1's `SPUAppcastSigningValidationStatus.h`.
    const SIGNING_SUCCEEDED: isize = 1;

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "TarnUpdaterDelegate"]
        #[ivars = Ivars]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        impl Delegate {
            /// Sparkle asks its delegate before user defaults, where any process running as the user
            /// could store another feed.
            #[unsafe(method_id(feedURLStringForUpdater:))]
            fn feed_url_string(&self, _updater: &AnyObject) -> Retained<NSString> {
                self.ivars().feed.clone()
            }

            #[unsafe(method(updater:didFinishLoadingAppcast:))]
            fn did_finish_loading_appcast(&self, _updater: &AnyObject, appcast: &AnyObject) {
                // SAFETY: `appcast` is an `SUAppcast`, whose `signingValidationStatus` is an NSInteger
                // enum and `items` a non-null array of `SUAppcastItem` with a non-null `versionString`.
                let (status, versions) = unsafe {
                    let status: isize = msg_send![appcast, signingValidationStatus];
                    let items: Retained<NSArray<AnyObject>> = msg_send![appcast, items];
                    let versions: Vec<String> = items
                        .iter()
                        .map(|item| {
                            let version: Retained<NSString> = msg_send![&item, versionString];
                            version.to_string()
                        })
                        .collect();
                    (status, versions)
                };
                if status == SIGNING_SUCCEEDED && super::lists_current(versions.iter().map(String::as_str), &self.ivars().version) {
                    (self.ivars().report)(Report::FeedVerified);
                }
            }

            /// Found, not downloaded: a user who turned automatic downloads off would otherwise never
            /// start the two days.
            #[unsafe(method(updater:didFindValidUpdate:))]
            fn did_find_valid_update(&self, _updater: &AnyObject, item: &AnyObject) {
                // SAFETY: `item` is an `SUAppcastItem`; `isCriticalUpdate` is a BOOL getter, NO
                // whenever the feed's signature failed.
                let critical: bool = unsafe { msg_send![item, isCriticalUpdate] };
                if critical {
                    (self.ivars().report)(Report::CriticalFound);
                }
            }
        }
    );

    /// Sparkle's `SPUStandardUpdaterController`; main thread only.
    pub struct Updater {
        controller: Retained<AnyObject>,
        /// Sparkle holds its delegate weakly.
        _delegate: Retained<Delegate>,
    }

    impl Updater {
        /// `None` when the bundle carries no valid feed and key or Sparkle does not load.
        /// `report` hears from Sparkle's delegate on the main thread, inside Sparkle's call.
        pub fn start(automatic: bool, _dir: &std::path::Path, report: impl Fn(Report) + Send + 'static) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let bundle = NSBundle::mainBundle();
            let info = |key: &str| Some(bundle.objectForInfoDictionaryKey(&NSString::from_str(key))?.downcast::<NSString>().ok()?.to_string());
            let feed = info("SUFeedURL");
            if !super::configured(feed.as_deref(), info("SUPublicEDKey").as_deref()) {
                return None;
            }
            let version = info("CFBundleVersion")?;
            let path = format!("{}/Sparkle.framework", bundle.privateFrameworksPath()?);
            let sparkle = NSBundle::bundleWithPath(&NSString::from_str(&path))?;
            // SAFETY: loads the Sparkle framework signed into this bundle; library validation
            // refuses any other signer under the hardened runtime.
            if !unsafe { sparkle.load() } {
                return None;
            }
            let class = AnyClass::get(c"SPUStandardUpdaterController")?;
            let delegate = Delegate::alloc(mtm).set_ivars(Ivars { report: Box::new(report), feed: NSString::from_str(feed.as_deref()?), version });
            // SAFETY: NSObject's designated initializer on a freshly allocated instance.
            let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
            let none: Option<&AnyObject> = None;
            // SAFETY: Sparkle's designated initialiser; Sparkle asks the updater delegate with
            // `respondsToSelector:` before each optional method, and a nil user-driver delegate
            // selects its standard behaviour.
            let controller: Option<Retained<AnyObject>> = unsafe {
                let allocated: Allocated<AnyObject> = msg_send![class, alloc];
                msg_send![allocated, initWithStartingUpdater: false, updaterDelegate: &*delegate, userDriverDelegate: none]
            };
            let updater = Updater { controller: controller?, _delegate: delegate };
            // SAFETY: main thread, as Sparkle requires; `updater` is a non-null property of the live
            // controller and `startUpdater:` returns a BOOL with an `NSError` out-parameter.
            let started: Result<(), Retained<NSError>> = unsafe {
                let sparkle: Retained<AnyObject> = msg_send![&updater.controller, updater];
                msg_send![&sparkle, startUpdater: _]
            };
            if let Err(e) = started {
                eprintln!("tarn: Sparkle did not start: {e}");
                return None;
            }
            updater.set_automatic(automatic);
            Some(updater)
        }

        /// Sparkle's own switch, mirrored from Tarn's setting; it reschedules checks itself.
        pub fn set_automatic(&self, on: bool) {
            // SAFETY: `updater` is a non-null property of the live controller; the setter takes a BOOL.
            unsafe {
                let updater: Retained<AnyObject> = msg_send![&self.controller, updater];
                let _: () = msg_send![&updater, setAutomaticallyChecksForUpdates: on];
            }
        }

        /// Checks now and shows Sparkle's progress and result windows.
        pub fn check_now(&self) {
            let none: Option<&AnyObject> = None;
            // SAFETY: `checkForUpdates:` is the controller's action method; the sender may be nil.
            unsafe {
                let _: () = msg_send![&self.controller, checkForUpdates: none];
            }
        }

        /// Sparkle installs a downloaded update itself when Tarn quits.
        pub fn install_on_quit(&self) -> bool {
            false
        }

        pub fn quitting(&self) {}
    }
}

#[test]
fn configuration_needs_an_https_feed_and_an_eddsa_key() {
    let key = "pfIShU4dEXqPd5ObYNfDBiQWcXozk7estwzTnF9BamQ=";
    let feed = "https://github.com/owner/tarn/releases/latest/download/appcast.xml";
    assert!(configured(Some(feed), Some(key)));
    assert!(!configured(None, None));
    assert!(!configured(Some(""), Some("")));
    assert!(!configured(Some(feed), None));
    assert!(!configured(None, Some(key)));
    assert!(!configured(Some("http://github.com/appcast.xml"), Some(key)));
    assert!(!configured(Some("https://user:pw@github.com/appcast.xml"), Some(key)));
    assert!(!configured(Some("file:///tmp/appcast.xml"), Some(key)));
    assert!(!configured(Some(feed), Some(&key[..43])));
    assert!(!configured(Some(feed), Some(&format!("{}==", &key[..42]))));
    assert!(!configured(Some(feed), Some(&key.replace('p', "-"))));
    assert!(!configured(Some(feed), Some("ééééééééééééééééééééé==")));
}

#[test]
fn only_an_appcast_listing_this_version_or_a_later_one_is_current() {
    assert!(lists_current(["0.1.0"], "0.1.0"));
    assert!(lists_current(["0.0.9", "0.2.0"], "0.1.0"));
    assert!(lists_current(["0.1"], "0.1.0"));
    assert!(lists_current(["0.1.0.1"], "0.1.0"));
    assert!(lists_current(["0.10.0"], "0.9.0"));
    assert!(lists_current(["0.2.0-beta.1"], "0.2.0-beta.1"));
    assert!(!lists_current([], "0.1.0"));
    assert!(!lists_current(["0.0.9", "0.0.10"], "0.1.0"));
    assert!(!lists_current(["0.9.0"], "0.10.0"));
    assert!(!lists_current(["0.1.0-beta.1"], "0.1.0"));
    assert!(!lists_current(["0.2.0"], "0.2.0-beta.1"));
    assert!(!lists_current(["", "x"], "0.1.0"));
    assert!(!lists_current(["0.1.0"], ""));
}
