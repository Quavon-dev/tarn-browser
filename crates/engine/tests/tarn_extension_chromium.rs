//! A Tarn extension in real Chromium views, over the in-process DevTools channel only: its
//! content script runs only in its own world and only where the gate lets it, the page's main
//! world sees no bridge and cannot load the extension's files, messages go both ways with the
//! sender's frame and origin taken from DevTools, in same-site and cross-site frames and after
//! reloads and navigations, and its network rule blocks only where granted.
//!
//! CEF starts only from an app bundle with its framework and helpers, so the test runs itself
//! again from a temporary bundle that holds a copy of a signed bundle's `Frameworks`:
//! `TARN_PROFILE=debug TARN_LOCAL_CHROMIUM=1 TARN_SIGN_IDENTITY=<identity> scripts/bundle.sh`, then
//! `cargo test -p engine --features chromium --test tarn_extension_chromium -- --ignored`
//! (`TARN_CHROMIUM_BUNDLE` names another bundle than `target/debug/Tarn.app`).

#[cfg(target_os = "macos")]
mod common;

#[cfg(target_os = "macos")]
mod window {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use engine::tarn_ext::{ExtensionView, FrameId, RunAt, Script, World, WorldScripts, bootstrap};
    use engine::{DataStore, Event, Host, WebView};
    use objc2::rc::Retained;
    use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_app_kit::{NSApplication, NSResponder};
    use objc2_foundation::NSObject;
    use serde_json::{Value, json};
    use url::Url;

    use super::common::{INSTALL, ORIGIN, View, fixture, serve, spin, window};

    define_class!(
        #[unsafe(super(NSApplication, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "GPUIApplication"]
        struct TestApplication;
    );

    type Heard = (World, FrameId, Option<Url>, Value);

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("main thread");
        // SAFETY: the shared application is made once, of the class CEF's protocols are added to.
        let _app: Retained<NSApplication> = unsafe { msg_send![TestApplication::class(), sharedApplication] };
        let window = window(mtm);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let port = serve(seen.clone());
        let dir = fixture();
        let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();
        let here = |host: &str, path: &str| format!("http://{host}:{port}{path}");
        let requested = |entry: &str| seen.lock().unwrap().iter().filter(|s| *s == entry).count();

        let world = World::for_install(INSTALL).unwrap();
        let config = json!({"id": ORIGIN, "base": format!("tarn-extension://{ORIGIN}/"), "manifest": "{}", "messages": {}, "locale": "en", "storage": true, "page": false});
        let scripts = Rc::new(WorldScripts {
            world: world.clone(),
            bootstrap: bootstrap(&config),
            scripts: vec![
                Script { run_at: RunAt::DocumentStart, all_frames: false, js: vec![read("content.js")], css: vec![read("content.css")] },
                Script { run_at: RunAt::DocumentEnd, all_frames: false, js: vec![read("late.js")], css: vec![] },
                Script { run_at: RunAt::DocumentEnd, all_frames: true, js: vec![read("frame.js")], css: vec![] },
            ],
        });

        let uuid = "6A1B2C3D-0000-4000-8000-00000000C715";
        let data = DataStore::persistent(uuid, uuid, Rc::new(|_| {})).unwrap();
        let root = tempfile::tempdir().unwrap();
        engine::chromium::close_in(3_600_000, i64::MAX);
        engine::chromium::set_shield(engine::request_filter::Shield {
            site_for: Box::new(|url| {
                let level = if url.path() == "/locked" { shield::ShieldLevel::Lockdown } else { shield::ShieldLevel::Standard };
                engine::nav_policy::Site::new(level)
            }),
            matcher: shield::matcher::Matcher::default(),
        });
        let dnr = shield::dnr::compile(&[br#"[{"id":1,"action":{"type":"block"},"condition":{"urlFilter":"blocked.js"}}]"#]).unwrap();
        let granted_rules = dnr.content_rules(&shield::dnr::Scope::Hosts(vec!["127.0.0.1".into()])).unwrap();
        engine::chromium::set_extension_rules(&[granted_rules]);
        let host = Host::from_window(&View(window.contentView().unwrap())).unwrap();

        let messages: Rc<RefCell<Vec<Heard>>> = Rc::default();
        let views: Rc<RefCell<Vec<Rc<WebView>>>> = Rc::default();
        let webkit_asked: Rc<RefCell<Vec<Url>>> = Rc::default();
        let sink = {
            let (messages, views, webkit_asked) = (messages.clone(), views.clone(), webkit_asked.clone());
            Rc::new(move |e: Event| {
                if let Event::WebEngineRequired(url) = &e {
                    webkit_asked.borrow_mut().push(url.clone());
                }
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
        let open = |url: &str, gated: bool| {
            let view = Rc::new(WebView::chromium(&host, &data, root.path(), uuid, false, sink.clone()).expect("Chromium starts in the bundle"));
            view.set_frame(0., 0., 800., 600.);
            if gated {
                view.set_extension_gate(Rc::new(|_, frame| {
                    let (host, query) = (frame.url.host_str(), frame.url.query());
                    let mut run = Vec::new();
                    if frame.main_frame && host == Some("127.0.0.1") {
                        run.extend([0, 1]);
                    }
                    if (host == Some("127.0.0.1") || (host == Some("localhost") && !frame.main_frame)) && query != Some("no") {
                        run.push(2);
                    }
                    run
                }));
                view.add_extension_scripts(scripts.clone());
            }
            views.borrow_mut().push(view.clone());
            view.load(&Url::parse(url).unwrap());
            view
        };

        let other = open(&here("localhost", "/other"), true);
        spin(30.0, &|| other.title().contains("|main:"));
        assert_eq!(other.title(), "PAGE|main:objectundefinedundefinedundefinedundefined", "a host the gate keeps out runs nothing");

        let granted = open(&here("127.0.0.1", "/page"), true);
        spin(8.0, &|| granted.title().contains("|reply:") && granted.title().contains("|main:"));
        let title = granted.title();
        assert!(
            title.starts_with("EXT:PAGE") && title.contains("|start:loading|late:string"),
            "the entry's files share their world, from document start on a granted host: {title}"
        );
        assert!(title.contains("|main:objectundefinedundefinedundefinedundefined"), "the page's main world sees no bridge: {title}");
        assert!(title.contains("|reply:2"), "a reply reaches the content script: {title}");
        {
            let got = messages.borrow();
            assert!(
                got.iter().all(|(w, f, o, _)| *w == world && *f == FrameId::MAIN && o.is_none()),
                "messages name the world and frame they came from: {got:?}"
            );
            assert!(got.iter().any(|(_, _, _, m)| m["call"] == "runtime.sendMessage" && m["args"][0]["ping"] == 1), "{got:?}");
            assert!(got.iter().any(|(_, _, _, m)| m["call"] == "storage.set" && m["args"][0]["seen"] == true), "{got:?}");
        }
        assert_eq!(requested(&format!("127.0.0.1:{port} /blocked.js")), 0, "the rule blocks on a granted site");
        assert_eq!(requested(&format!("localhost:{port} /blocked.js")), 1, "the rule does not act on a site without a grant");

        let ungated = open(&here("127.0.0.1", "/page?ungated"), false);
        spin(8.0, &|| ungated.title().contains("|main:"));
        assert_eq!(ungated.title(), "PAGE|main:objectundefinedundefinedundefinedundefined", "a view without the gate runs nothing");

        let probe = open(&here("127.0.0.1", &format!("/probe?o={ORIGIN}")), true);
        spin(10.0, &|| probe.title().contains("|s1:") && probe.title().contains("|reply:"));
        let title = probe.title();
        assert!(title.starts_with("EXT:PROBE"), "the extension runs in the probed page: {title}");
        assert!(title.contains("|main:undefined,undefined,,undefined"), "no chrome.runtime, browser, binding or handler in the main world: {title}");
        assert!(title.contains("|s1:refused,error,about:blank"), "a web page cannot fetch, show or frame the extension's files: {title}");

        messages.borrow_mut().clear();
        let locked = open(&here("127.0.0.1", "/locked"), true);
        spin(8.0, &|| !webkit_asked.borrow().is_empty());
        spin(1.0, &|| false);
        assert_eq!(webkit_asked.borrow().iter().map(Url::path).collect::<Vec<_>>(), ["/locked"], "a Lockdown site goes to WebKit");
        assert!(messages.borrow().is_empty() && requested(&format!("127.0.0.1:{port} /locked")) == 0, "and nothing of it loads or runs in Chromium");

        let frames_seen = |label: &str| {
            let sub = |host: &str, query: &str| here(host, &format!("/sub?{query}"));
            spin(15.0, &|| {
                let got = messages.borrow();
                [here("127.0.0.1", "/frames"), sub("127.0.0.1", "ok"), sub("localhost", "ok")]
                    .iter()
                    .all(|want| got.iter().any(|(_, _, _, m)| m["args"][0]["replied"].as_str() == Some(want.as_str())))
            });
            let got = messages.borrow().clone();
            let from = |want: &str| -> Vec<(FrameId, Option<String>)> {
                got.iter()
                    .filter(|(w, _, _, m)| *w == world && m["args"][0]["frame"].as_str() == Some(want))
                    .map(|(_, f, o, _)| (*f, o.as_ref().map(|u| u.origin().ascii_serialization())))
                    .collect()
            };
            let main = from(&here("127.0.0.1", "/frames"));
            let same = from(&sub("127.0.0.1", "ok"));
            let cross = from(&sub("localhost", "ok"));
            assert_eq!(main, vec![(FrameId::MAIN, None)], "{label}: the top document is the main frame: {got:?}");
            assert!(same.len() == 1 && same[0].0 != FrameId::MAIN, "{label}: a same-site frame is heard as a subframe whatever it claims: {got:?}");
            assert_eq!(same[0].1.as_deref(), Some(here("127.0.0.1", "").as_str()), "{label}: with its own origin: {got:?}");
            assert!(
                cross.len() == 1 && cross[0].0 != FrameId::MAIN && cross[0].0 != same[0].0,
                "{label}: a cross-site frame is heard as a frame of its own: {got:?}"
            );
            assert_eq!(cross[0].1.as_deref(), Some(here("localhost", "").as_str()), "{label}: with its own origin: {got:?}");
            assert!(from(&sub("127.0.0.1", "no")).is_empty(), "{label}: a frame the gate keeps out sends nothing: {got:?}");
            let replied = |want: &str| {
                got.iter()
                    .filter(|(_, _, _, m)| m["args"][0]["replied"].as_str() == Some(want) && m["args"][0]["pong"] == 2)
                    .map(|(_, f, _, _)| *f)
                    .collect::<Vec<_>>()
            };
            assert_eq!(replied(&sub("127.0.0.1", "ok")), vec![same[0].0], "{label}: a reply reaches the same-site frame: {got:?}");
            assert_eq!(replied(&sub("localhost", "ok")), vec![cross[0].0], "{label}: a reply reaches the cross-site frame: {got:?}");
        };

        messages.borrow_mut().clear();
        let frames = open(&here("127.0.0.1", "/frames"), true);
        frames_seen("first load");

        messages.borrow_mut().clear();
        frames.reload();
        frames_seen("reload");

        messages.borrow_mut().clear();
        frames.load(&Url::parse(&here("127.0.0.1", "/page")).unwrap());
        spin(8.0, &|| frames.title().contains("|reply:"));
        frames.load(&Url::parse(&here("127.0.0.1", "/frames")).unwrap());
        frames_seen("navigation");

        messages.borrow_mut().clear();
        let late = open(&here("127.0.0.1", "/late"), true);
        let heard = |w: &World, url: &str| {
            messages
                .borrow()
                .iter()
                .filter(|(from, _, _, m)| from == w && m["args"][0]["frame"].as_str() == Some(url))
                .map(|(_, f, o, _)| (*f, o.clone()))
                .collect::<Vec<_>>()
        };
        spin(10.0, &|| !heard(&world, &here("localhost", "/sub?late")).is_empty());
        assert!(!heard(&world, &here("localhost", "/sub?late")).is_empty(), "the cross-site frame is attached and runs the first install");
        let second = World::for_install("0a1b2d").unwrap();
        late.add_extension_scripts(Rc::new(WorldScripts { world: second.clone(), ..(*scripts).clone() }));
        let again = here("localhost", "/sub?ok&again");
        spin(15.0, &|| !heard(&second, &again).is_empty());
        let got = heard(&second, &again);
        assert!(
            got.len() == 1 && got[0].0 != FrameId::MAIN && got[0].1.as_ref().map(|u| u.origin().ascii_serialization()) == Some(here("localhost", "")),
            "an install added after a cross-site frame attached is heard from its next document: {:?}",
            messages.borrow()
        );

        drop(views.borrow_mut().drain(..));
        drop((other, granted, ungated, probe, locked, frames, late));
        spin(1.0, &|| false);
        engine::chromium::shutdown();
        DataStore::wipe_persistent(uuid);
        window.close();
        println!("tarn_extension_chromium: ok");
    }
}

/// Runs the test from a temporary bundle `tarn` beside a copy of the signed bundle's frameworks,
/// unless this process already is one.
#[cfg(target_os = "macos")]
fn in_bundle() -> bool {
    let exe = std::env::current_exe().unwrap();
    let contents = exe.parent().and_then(|p| p.parent()).unwrap();
    if exe.file_name().is_some_and(|n| n == "tarn") && contents.join("Frameworks").is_dir() {
        return true;
    }
    let bundle = std::env::var_os("TARN_CHROMIUM_BUNDLE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/Tarn.app"));
    let frameworks = bundle.join("Contents/Frameworks");
    assert!(frameworks.is_dir(), "no signed bundle at {}: build one with scripts/bundle.sh (see this test's header)", bundle.display());
    let temp = tempfile::tempdir().unwrap();
    let contents = temp.path().join("Test.app/Contents");
    std::fs::create_dir_all(contents.join("MacOS")).unwrap();
    std::fs::copy(bundle.join("Contents/Info.plist"), contents.join("Info.plist")).unwrap();
    std::fs::copy(&exe, contents.join("MacOS/tarn")).unwrap();
    let cloned = std::process::Command::new("cp").arg("-Rc").arg(&frameworks).arg(&contents).status().unwrap();
    assert!(cloned.success(), "copying {} failed", frameworks.display());
    let status = std::process::Command::new(contents.join("MacOS/tarn")).args(std::env::args_os().skip(1)).status().unwrap();
    assert!(status.success(), "the test failed in its bundle: {status}");
    false
}

fn main() {
    if !std::env::args().any(|a| a == "--ignored" || a == "--include-ignored") {
        println!("tarn_extension_chromium: skipped (opens a window and needs a signed bundle; pass --ignored)");
        return;
    }
    #[cfg(target_os = "macos")]
    {
        engine::chromium::register_early_malloc_zone();
        if in_bundle() {
            window::run();
        }
    }
}
