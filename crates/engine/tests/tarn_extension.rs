//! A Tarn extension in a real WebKit view: its content script runs only in its own world and only
//! where the gate lets it, the page's main world never sees the bridge or loads the extension's
//! files, messages go both ways with the sender's frame and origin from WebKit, in same-site and
//! cross-site frames, nothing runs in a Lockdown view, its network rule blocks, and its own pages
//! load from `tarn-extension://` and frame nothing but their origin. It opens a window, so it runs
//! only when asked: `cargo test -p engine --test tarn_extension -- --ignored`.

#[cfg(target_os = "macos")]
mod common;

#[cfg(target_os = "macos")]
mod window {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use engine::nav_policy::Site;
    use engine::tarn_ext::{ExtensionView, FrameId, PageSpec, Resource, RunAt, Script, World, WorldScripts, bootstrap};
    use engine::{DataStore, Event, ExtensionPage, FilterLists, Host, WebView};
    use objc2::MainThreadMarker;
    use serde_json::{Value, json};
    use shield::ShieldLevel;
    use url::Url;

    use super::common::{INSTALL, ORIGIN, View, fixture, serve, spin, window};

    const BLOCKED_BY_CONTENT_RULE: isize = 104;

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("main thread");
        let window = window(mtm);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let port = serve(seen.clone());
        let dir = fixture();
        let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();

        let world = World::for_install(INSTALL).unwrap();
        let origin = ORIGIN;
        let config = |page: bool| json!({"id": origin, "base": format!("tarn-extension://{origin}/"), "manifest": "{}", "messages": {}, "locale": "en", "storage": true, "page": page});
        let scripts = Rc::new(WorldScripts {
            world: world.clone(),
            bootstrap: bootstrap(&config(false)),
            scripts: vec![
                Script { run_at: RunAt::DocumentStart, all_frames: false, js: vec![read("content.js")], css: vec![read("content.css")] },
                Script { run_at: RunAt::DocumentEnd, all_frames: false, js: vec![read("late.js")], css: vec![] },
                Script { run_at: RunAt::DocumentEnd, all_frames: true, js: vec![read("frame.js")], css: vec![] },
            ],
        });

        let uuid = "6A1B2C3D-0000-4000-8000-00000000E715";
        let data = DataStore::persistent(uuid, uuid, Rc::new(|_| {})).unwrap();
        let rules = format!(
            r#"[{{"trigger":{{"url-filter":"^http://127\\.0\\.0\\.1:{port}/blocked\\.js"}},"action":{{"type":"block"}}}},{{"trigger":{{"url-filter":"^https://extensions\\.tarnbrowser\\.com/"}},"action":{{"type":"block"}}}}]"#
        );
        let compiled: Rc<RefCell<Option<FilterLists>>> = Rc::default();
        let slot = compiled.clone();
        FilterLists::compile("tarn-ext.test", "1", vec![rules], move |r| *slot.borrow_mut() = Some(r.expect("rules compile")));
        spin(10.0, &|| compiled.borrow().is_some());
        data.set_extension_rule_lists("tarn-ext.test", compiled.borrow().as_ref());

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
        let site_for: engine::SiteFor = Rc::new(|_| Site::new(ShieldLevel::Standard));
        let open_at = |url: &str, level: ShieldLevel| {
            let view = Rc::new(WebView::new(&host, &data, level, true, false, site_for.clone(), sink.clone()));
            view.set_frame(0., 0., 800., 600.);
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
            views.borrow_mut().push(view.clone());
            view.load(&Url::parse(url).unwrap());
            view
        };
        let open = |url: &str| open_at(url, ShieldLevel::Standard);

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
        let page_origin = Some(format!("http://127.0.0.1:{port}"));
        assert!(
            got.iter().all(|(w, f, o, _)| *w == world && *f == FrameId::MAIN && o.as_ref().map(|u| u.origin().ascii_serialization()) == page_origin),
            "messages name the world, frame and origin they came from: {got:?}"
        );
        assert!(got.iter().any(|(_, _, _, m)| m["call"] == "runtime.sendMessage" && m["args"][0]["ping"] == 1), "{got:?}");
        assert!(got.iter().any(|(_, _, _, m)| m["call"] == "storage.set" && m["args"][0]["seen"] == true), "{got:?}");
        let requested = |entry: &str| seen.lock().unwrap().iter().filter(|s| *s == entry).count();
        assert_eq!(requested(&format!("127.0.0.1:{port} /blocked.js")), 0, "the rule blocks the granted page's script");
        assert_eq!(requested(&format!("localhost:{port} /blocked.js")), 1, "and not the other's");

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

        let probe = open(&format!("http://127.0.0.1:{port}/probe?o={origin}"));
        spin(10.0, &|| probe.title().contains("|s1:") && probe.title().contains("|reply:"));
        let title = probe.title();
        assert!(title.starts_with("EXT:PROBE"), "the extension runs in the probed page: {title}");
        assert!(title.contains("|main:undefined,undefined,,undefined"), "no chrome.runtime, browser, binding or handler in the main world: {title}");
        assert!(title.contains("|s1:refused,error,about:blank"), "a web page cannot fetch, show or frame the extension's files: {title}");

        messages.borrow_mut().clear();
        let locked = open_at(&format!("http://127.0.0.1:{port}/page?locked"), ShieldLevel::Lockdown);
        spin(3.0, &|| false);
        assert_eq!(requested(&format!("127.0.0.1:{port} /page?locked")), 0, "a Lockdown view never loads a private-network page");
        assert!(!locked.title().starts_with("EXT:") && messages.borrow().is_empty(), "nothing runs in a Lockdown view: {}", locked.title());

        let frames_seen = |label: &str| {
            let here = |host: &str, path: &str| format!("http://{host}:{port}{path}");
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
            let (main, same, cross) = (from(&here("127.0.0.1", "/frames")), from(&sub("127.0.0.1", "ok")), from(&sub("localhost", "ok")));
            assert_eq!(main, vec![(FrameId::MAIN, Some(here("127.0.0.1", "")))], "{label}: the top document is the main frame, with its origin: {got:?}");
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
        let frames = open(&format!("http://127.0.0.1:{port}/frames"));
        frames_seen("first load");
        messages.borrow_mut().clear();
        frames.reload_from_origin();
        frames_seen("reload");

        let probes = [World::for_install("0a1b2e").unwrap(), World::for_install("0a1b2f").unwrap()];
        let idle = Script { run_at: RunAt::DocumentEnd, all_frames: false, js: vec![], css: vec![] };
        let source = "const say = step => chrome.runtime.sendMessage({ step, at: location.pathname + location.search });\n\
                     say(1).then(() => { location.hash = 'moved'; setTimeout(() => say(2).then(() => { const third = say(3); location.href = '/nocontent'; third.then(() => say(4)); }), 300); });";
        messages.borrow_mut().clear();
        for probe in &probes {
            frames.add_extension_scripts(Rc::new(WorldScripts {
                world: probe.clone(),
                bootstrap: bootstrap(&config(false)),
                scripts: vec![idle.clone(), idle.clone(), Script { run_at: RunAt::DocumentStart, all_frames: true, js: vec![source.into()], css: vec![] }],
            }));
        }
        frames.reload_from_origin();
        let steps = |probe: &World| {
            messages
                .borrow()
                .iter()
                .filter(|(w, _, o, m)| w == probe && m["args"][0]["at"] == "/sub?ok" && o.as_ref().and_then(Url::host_str) == Some("127.0.0.1"))
                .map(|(_, f, _, m)| (m["args"][0]["step"].as_u64(), *f))
                .collect::<Vec<_>>()
        };
        spin(15.0, &|| probes.iter().all(|p| steps(p).len() == 4));
        for probe in &probes {
            let got = steps(probe);
            assert!(
                got.len() == 4 && got[0].1 != FrameId::MAIN && got.iter().all(|(_, f)| *f == got[0].1),
                "{probe:?}: with another install in the frame, a frame keeps its id and gets its replies through a fragment navigation and while a navigation that loads no new document is pending: {:?}",
                messages.borrow()
            );
        }
        assert!(requested(&format!("127.0.0.1:{port} /nocontent")) >= 1, "the frames tried to leave");

        let moving = World::for_install("0a1b3a").unwrap();
        let source = "const say = m => chrome.runtime.sendMessage(Object.assign({ at: location.pathname + location.search }, m));\n\
                     chrome.runtime.onMessage.addListener(m => { say({ got: m }); });\n\
                     if (location.search === '?stale') say({ step: 'old' }).then(() => { location.search = '?fresh'; }); else say({ step: 'new' });";
        let twice = open(&format!("http://127.0.0.1:{port}/twice"));
        twice.add_extension_scripts(Rc::new(WorldScripts {
            world: moving.clone(),
            bootstrap: bootstrap(&config(false)),
            scripts: vec![idle.clone(), idle.clone(), Script { run_at: RunAt::DocumentEnd, all_frames: true, js: vec![source.into()], css: vec![] }],
        }));
        messages.borrow_mut().clear();
        twice.reload_from_origin();
        let hosts = ["127.0.0.1", "localhost"];
        let from = |host: &str, at: &str, key: &str| -> Vec<(FrameId, Value)> {
            messages
                .borrow()
                .iter()
                .filter(|(w, _, o, m)| *w == moving && m["args"][0]["at"] == at && o.as_ref().and_then(Url::host_str) == Some(host))
                .filter_map(|(_, f, _, m)| m["args"][0].get(key).map(|v| (*f, v.clone())))
                .collect()
        };
        spin(15.0, &|| hosts.iter().all(|h| !from(h, "/sub?fresh", "step").is_empty() && !from(h, "/sub?stale", "step").is_empty()));
        let event = |message: &str| json!({"event": "runtime.onMessage", "delivery": 1, "message": message, "sender": {}}).to_string();
        for host in hosts {
            let (old, new) = (from(host, "/sub?stale", "step"), from(host, "/sub?fresh", "step"));
            assert!(old.len() == 1 && new.len() == 1 && old[0].0 != new[0].0, "{host}: the frame moved to a new document: {:?}", messages.borrow());
            twice.post_extension_message(&moving, Some(old[0].0), &event("stale"));
            twice.post_extension_message(&moving, Some(new[0].0), &event("fresh"));
        }
        spin(5.0, &|| hosts.iter().all(|h| !from(h, "/sub?fresh", "got").is_empty()));
        spin(1.0, &|| false);
        for host in hosts {
            let got: Vec<Value> = from(host, "/sub?fresh", "got").into_iter().map(|(_, v)| v).collect();
            assert_eq!(got, [json!("fresh")], "{host}: the new document gets only what is sent to it, nothing sent to the old one");
        }

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
        page.load(&Url::parse(&format!("tarn-extension://{origin}/framer.html?port={port}")).unwrap());
        spin(2.0, &|| false);
        let reported = || page_messages.borrow().iter().any(|(_, _, j)| j.contains("framed:"));
        for _ in 0..8 {
            page.post(&json!({"event": "runtime.onMessage", "delivery": 1, "message": "report", "sender": {}}).to_string());
            spin(1.0, &reported);
            if reported() {
                break;
            }
        }
        let framed = page_messages.borrow().iter().find(|(_, _, j)| j.contains("framed:")).map(|(_, _, j)| j.clone());
        assert!(
            framed.as_deref().is_some_and(|j| j.contains(&format!("framed:tarn-extension://{origin}/inner.html,about:blank"))),
            "an extension page frames its own pages, never a web page: {framed:?}"
        );
        assert_eq!(requested(&format!("127.0.0.1:{port} /sub?from-extension")), 0, "the web frame was never requested");

        drop(page);
        drop(views.borrow_mut().drain(..));
        drop((granted, other, ungated, tab, probe, locked, frames, twice));
        let asked: Rc<RefCell<Vec<String>>> = Rc::default();
        let failed: Rc<RefCell<Vec<isize>>> = Rc::default();
        let store_sink = {
            let (asked, failed) = (asked.clone(), failed.clone());
            Rc::new(move |e: Event| match e {
                Event::StoreInstall(id) => asked.borrow_mut().push(id),
                Event::Failed { code, .. } => failed.borrow_mut().push(code),
                _ => {}
            })
        };
        let link = "https://extensions.tarnbrowser.com/install/helper";
        let store = WebView::new(&host, &data, ShieldLevel::Standard, true, false, site_for.clone(), store_sink);
        store.set_frame(0., 0., 800., 600.);
        for page in ["store?click", "store?assign", "store-redirect"] {
            failed.borrow_mut().clear();
            store.load(&Url::parse(&format!("http://127.0.0.1:{port}/{page}")).unwrap());
            let reached = || failed.borrow().contains(&BLOCKED_BY_CONTENT_RULE);
            spin(10.0, &reached);
            assert!(reached(), "{page}: the navigation to the store link was let through (and blocked by the test's rule): {:?}", failed.borrow());
            if page != "store-redirect" {
                assert_eq!(store.title(), "RAN", "{page}: the page's script ran");
            } else {
                assert_eq!(requested(&format!("127.0.0.1:{port} /store-redirect")), 1, "the redirect was served");
            }
            assert!(asked.borrow().is_empty(), "{page}: a script, a frame or a redirect never asks to install: {:?}", asked.borrow());
        }
        store.load(&Url::parse(&format!("{link}/")).unwrap());
        spin(3.0, &|| !asked.borrow().is_empty());
        assert_eq!(*asked.borrow(), ["helper"], "the user's own load asks");
        drop(store);
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
