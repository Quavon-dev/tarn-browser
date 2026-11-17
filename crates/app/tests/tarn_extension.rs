//! A Tarn extension installed through the shared install path, with its site answer stored as
//! the site prompt stores it, running in the real app: its content script runs on the granted
//! host only, talks to its background page, keeps storage and blocks with its rule, the page sees
//! no bridge, and a Lockdown profile runs nothing. Opens windows, so it runs only when asked:
//! `cargo test -p app --test tarn_extension -- --ignored`. With `TARN_CHROMIUM_BUNDLE` set to a
//! bundle from `TARN_PROFILE=debug TARN_LOCAL_CHROMIUM=1 scripts/bundle.sh`, the same runs in
//! Chromium tabs of that bundle.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use browser_core::extensions::Install;
use browser_core::extensions::install::{self, Fetched};
use browser_core::github::{Fetch, FetchError};
use shield::ShieldLevel;
use store::Store;

const INSTALL_ID: &str = "0a1b2c3d4e5f";

const FILES: &[(&str, &str)] = &[
    (
        "manifest.json",
        r#"{"manifest_version": 3, "name": "Tarn Runtime Test", "version": "1.0",
            "browser_specific_settings": {"tarn": {"runtime": 1}},
            "permissions": ["storage", "declarativeNetRequest"],
            "host_permissions": ["http://127.0.0.1/*", "http://localhost/*"],
            "content_scripts": [
                {"matches": ["http://127.0.0.1/*"], "js": ["content.js"], "run_at": "document_end"},
                {"matches": ["http://localhost/*"], "js": ["frame.js"], "all_frames": true}
            ],
            "declarative_net_request": {"rule_resources": [{"id": "block", "enabled": true, "path": "rules.json"}]},
            "background": {"service_worker": "background.js"}}"#,
    ),
    (
        "content.js",
        "const beacon = s => { new Image().src = '/beacon?' + encodeURIComponent(s); };\n\
         beacon('content ' + location.pathname);\n\
         chrome.runtime.sendMessage({ ping: 1 }).then(r => beacon('reply ' + JSON.stringify(r)), e => beacon('error ' + e.message));\n\
         chrome.storage.local.set({ n: 7 }).then(() => chrome.storage.local.get('n')).then(v => beacon('storage ' + v.n));",
    ),
    ("frame.js", "new Image().src = '/beacon?' + encodeURIComponent('frame ' + location.host);"),
    ("background.js", "chrome.runtime.onMessage.addListener((m, sender, reply) => { reply({ pong: m.ping, frame: sender.frameId, url: sender.url }); });"),
    ("rules.json", r#"[{"id": 1, "action": {"type": "block"}, "condition": {"urlFilter": "blocked.js"}}]"#),
];

const PAGE: &str = "<title>granted</title><script src=\"/blocked.js\"></script><script>\
    addEventListener('load', () => { new Image().src = '/beacon?' + encodeURIComponent('main ' + [typeof (window.chrome && chrome.runtime && chrome.runtime.id), typeof window.browser, Object.getOwnPropertyNames(globalThis).filter(n => n.startsWith('__tarnExt')).join()].join()); });\
    document.write('<iframe src=\"http://localhost:' + location.port + '/inner\"></iframe>');\
    </script>";

struct NoNetwork;

impl Fetch for NoNetwork {
    fn get(&self, _url: &str, _max: u64) -> Result<Vec<u8>, FetchError> {
        Err(FetchError::Status(0))
    }
}

/// Serves the pages; `seen` gets `host path` of every request, and whether Chromium asked for the page.
fn serve(seen: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut buf = [0; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = request.split_whitespace().nth(1).unwrap_or_default().to_owned();
            let host = request.lines().find_map(|l| l.strip_prefix("Host: ").or_else(|| l.strip_prefix("host: "))).unwrap_or_default().trim().to_owned();
            let chrome = request.lines().any(|l| l.to_ascii_lowercase().starts_with("user-agent:") && l.contains(" Chrome/"));
            seen.lock().unwrap().push(format!("{host} {path}{}", if chrome && path == "/granted" { " from Chrome" } else { "" }));
            let body = if path == "/granted" { PAGE } else { "" };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).ok();
        }
    });
    port
}

fn uuid_for(path: &Path) -> String {
    let n = path.to_string_lossy().bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(u64::from(b)));
    format!("{:08x}-0000-4000-8000-{:012x}", n as u32, n & 0xffff_ffff_ffff)
}

/// A profile with the extension installed from a file through `browser_core`'s install path and
/// 127.0.0.1 granted, as the consent sheet and the site prompt leave it.
fn profile_with_extension(chromium: bool) -> tempfile::TempDir {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).unwrap();
    store.set_setting("filter_updates", "off").unwrap();
    if chromium {
        store.set_setting("default_web_engine", "chromium").unwrap();
        store.set_setting("chromium_notice_seen", "1").unwrap();
    }
    let profile = store.create_profile("Personal", &uuid_for(data.path()), false).unwrap();
    store.create_space(profile.id, "Home", "").unwrap();

    let package = tempfile::tempdir().unwrap();
    for (name, text) in FILES {
        std::fs::write(package.path().join(name), text).unwrap();
    }
    let archive = package.path().join("package.zip");
    let names: Vec<&str> = FILES.iter().map(|(n, _)| *n).collect();
    assert!(Command::new("zip").arg("-qX").arg(&archive).args(&names).current_dir(package.path()).status().unwrap().success());

    let root = data.path().join("Extensions");
    let slot = install::slot(&store, profile.id, &root, None, || INSTALL_ID.to_owned()).unwrap();
    let Fetched::Offer(offer) =
        install::fetch(&NoNetwork, Install::File(archive), &slot, &BTreeSet::new(), &profile.data_store_uuid, &["en".to_owned()]).unwrap()
    else {
        panic!("a file install offers the package");
    };
    install::confirm(&store, &install::pending(slot, offer).unwrap()).unwrap();
    store.set_extension_host(profile.id, INSTALL_ID, "127.0.0.1", true).unwrap();
    data
}

fn set_level(data: &Path, level: ShieldLevel) {
    let store = Store::open(data).unwrap();
    let profile = store.profiles().unwrap()[0].id;
    store.set_profile_shield_level(profile, level).unwrap();
}

/// Launches `app` on `data` with the granted page, waits until `done` holds for the requests the
/// site saw (or 25 seconds pass), quits it and returns those requests.
fn visit(app: &Path, data: &Path, port: u16, seen: &Arc<Mutex<Vec<String>>>, done: impl Fn(&[String]) -> bool) -> Vec<String> {
    seen.lock().unwrap().clear();
    let mut child = Command::new(app)
        .arg(format!("http://127.0.0.1:{port}/granted"))
        .env("TARN_DATA_DIR", data)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let until = Instant::now() + Duration::from_secs(25);
    while Instant::now() < until && !done(&seen.lock().unwrap()) {
        thread::sleep(Duration::from_millis(200));
    }
    thread::sleep(Duration::from_secs(2));
    Command::new("kill").arg(child.id().to_string()).status().ok();
    thread::sleep(Duration::from_secs(2));
    child.kill().ok();
    child.wait().ok();
    seen.lock().unwrap().clone()
}

/// What the content scripts and the page reported to 127.0.0.1, decoded.
fn beacons(seen: &[String], port: u16) -> Vec<String> {
    let prefix = format!("127.0.0.1:{port} /beacon?");
    seen.iter().filter_map(|s| s.strip_prefix(&prefix)).map(decode).collect()
}

fn decode(query: &str) -> String {
    let mut out = Vec::new();
    let mut rest = query.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        match (b, tail.get(..2).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                rest = &tail[2..];
            }
            _ => {
                out.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn runs_in(app: &Path, chromium: bool) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = serve(seen.clone());
    let data = profile_with_extension(chromium);
    let engine = if chromium { "Chromium" } else { "WebKit" };

    let got = visit(app, data.path(), port, &seen, |s| {
        let b = beacons(s, port);
        ["main ", "reply ", "storage "].iter().all(|w| b.iter().any(|x| x.starts_with(w)))
    });
    assert_eq!(got.contains(&format!("127.0.0.1:{port} /granted from Chrome")), chromium, "{engine} shows the page: {got:?}");
    let b = beacons(&got, port);
    assert!(b.contains(&"content /granted".to_owned()), "{engine}: the content script runs on the granted host: {got:?}");
    assert!(
        b.iter().any(|x| x.starts_with("reply ")
            && x.contains("\"pong\":1")
            && x.contains("\"frame\":0")
            && x.contains(&format!("\"url\":\"http://127.0.0.1:{port}/granted\""))),
        "{engine}: the background page answers, naming the sender's frame and page: {b:?}"
    );
    assert!(b.contains(&"storage 7".to_owned()), "{engine}: storage keeps what the script set: {b:?}");
    assert!(b.contains(&"main undefined,undefined,".to_owned()), "{engine}: the page's main world sees no bridge: {b:?}");
    assert!(!got.iter().any(|s| s.ends_with(" /blocked.js")), "{engine}: the rule blocks on the granted site: {got:?}");
    assert!(got.contains(&format!("localhost:{port} /inner")), "{engine}: the frame of the host without a grant loads: {got:?}");
    assert!(!got.iter().any(|s| s.contains("frame%20")), "{engine}: and runs nothing: {got:?}");

    set_level(data.path(), ShieldLevel::Lockdown);
    let got = visit(app, data.path(), port, &seen, |s| s.iter().any(|x| x.contains(" /granted")));
    assert!(got.contains(&format!("127.0.0.1:{port} /granted")), "{engine}: the Lockdown profile shows the page in WebKit: {got:?}");
    assert!(!got.iter().any(|s| s.contains("/beacon?")), "{engine}: and runs no extension: {got:?}");
}

#[test]
#[ignore = "opens windows"]
fn tarn_extension_runs_in_webkit_tabs() {
    runs_in(Path::new(env!("CARGO_BIN_EXE_tarn")), false);
}

#[test]
#[ignore = "opens windows and needs a signed bundle (TARN_CHROMIUM_BUNDLE)"]
fn tarn_extension_runs_in_chromium_tabs() {
    let Some(bundle) = std::env::var_os("TARN_CHROMIUM_BUNDLE").map(PathBuf::from) else {
        eprintln!("tarn_extension_runs_in_chromium_tabs: skipped (set TARN_CHROMIUM_BUNDLE)");
        return;
    };
    runs_in(&bundle.join("Contents/MacOS/tarn"), true);
}
