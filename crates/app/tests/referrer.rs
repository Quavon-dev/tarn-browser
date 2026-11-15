//! WebKit's default sends only the origin to another origin; Strict sends no referrer at all.
//! Opens a window, so it runs only when asked: `cargo test -p app --test referrer -- --ignored`.

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
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            requests.send(String::from_utf8_lossy(&buf[..n]).into_owned()).ok();
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(response.as_bytes()).ok();
        }
    });
}

fn referer(level: &str) -> Option<String> {
    let page = TcpListener::bind("127.0.0.1:0").unwrap();
    let image = TcpListener::bind("127.0.0.1:0").unwrap();
    let (page_port, image_port) = (page.local_addr().unwrap().port(), image.local_addr().unwrap().port());
    let (tx, rx) = mpsc::channel();
    serve(page, format!("<img src=\"http://localhost:{image_port}/pixel\">"), mpsc::channel().0);
    serve(image, String::new(), tx);

    let data = tempfile::tempdir().unwrap();
    {
        let store = store::Store::open(data.path()).unwrap();
        store.set_setting("filter_updates", "off").unwrap();
        let profile = store.create_profile("Personal", &uuid(), false).unwrap();
        store.create_space(profile.id, "Home", "").unwrap();
        store.set_profile_shield_level(profile.id, shield::ShieldLevel::parse(level).unwrap()).unwrap();
    }
    let mut app = Command::new(env!("CARGO_BIN_EXE_tarn"))
        .arg(format!("http://127.0.0.1:{page_port}/secret/path?token=1"))
        .env("TARN_DATA_DIR", data.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let request = rx.recv_timeout(Duration::from_secs(15)).ok();
    app.kill().ok();
    app.wait().ok();
    let request = request?;
    Some(request.lines().find_map(|l| l.strip_prefix("Referer: ").or_else(|| l.strip_prefix("referer: "))).unwrap_or("").to_owned())
}

fn uuid() -> String {
    "6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b".into()
}

#[test]
#[ignore = "opens a window"]
fn strict_sends_no_cross_origin_referrer() {
    let standard = referer("standard").expect("standard: image never requested");
    assert!(standard.starts_with("http://127.0.0.1:") && !standard.contains("secret"), "standard referer: {standard:?}");
    let strict = referer("strict").expect("strict: image never requested");
    assert_eq!(strict, "", "strict referer");
    let lockdown = referer("lockdown").expect("lockdown: image never requested");
    assert_eq!(lockdown, "", "lockdown referer");
}
