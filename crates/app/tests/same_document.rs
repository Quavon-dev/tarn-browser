//! A same-document navigation (pushState) is recorded like a page load, with the title the page
//! sets afterwards, while fragment changes and URL rewrite floods are not. Opens a window, so it
//! runs only when asked: `cargo test -p app --test same_document -- --ignored`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
#[ignore = "opens a window"]
fn push_state_reaches_tab_and_history() {
    let page = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = page.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    let body = "<title>SPA</title><script>
        setTimeout(() => {
            history.pushState({}, '', '/pushed/route');
            document.title = 'Pushed';
            setTimeout(() => {
                for (let i = 0; i < 50; i++) location.hash = 'h' + i;
                for (let i = 0; i < 40; i++) history.replaceState({}, '', '/flood/' + i);
                setTimeout(() => { document.title = 'Late'; setTimeout(() => fetch('/done'), 1000); }, 500);
            }, 1000);
        }, 1500);
    </script>";
    thread::spawn(move || {
        for stream in page.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            tx.send(String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or_default().to_owned()).ok();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).ok();
        }
    });
    let data = tempfile::tempdir().unwrap();
    let profile = {
        let store = store::Store::open(data.path()).unwrap();
        store.set_setting("filter_updates", "off").unwrap();
        let profile = store.create_profile("Personal", "6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b", false).unwrap();
        store.create_space(profile.id, "Home", "").unwrap();
        profile.id
    };
    let mut app = Command::new(env!("CARGO_BIN_EXE_tarn"))
        .arg(format!("http://127.0.0.1:{port}/start"))
        .env("TARN_DATA_DIR", data.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let done = (0..4).any(|_| rx.recv_timeout(Duration::from_secs(15)).is_ok_and(|l| l.contains("/done")));
    thread::sleep(Duration::from_millis(500));
    app.kill().ok();
    app.wait().ok();
    assert!(done, "the page never finished");
    let store = store::Store::open(data.path()).unwrap();
    let history: Vec<String> = store.history(profile, 50).unwrap().into_iter().map(|e| e.url).collect();
    let entries = store.history(profile, 500).unwrap();
    let pushed = entries.iter().find(|e| e.url.ends_with("/pushed/route")).unwrap_or_else(|| panic!("history: {history:?}"));
    assert_eq!(pushed.title, "Pushed", "the visit takes the title the page set after pushState, and keeps it after the page moved on");
    assert!(!history.iter().any(|u| u.contains("#h")), "fragment changes are not visits: {history:?}");
    assert!(history.iter().filter(|u| u.contains("/flood/")).count() <= 2, "URL rewrites are rate limited: {history:?}");
    let tabs: Vec<String> = store.spaces(profile).unwrap().iter().flat_map(|s| store.tabs(s.id).unwrap()).map(|t| t.url).collect();
    assert!(tabs.iter().any(|u| u.ends_with("/flood/39")), "the tab follows the page's last URL: {tabs:?}");
}
