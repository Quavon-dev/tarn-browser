//! A URL the user opens keeps counting as theirs across server redirects, so it reaches a page on
//! the local network (S10 only refuses navigations the page starts). Opens a window, so it runs
//! only when asked: `cargo test -p app --test redirects -- --ignored`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[test]
#[ignore = "opens a window"]
fn an_opened_url_follows_redirects_to_a_local_page() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for stream in server.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let line = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or_default().to_owned();
            let path = line.split(' ').nth(1).unwrap_or_default().to_owned();
            let response = match path.as_str() {
                "/start" => "HTTP/1.1 302 Found\r\nLocation: /hop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                "/hop" => "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                _ => {
                    let body = if path == "/final" { "<script>setTimeout(() => { location = '/script' }, 300)</script>" } else { "ok" };
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                }
            };
            stream.write_all(response.as_bytes()).ok();
            tx.send(path).ok();
        }
    });
    let data = tempfile::tempdir().unwrap();
    {
        let store = store::Store::open(data.path()).unwrap();
        store.set_setting("filter_updates", "off").unwrap();
        let profile = store.create_profile("Personal", "6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b", false).unwrap();
        store.create_space(profile.id, "Home", "").unwrap();
    }
    let mut app = Command::new(env!("CARGO_BIN_EXE_tarn"))
        .arg(format!("http://127.0.0.1:{port}/start"))
        .env("TARN_DATA_DIR", data.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut seen = Vec::new();
    while let Ok(path) = rx.recv_timeout(Duration::from_secs(15)) {
        seen.push(path);
        if seen.last().is_some_and(|p| p == "/script") {
            break;
        }
    }
    app.kill().ok();
    app.wait().ok();
    for path in ["/hop", "/final", "/script"] {
        assert!(seen.iter().any(|p| p == path), "{path} was never requested: {seen:?}");
    }
}
