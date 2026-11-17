use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::extensions::Note;
use crate::extensions::tests::{Fixture, fixture};
use store::Id;

const PAGE: &str = "<!doctype html><html><body><div id=\"marker\">m</div><script src=\"/blocked.js\"></script><script src=\"/ok.js\"></script></body></html>";
const FRAME: &str = "<!doctype html><body>frame<script>setInterval(() => { const d = document.documentElement.dataset; \
    parent.postMessage([d.tarnFrame, d.tarnFrameReply].join('|'), '*'); }, 100);</script></body>";
const WORLD: &str = "tarn-ext-fixture01";
const UNSENT: &str = "localhost|error: The message could not be sent.";

fn spin_until(done: impl Fn() -> bool, secs: u64) -> bool {
    let context = glib::MainContext::default();
    let end = Instant::now() + Duration::from_secs(secs);
    while !done() && Instant::now() < end {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

struct Server {
    url: String,
    hits: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn hit(&self, path: &str) -> bool {
        self.hits.lock().unwrap().iter().any(|p| p == path)
    }
}

fn server() -> Server {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(Mutex::new(Vec::new()));
    let seen = hits.clone();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0; 2048];
            let n = stream.read(&mut request).unwrap_or(0);
            let line = String::from_utf8_lossy(&request[..n]).lines().next().unwrap_or_default().to_owned();
            let path = line.split(' ').nth(1).unwrap_or("/").to_owned();
            seen.lock().unwrap().push(path.clone());
            let (kind, body) = match path.as_str() {
                "/blocked.js" => ("text/javascript", "window.blockedRan = 1;".to_owned()),
                "/ok.js" => ("text/javascript", "window.okRan = 1;".to_owned()),
                "/framed" => (
                    "text/html",
                    format!(
                        "<!doctype html><body><script>window.report = 'none'; addEventListener('message', e => {{ window.report = String(e.data); }});</script>\
                         <iframe src=\"http://localhost:{port}/frame\"></iframe></body>"
                    ),
                ),
                "/frame" => ("text/html", FRAME.to_owned()),
                _ => ("text/html", PAGE.to_owned()),
            };
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body.as_bytes()));
        }
    });
    Server { url: format!("http://127.0.0.1:{port}/"), hits }
}

fn eval(web: &WebView, script: &str, world: Option<&str>) -> String {
    let out = Rc::new(RefCell::new(None));
    let slot = out.clone();
    web.call_async_javascript_function(script, None, world, None, None::<&webkit6::gio::Cancellable>, move |r| {
        *slot.borrow_mut() = Some(r.map(|v| v.to_str().to_string()).unwrap_or_else(|e| format!("error {e}")));
    });
    assert!(spin_until(|| out.borrow().is_some(), 20), "the page answered {script}");
    out.take().unwrap_or_default()
}

fn forward(state: &State, f: &Fixture) {
    let messages: Vec<JsonValue> = {
        let mut events = state.events.borrow_mut();
        let (ext, rest): (Vec<_>, Vec<_>) = events.drain(..).partition(|(n, _)| n == "extensionMessage");
        *events = rest;
        ext.into_iter().map(|(_, d)| d).collect()
    };
    for m in messages {
        crate::extensions::message(f.profile, f.tab, m["world"].as_str().unwrap(), m["json"].as_str().unwrap());
    }
}

/// Runs the shell's side of the bridge on this thread too, as the single-thread tests need.
fn bridge() {
    use futures_util::StreamExt;
    let mut events = crate::extensions::connect();
    glib::MainContext::default().spawn_local(async move {
        while let Some(event) = events.next().await {
            crate::extensions::on_event(event);
        }
    });
}

fn tab_view(f: &Fixture, profile: &str, level: ShieldLevel) -> Rc<State> {
    let state = Rc::new(State { container: Some(gtk::Box::new(gtk::Orientation::Vertical, 0)), ..State::default() });
    state.level.set(level);
    *state.levels.borrow_mut() = Levels::parse(&json!({}), level);
    *state.profile.borrow_mut() = profile.to_owned();
    *state.tab.borrow_mut() = format!("false-{}", f.tab);
    VIEWS.with_borrow_mut(|views| views.push(Rc::downgrade(&state)));
    let wanted = Build { key: StoreKey::parse(profile).unwrap(), level, lists: Vec::new(), inspectable: false };
    *state.built.borrow_mut() = Some(wanted.clone());
    build(&state, &wanted, &[]);
    state
}

fn persistent_view(f: &Fixture) -> Rc<State> {
    tab_view(f, &f.core.disk.profile(f.profile).unwrap().data_store_uuid, ShieldLevel::Standard)
}

fn load(state: &State, url: &str, done: &[&str]) -> WebView {
    let web = state.web().unwrap();
    state.events.borrow_mut().clear();
    *state.shell_url.borrow_mut() = Some(url.to_owned());
    web.load_uri(url);
    assert!(spin_until(|| state.events.borrow().iter().any(|(n, _)| done.contains(&n.as_str())), 30), "{url} loaded: {:?}", state.events.borrow());
    web
}

fn open(state: &State, url: &str) -> WebView {
    load(state, url, &["finished"])
}

fn data(web: &WebView, name: &str) -> String {
    eval(web, &format!("return String(document.documentElement.dataset.{name});"), None)
}

fn grant(f: &Fixture, host: &str, allow: bool, remember: bool) {
    crate::extensions::answer(f.profile, "fixture01", host, allow, remember, f.tab);
}

fn send_late(state: &State, f: &Fixture, web: &WebView) -> String {
    eval(web, "delete document.documentElement.dataset.tarnLate; return 'ok';", None);
    let sent = "chrome.runtime.sendMessage({ hello: 'late' }).then(r => { document.documentElement.dataset.tarnLate = r && r.echo; }); return 'sent';";
    assert_eq!(eval(web, sent, Some(WORLD)), "sent", "the extension's world is still in the document");
    spin_until(
        || {
            forward(state, f);
            data(web, "tarnLate") != "undefined"
        },
        4,
    );
    data(web, "tarnLate")
}

fn shut_out(state: &State, f: &Fixture, url: &str, why: &str) {
    let web = state.web().unwrap();
    assert_eq!(send_late(state, f, &web), "undefined", "{why}: the document's world is no longer heard");
    let web = open(state, url);
    spin_until(|| false, 1);
    assert_eq!(data(&web, "tarnExt"), "undefined", "{why}: nothing runs in the next document");
    assert_eq!(eval(&web, "return String(window.blockedRan);", None), "1", "{why}: nor do the install's rules");
}

/// Runs `test`, then drops the extension runtime and its views and lets WebKit close their pages
/// while the test thread still runs: a page left to the thread's exit deadlocks WebKitGTK there.
fn settled(test: fn()) {
    gtk::init().expect("a display");
    test();
    crate::extensions::tests::shut_down();
    VIEWS.with_borrow_mut(Vec::clear);
    spin_until(|| false, 2);
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_tarn_extension_runs_isolated_messages_stores_and_blocks() {
    settled(a_tarn_extension_runs_isolated_messages_stores_and_blocks_in_a_view);
}

fn a_tarn_extension_runs_isolated_messages_stores_and_blocks_in_a_view() {
    let f = fixture("file");
    bridge();
    grant(&f, "127.0.0.1", true, true);
    let uuid = f.core.disk.profile(f.profile).unwrap().data_store_uuid;
    let state = persistent_view(&f);
    assert!(spin_until(|| state.ext.filters.borrow().len() == 1, 5), "the install's rules are wanted");
    spin_until(|| false, 3);
    let s = server();
    let web = open(&state, &s.url);
    let reply = || {
        forward(&state, &f);
        data(&web, "tarnReply")
    };
    assert!(spin_until(|| reply() == "content via tab", 40), "a message reaches the background and its answer comes back: {}", reply());
    assert_eq!(data(&web, "tarnExt"), "function", "the content script ran with chrome.runtime");
    let badge = crate::extensions::buttons(f.profile, Some(f.tab)).first().map(|b| b.state.badge_text.clone());
    assert_eq!(badge.as_deref(), Some("ok"), "the background's badge reaches the toolbar state");
    assert!(
        spin_until(
            || {
                forward(&state, &f);
                data(&web, "tarnStored") == "/"
            },
            20
        ),
        "storage answers the content script"
    );
    assert_eq!(f.core.disk.extension_storage(f.profile, "fixture01").unwrap(), [("seen".to_owned(), "\"/\"".to_owned())]);
    let handlers = state.ext.handlers.borrow().clone();
    crate::extensions::tests::reload_now(&f);
    spin_until(|| false, 1);
    assert_eq!(*state.ext.handlers.borrow(), handlers, "handing the view the same worlds again keeps its handlers");
    assert_eq!(send_late(&state, &f, &web), "late via tab", "the open page is still heard after its worlds are handed over again");
    let probe = "return [typeof chrome, typeof browser, typeof __tarnExtPost, typeof __tarnExtReceive, \
                 typeof (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers['tarn-ext-fixture01'])].join(',');";
    assert_eq!(eval(&web, probe, None), "undefined,undefined,undefined,undefined,undefined", "the page's own world sees none of it");
    assert_eq!(eval(&web, "return getComputedStyle(document.getElementById('marker')).outlineWidth;", None), "7px", "the content CSS applies");
    assert_eq!(eval(&web, "return String(window.okRan) + ',' + String(window.blockedRan);", None), "1,undefined", "the DNR rule blocked the script");
    let origin = browser_core::tarn_ext::origin_id(&uuid, "fixture01");
    let fetched = eval(&web, &format!("return await fetch('tarn-extension://{origin}/popup.html').then(() => 'loaded', () => 'refused');"), None);
    assert_eq!(fetched, "refused", "a web page cannot load the extension's origin");
    let stranger = WebView::new();
    let status = Rc::new(Cell::new(0));
    let seen = status.clone();
    stranger.connect_load_changed(move |w, e| {
        if e == LoadEvent::Finished {
            seen.set(w.main_resource().and_then(|r| r.response()).map_or(1, |r| r.status_code()));
        }
    });
    stranger.load_uri(&format!("tarn-extension://{origin}/popup.html"));
    assert!(spin_until(|| status.get() != 0, 20));
    assert_eq!(status.get(), 403, "the scheme handler refuses a view that is not the extension's own page");

    let page = crate::extensions::open_popup(f.profile, "fixture01").expect("the popup opens");
    let shown = gtk::Box::new(gtk::Orientation::Vertical, 0);
    ext::show_page(page, &shown);
    assert!(spin_until(|| shown.first_child().is_some(), 20), "the popup opened");
    let popup = shown.first_child().and_then(|c| c.downcast::<WebView>().ok()).unwrap();
    let seen = || eval(&popup, "return document.getElementById('seen') ? document.getElementById('seen').textContent : 'none';", None);
    assert!(spin_until(|| seen() == "seen /", 20), "the popup's own script ran under its CSP and read storage: {}", seen());
    let frames = format!(
        "const add = src => new Promise(done => {{ const f = document.createElement('iframe'); f.onload = () => done(f); f.src = src; \
         document.body.appendChild(f); setTimeout(() => done(f), 3000); }}); \
         const own = await add('popup.html'); const blank = await add('about:blank'); \
         await add('{}from-popup'); await add('tarn-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/popup.html'); \
         return [own.contentDocument && own.contentDocument.getElementById('seen') ? 'own' : 'no own', \
         blank.contentDocument ? 'blank' : 'no blank'].join(',');",
        s.url
    );
    assert_eq!(eval(&popup, &frames, None), "own,blank", "the popup frames its own origin and about:blank");
    spin_until(|| false, 1);
    assert!(!s.hit("/from-popup"), "the popup cannot frame a web document");
    let own = popup.uri().map(|u| u.to_string());
    eval(&popup, &format!("location.href = '{}from-popup-top'; return 'ok';", s.url), None);
    spin_until(|| false, 2);
    assert!(!s.hit("/from-popup-top"), "the popup cannot navigate to a web document");
    assert_eq!(popup.uri().map(|u| u.to_string()), own, "the popup stays on its own page");
    crate::extensions::popup_closed(f.profile, "fixture01", page);
    assert!(spin_until(|| shown.first_child().is_none(), 5), "a closed popup leaves the panel");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn nothing_runs_in_a_private_or_lockdown_view_or_before_a_grant() {
    settled(nothing_runs_in_a_private_or_lockdown_view_or_before_a_grant_in_a_view);
}

fn nothing_runs_in_a_private_or_lockdown_view_or_before_a_grant_in_a_view() {
    let f = fixture("file");
    bridge();
    let s = server();
    let private = tab_view(&f, "private:ext-display", ShieldLevel::Standard);
    let web = open(&private, &s.url);
    spin_until(|| false, 1);
    assert_eq!(data(&web, "tarnExt"), "undefined", "a private view runs no extension");
    assert_eq!(eval(&web, "return String(window.blockedRan);", None), "1", "nor its rules");
    let persistent = persistent_view(&f);
    let web = open(&persistent, &s.url);
    spin_until(|| false, 1);
    assert_eq!(data(&web, "tarnExt"), "undefined", "an ungranted host runs nothing before the user answers");
    assert_eq!(eval(&web, "return String(window.blockedRan);", None), "1", "nor do the install's rules act there");

    grant(&f, "127.0.0.1", true, true);
    let uuid = f.core.disk.profile(f.profile).unwrap().data_store_uuid;
    let lockdown = tab_view(&f, &uuid, ShieldLevel::Lockdown);
    load(&lockdown, &s.url, &["finished", "failed"]);
    let empty = |state: &State, why: &str| {
        assert!(state.ext.store.borrow().is_none(), "{why}: asks no gate");
        assert!(state.ext.worlds.borrow().is_empty() && state.ext.added.borrow().is_empty(), "{why}: no user content");
        assert!(state.ext.handlers.borrow().is_empty() && state.ext.filters.borrow().is_empty(), "{why}: no handler, no rules");
    };
    empty(&lockdown, "a Lockdown view");
    f.core.disk.set_site_override(f.profile, "127.0.0.1", Some(ShieldLevel::Lockdown)).unwrap();
    f.core.profiles.borrow().get(&(false, f.profile)).unwrap().overrides.borrow_mut().insert("127.0.0.1".into(), ShieldLevel::Lockdown);
    let level = f.core.level(false, f.profile, Some(&Url::parse(&s.url).unwrap())).unwrap();
    assert_eq!(level, ShieldLevel::Lockdown, "the host override decides the view's level");
    let overridden = tab_view(&f, &uuid, level);
    load(&overridden, &s.url, &["finished", "failed"]);
    empty(&overridden, "a host set to Lockdown");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_cross_site_frame_runs_all_frames_scripts_only_with_a_grant_for_its_own_host() {
    settled(a_cross_site_frame_runs_all_frames_scripts_only_with_a_grant_for_its_own_host_in_a_view);
}

fn a_cross_site_frame_runs_all_frames_scripts_only_with_a_grant_for_its_own_host_in_a_view() {
    let f = fixture("file");
    bridge();
    grant(&f, "127.0.0.1", true, true);
    let state = persistent_view(&f);
    let s = server();
    let framed = format!("{}framed", s.url);
    let report = |web: &WebView| eval(web, "return String(window.report);", None);
    let web = open(&state, &framed);
    assert!(spin_until(|| report(&web) != "none", 10), "the frame on localhost loaded");
    spin_until(|| false, 2);
    assert_eq!(report(&web), "|", "a grant for the top page's host does not reach a cross-site frame");
    assert_eq!(data(&web, "tarnExt"), "function", "the top page's own script ran");

    grant(&f, "localhost", true, true);
    let web = open(&state, &framed);
    assert!(spin_until(|| report(&web) == UNSENT, 10), "the frame's own grant runs the all_frames script there: {}", report(&web));
    assert!(state.ext.allowed.borrow().keys().any(|(_, ix)| *ix == 1), "the frame's document was allowed by its own address");

    f.core.disk.set_site_override(f.profile, "localhost", Some(ShieldLevel::Lockdown)).unwrap();
    f.core.profiles.borrow().get(&(false, f.profile)).unwrap().overrides.borrow_mut().insert("localhost".into(), ShieldLevel::Lockdown);
    let web = open(&state, &framed);
    assert!(spin_until(|| report(&web) != "none", 10));
    spin_until(|| false, 2);
    assert_eq!(report(&web), "|", "a frame host set to Lockdown runs nothing even when granted");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_once_grant_that_ends_or_a_deny_stops_scripts_messages_and_rules() {
    settled(a_once_grant_that_ends_or_a_deny_stops_scripts_messages_and_rules_in_a_view);
}

fn a_once_grant_that_ends_or_a_deny_stops_scripts_messages_and_rules_in_a_view() {
    let f = fixture("file");
    bridge();
    let state = persistent_view(&f);
    let s = server();
    let ready = |web: &WebView| {
        spin_until(
            || {
                forward(&state, &f);
                data(web, "tarnReply") == "content via tab"
            },
            30,
        )
    };

    grant(&f, "127.0.0.1", true, false);
    let web = open(&state, &s.url);
    assert!(ready(&web), "a Once grant runs the script and answers it");
    assert_eq!(send_late(&state, &f, &web), "late via tab", "the world is heard while the grant lasts");
    assert_eq!(eval(&web, "return String(window.blockedRan);", None), "undefined", "the rules act while the grant lasts");
    crate::extensions::tab_closed(f.tab);
    shut_out(&state, &f, &s.url, "a Once grant whose tab closed");

    grant(&f, "127.0.0.1", true, false);
    let web = open(&state, &s.url);
    assert!(ready(&web), "a new Once grant runs it again");
    grant(&f, "127.0.0.1", false, false);
    shut_out(&state, &f, &s.url, "a Deny for this session");

    grant(&f, "127.0.0.1", true, true);
    let web = open(&state, &s.url);
    assert!(ready(&web), "Always allow runs it again");
    grant(&f, "127.0.0.1", false, true);
    shut_out(&state, &f, &s.url, "a remembered Deny");
    assert_eq!(f.core.disk.extension_hosts(f.profile, "fixture01").unwrap(), [("127.0.0.1".to_owned(), false)]);
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn the_runtime_on_the_shell_thread_drives_views_on_the_gtk_thread() {
    settled(the_runtime_on_the_shell_thread_drives_views_on_the_gtk_thread_in_a_view);
}

type Job = Box<dyn FnOnce() + Send>;

/// The extension runtime on a thread of its own, as on the app's engine thread: it answers the
/// views' questions and runs the jobs the test sends, until the test hangs up.
fn shell_thread(jobs: std::sync::mpsc::Receiver<Job>, ready: std::sync::mpsc::Sender<(Id, u64, String)>) {
    let f = fixture("file");
    grant(&f, "127.0.0.1", true, true);
    let mut events = crate::extensions::connect();
    ready.send((f.profile, f.tab, f.core.disk.profile(f.profile).unwrap().data_store_uuid)).unwrap();
    loop {
        while let Ok(event) = events.try_recv() {
            crate::extensions::on_event(event);
        }
        match jobs.recv_timeout(Duration::from_millis(5)) {
            Ok(job) => job(),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    crate::extensions::tests::shut_down();
    drop(f);
}

fn the_runtime_on_the_shell_thread_drives_views_on_the_gtk_thread_in_a_view() {
    let (jobs, inbox) = std::sync::mpsc::channel::<Job>();
    let (ready_tx, ready) = std::sync::mpsc::channel();
    let shell = std::thread::Builder::new().name("shell".into()).spawn(move || shell_thread(inbox, ready_tx)).unwrap();
    let (profile, tab, uuid) = ready.recv().unwrap();
    let ask = |job: Box<dyn FnOnce() -> JsonValue + Send>| {
        let (tx, rx) = std::sync::mpsc::channel();
        jobs.send(Box::new(move || drop(tx.send(job())))).unwrap();
        let answer = RefCell::new(None);
        let arrived = || {
            if let Ok(a) = rx.try_recv() {
                *answer.borrow_mut() = Some(a);
            }
            answer.borrow().is_some()
        };
        assert!(spin_until(arrived, 10), "the shell thread answered");
        answer.take().unwrap()
    };
    let state = Rc::new(State { container: Some(gtk::Box::new(gtk::Orientation::Vertical, 0)), ..State::default() });
    state.level.set(ShieldLevel::Standard);
    *state.levels.borrow_mut() = Levels::parse(&json!({}), ShieldLevel::Standard);
    *state.profile.borrow_mut() = uuid.clone();
    *state.tab.borrow_mut() = format!("false-{tab}");
    VIEWS.with_borrow_mut(|views| views.push(Rc::downgrade(&state)));
    let wanted = Build { key: StoreKey::parse(&uuid).unwrap(), level: ShieldLevel::Standard, lists: Vec::new(), inspectable: false };
    *state.built.borrow_mut() = Some(wanted.clone());
    build(&state, &wanted, &[]);
    assert!(spin_until(|| state.ext.filters.borrow().len() == 1, 10), "the shell handed the view its worlds and the granted rules");
    let s = server();
    let web = open(&state, &s.url);
    let forward = || {
        let messages: Vec<JsonValue> = {
            let mut events = state.events.borrow_mut();
            let (ext, rest): (Vec<_>, Vec<_>) = events.drain(..).partition(|(n, _)| n == "extensionMessage");
            *events = rest;
            ext.into_iter().map(|(_, d)| d).collect()
        };
        for m in messages {
            let (world, json) = (m["world"].as_str().unwrap().to_owned(), m["json"].as_str().unwrap().to_owned());
            jobs.send(Box::new(move || crate::extensions::message(profile, tab, &world, &json))).unwrap();
        }
    };
    let reply = || {
        forward();
        data(&web, "tarnReply")
    };
    assert!(spin_until(|| reply() == "content via tab", 40), "the background answers the content script across threads: {}", reply());
    assert_eq!(data(&web, "tarnExt"), "function", "the content script ran on the granted page");
    assert_eq!(eval(&web, "return String(window.okRan) + ',' + String(window.blockedRan);", None), "1,undefined", "the granted rules block");
    assert!(
        spin_until(
            || {
                forward();
                data(&web, "tarnStored") == "/"
            },
            20
        ),
        "storage answers the content script"
    );
    let badge = ask(Box::new(move || json!(crate::extensions::buttons(profile, Some(tab)).first().map(|b| b.state.badge_text.clone()))));
    assert_eq!(badge, json!("ok"), "the background's badge reaches the toolbar state");

    ask(Box::new(|| json!(crate::extensions::tests::asks().len())));
    let other = s.url.replace("127.0.0.1", "localhost");
    let web = open(&state, &other);
    spin_until(|| false, 1);
    assert_eq!(data(&web, "tarnExt"), "undefined", "an ungranted host runs nothing");
    let localhost = Note::Ask { profile, tab, install: "fixture01".into(), host: "localhost".into() };
    let asked = ask(Box::new(move || json!(crate::extensions::tests::asks().contains(&localhost))));
    assert_eq!(asked, json!(true), "the site prompt is raised for the ungranted host");

    let page = ask(Box::new(move || json!(crate::extensions::open_popup(profile, "fixture01")))).as_u64().expect("the popup opens");
    let shown = gtk::Box::new(gtk::Orientation::Vertical, 0);
    ext::show_page(page, &shown);
    assert!(spin_until(|| shown.first_child().is_some(), 20), "the popup is made on the GTK thread");
    let popup = shown.first_child().and_then(|c| c.downcast::<WebView>().ok()).unwrap();
    let seen = || eval(&popup, "return document.getElementById('seen') ? document.getElementById('seen').textContent : 'none';", None);
    assert!(spin_until(|| seen() == "seen /", 20), "the popup reads the stored data through the shell: {}", seen());
    ask(Box::new(move || {
        crate::extensions::popup_closed(profile, "fixture01", page);
        JsonValue::Null
    }));
    assert!(spin_until(|| shown.first_child().is_none(), 5), "the closed popup leaves the panel");
    drop(jobs);
    shell.join().unwrap();
}

type Held = RefCell<Vec<Box<ext::Frame>>>;

/// Answers what the view asks, except the subframes' questions, which wait in `held`.
fn answer_but_frames(events: &RefCell<futures_channel::mpsc::UnboundedReceiver<ext::Event>>, held: &Held) {
    while let Ok(event) = events.borrow_mut().try_recv() {
        match event {
            ext::Event::Frame(frame) if !frame.main => held.borrow_mut().push(frame),
            other => crate::extensions::on_event(other),
        }
    }
}

/// Starts the shell's load of `url` without waiting for it.
fn start(state: &State, url: &str) {
    *state.shell_url.borrow_mut() = Some(url.to_owned());
    state.web().unwrap().load_uri(url);
}

fn allows_localhost(state: &State) -> bool {
    state.ext.allowed.borrow().values().flatten().any(|p| p.contains("localhost"))
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_frames_late_answer_allows_nothing_under_the_next_page() {
    settled(a_frames_late_answer_allows_nothing_under_the_next_page_in_a_view);
}

fn a_frames_late_answer_allows_nothing_under_the_next_page_in_a_view() {
    let f = fixture("file");
    grant(&f, "127.0.0.1", true, true);
    grant(&f, "localhost", true, true);
    let events = RefCell::new(crate::extensions::connect());
    let held = Held::default();
    let state = persistent_view(&f);
    assert!(spin_until(|| !state.ext.worlds.borrow().is_empty(), 5));
    let s = server();
    start(&state, &format!("{}framed", s.url));
    assert!(
        spin_until(
            || {
                answer_but_frames(&events, &held);
                !held.borrow().is_empty()
            },
            20
        ),
        "the frame's document waits for the gate"
    );
    state.events.borrow_mut().clear();
    start(&state, &s.url);
    let finished = || state.events.borrow().iter().any(|(n, _)| n == "finished");
    assert!(
        spin_until(
            || {
                answer_but_frames(&events, &held);
                finished()
            },
            30
        ),
        "the next page loaded"
    );
    for frame in held.take() {
        crate::extensions::on_event(ext::Event::Frame(frame));
    }
    spin_until(|| false, 1);
    assert!(!allows_localhost(&state), "the earlier page's frame adds nothing under the next page: {:?}", state.ext.allowed.borrow());
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_frame_runs_nothing_when_the_shell_cannot_be_asked() {
    settled(a_frame_runs_nothing_when_the_shell_cannot_be_asked_in_a_view);
}

fn a_frame_runs_nothing_when_the_shell_cannot_be_asked_in_a_view() {
    let f = fixture("file");
    grant(&f, "127.0.0.1", true, true);
    grant(&f, "localhost", true, true);
    let events = RefCell::new(crate::extensions::connect());
    let held = Held::default();
    let state = persistent_view(&f);
    assert!(spin_until(|| !state.ext.worlds.borrow().is_empty(), 5));
    let s = server();
    let web = state.web().unwrap();
    start(&state, &format!("{}framed", s.url));
    let report = || eval(&web, "return String(window.report);", None);
    let answered = || {
        answer_but_frames(&events, &held);
        for frame in held.take() {
            crate::extensions::on_event(ext::Event::Frame(frame));
        }
    };
    assert!(
        spin_until(
            || {
                answered();
                report() == UNSENT
            },
            20
        ),
        "the granted frame runs its script: {}",
        report()
    );
    assert!(allows_localhost(&state));
    drop(events);
    eval(&web, "window.report = 'none'; const f = document.querySelector('iframe'); f.src = f.src; return 'ok';", None);
    assert!(spin_until(|| report() == "|", 20), "the frame's next document runs nothing without the gate: {}", report());
    assert!(!allows_localhost(&state), "and its address is no longer allowed");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_frame_of_the_page_still_shown_is_gated_against_that_page() {
    settled(a_frame_of_the_page_still_shown_is_gated_against_that_page_in_a_view);
}

fn a_frame_of_the_page_still_shown_is_gated_against_that_page_in_a_view() {
    let f = fixture("file");
    grant(&f, "127.0.0.1", true, true);
    grant(&f, "localhost", true, true);
    let events = RefCell::new(crate::extensions::connect());
    let state = persistent_view(&f);
    assert!(spin_until(|| !state.ext.worlds.borrow().is_empty(), 5));
    let s = server();
    let framed = format!("{}framed", s.url);
    let next = format!("{}next", s.url.replace("127.0.0.1", "localhost"));
    let held = Held::default();
    let pump = || {
        while let Ok(event) = events.borrow_mut().try_recv() {
            match event {
                ext::Event::Frame(frame) if frame.url.as_str() == next || !frame.main && held.borrow().iter().any(|f| f.main) => held.borrow_mut().push(frame),
                other => crate::extensions::on_event(other),
            }
        }
    };
    let web = state.web().unwrap();
    state.events.borrow_mut().clear();
    start(&state, &framed);
    let report = || eval(&web, "return String(window.report);", None);
    assert!(
        spin_until(
            || {
                pump();
                report() == UNSENT
            },
            20
        ),
        "the framed page and its frame run: {}",
        report()
    );
    start(&state, &next);
    assert!(
        spin_until(
            || {
                pump();
                held.borrow().iter().any(|f| f.main)
            },
            20
        ),
        "the next page waits for the gate"
    );
    eval(&web, "const f = document.querySelector('iframe'); f.src = f.src; return 'ok';", None);
    assert!(
        spin_until(
            || {
                pump();
                held.borrow().iter().any(|f| !f.main)
            },
            20
        ),
        "the shown page's frame loads again"
    );
    let frame = held.borrow().iter().find(|f| !f.main).map(|f| (f.url.to_string(), f.top.to_string())).unwrap();
    assert_eq!(frame.1, framed, "the frame {} is gated against the page it is in", frame.0);
    for frame in held.take() {
        crate::extensions::on_event(ext::Event::Frame(frame));
    }
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_page_restored_from_the_back_forward_cache_is_still_heard() {
    settled(a_page_restored_from_the_back_forward_cache_is_still_heard_in_a_view);
}

fn a_page_restored_from_the_back_forward_cache_is_still_heard_in_a_view() {
    let f = fixture("file");
    bridge();
    grant(&f, "127.0.0.1", true, true);
    let state = persistent_view(&f);
    let s = server();
    let web = open(&state, &s.url);
    let reply = || {
        forward(&state, &f);
        data(&web, "tarnReply")
    };
    assert!(spin_until(|| reply() == "content via tab", 40), "the first page is heard: {}", reply());
    eval(&web, "window.kept = 'kept'; return 'ok';", None);
    open(&state, &format!("{}next", s.url));
    state.events.borrow_mut().clear();
    web.go_back();
    assert!(spin_until(|| state.events.borrow().iter().any(|(n, _)| n == "finished"), 30), "the first page came back");
    assert_eq!(eval(&web, "return String(window.kept);", None), "kept", "it came back from the back/forward cache");
    assert_eq!(send_late(&state, &f, &web), "late via tab", "the restored page is still heard");

    eval(&web, "history.pushState({}, '', '/pushed'); window.kept = 'pushed'; return 'ok';", None);
    open(&state, &format!("{}next", s.url));
    state.events.borrow_mut().clear();
    web.go_back();
    assert!(spin_until(|| state.events.borrow().iter().any(|(n, _)| n == "finished"), 30), "the page came back to its pushed entry");
    assert!(web.uri().is_some_and(|u| u.ends_with("/pushed")), "at its pushed entry: {:?}", web.uri());
    assert_eq!(eval(&web, "return String(window.kept);", None), "pushed", "it came back from the back/forward cache");
    assert_eq!(send_late(&state, &f, &web), "late via tab", "a page restored at an entry it pushed is still heard");

    open(&state, &format!("{}next", s.url));
    WebViewExt::settings(&web).expect("settings").set_enable_page_cache(false);
    state.events.borrow_mut().clear();
    web.go_back();
    assert!(spin_until(|| state.events.borrow().iter().any(|(n, _)| n == "finished"), 30), "the page loaded again at its pushed entry");
    assert_eq!(eval(&web, "return String(window.kept);", None), "undefined", "a fresh document, not the cached one");
    assert_eq!(send_late(&state, &f, &web), "late via tab", "a fresh document at a remembered entry draws its own tokens and is heard");
}
