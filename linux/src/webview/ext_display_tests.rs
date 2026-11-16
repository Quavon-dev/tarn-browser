use std::time::Duration;

use super::*;
use crate::extensions::tests::{Fixture, fixture};

const PAGE: &str = "<!doctype html><html><body><div id=\"marker\">m</div><script src=\"/blocked.js\"></script><script src=\"/ok.js\"></script></body></html>";

fn spin_until(done: impl Fn() -> bool, secs: u64) -> bool {
    let context = glib::MainContext::default();
    let end = Instant::now() + Duration::from_secs(secs);
    while !done() && Instant::now() < end {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

fn server() -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0; 2048];
            let n = stream.read(&mut request).unwrap_or(0);
            let line = String::from_utf8_lossy(&request[..n]).lines().next().unwrap_or_default().to_owned();
            let (kind, body) = match line.split(' ').nth(1).unwrap_or("/") {
                "/blocked.js" => ("text/javascript", "window.blockedRan = 1;"),
                "/ok.js" => ("text/javascript", "window.okRan = 1;"),
                _ => ("text/html", PAGE),
            };
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body.as_bytes()));
        }
    });
    format!("http://127.0.0.1:{port}/")
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

fn tab_view(f: &Fixture, profile: &str) -> Rc<State> {
    let state = Rc::new(State { container: Some(gtk::Box::new(gtk::Orientation::Vertical, 0)), ..State::default() });
    state.level.set(ShieldLevel::Standard);
    *state.profile.borrow_mut() = profile.to_owned();
    *state.tab.borrow_mut() = format!("false-{}", f.tab);
    VIEWS.with_borrow_mut(|views| views.push(Rc::downgrade(&state)));
    let wanted = Build { key: StoreKey::parse(profile).unwrap(), level: ShieldLevel::Standard, lists: Vec::new() };
    *state.built.borrow_mut() = Some(wanted.clone());
    build(&state, &wanted, &[]);
    state
}

fn go(state: &State, web: &WebView) {
    let url = server();
    *state.shell_url.borrow_mut() = Some(url.clone());
    web.load_uri(&url);
}

fn data(web: &WebView, name: &str) -> String {
    eval(web, &format!("return String(document.documentElement.dataset.{name});"), None)
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_tarn_extension_runs_isolated_messages_stores_and_blocks() {
    gtk::init().expect("a display");
    let f = fixture("file");
    ext::set_provider(crate::extensions::provider());
    crate::extensions::answer(f.profile, "fixture01", "127.0.0.1", true, true, f.tab);
    let uuid = f.core.disk.profile(f.profile).unwrap().data_store_uuid;
    let state = tab_view(&f, &uuid);
    assert!(spin_until(|| state.ext.filters.borrow().len() == 1, 5), "the install's rules are wanted");
    spin_until(|| false, 3);
    let web = state.web().unwrap();
    go(&state, &web);
    assert!(spin_until(|| state.events.borrow().iter().any(|(n, _)| n == "finished"), 30));
    let reply = || {
        forward(&state, &f);
        data(&web, "tarnReply")
    };
    assert!(spin_until(|| reply() == "content via tab", 40), "a message reaches the background and its answer comes back: {}", reply());
    assert_eq!(data(&web, "tarnExt"), "function", "the content script ran with chrome.runtime");
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
    let probe = "return [typeof chrome, typeof browser, typeof __tarnExtPost, typeof __tarnExtReceive, \
                 typeof (window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers['tarn-ext-fixture01'])].join(',');";
    assert_eq!(eval(&web, probe, None), "undefined,undefined,undefined,undefined,undefined", "the page's own world sees none of it");
    assert_eq!(eval(&web, "return getComputedStyle(document.getElementById('marker')).outlineWidth;", None), "7px", "the content CSS applies");
    assert_eq!(eval(&web, "return String(window.okRan) + ',' + String(window.blockedRan);", None), "1,undefined", "the DNR rule blocked the script");
    let origin = browser_core::tarn_ext::origin_id(&uuid, "fixture01");
    let fetched = eval(&web, &format!("return await fetch('tarn-extension://{origin}/popup.html').then(() => 'loaded', () => 'refused');"), None);
    assert_eq!(fetched, "refused", "a web page cannot load the extension's origin");
    let popup: Rc<RefCell<Option<Rc<ext::PageView>>>> = Rc::default();
    let slot = popup.clone();
    crate::extensions::open_popup(f.profile, "fixture01", move |page| *slot.borrow_mut() = Some(page));
    assert!(spin_until(|| popup.borrow().is_some(), 20), "the popup opened");
    let page = popup.borrow().clone().unwrap();
    let seen = || eval(&page.web, "return document.getElementById('seen') ? document.getElementById('seen').textContent : 'none';", None);
    assert!(spin_until(|| seen() == "seen /", 20), "the popup's own script ran under its CSP and read storage: {}", seen());
    crate::extensions::popup_closed(f.profile, "fixture01");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn nothing_runs_in_a_private_view_or_before_a_grant() {
    gtk::init().expect("a display");
    let f = fixture("file");
    ext::set_provider(crate::extensions::provider());
    let private = tab_view(&f, "private:ext-display");
    let web = private.web().unwrap();
    go(&private, &web);
    assert!(spin_until(|| private.events.borrow().iter().any(|(n, _)| n == "finished"), 30));
    spin_until(|| false, 1);
    assert_eq!(data(&web, "tarnExt"), "undefined", "a private view runs no extension");
    assert_eq!(eval(&web, "return String(window.blockedRan);", None), "1", "nor its rules");
    let uuid = f.core.disk.profile(f.profile).unwrap().data_store_uuid;
    let persistent = tab_view(&f, &uuid);
    let web = persistent.web().unwrap();
    go(&persistent, &web);
    assert!(spin_until(|| persistent.events.borrow().iter().any(|(n, _)| n == "finished"), 30));
    spin_until(|| false, 1);
    assert_eq!(data(&web, "tarnExt"), "undefined", "an ungranted host runs nothing before the user answers");
}
