//! A Lockdown profile keeps no website data: a cookie set before switching to Lockdown is gone
//! after a Lockdown session. Opens windows, so it runs only when asked:
//! `cargo test -p app --test lockdown -- --ignored`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use shield::ShieldLevel;
use store::Store;

struct Server {
    port: u16,
    requests: mpsc::Receiver<String>,
}

fn server() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, requests) = mpsc::channel();
    thread::spawn(move || {
        // Only the first response sets the cookie: restored tabs reload in later sessions and
        // would otherwise set it again in that session's own store.
        let mut cookie = "Set-Cookie: tarn=1; Max-Age=3600; Path=/\r\n";
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            if !request.starts_with("GET /?visit=") {
                stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").ok();
                continue;
            }
            tx.send(request).ok();
            let body = "<p>cookie</p>";
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            cookie = "";
            stream.write_all(response.as_bytes()).ok();
        }
    });
    Server { port, requests }
}

/// Launches at `level` and returns the Cookie header of this visit's page request. Session restore
/// may also reload an earlier visit's tab, so requests are matched by their `visit` tag.
fn visit(data: &Path, server: &Server, level: ShieldLevel, tag: u32) -> String {
    let store = Store::open(data).unwrap();
    let profile = store.profiles().unwrap()[0].id;
    store.set_profile_shield_level(profile, level).unwrap();
    drop(store);
    let mut app = Command::new(env!("CARGO_BIN_EXE_tarn"))
        .arg(format!("http://127.0.0.1:{}/?visit={tag}", server.port))
        .env("TARN_DATA_DIR", data)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let request = loop {
        let request = server.requests.recv_timeout(Duration::from_secs(15)).expect("page never requested");
        if request.starts_with(&format!("GET /?visit={tag} ")) {
            break request;
        }
    };
    // Give WebKit time to store the cookie and, under Lockdown, to empty the persistent store.
    thread::sleep(Duration::from_secs(3));
    Command::new("kill").arg(app.id().to_string()).status().ok();
    thread::sleep(Duration::from_secs(2));
    app.kill().ok();
    app.wait().ok();
    request.lines().find_map(|l| l.strip_prefix("Cookie: ")).unwrap_or("").to_owned()
}

fn fresh_profile() -> tempfile::TempDir {
    let data = tempfile::tempdir().unwrap();
    let store = Store::open(data.path()).unwrap();
    store.set_setting("filter_updates", "off").unwrap();
    let profile = store.create_profile("Personal", &uuid_for(data.path()), false).unwrap();
    store.create_space(profile.id, "Home", "").unwrap();
    data
}

/// A distinct WebKit data store per test directory.
fn uuid_for(path: &Path) -> String {
    let n = path.to_string_lossy().bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    format!("{:08x}-0000-4000-8000-{:012x}", n as u32, n & 0xffff_ffff_ffff)
}

#[test]
#[ignore = "opens windows"]
fn lockdown_session_leaves_no_cookies() {
    let control = fresh_profile();
    let s = server();
    assert_eq!(visit(control.path(), &s, ShieldLevel::Standard, 1), "");
    assert_eq!(visit(control.path(), &s, ShieldLevel::Standard, 2), "tarn=1", "cookies do not persist at Standard");

    let data = fresh_profile();
    let s = server();
    assert_eq!(visit(data.path(), &s, ShieldLevel::Standard, 1), "");
    assert_eq!(visit(data.path(), &s, ShieldLevel::Lockdown, 2), "", "Lockdown used the persistent store");
    assert_eq!(visit(data.path(), &s, ShieldLevel::Standard, 3), "", "Lockdown left the earlier cookie behind");
}
