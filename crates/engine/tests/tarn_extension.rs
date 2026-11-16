//! A Tarn extension in a real WebKit view: its content script runs only in its own world and only
//! where the gate lets it, the page's main world never sees the bridge, messages go both ways,
//! its network rule blocks, and its background page loads from `tarn-extension://`. It opens a
//! window, so it runs only when asked:
//! `cargo test -p engine --test tarn_extension -- --ignored`.

#[cfg(target_os = "macos")]
mod window {
    use std::cell::RefCell;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::ptr::NonNull;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use engine::nav_policy::Site;
    use engine::tarn_ext::{ExtensionView, FrameId, PageSpec, Resource, RunAt, Script, World, WorldScripts, bootstrap};
    use engine::{DataStore, Event, ExtensionPage, FilterLists, Host, WebView};
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
    use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
    use serde_json::{Value, json};
    use shield::ShieldLevel;
    use url::Url;

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

    const PAGE: &str = "<title>PAGE</title><script src=\"/blocked.js\"></script><script>\
        addEventListener('load', () => { document.title += '|main:' + typeof window.chrome + typeof window.browser + typeof window.__tarnExtPost + typeof window.__tarnExtRun + typeof (window.webkit && window.webkit.messageHandlers.tarnExt); });\
        </script><p>page</p>";

    fn serve(seen: Arc<Mutex<Vec<String>>>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let seen = seen.clone();
                std::thread::spawn(move || {
                    let mut first = String::new();
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    reader.read_line(&mut first).ok();
                    while reader.read_line(&mut String::new()).unwrap_or(0) > 2 {}
                    let path = first.split_whitespace().nth(1).unwrap_or_default().to_owned();
                    seen.lock().unwrap().push(path.clone());
                    if path == "/slow" {
                        std::thread::sleep(Duration::from_millis(1500));
                    }
                    let (kind, body) = if path.ends_with(".js") { ("text/javascript", "window.blockedRan = 1;") } else { ("text/html", PAGE) };
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).ok();
                });
            }
        });
        port
    }

    /// The fixture's files, as a package would ship them.
    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let files = [
            ("content.js", "'use strict';\nlet started = document.readyState;\nchrome.storage.local.set({ seen: true });"),
            (
                "late.js",
                "document.title = 'EXT:' + document.title + '|start:' + started + '|late:' + typeof chrome.runtime.id;\nchrome.runtime.sendMessage({ ping: 1 }).then(r => { document.title += '|reply:' + r.pong; });",
            ),
            ("content.css", "p { color: rgb(1, 2, 3); }"),
            ("bg.html", "<!doctype html><script src=\"bg.js\"></script>"),
            ("bg.js", "chrome.runtime.sendMessage('from-background');"),
        ];
        for (name, text) in files {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        dir
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let port = serve(seen.clone());
        let dir = fixture();
        let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();

        let world = World::for_install("0a1b2c").unwrap();
        let origin = "0123456789abcdef0123456789abcdef";
        let config = |page: bool| json!({"id": origin, "base": format!("tarn-extension://{origin}/"), "manifest": "{}", "messages": {}, "locale": "en", "storage": true, "page": page});
        let scripts = Rc::new(WorldScripts {
            world: world.clone(),
            bootstrap: bootstrap(&config(false)),
            scripts: vec![
                Script { run_at: RunAt::DocumentStart, all_frames: false, js: vec![read("content.js")], css: vec![read("content.css")] },
                Script { run_at: RunAt::DocumentEnd, all_frames: false, js: vec![read("late.js")], css: vec![] },
            ],
        });

        let uuid = "6A1B2C3D-0000-4000-8000-00000000E715";
        let data = DataStore::persistent(uuid, uuid, Rc::new(|_| {})).unwrap();
        let rules = format!(r#"[{{"trigger":{{"url-filter":"^http://127\\.0\\.0\\.1:{port}/blocked\\.js"}},"action":{{"type":"block"}}}}]"#);
        let compiled: Rc<RefCell<Option<FilterLists>>> = Rc::default();
        let slot = compiled.clone();
        FilterLists::compile("tarn-ext.test", "1", vec![rules], move |r| *slot.borrow_mut() = Some(r.expect("rules compile")));
        spin(10.0, &|| compiled.borrow().is_some());
        data.set_extension_rule_lists("tarn-ext.test", compiled.borrow().as_ref());

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

        type Heard = (World, FrameId, Option<Url>, Value);
        let messages: Rc<RefCell<Vec<Heard>>> = Rc::default();
        let views: Rc<RefCell<Vec<Rc<WebView>>>> = Rc::default();
        let sink = {
            let (messages, views) = (messages.clone(), views.clone());
            Rc::new(move |e: Event| {
                let Event::ExtensionMessage { world, frame, origin, json } = e else { return };
                let message: Value = serde_json::from_str(&json).unwrap();
                if message["call"] == "runtime.sendMessage"
                    && let Some(id) = message["id"].as_u64()
                {
                    for view in views.borrow().iter() {
                        view.post_extension_message(&world, Some(frame), &json!({"id": id, "result": {"pong": 2}}).to_string());
                    }
                }
                messages.borrow_mut().push((world, frame, origin, message));
            })
        };
        let granted_host = "127.0.0.1";
        let site_for: engine::SiteFor = Rc::new(|_| Site::new(ShieldLevel::Standard));
        let open = |url: &str| {
            let view = Rc::new(WebView::new(&host, &data, ShieldLevel::Standard, true, false, site_for.clone(), sink.clone()));
            view.set_frame(0., 0., 800., 600.);
            view.set_extension_gate(Rc::new(move |_, frame| if frame.url.host_str() == Some(granted_host) { vec![0, 1] } else { Vec::new() }));
            view.add_extension_scripts(scripts.clone());
            views.borrow_mut().push(view.clone());
            view.load(&Url::parse(url).unwrap());
            view
        };

        let other = open(&format!("http://localhost:{port}/other"));
        spin(5.0, &|| other.title().contains("|main:"));
        assert_eq!(other.title(), "PAGE|main:undefinedundefinedundefinedundefinedundefined", "an ungranted host runs nothing");

        let granted = open(&format!("http://127.0.0.1:{port}/page"));
        spin(8.0, &|| granted.title().contains("|reply:") && granted.title().contains("|main:"));
        let title = granted.title();
        assert!(title.starts_with("EXT:PAGE|start:loading|late:string"), "the entry's files share their world, from document start on a granted host: {title}");
        assert!(title.contains("|main:undefinedundefinedundefinedundefinedundefined"), "the page's main world sees no bridge: {title}");
        assert!(title.contains("|reply:2"), "a reply reaches the content script: {title}");
        let got = messages.borrow().clone();
        assert!(got.iter().all(|(w, f, o, _)| *w == world && *f == FrameId::MAIN && o.is_none()), "messages name the world and frame they came from: {got:?}");
        assert!(got.iter().any(|(_, _, _, m)| m["call"] == "runtime.sendMessage" && m["args"][0]["ping"] == 1), "{got:?}");
        assert!(got.iter().any(|(_, _, _, m)| m["call"] == "storage.set" && m["args"][0]["seen"] == true), "{got:?}");
        let requested = seen.lock().unwrap().clone();
        assert_eq!(requested.iter().filter(|p| *p == "/blocked.js").count(), 1, "the rule blocks the granted page's script, not the other's: {requested:?}");

        let ungated = Rc::new(WebView::new(&host, &data, ShieldLevel::Standard, true, false, site_for.clone(), sink.clone()));
        ungated.set_frame(0., 0., 800., 600.);
        ungated.load(&Url::parse(&format!("http://127.0.0.1:{port}/page")).unwrap());
        spin(5.0, &|| ungated.title().contains("|main:"));
        assert_eq!(ungated.title(), "PAGE|main:undefinedundefinedundefinedundefinedundefined", "a view of the profile without the gate runs nothing");

        let slow = Url::parse(&format!("http://127.0.0.1:{port}/slow")).unwrap();
        let peek = Rc::new(WebView::new(&host, &data, ShieldLevel::Standard, true, false, site_for.clone(), sink.clone()));
        peek.set_frame(0., 0., 800., 600.);
        peek.load(&slow);
        spin(0.3, &|| false);
        let tab = open(slow.as_str());
        spin(8.0, &|| peek.title().contains("|main:") && tab.title().contains("|main:"));
        assert_eq!(
            peek.title(),
            "PAGE|main:undefinedundefinedundefinedundefinedundefined",
            "a view without the gate runs nothing while a tab loads the same page"
        );
        drop(peek);
        tab.reload_from_origin();
        spin(8.0, &|| tab.title().starts_with("EXT:") && tab.title().contains("|main:"));
        assert!(tab.title().starts_with("EXT:PAGE"), "the page runs in the tab once that view is gone: {}", tab.title());

        let fixture_dir = dir.path().to_owned();
        let resources = Arc::new(move |url: &Url| {
            let name = url.path().trim_start_matches('/');
            match std::fs::read(fixture_dir.join(name)) {
                Ok(body) => Resource {
                    status: 200,
                    mime: if name.ends_with(".js") { "text/javascript" } else { "text/html" },
                    csp: Some("script-src 'self'; object-src 'none'"),
                    body,
                },
                Err(_) => Resource { status: 404, mime: "text/plain", csp: None, body: Vec::new() },
            }
        });
        let page_messages: Rc<RefCell<Vec<(World, FrameId, String)>>> = Rc::default();
        let page_sink = {
            let page_messages = page_messages.clone();
            Rc::new(move |e: Event| {
                if let Event::ExtensionMessage { world, frame, json, .. } = e {
                    page_messages.borrow_mut().push((world, frame, json));
                }
            })
        };
        let spec = PageSpec { world: world.clone(), origin: origin.into(), bootstrap: bootstrap(&config(true)), resources, inspectable: false };
        let page = ExtensionPage::new(&host, &data, ShieldLevel::Standard, spec, page_sink, Rc::new(|| {})).unwrap();
        page.load(&Url::parse(&format!("tarn-extension://{origin}/bg.html")).unwrap());
        spin(8.0, &|| page_messages.borrow().iter().any(|(_, _, j)| j.contains("from-background")));
        let got = page_messages.borrow().clone();
        assert!(
            got.iter().any(|(w, f, j)| *w == world && *f == FrameId::MAIN && j.contains("from-background")),
            "the background page talks through the bridge: {got:?}"
        );

        drop(page);
        drop(views.borrow_mut().drain(..));
        drop((granted, other, ungated, tab));
        DataStore::wipe_persistent(uuid);
        FilterLists::remove("tarn-ext.test");
        window.close();
        println!("tarn_extension: ok");
    }
}

fn main() {
    if !std::env::args().any(|a| a == "--ignored" || a == "--include-ignored") {
        println!("tarn_extension: skipped (opens a window; pass --ignored)");
    } else if cfg!(target_os = "macos") {
        #[cfg(target_os = "macos")]
        window::run();
    }
}
