//! Updates. macOS uses Sparkle 2, loaded from the app bundle only when its Info.plist names a
//! feed and a public key, so unbundled builds and tests never load it. Windows release builds
//! check a signed JSON feed themselves (`windows`).

/// This build's version: a nightly's `TARN_BUILD_VERSION` (`1.2.3.45`), else the crate's.
pub const VERSION: &str = match option_env!("TARN_BUILD_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

/// The channel name the Nightly channel's feeds carry inside their signed bytes: Sparkle's
/// `sparkle:channel` of each appcast item, and `channel` of the Windows feed.
pub const NIGHTLY: &str = "nightly";

/// The Nightly channel's feed beside the release feed `feed`: the same file in the `nightly`
/// release in place of the latest one.
pub fn nightly_feed(feed: &str) -> Option<String> {
    let (repo, file) = feed.split_once("/releases/latest/download/")?;
    Some(format!("{repo}/releases/download/nightly/{file}"))
}

/// The version a feed of the chosen channel must list for the check to count: on the Release
/// channel a nightly counts as its release (`1.2.3.45` as `1.2.3`), which it stays on until a
/// later release.
pub fn channel_version(running: &str, nightly: bool) -> &str {
    if nightly {
        return running;
    }
    running.match_indices('.').nth(2).map_or(running, |(at, _)| &running[..at])
}

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
    use std::cell::Cell;

    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};
    use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{NSArray, NSBundle, NSError, NSObject, NSString};

    use super::Report;

    struct Ivars {
        report: Box<dyn Fn(Report)>,
        feed: Retained<NSString>,
        nightly_feed: Option<Retained<NSString>>,
        nightly: Cell<bool>,
        version: String,
        check_pending: Cell<Option<Pending>>,
        checked: Cell<Option<bool>>,
    }

    #[derive(Clone, Copy)]
    struct Pending {
        nightly: bool,
        switched: bool,
    }

    fn still_due(pending: Pending, finished: Option<isize>, checked: Option<bool>) -> bool {
        !(finished == Some(USER_CHECK) && checked == Some(pending.nightly))
    }

    impl Ivars {
        fn nightly(&self) -> bool {
            self.nightly.get() && self.nightly_feed.is_some()
        }
    }

    /// `SPUAppcastSigningValidationStatusSucceeded` in Sparkle 2.10.1's `SPUAppcastSigningValidationStatus.h`.
    const SIGNING_SUCCEEDED: isize = 1;

    const USER_CHECK: isize = 0;

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
                let ivars = self.ivars();
                ivars.nightly_feed.as_ref().filter(|_| ivars.nightly()).unwrap_or(&ivars.feed).clone()
            }

            /// Items of the Nightly channel are offered only while it is chosen; Sparkle offers
            /// items without a channel to everyone.
            #[unsafe(method_id(allowedChannelsForUpdater:))]
            fn allowed_channels(&self, _updater: &AnyObject) -> Retained<AnyObject> {
                let set = AnyClass::get(c"NSSet").expect("Foundation defines NSSet");
                // SAFETY: `+[NSSet set]` and `+[NSSet setWithObject:]` return a new autoreleased set;
                // the object is a non-null NSString.
                unsafe {
                    if self.ivars().nightly() {
                        msg_send![set, setWithObject: &*NSString::from_str(super::NIGHTLY)]
                    } else {
                        msg_send![set, set]
                    }
                }
            }

            #[unsafe(method(updater:didFinishLoadingAppcast:))]
            fn did_finish_loading_appcast(&self, _updater: &AnyObject, appcast: &AnyObject) {
                let ivars = self.ivars();
                // SAFETY: `appcast` is an `SUAppcast`, whose `signingValidationStatus` is an NSInteger
                // enum and `items` a non-null array of `SUAppcastItem` with a non-null `versionString`
                // and a nullable `channel`.
                let (status, versions) = unsafe {
                    let status: isize = msg_send![appcast, signingValidationStatus];
                    let items: Retained<NSArray<AnyObject>> = msg_send![appcast, items];
                    let versions: Vec<String> = items
                        .iter()
                        .filter(|item| {
                            let channel: Option<Retained<NSString>> = msg_send![&**item, channel];
                            channel.is_none_or(|c| ivars.nightly() && c.to_string() == super::NIGHTLY)
                        })
                        .map(|item| {
                            let version: Retained<NSString> = msg_send![&item, versionString];
                            version.to_string()
                        })
                        .collect();
                    (status, versions)
                };
                let running = super::channel_version(&ivars.version, ivars.nightly());
                if status == SIGNING_SUCCEEDED && super::lists_current(versions.iter().map(String::as_str), running) {
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

            #[unsafe(method(updater:didFinishUpdateCycleForUpdateCheck:error:))]
            fn did_finish_update_cycle(&self, updater: &AnyObject, check: isize, _error: Option<&NSError>) {
                self.run_pending_check(updater, Some(check));
            }

            #[unsafe(method(updater:willScheduleUpdateCheckAfterDelay:))]
            fn will_schedule_update_check(&self, updater: &AnyObject, _delay: f64) {
                self.run_pending_check(updater, None);
            }
        }
    );

    impl Delegate {
        fn run_pending_check(&self, updater: &AnyObject, finished: Option<isize>) {
            let ivars = self.ivars();
            if ivars.check_pending.take().is_some_and(|pending| still_due(pending, finished, ivars.checked.get())) {
                self.check(updater);
            }
        }

        fn check(&self, updater: &AnyObject) {
            // SAFETY: `updater` is the live `SPUUpdater`; `sessionInProgress` is a BOOL getter and
            // `checkForUpdates` takes no arguments.
            let started = unsafe {
                let session: bool = msg_send![updater, sessionInProgress];
                let _: () = msg_send![updater, checkForUpdates];
                !session
            };
            if started {
                self.ivars().checked.set(Some(self.ivars().nightly()));
            }
        }
    }

    /// Sparkle's `SPUStandardUpdaterController`; main thread only.
    pub struct Updater {
        controller: Retained<AnyObject>,
        /// Sparkle holds its delegate weakly.
        delegate: Retained<Delegate>,
    }

    impl Updater {
        /// `None` when the bundle carries no valid feed and key or Sparkle does not load.
        /// `report` hears from Sparkle's delegate on the main thread, inside Sparkle's call.
        pub fn start(automatic: bool, nightly: bool, _dir: &std::path::Path, report: impl Fn(Report) + Send + 'static) -> Option<Self> {
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
            let feed = feed?;
            let nightly_feed = super::nightly_feed(&feed).map(|f| NSString::from_str(&f));
            let ivars = Ivars {
                report: Box::new(report),
                feed: NSString::from_str(&feed),
                nightly_feed,
                nightly: Cell::new(nightly),
                version,
                check_pending: Cell::new(None),
                checked: Cell::new(None),
            };
            let delegate = Delegate::alloc(mtm).set_ivars(ivars);
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
            let updater = Updater { controller: controller?, delegate };
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
            let pending = &self.delegate.ivars().check_pending;
            if !on && pending.get().is_some_and(|p| p.switched) {
                pending.take();
            }
        }

        /// Switches between the Release and Nightly feeds and, while automatic checks are on, checks
        /// the chosen one as Check Now does.
        pub fn set_nightly(&self, on: bool) {
            self.delegate.ivars().nightly.set(on);
            // SAFETY: `updater` is a non-null property of the live controller; the getter returns a BOOL.
            let automatic: bool = unsafe {
                let updater: Retained<AnyObject> = msg_send![&self.controller, updater];
                msg_send![&updater, automaticallyChecksForUpdates]
            };
            if automatic {
                self.request_check(true);
            }
        }

        /// Checks now and shows Sparkle's progress and result windows; while Sparkle's session
        /// runs, once that session ends.
        pub fn check_now(&self) {
            self.request_check(false);
        }

        fn request_check(&self, switched: bool) {
            // SAFETY: `updater` is a non-null property of the live controller; `sessionInProgress` and
            // `canCheckForUpdates` are BOOL getters.
            let (updater, session, can) = unsafe {
                let updater: Retained<AnyObject> = msg_send![&self.controller, updater];
                let session: bool = msg_send![&updater, sessionInProgress];
                let can: bool = msg_send![&updater, canCheckForUpdates];
                (updater, session, can)
            };
            let busy = if switched { session } else { !can };
            let ivars = self.delegate.ivars();
            if busy {
                let switched = switched && ivars.check_pending.get().is_none_or(|p| p.switched);
                ivars.check_pending.set(Some(Pending { nightly: ivars.nightly(), switched }));
            } else {
                self.delegate.check(&updater);
            }
        }

        /// Sparkle shows and installs a downloaded update itself.
        pub fn ready_version(&self) -> Option<String> {
            None
        }

        /// Sparkle installs a downloaded update itself when Tarn quits.
        pub fn install_on_quit(&self) -> bool {
            false
        }

        pub fn quitting(&self) {}
    }

    #[test]
    fn a_queued_check_runs_unless_a_check_on_its_channel_just_answered_it() {
        let pending = Pending { nightly: true, switched: false };
        assert!(!still_due(pending, Some(USER_CHECK), Some(true)));
        assert!(still_due(pending, Some(USER_CHECK), Some(false)));
        assert!(still_due(pending, Some(USER_CHECK), None));
        assert!(still_due(pending, Some(1), Some(true)));
        assert!(still_due(pending, None, Some(true)));
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
fn the_nightly_channel_reads_the_nightly_release_and_counts_a_nightly_as_its_release_elsewhere() {
    assert_eq!(
        nightly_feed("https://github.com/o/r/releases/latest/download/appcast.xml").as_deref(),
        Some("https://github.com/o/r/releases/download/nightly/appcast.xml")
    );
    assert_eq!(nightly_feed("https://example.com/appcast.xml"), None);
    assert_eq!(channel_version("1.2.3.45", true), "1.2.3.45");
    assert_eq!(channel_version("1.2.3.45", false), "1.2.3");
    assert_eq!(channel_version("1.2.3", false), "1.2.3");
    assert!(lists_current(["1.2.3"], channel_version("1.2.3.45", false)));
    assert!(!lists_current(["1.2.3"], channel_version("1.2.3.45", true)));
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
