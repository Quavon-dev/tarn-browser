//! Find in a real WebKit view: every match counts, the reported match is the one the page
//! selected, stepping wraps both ways, a find after `clear_find` starts again, and a Lockdown page,
//! where no script of Tarn's runs, still counts. It opens a window, so it runs only when asked:
//! `cargo test -p engine --test find -- --ignored`.

#[cfg(target_os = "macos")]
mod window {
    use std::cell::{Cell, RefCell};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::ptr::NonNull;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use engine::nav_policy::Site;
    use engine::{DataStore, Event, Host, WebView};
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSView, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
    use raw_window_handle::{AppKitWindowHandle, HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};
    use shield::ShieldLevel;
    use url::Url;

    /// A find's match count and current match.
    type Answer = (usize, Option<usize>);

    struct View(Retained<NSView>);

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

    fn window(mtm: MainThreadMarker) -> Retained<NSWindow> {
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        // SAFETY: main-thread creation of a fresh window, which this binding owns.
        let window = unsafe {
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(100., 100.), NSSize::new(800., 600.)),
                NSWindowStyleMask::Titled,
                NSBackingStoreType::Buffered,
                false,
            );
            window.setReleasedWhenClosed(false);
            window
        };
        window.makeKeyAndOrderFront(None);
        window
    }

    /// Five paragraphs with one match each, and a title naming the paragraph the selection is in.
    const PAGE: &str = "<title>-</title>\
        <p>one fox</p><p>two fox</p><p>three fox</p><p>four fox</p><p>five fox</p>\
        <script>document.addEventListener('selectionchange', () => {\
          const p = getSelection().anchorNode && getSelection().anchorNode.parentElement.closest('p');\
          document.title = p ? String([...document.querySelectorAll('p')].indexOf(p) + 1) : '-';\
        });</script>";

    fn serve() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut buf = [0; 4096];
                if stream.read(&mut buf).unwrap_or(0) == 0 {
                    continue;
                }
                let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", PAGE.len());
                stream.write_all(head.as_bytes()).ok();
                stream.write_all(PAGE.as_bytes()).ok();
            }
        });
        port
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("main thread");
        let window = window(mtm);
        let host = Host::from_window(&View(window.contentView().unwrap())).unwrap();
        let data = DataStore::ephemeral(Rc::new(|_| {})).unwrap();
        let url = Url::parse(&format!("http://127.0.0.1:{}/", serve())).unwrap();
        let title = Rc::new(RefCell::new(String::new()));
        let loaded = Rc::new(Cell::new(false));
        let sink = {
            let (title, loaded) = (title.clone(), loaded.clone());
            Rc::new(move |e: Event| match e {
                Event::TitleChanged(t) => *title.borrow_mut() = t,
                Event::Finished { .. } => loaded.set(true),
                _ => {}
            })
        };
        let open = |level: ShieldLevel| {
            loaded.set(false);
            let site_for: engine::SiteFor = Rc::new(move |_| Site::new(level));
            let view = WebView::new(&host, &data, level, engine::Features { filtered: true, extensions: false }, false, site_for, sink.clone());
            view.set_frame(0., 0., 800., 600.);
            view.load(&url);
            spin(10.0, &|| loaded.get());
            assert!(loaded.get(), "the page loaded");
            view
        };
        let found: Rc<Cell<Option<Answer>>> = Rc::default();
        let step = |view: &WebView, text: &str, backwards: bool| {
            found.set(None);
            let slot = found.clone();
            view.find(text, backwards, move |count, current| slot.set(Some((count, current))));
            spin(5.0, &|| found.get().is_some());
            spin(0.3, &|| false);
            found.get().expect("the find answered")
        };

        let view = open(ShieldLevel::Standard);
        let shown = || title.borrow().clone();
        assert_eq!(step(&view, "fox", false), (5, Some(1)));
        assert_eq!(shown(), "1", "the first match is selected");
        assert_eq!(step(&view, "fox", false), (5, Some(2)));
        assert_eq!(shown(), "2");
        assert_eq!(step(&view, "fox", true), (5, Some(1)), "back one");
        assert_eq!(shown(), "1");
        assert_eq!(step(&view, "fox", true), (5, Some(5)), "backwards wraps to the last match");
        assert_eq!(shown(), "5");
        assert_eq!(step(&view, "fox", false), (5, Some(1)), "forwards wraps to the first match");
        assert_eq!(step(&view, "FOX", false).0, 5, "without case");
        assert_eq!(step(&view, "zebra", false), (0, None));

        assert_eq!(step(&view, "fox", false), (5, Some(1)));
        step(&view, "fox", false);
        view.clear_find();
        assert_eq!(step(&view, "fox", false), (5, Some(1)), "after clear_find a find starts again at the first match");
        assert_eq!(shown(), "1", "and selects it");
        step(&view, "fox", false);
        step(&view, "fox", false);
        view.find("fox", false, |_, _| {});
        view.clear_find();
        assert_eq!(step(&view, "fox", false), (5, Some(1)), "a restart cleared away does not search");
        assert_eq!(shown(), "1");

        let locked = open(ShieldLevel::Lockdown);
        assert_eq!(step(&locked, "fox", false).0, 5, "a Lockdown page counts every match");
        drop((view, locked));
        window.close();
        println!("find: ok");
    }
}

fn main() {
    if !std::env::args().any(|a| a == "--ignored" || a == "--include-ignored") {
        println!("find: skipped (opens a window; pass --ignored)");
    } else if cfg!(target_os = "macos") {
        #[cfg(target_os = "macos")]
        window::run();
    }
}
