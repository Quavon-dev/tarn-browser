//! A page cannot open a window without the user's click. Opens a window, so it runs only when
//! asked: `cargo test -p app --test blocking -- --ignored`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn serve(listener: TcpListener, body: String, requests: mpsc::Sender<String>) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (body, requests) = (body.clone(), requests.clone());
            // One thread per connection: Chromium opens sockets early and sends on them later.
            thread::spawn(move || {
                let mut buf = [0; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    return;
                }
                let line = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or_default().to_owned();
                let redirect = line.split(' ').nth(1).and_then(|path| path.strip_prefix("/redirect?to=")).map(str::to_owned);
                requests.send(line).ok();
                let response = match redirect {
                    Some(to) => format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                    None => format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()),
                };
                stream.write_all(response.as_bytes()).ok();
            });
        }
    });
}

#[test]
#[ignore = "opens a window"]
fn pages_cannot_open_windows_on_their_own() {
    let page = TcpListener::bind("127.0.0.1:0").unwrap();
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let (page_port, target_port) = (page.local_addr().unwrap().port(), target.local_addr().unwrap().port());
    let (loaded, page_requests) = mpsc::channel();
    let (opened, target_requests) = mpsc::channel();
    let script = format!(
        "<script>
            setTimeout(() => window.open('http://127.0.0.1:{target_port}/timer'), 300);
            window.open('http://127.0.0.1:{target_port}/load');
            const a = document.createElement('a');
            a.href = 'http://127.0.0.1:{target_port}/synthetic-click';
            a.target = '_blank';
            document.documentElement.append(a);
            a.click();
            fetch('/done');
        </script>"
    );
    serve(page, script, loaded);
    serve(target, String::new(), opened);

    let data = tempfile::tempdir().unwrap();
    {
        let store = store::Store::open(data.path()).unwrap();
        store.set_setting("filter_updates", "off").unwrap();
        let profile = store.create_profile("Personal", "6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b", false).unwrap();
        store.create_space(profile.id, "Home", "").unwrap();
    }
    let mut app = Command::new(env!("CARGO_BIN_EXE_tarn"))
        .arg(format!("http://127.0.0.1:{page_port}/"))
        .env("TARN_DATA_DIR", data.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ran = (0..2).all(|_| page_requests.recv_timeout(Duration::from_secs(15)).is_ok());
    let popup = target_requests.recv_timeout(Duration::from_secs(3)).ok();
    app.kill().ok();
    app.wait().ok();
    assert!(ran, "the page or its script never ran");
    assert_eq!(popup, None, "a window opened without a click");
}

/// Writes a bundle signed with `key` where `filters::load_cached` reads it: an ad and a cookie-banner
/// filter list and a malware set and rule list naming 127.0.0.1.
fn install_bundle(data: &std::path::Path, key: &ed25519_dalek::SigningKey) {
    use ed25519_dalek::Signer;
    use shield::bundle::{FORMAT, Kind, ListEntry, MANIFEST, Manifest, SIGNATURE, sha256_hex};
    let files: [(&str, &str, Kind, Vec<u8>); 4] = [
        ("ads.txt", "ads", Kind::Filters, b"/ad-banner.png\n##.ad-slot\n".to_vec()),
        ("cookies.txt", "annoyances", Kind::Filters, b"/consent-banner.js\n##.cookie-notice\n".to_vec()),
        ("malware.set", "malware", Kind::Set, shield::domains::DomainSet::encode(["127.0.0.1"])),
        ("malware-rules.txt", "malware", Kind::Domains, b"127.0.0.1\n".to_vec()),
    ];
    let dir = data.join("filters/bundle");
    std::fs::create_dir_all(&dir).unwrap();
    let mut lists = Vec::new();
    for (name, category, kind, bytes) in files {
        std::fs::write(dir.join(name), &bytes).unwrap();
        lists.push(ListEntry { name: name.into(), sha256: sha256_hex(&bytes), size: bytes.len() as u64, category: category.into(), kind, count: 1 });
    }
    let manifest = serde_json::to_vec(&Manifest { format: FORMAT, version: 1, created: "2026-11-13T00:00:00Z".into(), lists }).unwrap();
    std::fs::write(dir.join(SIGNATURE), key.sign(&manifest).to_bytes()).unwrap();
    std::fs::write(dir.join(MANIFEST), manifest).unwrap();
}

/// Requests each server saw while the app ran: the page on `localhost` and the listed host on 127.0.0.1.
/// With `chromium`, the bundled app at that path runs the page in a Chromium tab.
fn run_listed(paused: bool, chromium: Option<&std::path::Path>) -> (Vec<String>, Vec<String>) {
    let page = TcpListener::bind("127.0.0.1:0").unwrap();
    let listed = TcpListener::bind("127.0.0.1:0").unwrap();
    let (page_port, listed_port) = (page.local_addr().unwrap().port(), listed.local_addr().unwrap().port());
    let (page_tx, page_rx) = mpsc::channel();
    let (listed_tx, listed_rx) = mpsc::channel();
    // Rule lists compile after launch, so the page waits before it loads anything listed.
    let body = format!(
        "<div class=\"ad-slot\">ad</div><div class=\"cookie-notice\">cookies</div><script>
            setTimeout(() => {{
                const img = new Image();
                img.src = '/ad-banner.png';
                const pixel = new Image();
                pixel.src = 'http://127.0.0.1:{listed_port}/img';
                const frame = document.createElement('iframe');
                frame.src = 'http://127.0.0.1:{listed_port}/frame';
                const consent = document.createElement('script');
                consent.src = '/consent-banner.js';
                document.body.append(img, frame, consent);
                fetch('/cosmetic?' + getComputedStyle(document.querySelector('.ad-slot')).display);
                fetch('/cookie-notice?' + getComputedStyle(document.querySelector('.cookie-notice')).display);
                setTimeout(() => {{ location = '/redirect?to=http://127.0.0.1:{listed_port}/navigate'; }}, 500);
                setTimeout(() => fetch('/done'), 1500);
            }}, 3000);
        </script>"
    );
    serve(page, body, page_tx);
    serve(listed, String::new(), listed_tx);

    let data = tempfile::tempdir().unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&[9; 32]);
    install_bundle(data.path(), &key);
    {
        let store = store::Store::open(data.path()).unwrap();
        store.set_setting("filter_updates", "off").unwrap();
        let profile = store.create_profile("Personal", "6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b", false).unwrap();
        store.create_space(profile.id, "Home", "").unwrap();
        store.set_site_paused(profile.id, "localhost", paused).unwrap();
        if chromium.is_some() {
            store.set_setting("default_web_engine", "chromium").unwrap();
        }
    }
    let hex: String = key.verifying_key().to_bytes().iter().map(|b| format!("{b:02x}")).collect();
    // shortcut: a new Chromium profile's first start never loads its page, so the profile is started once first; drop this when that is fixed.
    if let Some(app) = chromium {
        let mut warm =
            Command::new(app).env("TARN_DATA_DIR", data.path()).env("TARN_TEST_FILTER_KEY", &hex).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        thread::sleep(Duration::from_secs(15));
        warm.kill().ok();
        warm.wait().ok();
        let _ = std::fs::remove_file(data.path().join("Chromium/SingletonLock"));
    }
    let mut app = Command::new(chromium.unwrap_or(std::path::Path::new(env!("CARGO_BIN_EXE_tarn"))))
        .arg(format!("http://localhost:{page_port}/"))
        .env("TARN_DATA_DIR", data.path())
        .env("TARN_TEST_FILTER_KEY", hex)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut seen = Vec::new();
    while let Ok(line) = page_rx.recv_timeout(Duration::from_secs(if chromium.is_some() { 40 } else { 20 })) {
        let done = line.contains("/done");
        seen.push(line);
        if done {
            break;
        }
    }
    thread::sleep(Duration::from_millis(500));
    app.kill().ok();
    app.wait().ok();
    (seen, listed_rx.try_iter().collect())
}

#[test]
#[ignore = "opens a window"]
fn signed_lists_block_ads_cookie_banners_and_dangerous_sites_and_a_pause_lifts_only_page_lists() {
    let (page, listed) = run_listed(false, None);
    assert!(page.iter().any(|l| l.contains("/done")), "the page never finished: {page:?}");
    assert!(!page.iter().any(|l| l.contains("/ad-banner.png")), "the ad was requested: {page:?}");
    assert!(page.iter().any(|l| l.contains("/cosmetic?none")), "the ad slot was not hidden: {page:?}");
    assert!(!page.iter().any(|l| l.contains("/consent-banner.js")), "the consent script was requested: {page:?}");
    assert!(page.iter().any(|l| l.contains("/cookie-notice?none")), "the cookie notice was not hidden: {page:?}");
    assert!(page.iter().any(|l| l.contains("/redirect?to=")), "the redirect was never requested: {page:?}");
    assert!(listed.is_empty(), "the listed host was reached: {listed:?}");

    let (page, listed) = run_listed(true, None);
    assert!(page.iter().any(|l| l.contains("/done")), "the paused page never finished: {page:?}");
    assert!(page.iter().any(|l| l.contains("/ad-banner.png")), "a paused site still blocks ads: {page:?}");
    assert!(page.iter().any(|l| l.contains("/cosmetic?block")), "a paused site still hides ad slots: {page:?}");
    assert!(page.iter().any(|l| l.contains("/consent-banner.js")), "a paused site still blocks consent scripts: {page:?}");
    assert!(page.iter().any(|l| l.contains("/cookie-notice?block")), "a paused site still hides cookie notices: {page:?}");
    assert!(listed.is_empty(), "a pause let the listed host through: {listed:?}");
}

/// The same lists in a Chromium tab. Needs a debug Chromium bundle, the only kind that takes the
/// test key: `TARN_PROFILE=debug TARN_SIGN_IDENTITY=<team identity> scripts/bundle.sh`, then
/// `TARN_TEST_CHROMIUM_APP=target/debug/Tarn.app/Contents/MacOS/tarn`. Element hiding is not built for Chromium yet.
#[test]
#[ignore = "opens a window and needs the Chromium bundle"]
fn chromium_tabs_block_like_webkit() {
    let Some(app) = std::env::var_os("TARN_TEST_CHROMIUM_APP") else { return };
    let app = std::path::PathBuf::from(app);
    let (page, listed) = run_listed(false, Some(&app));
    assert!(page.iter().any(|l| l.contains("/done")), "the page never finished: {page:?}");
    assert!(!page.iter().any(|l| l.contains("/ad-banner.png")), "the ad was requested: {page:?}");
    assert!(!page.iter().any(|l| l.contains("/consent-banner.js")), "the consent script was requested: {page:?}");
    assert!(page.iter().any(|l| l.contains("/redirect?to=")), "the redirect was never requested: {page:?}");
    assert!(listed.is_empty(), "the listed host was reached: {listed:?}");

    let (page, listed) = run_listed(true, Some(&app));
    assert!(page.iter().any(|l| l.contains("/done")), "the paused page never finished: {page:?}");
    assert!(page.iter().any(|l| l.contains("/ad-banner.png")), "a paused site still blocks ads: {page:?}");
    assert!(page.iter().any(|l| l.contains("/consent-banner.js")), "a paused site still blocks consent scripts: {page:?}");
    assert!(listed.is_empty(), "a pause let the listed host through: {listed:?}");
}
