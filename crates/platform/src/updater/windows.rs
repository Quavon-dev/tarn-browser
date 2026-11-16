use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig};
use url::Url;

use super::Report;

const CONF: &str = include_str!("../../../../packaging/updates.conf");
const FEED_NAME: &str = "windows-update.json";
const MAX_FEED: usize = 64 * 1024;
const MAX_SIGNATURE: u64 = 1024;
const MAX_INSTALLER: u64 = 512 << 20;
const DAY: Duration = Duration::from_secs(86_400);
const HOUR: Duration = Duration::from_secs(3600);
const INSTALLER_NAME: &str = "Tarn-setup.exe";
/// `/TARNUPDATE` makes the installer start Tarn again; a plain quit installs without it, as
/// Sparkle installs a waiting update on quit.
const INSTALLER_ARGS: [&str; 4] = ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/TARNUPDATE"];
const RUNNING: &str = super::VERSION;

#[derive(Debug, PartialEq, Eq)]
enum Refused {
    Oversized,
    BadSignature,
    Malformed,
    WrongHost,
    BadSha256,
    BadLength,
    /// A Nightly channel feed read on the Release channel.
    WrongChannel,
}

#[derive(Deserialize)]
struct Feed {
    version: String,
    /// `nightly` for the Nightly channel's feed, inside the signed bytes.
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    critical: bool,
    installer: Installer,
}

#[derive(Deserialize)]
struct Installer {
    url: String,
    sha256: String,
    length: u64,
    signature: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Offer {
    version: String,
    critical: bool,
    url: String,
    sha256: String,
    length: u64,
    signature: [u8; 64],
}

#[derive(Debug, PartialEq, Eq)]
struct Checked {
    /// The feed lists the running version or a later one.
    current: bool,
    /// A later version than the running one.
    offer: Option<Offer>,
}

struct Config {
    feed: Url,
    nightly_feed: Option<Url>,
    key: VerifyingKey,
    /// Installers may come only from under this URL: the feed's own repository's release downloads.
    downloads: Url,
}

fn config(conf: &str) -> Option<Config> {
    let value = |name: &str| conf.lines().find_map(|line| line.trim_end().strip_prefix(name)?.strip_prefix('='));
    let (feed, key) = (value("feed_url"), value("public_ed_key"));
    if !super::configured(feed, key) {
        return None;
    }
    let nightly_feed = super::nightly_feed(feed?).and_then(|f| Url::parse(&f).ok()?.join(FEED_NAME).ok());
    let feed = Url::parse(feed?).ok()?.join(FEED_NAME).ok()?;
    let key = VerifyingKey::from_bytes(&BASE64.decode(key?).ok()?.try_into().ok()?).ok()?;
    let downloads = feed.join("../../download/").ok()?;
    Some(Config { feed, nightly_feed, key, downloads })
}

fn signature(text: &[u8]) -> Result<Signature, Refused> {
    let bytes = BASE64.decode(text.trim_ascii()).map_err(|_| Refused::BadSignature)?;
    Signature::from_slice(&bytes).map_err(|_| Refused::BadSignature)
}

/// Checks the feed's signature over its exact bytes before reading anything in it.
fn read_feed(feed: &[u8], sig: &[u8], config: &Config, running: &str, nightly: bool) -> Result<Checked, Refused> {
    if feed.len() > MAX_FEED {
        return Err(Refused::Oversized);
    }
    config.key.verify_strict(feed, &signature(sig)?).map_err(|_| Refused::BadSignature)?;
    let feed: Feed = serde_json::from_slice(feed).map_err(|_| Refused::Malformed)?;
    match feed.channel.as_deref() {
        None => {}
        Some(super::NIGHTLY) if nightly => {}
        Some(super::NIGHTLY) => return Err(Refused::WrongChannel),
        Some(_) => return Err(Refused::Malformed),
    }
    if feed.version.is_empty() || feed.version.len() > 32 || !feed.version.bytes().all(|b| b.is_ascii_digit() || b == b'.') || feed.version.starts_with('.') {
        return Err(Refused::Malformed);
    }
    let installer = feed.installer;
    let url = Url::parse(&installer.url).map_err(|_| Refused::WrongHost)?;
    let path = url.path().to_ascii_lowercase();
    let confined = url.scheme() == "https"
        && url.host_str() == config.downloads.host_str()
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().starts_with(config.downloads.path())
        && !path.contains("%2f")
        && !path.contains("%5c")
        && !path.contains('\\');
    if !confined {
        return Err(Refused::WrongHost);
    }
    if installer.sha256.len() != 64 || !installer.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(Refused::BadSha256);
    }
    if installer.length == 0 || installer.length > MAX_INSTALLER {
        return Err(Refused::BadLength);
    }
    let signature = signature(installer.signature.as_bytes())?.to_bytes();
    let current = super::lists_current([feed.version.as_str()], running);
    let newer = current && !super::lists_current([running], &feed.version);
    let offer =
        newer.then(|| Offer { version: feed.version, critical: feed.critical, url: url.into(), sha256: installer.sha256, length: installer.length, signature });
    Ok(Checked { current, offer })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn verify_installer(bytes: &[u8], offer: &Offer, key: &VerifyingKey) -> Result<(), Refused> {
    if bytes.len() as u64 != offer.length {
        return Err(Refused::BadLength);
    }
    if sha256_hex(bytes) != offer.sha256 {
        return Err(Refused::BadSha256);
    }
    key.verify_strict(bytes, &Signature::from_bytes(&offer.signature)).map_err(|_| Refused::BadSignature)
}

fn agent(timeout: Duration) -> ureq::Agent {
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
        .timeout_global(Some(timeout))
        .build()
        .into()
}

fn get(agent: &ureq::Agent, url: &str, max: u64) -> Result<Vec<u8>, String> {
    let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
    response.body_mut().with_config().limit(max).read_to_vec().map_err(|e| e.to_string())
}

/// A verified installer on disk, waiting for Restart to Update.
struct Ready {
    path: PathBuf,
    version: String,
    sha256: String,
    length: u64,
}

enum Message {
    Automatic(bool),
    Nightly,
    CheckNow,
}

struct Checker {
    config: Config,
    dir: PathBuf,
    ready: Arc<Mutex<Option<Ready>>>,
    nightly: Arc<AtomicBool>,
    report: Box<dyn Fn(Report) + Send>,
}

impl Checker {
    /// Wakes hourly and compares wall-clock time, since waits stop while Windows sleeps: a day
    /// after a check that reached the feed, an hour after one that did not.
    fn run(self, mut automatic: bool, messages: mpsc::Receiver<Message>) {
        let mut next = SystemTime::now();
        if automatic {
            next += if self.check(false) { DAY } else { HOUR };
        }
        loop {
            let manual = match messages.recv_timeout(if automatic { HOUR } else { Duration::MAX }) {
                Ok(Message::Automatic(on)) => {
                    automatic = on;
                    continue;
                }
                Ok(Message::Nightly) if automatic => false,
                Ok(Message::Nightly) => continue,
                Ok(Message::CheckNow) => true,
                Err(RecvTimeoutError::Timeout) if SystemTime::now() >= next => false,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => return,
            };
            next = SystemTime::now() + if self.check(manual) { DAY } else { HOUR };
        }
    }

    /// Whether the feed was reached and verified.
    fn check(&self, manual: bool) -> bool {
        let agent = agent(Duration::from_secs(120));
        let nightly_feed = self.config.nightly_feed.as_ref().filter(|_| self.nightly.load(Ordering::Relaxed));
        let feed_url = nightly_feed.unwrap_or(&self.config.feed).as_str();
        let running = super::channel_version(RUNNING, nightly_feed.is_some());
        let checked = get(&agent, feed_url, MAX_FEED as u64 + 1)
            .and_then(|feed| Ok((feed, get(&agent, &format!("{feed_url}.sig"), MAX_SIGNATURE)?)))
            .and_then(|(feed, sig)| read_feed(&feed, &sig, &self.config, running, nightly_feed.is_some()).map_err(|e| format!("feed refused: {e:?}")));
        let checked = match checked {
            Ok(checked) => checked,
            Err(e) => {
                eprintln!("tarn: update check failed: {e}");
                if manual {
                    (self.report)(Report::CheckFailed);
                }
                return false;
            }
        };
        if checked.current {
            (self.report)(Report::FeedVerified);
        }
        let Some(offer) = checked.offer else {
            if manual {
                (self.report)(Report::UpToDate);
            }
            return true;
        };
        if offer.critical {
            (self.report)(Report::CriticalFound);
        }
        let known = self.ready.lock().ok().and_then(|ready| ready.as_ref().map(|r| r.version == offer.version));
        if known == Some(true) {
            if manual {
                (self.report)(Report::UpdateReady(offer.version));
            }
            return true;
        }
        match self.download(&offer) {
            Ok(path) => {
                let ready = Ready { path, version: offer.version.clone(), sha256: offer.sha256, length: offer.length };
                // Under the lock `set_nightly` takes: an installer of a channel left meanwhile is dropped.
                let kept = self.ready.lock().is_ok_and(|mut slot| {
                    let same = self.nightly.load(Ordering::Relaxed) == nightly_feed.is_some();
                    if same {
                        *slot = Some(ready);
                    }
                    same
                });
                if kept {
                    (self.report)(Report::UpdateReady(offer.version));
                }
            }
            Err(e) => {
                eprintln!("tarn: update download failed: {e}");
                if manual {
                    (self.report)(Report::CheckFailed);
                }
            }
        }
        true
    }

    /// The installer of `offer` in its version's folder: one kept from an earlier launch when it
    /// still checks out, else downloaded and written only once its length, hash and signature do.
    /// Other versions' folders go.
    fn download(&self, offer: &Offer) -> Result<PathBuf, String> {
        let folder = self.dir.join(&offer.version);
        let path = folder.join(INSTALLER_NAME);
        if !std::fs::read(&path).is_ok_and(|bytes| verify_installer(&bytes, offer, &self.config.key).is_ok()) {
            let bytes = get(&agent(HOUR), &offer.url, offer.length)?;
            verify_installer(&bytes, offer, &self.config.key).map_err(|e| format!("installer refused: {e:?}"))?;
            std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
            std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        }
        for entry in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            if entry.path() != folder {
                std::fs::remove_dir_all(entry.path()).ok();
            }
        }
        Ok(path)
    }
}

/// Checks `windows-update.json` next to the feed in `packaging/updates.conf` once a day while
/// automatic checks are on, downloads a verified later installer and runs it after Tarn quits.
pub struct Updater {
    messages: mpsc::Sender<Message>,
    nightly: Arc<AtomicBool>,
    ready: Arc<Mutex<Option<Ready>>>,
    install: std::cell::Cell<bool>,
    automatic: std::cell::Cell<bool>,
}

impl Updater {
    /// `None` in debug builds and when `packaging/updates.conf` names no valid feed and key.
    /// Downloads go into `dir`; folders of versions not newer than this one go first. `report`
    /// hears from a background thread.
    pub fn start(automatic: bool, nightly: bool, dir: &Path, report: impl Fn(Report) + Send + 'static) -> Option<Self> {
        if cfg!(debug_assertions) || !dir.is_absolute() {
            return None;
        }
        let config = config(CONF)?;
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if super::lists_current([RUNNING], &name) {
                std::fs::remove_dir_all(entry.path()).ok();
            }
        }
        let (messages, receiver) = mpsc::channel();
        let ready = Arc::new(Mutex::new(None));
        let nightly = Arc::new(AtomicBool::new(nightly));
        let checker = Checker { config, dir: dir.to_owned(), ready: ready.clone(), nightly: nightly.clone(), report: Box::new(report) };
        std::thread::Builder::new().name("tarn-updater".into()).spawn(move || checker.run(automatic, receiver)).ok()?;
        Some(Updater { messages, nightly, ready, install: std::cell::Cell::new(false), automatic: std::cell::Cell::new(automatic) })
    }

    pub fn set_automatic(&self, on: bool) {
        self.automatic.set(on);
        self.messages.send(Message::Automatic(on)).ok();
    }

    /// Switches between the Release and Nightly feeds and, while automatic checks are on, checks
    /// the chosen one now. An installer downloaded from the other channel no longer installs.
    pub fn set_nightly(&self, on: bool) {
        let changed = match self.ready.lock() {
            Ok(mut ready) => {
                let changed = self.nightly.swap(on, Ordering::Relaxed) != on;
                if changed {
                    ready.take();
                }
                changed
            }
            Err(_) => self.nightly.swap(on, Ordering::Relaxed) != on,
        };
        if changed {
            self.install.set(false);
        }
        self.messages.send(Message::Nightly).ok();
    }

    pub fn check_now(&self) {
        self.messages.send(Message::CheckNow).ok();
    }

    /// Marks the downloaded installer to run once Tarn has quit; `false` when none is ready.
    pub fn install_on_quit(&self) -> bool {
        let ready = self.ready.lock().is_ok_and(|ready| ready.as_ref().is_some_and(|r| r.path.is_file()));
        self.install.set(ready);
        ready
    }

    /// Starts a downloaded installer as Tarn quits, after checking once more, through a handle
    /// that keeps the file from being changed until the installer has started, that it is the
    /// verified one. It starts Tarn again only after Restart to Update (`install_on_quit`).
    pub fn quitting(&self) {
        use std::os::windows::fs::OpenOptionsExt;
        // A plain quit installs only while automatic checks are on, as Sparkle's automatic updates do.
        if !self.install.get() && !self.automatic.get() {
            return;
        }
        let Some(ready) = self.ready.lock().ok().and_then(|mut ready| ready.take()) else { return };
        // FILE_SHARE_READ only: no other process may write, rename or delete it meanwhile.
        let Ok(mut file) = std::fs::OpenOptions::new().read(true).share_mode(0x1).open(&ready.path) else {
            return eprintln!("tarn: the downloaded installer could not be opened");
        };
        let mut bytes = Vec::new();
        if std::io::Read::read_to_end(&mut file, &mut bytes).is_err() || bytes.len() as u64 != ready.length || sha256_hex(&bytes) != ready.sha256 {
            return eprintln!("tarn: the downloaded installer changed; not starting it");
        }
        let args = if self.install.get() { &INSTALLER_ARGS[..] } else { &INSTALLER_ARGS[..3] };
        if let Err(e) = std::process::Command::new(&ready.path).args(args).spawn() {
            eprintln!("tarn: could not start the installer: {e}");
        }
        drop(file);
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    const INSTALLER: &[u8] = b"MZ installer bytes";

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn test_config() -> Config {
        let conf = format!(
            "feed_url=https://github.com/Quavon-dev/tarn-browser/releases/latest/download/appcast.xml\r\npublic_ed_key={}\r\n",
            BASE64.encode(key().verifying_key().to_bytes())
        );
        config(&conf).unwrap()
    }

    fn feed_json(version: &str, critical: bool, url: &str, sha256: &str) -> String {
        let signature = BASE64.encode(key().sign(INSTALLER).to_bytes());
        format!(
            r#"{{"version":"{version}","critical":{critical},"installer":{{"url":"{url}","sha256":"{sha256}","length":{},"signature":"{signature}"}}}}"#,
            INSTALLER.len()
        )
    }

    fn url(version: &str) -> String {
        format!("https://github.com/Quavon-dev/tarn-browser/releases/download/v{version}/Tarn-v{version}-windows-x64-setup.exe")
    }

    fn signed(feed: &str) -> (Vec<u8>, Vec<u8>) {
        (feed.as_bytes().to_vec(), format!("{}\n", BASE64.encode(key().sign(feed.as_bytes()).to_bytes())).into_bytes())
    }

    fn check(feed: &str, running: &str) -> Result<Checked, Refused> {
        let (feed, sig) = signed(feed);
        read_feed(&feed, &sig, &test_config(), running, false)
    }

    #[test]
    fn the_embedded_configuration_names_the_windows_feed() {
        let embedded = config(CONF).unwrap();
        assert_eq!(embedded.feed.as_str(), "https://github.com/Quavon-dev/tarn-browser/releases/latest/download/windows-update.json");
        assert_eq!(
            embedded.nightly_feed.as_ref().map(Url::as_str),
            Some("https://github.com/Quavon-dev/tarn-browser/releases/download/nightly/windows-update.json")
        );
        assert_eq!(embedded.downloads.as_str(), "https://github.com/Quavon-dev/tarn-browser/releases/download/");
        assert!(config("feed_url=http://github.com/a.xml\npublic_ed_key=x").is_none());
        assert!(config("").is_none());
    }

    #[test]
    fn a_signed_later_version_is_offered() {
        let checked = check(&feed_json("9.0.0", true, &url("9.0.0"), &sha256_hex(INSTALLER)), "0.4.0").unwrap();
        assert!(checked.current);
        let offer = checked.offer.unwrap();
        assert_eq!((offer.version.as_str(), offer.critical, offer.length), ("9.0.0", true, INSTALLER.len() as u64));
        assert_eq!(verify_installer(INSTALLER, &offer, &key().verifying_key()), Ok(()));
    }

    #[test]
    fn a_signed_feed_for_the_unsigned_installer_is_offered() {
        let unsigned = url("9.0.0").replace("-setup.exe", "-setup-unsigned.exe");
        let offer = check(&feed_json("9.0.0", false, &unsigned, &sha256_hex(INSTALLER)), "0.4.0").unwrap().offer.unwrap();
        assert_eq!(offer.url, unsigned);
        assert_eq!(verify_installer(INSTALLER, &offer, &key().verifying_key()), Ok(()));
    }

    #[test]
    fn a_nightly_feed_is_read_only_on_the_nightly_channel() {
        let nightly = "https://github.com/Quavon-dev/tarn-browser/releases/download/nightly/Tarn-nightly-windows-x64-setup-unsigned.exe";
        let feed = feed_json("0.4.0.7", false, nightly, &sha256_hex(INSTALLER)).replacen("{", "{\"channel\":\"nightly\",", 1);
        let (bytes, sig) = signed(&feed);
        let offer = read_feed(&bytes, &sig, &test_config(), "0.4.0.6", true).unwrap().offer.unwrap();
        assert_eq!((offer.version.as_str(), offer.url.as_str()), ("0.4.0.7", nightly));
        assert_eq!(read_feed(&bytes, &sig, &test_config(), "0.4.0", false), Err(Refused::WrongChannel));
        let (other, other_sig) = signed(&feed.replace("\"nightly\",", "\"beta\","));
        assert_eq!(read_feed(&other, &other_sig, &test_config(), "0.4.0.6", true), Err(Refused::Malformed));
        let release = check(&feed_json("0.4.0", false, &url("0.4.0"), &sha256_hex(INSTALLER)), super::super::channel_version("0.4.0.7", false)).unwrap();
        assert!(release.current && release.offer.is_none());
    }

    #[test]
    fn the_running_version_is_verified_but_not_offered_and_an_older_one_neither() {
        let same = check(&feed_json("0.4.0", false, &url("0.4.0"), &sha256_hex(INSTALLER)), "0.4.0").unwrap();
        assert_eq!(same, Checked { current: true, offer: None });
        let older = check(&feed_json("0.3.9", true, &url("0.3.9"), &sha256_hex(INSTALLER)), "0.4.0").unwrap();
        assert_eq!(older, Checked { current: false, offer: None });
    }

    #[test]
    fn a_bad_or_foreign_signature_is_refused() {
        let feed = feed_json("9.0.0", false, &url("9.0.0"), &sha256_hex(INSTALLER));
        let config = test_config();
        let other = BASE64.encode(SigningKey::from_bytes(&[8; 32]).sign(feed.as_bytes()).to_bytes());
        assert_eq!(read_feed(feed.as_bytes(), other.as_bytes(), &config, "0.4.0", false), Err(Refused::BadSignature));
        assert_eq!(read_feed(feed.as_bytes(), b"not base64!", &config, "0.4.0", false), Err(Refused::BadSignature));
        assert_eq!(read_feed(feed.as_bytes(), BASE64.encode([0; 63]).as_bytes(), &config, "0.4.0", false), Err(Refused::BadSignature));
    }

    #[test]
    fn a_tampered_feed_is_refused() {
        let (feed, sig) = signed(&feed_json("0.4.1", false, &url("0.4.1"), &sha256_hex(INSTALLER)));
        let tampered = String::from_utf8(feed).unwrap().replace("0.4.1", "9.9.9");
        assert_eq!(read_feed(tampered.as_bytes(), &sig, &test_config(), "0.4.0", false), Err(Refused::BadSignature));
    }

    #[test]
    fn an_oversized_feed_is_refused_before_its_signature() {
        let feed = format!("{}{}", feed_json("9.0.0", false, &url("9.0.0"), &sha256_hex(INSTALLER)), " ".repeat(MAX_FEED));
        assert_eq!(check(&feed, "0.4.0"), Err(Refused::Oversized));
    }

    #[test]
    fn an_installer_from_elsewhere_is_refused() {
        let sha = sha256_hex(INSTALLER);
        for url in [
            "https://evil.example/Quavon-dev/tarn-browser/releases/download/v9.0.0/setup.exe",
            "http://github.com/Quavon-dev/tarn-browser/releases/download/v9.0.0/setup.exe",
            "https://github.com/someone/tarn-browser/releases/download/v9.0.0/setup.exe",
            "https://github.com/Quavon-dev/tarn-browser/releases/download/../../../someone/x/releases/download/a.exe",
            "https://github.com.evil.example/Quavon-dev/tarn-browser/releases/download/v9.0.0/setup.exe",
            "https://user@github.com/Quavon-dev/tarn-browser/releases/download/v9.0.0/setup.exe",
            "https://github.com:8443/Quavon-dev/tarn-browser/releases/download/v9.0.0/setup.exe",
            "not a url",
        ] {
            assert_eq!(check(&feed_json("9.0.0", false, url, &sha), "0.4.0"), Err(Refused::WrongHost), "{url}");
        }
    }

    #[test]
    fn a_malformed_hash_or_length_is_refused() {
        let upper = sha256_hex(INSTALLER).to_uppercase();
        assert_eq!(check(&feed_json("9.0.0", false, &url("9.0.0"), &upper), "0.4.0"), Err(Refused::BadSha256));
        assert_eq!(check(&feed_json("9.0.0", false, &url("9.0.0"), "abc"), "0.4.0"), Err(Refused::BadSha256));
        let huge =
            feed_json("9.0.0", false, &url("9.0.0"), &sha256_hex(INSTALLER)).replace(&format!("\"length\":{}", INSTALLER.len()), "\"length\":1073741825");
        assert_eq!(check(&huge, "0.4.0"), Err(Refused::BadLength));
        assert_eq!(check("{\"version\":\"9.0.0\"}", "0.4.0"), Err(Refused::Malformed));
    }

    #[test]
    fn an_installer_that_does_not_match_the_feed_is_refused() {
        let checked = check(&feed_json("9.0.0", false, &url("9.0.0"), &sha256_hex(b"other bytes of the same len")), "0.4.0").unwrap();
        let offer = checked.offer.unwrap();
        let key = key().verifying_key();
        assert_eq!(verify_installer(b"short", &offer, &key), Err(Refused::BadLength));
        assert_eq!(verify_installer(INSTALLER, &offer, &key), Err(Refused::BadSha256));
        let mut forged = INSTALLER.to_vec();
        forged[0] ^= 1;
        let offer = Offer { sha256: sha256_hex(&forged), ..offer };
        assert_eq!(verify_installer(&forged, &offer, &key), Err(Refused::BadSignature));
    }
}
