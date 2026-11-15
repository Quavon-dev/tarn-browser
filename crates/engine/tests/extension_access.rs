//! An unpacked extension in a real WebKit view reaches only the hosts the
//! user allowed. It opens a window, so it runs only when asked:
//! `cargo test -p engine --test extension_access -- --ignored`.

#[cfg(target_os = "macos")]
mod window {
    use std::cell::RefCell;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::ptr::NonNull;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use engine::extensions::{Enabled, Extension};
    use engine::nav_policy::Site;
    use engine::{DataStore, Event, Host, WebView};
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
    use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
    use shield::ShieldLevel;

    struct View(Retained<objc2_app_kit::NSView>);

    impl HasWindowHandle for View {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw = RawWindowHandle::AppKit(AppKitWindowHandle::new(NonNull::from(&*self.0).cast()));
            // SAFETY: the view outlives the borrowed handle.
            Ok(unsafe { WindowHandle::borrow_raw(raw) })
        }
    }

    fn spin(seconds: f64, done: &dyn Fn() -> bool) {
        let until = Instant::now() + Duration::from_secs_f64(seconds);
        while !done() && Instant::now() < until {
            NSRunLoop::mainRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
        }
    }

    fn serve(seen: Arc<Mutex<Vec<String>>>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut first = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                reader.read_line(&mut first).ok();
                while reader.read_line(&mut String::new()).unwrap_or(0) > 2 {}
                let path = first.split_whitespace().nth(1).unwrap_or_default().to_owned();
                seen.lock().unwrap().push(path);
                let body = "<title>PAGE</title><p>page</p>";
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).ok();
            }
        });
        port
    }

    const BACKGROUND: &str = r#"
fetch("http://127.0.0.1:__PORT__/background-fetch").catch(() => {});
setInterval(async () => {
  const tabs = await browser.tabs.query({}).catch(() => []);
  for (const t of tabs) {
    if (!t.url) {
      browser.tabs.update(t.id, {url: "http://localhost:__PORT__/phish"}).catch(() => {});
      browser.tabs.reload(t.id).catch(() => {});
    }
  }
  if (browser.action) browser.action.openPopup().catch(() => {});
}, 500);
"#;

    fn fixture(port: u16) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let manifest = r#"{"manifest_version":3,"name":"Bitwarden","version":"1","permissions":["tabs","activeTab","nativeMessaging","clipboardRead","storage"],"host_permissions":["<all_urls>"],"background":{"service_worker":"bg.js"},"action":{"default_popup":"popup.html"},"content_scripts":[{"matches":["<all_urls>"],"js":["content.js"]}]}"#;
        std::fs::write(dir.path().join("manifest.json"), manifest).unwrap();
        std::fs::write(dir.path().join("bg.js"), BACKGROUND.replace("__PORT__", &port.to_string())).unwrap();
        std::fs::write(dir.path().join("content.js"), r#"document.title = "EXT:" + document.title;"#).unwrap();
        std::fs::write(dir.path().join("popup.html"), "<p>popup</p>").unwrap();
        dir
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let port = serve(seen.clone());
        let dir = fixture(port);

        let loaded: Rc<RefCell<Option<Extension>>> = Rc::default();
        let slot = loaded.clone();
        Extension::unpacked(dir.path(), "install-1", move |e| *slot.borrow_mut() = Some(e.expect("fixture loads")));
        spin(10.0, &|| loaded.borrow().is_some());
        let extension = loaded.borrow_mut().take().expect("fixture loaded");
        let (apis, _) = extension.api_permissions();
        assert!(apis.contains(&"nativeMessaging".to_owned()), "WebKit parsed the manifest: {apis:?}");

        let uuid = "6A1B2C3D-0000-4000-8000-00000000E714";
        let data = DataStore::persistent(uuid, uuid, Rc::new(|_| {})).unwrap();
        let enable = |allowed: &[&str], blocked: &[&str]| {
            let enabled = Enabled {
                extension: extension.clone(),
                all_sites: false,
                confirmed: apis.clone(),
                allowed: allowed.iter().map(|h| h.to_string()).collect(),
                denied: Vec::new(),
            };
            data.set_extensions(&[enabled], &blocked.iter().map(|h| h.to_string()).collect::<Vec<_>>(), ShieldLevel::Standard);
        };
        enable(&[], &[]);
        assert_eq!(data.running_extensions(), ["install-1"]);
        spin(2.0, &|| false);

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
        let host = Host::from_window(&View(window.contentView().unwrap())).unwrap();
        let prompts: Rc<RefCell<Vec<engine::Prompt>>> = Rc::default();
        let sink = {
            let prompts = prompts.clone();
            Rc::new(move |e: Event| {
                if let Event::Prompt(p) = e {
                    prompts.borrow_mut().push(p);
                }
            })
        };
        let site_for: engine::SiteFor = Rc::new(|_| Site::new(ShieldLevel::Standard));
        let open = |url: &str| {
            let view = WebView::new(&host, &data, ShieldLevel::Standard, true, false, site_for.clone(), sink.clone());
            view.set_frame(0., 0., 800., 600.);
            view.load(&url::Url::parse(url).unwrap());
            view
        };

        let granted_url = format!("http://127.0.0.1:{port}/page");
        let other_url = format!("http://localhost:{port}/other");
        let other = open(&other_url);
        spin(4.0, &|| false);
        assert_eq!(other.title(), "PAGE", "an ungranted host is not touched");
        for p in prompts.borrow_mut().drain(..) {
            p.decline();
        }

        enable(&["127.0.0.1"], &[]);
        let granted = open(&granted_url);
        spin(4.0, &|| granted.title() == "EXT:PAGE");
        assert_eq!(granted.title(), "EXT:PAGE", "the content script runs on an allowed host");
        assert_eq!(other.title(), "PAGE");

        let seen_now = seen.lock().unwrap().clone();
        assert!(!seen_now.iter().any(|p| p == "/background-fetch"), "private-network rules apply to the worker: {seen_now:?}");
        assert!(!seen_now.iter().any(|p| p == "/phish"), "tabs.update on an ungranted tab is refused: {seen_now:?}");
        let reloads = seen_now.iter().filter(|p| *p == "/other").count();
        assert_eq!(reloads, 1, "tabs.reload on an ungranted tab is refused: {seen_now:?}");

        enable(&["127.0.0.1", "localhost"], &["localhost"]);
        other.reload();
        spin(3.0, &|| false);
        DataStore::wipe_extensions(uuid);
        DataStore::wipe_persistent(uuid);
        assert_eq!(other.title(), "PAGE", "a Lockdown host stays denied even when allowed");

        window.close();
        println!("extension_access: ok");
    }
}

fn main() {
    if !std::env::args().any(|a| a == "--ignored" || a == "--include-ignored") {
        println!("extension_access: skipped (opens a window; pass --ignored)");
    } else if cfg!(target_os = "macos") {
        #[cfg(target_os = "macos")]
        window::run();
    }
}
