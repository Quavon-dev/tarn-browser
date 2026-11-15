//! The shell process opens no listening socket and, with filter-list updates off, no connection
//! of its own (PLAN rules 4 and 5). Pages load in WebKit's own processes, so even a loaded page
//! leaves the shell without sockets. Sockets are sampled every 500 ms.
//! It opens a window, so it runs only when asked: `cargo test -p app --test egress -- --ignored`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

fn shell_sockets(pid: u32) -> String {
    let out = Command::new("lsof").args(["-a", "-n", "-P", "-i", "-p", &pid.to_string()]).output().expect("lsof");
    assert!(out.stderr.is_empty(), "lsof failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
#[ignore = "opens a window"]
fn shell_has_no_sockets_while_a_page_loads() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}/", server.local_addr().unwrap().port());
    let served = thread::spawn(move || {
        let (mut stream, _) = server.accept().unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request);
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 13\r\n\r\n<p>egress</p>").unwrap();
    });
    let data = tempfile::tempdir().unwrap();
    store::Store::open(data.path()).unwrap().set_setting("filter_updates", "off").unwrap();
    let mut app =
        Command::new(env!("CARGO_BIN_EXE_tarn")).arg(&url).env("TARN_DATA_DIR", data.path()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let mut seen = String::new();
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(500));
        seen.push_str(&shell_sockets(app.id()));
        if served.is_finished() {
            break;
        }
    }
    thread::sleep(Duration::from_secs(2));
    seen.push_str(&shell_sockets(app.id()));
    let page_loaded = served.is_finished();
    let running = matches!(app.try_wait(), Ok(None));
    app.kill().ok();
    app.wait().ok();
    assert!(page_loaded, "WebKit never requested the test page");
    assert!(running, "the shell exited during the test");
    assert!(seen.is_empty(), "shell process opened sockets:\n{seen}");
}
