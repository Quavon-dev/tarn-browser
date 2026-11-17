//! The local site, extension files and window the Tarn extension tests share; every page is
//! served by one listener on 127.0.0.1, reached as `127.0.0.1` and as `localhost`, two sites.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};

pub const ORIGIN: &str = "0123456789abcdef0123456789abcdef";
pub const INSTALL: &str = "0a1b2c";

pub const PAGE: &str = "<title>PAGE</title><script src=\"/blocked.js\"></script><script>\
    addEventListener('load', () => { document.title += '|main:' + typeof window.chrome + typeof window.browser + typeof window.__tarnExtPost + typeof window.__tarnExtRun + typeof (window.webkit && window.webkit.messageHandlers.tarnExt); });\
    </script><p>page</p>";

/// What the page's main world can see of any extension, and whether it can load the extension's
/// own files (S1), in its title.
const PROBE: &str = "<title>PROBE</title><p>probe</p><script>\
    const own = 'tarn-extension://' + new URLSearchParams(location.search).get('o') + '/';\
    addEventListener('load', async () => {\
      document.title += '|main:' + [typeof (window.chrome && chrome.runtime && chrome.runtime.id), typeof window.browser, Object.getOwnPropertyNames(globalThis).filter(n => n.startsWith('__tarnExt')).join(), typeof (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.tarnExt)].join(',');\
      const fetched = await fetch(own + 'content.js').then(() => 'loaded', () => 'refused');\
      const image = await new Promise(r => { const i = new Image(); i.onload = () => r('loaded'); i.onerror = () => r('error'); i.src = own + 'icon.png'; });\
      const f = document.createElement('iframe'); f.src = own + 'bg.html'; document.body.append(f);\
      await fetch('/slow');\
      let framed; try { framed = f.contentWindow.location.href; } catch (e) { framed = 'loaded'; }\
      document.title += '|s1:' + [fetched, image, framed].join();\
    });</script>";

/// A page of 127.0.0.1 with a same-site frame the gate lets in, one it keeps out, and a
/// cross-site frame of localhost, which Chromium puts in a process of its own.
const FRAMES: &str = "<title>FRAMES</title><p>frames</p><script>\
    for (const src of ['/sub?ok', '/sub?no', 'http://localhost:' + location.port + '/sub?ok']) { const f = document.createElement('iframe'); f.src = src; document.body.append(f); }\
    </script>";

/// A page with a same-site and a cross-site frame, each of which an extension moves to a new document.
const TWICE: &str = "<title>TWICE</title><p>twice</p><script>\
    for (const src of ['/sub?stale', 'http://localhost:' + location.port + '/sub?stale']) { const f = document.createElement('iframe'); f.src = src; document.body.append(f); }\
    </script>";

const SUB: &str = "<title>SUB</title><p>sub</p>";

/// A page with one cross-site frame that navigates itself, in its own process, after a while.
const LATE: &str = "<title>LATE</title><p>late</p><script>\
    const f = document.createElement('iframe'); f.src = 'http://localhost:' + location.port + '/sub?late'; document.body.append(f);\
    </script>";

/// A page with one cross-site frame that moves to `/sub?wait`, which the server answers only after
/// the test has put `release` into the requests seen, and from there to `/sub?ok&released`.
const HELD: &str = "<title>HELD</title><p>held</p><script>\
    const f = document.createElement('iframe'); f.src = 'http://localhost:' + location.port + '/sub?held'; document.body.append(f);\
    </script>";

const SUB_HELD: &str = "<title>SUB</title><script>addEventListener('load', () => setTimeout(() => { location.search = '?wait'; }, 500));</script>";

const SUB_WAIT: &str = "<title>SUB</title><script>addEventListener('load', () => setTimeout(() => { location.search = '?ok&released'; }, 500));</script>";

const SUB_LATE: &str = "<title>SUB</title><script>setTimeout(() => { location.search = '?ok&again'; }, 4000);</script>";

const STORE_LINK: &str = "https://extensions.tarnbrowser.com/install/helper";

/// Serves the test site; `seen` gets `host path` of every request.
pub fn serve(seen: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let seen = seen.clone();
            std::thread::spawn(move || {
                let mut first = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                reader.read_line(&mut first).ok();
                let mut host = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) <= 2 {
                        break;
                    }
                    if let Some(h) = line.strip_prefix("Host: ").or_else(|| line.strip_prefix("host: ")) {
                        host = h.trim().to_owned();
                    }
                }
                let path = first.split_whitespace().nth(1).unwrap_or_default().to_owned();
                seen.lock().unwrap().push(format!("{host} {path}"));
                if path == "/slow" {
                    std::thread::sleep(Duration::from_millis(1500));
                }
                let route = path.split('?').next().unwrap_or_default();
                let until = Instant::now() + Duration::from_secs(30);
                while path.ends_with("?wait") && !seen.lock().unwrap().iter().any(|s| s == "release") && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(50));
                }
                if route == "/store-redirect" {
                    write!(stream, "HTTP/1.1 302 Found\r\nLocation: {STORE_LINK}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").ok();
                    return;
                }
                if route == "/nocontent" {
                    std::thread::sleep(Duration::from_millis(1000));
                    write!(stream, "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n").ok();
                    return;
                }
                let store = format!(
                    "<title>STORE</title><a id=l href={STORE_LINK} style='position:fixed;inset:0'>link</a><iframe src={STORE_LINK}></iframe>\
                     <script>addEventListener('load', () => setTimeout(() => {{ document.title = 'RAN'; {} }}, 300));</script>",
                    match path.split('?').nth(1) {
                        Some("click") => "document.getElementById('l').click()".to_owned(),
                        Some("assign") => format!("location.href = '{STORE_LINK}'"),
                        _ => String::new(),
                    }
                );
                let (kind, body) = match route {
                    "/store" => ("text/html", store.as_str()),
                    _ if route.ends_with(".js") => ("text/javascript", "window.blockedRan = 1;"),
                    "/probe" => ("text/html", PROBE),
                    "/frames" => ("text/html", FRAMES),
                    "/twice" => ("text/html", TWICE),
                    "/sub" if path.ends_with("?late") => ("text/html", SUB_LATE),
                    "/sub" if path.ends_with("?held") => ("text/html", SUB_HELD),
                    "/sub" if path.ends_with("?wait") => ("text/html", SUB_WAIT),
                    "/held" => ("text/html", HELD),
                    "/sub" => ("text/html", SUB),
                    "/late" => ("text/html", LATE),
                    "/beacon" => ("text/plain", ""),
                    _ => ("text/html", PAGE),
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).ok();
            });
        }
    });
    port
}

/// The fixture's files, as a package would ship them. `frame.js` is the all-frames entry: it
/// claims to be the main frame, which the engine must not believe, and reports the reply it gets.
/// `framer.html` is an extension page that frames one of its own pages and a web page, and says
/// where its frames are when it hears "report".
pub fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("content.js", "'use strict';\nlet started = document.readyState;\nchrome.storage.local.set({ seen: true });"),
        (
            "late.js",
            "document.title = 'EXT:' + document.title + '|start:' + started + '|late:' + typeof chrome.runtime.id;\nchrome.runtime.sendMessage({ ping: 1 }).then(r => { document.title += '|reply:' + r.pong; });",
        ),
        (
            "frame.js",
            "chrome.runtime.sendMessage({ frame: location.href, sender: { frameId: 0, url: 'https://bank.test/' } }).then(r => chrome.runtime.sendMessage({ replied: location.href, pong: r.pong }));",
        ),
        ("content.css", "p { color: rgb(1, 2, 3); }"),
        ("bg.html", "<!doctype html><script src=\"bg.js\"></script>"),
        ("bg.js", "chrome.runtime.sendMessage('from-background');"),
        ("framer.html", "<!doctype html><body><script src=\"framer.js\"></script></body>"),
        (
            "framer.js",
            "const port = new URLSearchParams(location.search).get('port');\n\
             const frames = ['inner.html', 'http://127.0.0.1:' + port + '/sub?from-extension'].map(src => { const f = document.createElement('iframe'); f.src = src; document.body.append(f); return f; });\n\
             const at = f => { try { return f.contentWindow.location.href; } catch (e) { return 'loaded'; } };\n\
             chrome.runtime.onMessage.addListener(m => { if (m === 'report') chrome.runtime.sendMessage('framed:' + frames.map(at).join()); });",
        ),
        ("inner.html", "<!doctype html><p>inner</p>"),
    ];
    for (name, text) in files {
        std::fs::write(dir.path().join(name), text).unwrap();
    }
    dir
}

pub struct View(pub Retained<NSView>);

impl HasWindowHandle for View {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(NonNull::from(&*self.0).cast()));
        // SAFETY: the view outlives the borrowed handle.
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

pub fn spin(seconds: f64, done: &dyn Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs_f64(seconds);
    while !done() && Instant::now() < until {
        NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
    }
}

/// Starts the app as an accessory and opens a window for the views.
pub fn window(mtm: MainThreadMarker) -> Retained<NSWindow> {
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.finishLaunching();
    // SAFETY: main-thread creation of a fresh window.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(100., 100.), NSSize::new(800., 600.)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: the window is not released on close; this binding owns it.
    unsafe { window.setReleasedWhenClosed(false) };
    window.makeKeyAndOrderFront(None);
    window
}
