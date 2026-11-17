use std::time::Duration;

use super::*;

const PAGE: &str = "<html><head><link rel=\"icon\" href=\"/i.png\" sizes=\"32x32\"></head><body><h1>Title</h1><div id=\"ad\" class=\"ad\">advert</div>\
    <p id=\"p\">The quick zebrafish swims past the reef while the tide turns slowly over the sand.</p></body></html>";

fn spin_until(done: impl Fn() -> bool, secs: u64) -> bool {
    let context = glib::MainContext::default();
    let end = Instant::now() + Duration::from_secs(secs);
    while !done() && Instant::now() < end {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

fn event(state: &State, name: &str) -> Option<JsonValue> {
    state.events.borrow().iter().rev().find(|(n, _)| n == name).map(|(_, d)| d.clone())
}

fn page_eval(web: &WebView, script: &str) -> String {
    let out = Rc::new(RefCell::new(None));
    let slot = out.clone();
    web.evaluate_javascript(script, None, None, None::<&webkit6::gio::Cancellable>, move |r| {
        *slot.borrow_mut() = Some(r.map(|v| v.to_str().to_string()).unwrap_or_else(|e| format!("error {e}")));
    });
    assert!(spin_until(|| out.borrow().is_some(), 20), "the page answered");
    out.take().unwrap_or_default()
}

fn view(level: ShieldLevel, profile: &str) -> Rc<State> {
    let state = Rc::new(State { container: Some(gtk::Box::new(gtk::Orientation::Vertical, 0)), ..State::default() });
    state.level.set(level);
    *state.profile.borrow_mut() = profile.to_owned();
    *state.styles.borrow_mut() = scripts::styles(&json!({ "page.test": "p { margin-top: 37px !important }" }));
    *state.theme_styles.borrow_mut() =
        scripts::theme_styles(&json!([["*", "p { padding-left: 11px !important }"], ["*.page.test", "p { margin-top: 5px !important }"]]));
    *state.zaps.borrow_mut() = scripts::zaps(&json!({ "page.test": ["div.ad"] }));
    let wanted = Build { key: StoreKey::parse(profile).unwrap(), level, lists: Vec::new() };
    *state.built.borrow_mut() = Some(wanted.clone());
    build(&state, &wanted, &[]);
    state
}

fn open(state: &Rc<State>) -> WebView {
    let web = state.web().unwrap();
    web.load_html(PAGE, Some("http://page.test/"));
    assert!(spin_until(|| event(state, "finished").is_some(), 30), "the page loaded");
    web
}

fn ask(state: &Rc<State>, name: &str, args: JsonValue, answer: &str) -> JsonValue {
    state.events.borrow_mut().clear();
    assert!(scripts::command(state, name, &args));
    assert!(spin_until(|| event(state, answer).is_some(), 20), "{name} answered");
    event(state, answer).unwrap()
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn contract_v3_scripts_run_in_the_tarn_world_and_styles_and_zaps_apply() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    assert!(spin_until(|| page_eval(&web, "getComputedStyle(document.getElementById('ad')).display") == "none", 20), "the zapped element is hidden");
    assert_eq!(page_eval(&web, "getComputedStyle(document.getElementById('p')).marginTop"), "37px", "the site's style sheet applies after the theme's");
    assert_eq!(page_eval(&web, "getComputedStyle(document.getElementById('p')).paddingLeft"), "11px", "the theme's every-site sheet applies");
    let found = ask(&state, "findText", json!({ "seq": 4, "query": "zebrafish" }), "textFound");
    assert_eq!(found["seq"], 4);
    assert_eq!(found["url"], "http://page.test/");
    assert!(found["snippets"][0].as_str().unwrap().contains("quick zebrafish swims"), "{found}");
    let text = ask(&state, "readText", json!({ "seq": 5, "maxChars": 30 }), "pageText");
    assert!(!text["text"].as_str().unwrap().is_empty() && text["text"].as_str().unwrap().encode_utf16().count() <= 30, "{text}");
    let links = ask(&state, "readIconLinks", json!({ "seq": 8 }), "iconLinks");
    assert!(links["text"].as_str().unwrap().starts_with("http://page.test/i.png\t32x32\n"), "{links}");
    let icon = ask(&state, "fetchIcon", json!({ "seq": 9, "page": "http://page.test/", "urls": ["http://page.test/i.png"] }), "iconData");
    assert_eq!((icon["seq"].clone(), icon["text"].clone()), (json!(9), json!("")), "only https icons are fetched");
    let elsewhere = ask(&state, "fetchIcon", json!({ "seq": 10, "page": "https://elsewhere.test/", "urls": ["https://elsewhere.test/i.png"] }), "iconData");
    assert_eq!(elsewhere["text"], "", "no fetch once the tab shows another origin");
    let probe = ask(&state, "readArticle", json!({ "seq": 6, "probe": true }), "article");
    assert_eq!(probe["text"], "", "a short page has no article");
    state.events.borrow_mut().clear();
    assert!(scripts::command(&state, "pick", &json!({ "seq": 7 })));
    spin_until(|| false, 1);
    assert_eq!(page_eval(&web, "typeof window.tarnPick"), "undefined", "the picker lives in the tarn world, not the page's");
    assert!(scripts::command(&state, "pickCancel", &JsonValue::Null));
    assert!(spin_until(|| event(&state, "picked").is_some(), 20), "a cancelled pick answers");
    assert_eq!(event(&state, "picked").unwrap()["selector"], "");
    eprintln!("v3 scripts, styles and zaps verified on {}", web.uri().unwrap_or_default());
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn only_the_users_own_navigation_to_a_store_link_asks_to_install() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    let block: Rc<RefCell<Option<UserContentFilter>>> = Rc::default();
    let slot = block.clone();
    let rules = r#"[{"trigger":{"url-filter":"^https://extensions\\.tarnbrowser\\.com/"},"action":{"type":"block"}}]"#;
    filters::compile_one("store-block.test".into(), rules.into(), move |f| *slot.borrow_mut() = Some(f));
    assert!(spin_until(|| block.borrow().is_some(), 20), "the test's rule compiled");
    state.content.borrow().as_ref().unwrap().add_filter(block.borrow().as_ref().unwrap());
    let link = "https://extensions.tarnbrowser.com/install/helper";
    let decided = |state: &State| state.events.borrow().iter().any(|(n, d)| n == "decided" && d["url"].as_str().is_some_and(|u| u.starts_with(link)));
    for script in [
        format!("location.href = '{link}'"),
        format!("const a = document.createElement('a'); a.href = '{link}/'; document.body.append(a); a.click()"),
        format!("const f = document.createElement('iframe'); f.src = '{link}'; document.body.append(f)"),
    ] {
        state.events.borrow_mut().clear();
        web.load_html(&format!("<html><body><script>{script}</script></body></html>"), Some("http://page.test/"));
        assert!(spin_until(|| decided(&state), 20), "{script}: the navigation reached the policy: {:?}", state.events.borrow());
        spin_until(|| false, 1);
        assert!(event(&state, "committed").is_some(), "{script}: the page loaded");
        assert!(event(&state, engine::tarn_ext::STORE_EVENT).is_none(), "{script}: {:?}", state.events.borrow());
        if !script.contains("iframe") {
            let blocked = event(&state, "failed").is_some_and(|d| d["url"].as_str().is_some_and(|u| u.starts_with(link)));
            assert!(blocked, "{script}: the test's rule stopped the request: {:?}", state.events.borrow());
        }
    }
    state.events.borrow_mut().clear();
    *state.load.borrow_mut() = Some((link.to_owned(), 1));
    state.load_link.set(true);
    load(&state, None);
    assert!(spin_until(|| decided(&state), 20), "the pop-up's load reached the policy");
    spin_until(|| false, 1);
    assert!(event(&state, engine::tarn_ext::STORE_EVENT).is_none(), "a page's pop-up tab never asks: {:?}", state.events.borrow());
    web.load_html(PAGE, Some("http://page.test/"));
    assert!(spin_until(|| event(&state, "finished").is_some(), 30), "the page loaded");
    *state.load.borrow_mut() = Some((link.to_owned(), 2));
    state.load_link.set(false);
    load(&state, None);
    assert!(spin_until(|| event(&state, engine::tarn_ext::STORE_EVENT).is_some(), 20), "the user's own load asks");
    assert_eq!(event(&state, engine::tarn_ext::STORE_EVENT).unwrap(), json!({ "id": "helper" }));
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn nothing_runs_or_applies_under_lockdown() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Lockdown, "private:display-lockdown");
    let web = open(&state);
    spin_until(|| false, 2);
    let found = ask(&state, "findText", json!({ "seq": 1, "query": "zebrafish" }), "textFound");
    assert_eq!(found["snippets"], json!([]));
    assert_eq!(ask(&state, "readIconLinks", json!({ "seq": 2 }), "iconLinks")["text"], "");
    assert!(state.zap_filter.borrow().is_none(), "no zap filter under Lockdown");
    let settings = WebViewExt::settings(&web).unwrap();
    assert!(!settings.enables_javascript());
}

fn patched_on(state: &Rc<State>, base: &str) -> String {
    let web = state.web().unwrap();
    state.events.borrow_mut().clear();
    web.load_html(PAGE, Some(base));
    assert!(spin_until(|| event(state, "finished").is_some(), 30), "{base} loaded");
    let probe = "[document.hasFocus, JSON.parse].map(f => Function.prototype.toString.call(f).includes('[native code]') ? '-' : 'patched').join()";
    page_eval(&web, probe)
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn youtube_and_netflix_scripts_run_only_on_their_hosts_and_never_under_lockdown() {
    gtk::init().expect("a display");
    let built = |level: ShieldLevel, profile: &str, youtube: bool| {
        let state = Rc::new(State { container: Some(gtk::Box::new(gtk::Orientation::Vertical, 0)), ..State::default() });
        state.level.set(level);
        *state.profile.borrow_mut() = profile.to_owned();
        *state.levels.borrow_mut() = Levels::parse(&json!({ "*": level.as_str() }), level);
        let lists = if youtube { vec![shield::groups::YOUTUBE_GROUP.to_owned()] } else { Vec::new() };
        let wanted = Build { key: StoreKey::parse(profile).unwrap(), level, lists };
        *state.built.borrow_mut() = Some(wanted.clone());
        build(&state, &wanted, &[]);
        state
    };
    let (neither, netflix, youtube) = ("-,-", "patched,-", "-,patched");

    let on = built(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef", true);
    assert_eq!(on.page_scripts.borrow().len(), 2);
    assert_eq!(patched_on(&on, "https://www.youtube.com/watch?v=1"), youtube);
    assert_eq!(patched_on(&on, "https://m.youtube.com/"), youtube);
    assert_eq!(patched_on(&on, "https://accounts.youtube.com/"), neither, "never on accounts.youtube.com");
    assert_eq!(patched_on(&on, "https://notyoutube.com/"), neither);
    assert_eq!(patched_on(&on, "https://www.netflix.com/browse"), netflix);
    assert_eq!(patched_on(&on, "https://netflix.com.evil.test/"), neither);
    assert_eq!(patched_on(&on, "http://page.test/"), neither);

    *on.levels.borrow_mut() = Levels::parse(&json!({ "*": "standard", "www.youtube.com": "lockdown", "www.netflix.com": "lockdown" }), ShieldLevel::Standard);
    scripts::apply_page_scripts(&on, on.content.borrow().as_ref().unwrap());
    assert_eq!(on.page_scripts.borrow().len(), 2, "the old scripts were replaced, not added to");
    assert_eq!(patched_on(&on, "https://m.youtube.com/"), youtube);
    let lockdown = on.levels.replace(Levels::parse(&json!({ "*": "standard" }), ShieldLevel::Standard));
    assert_eq!(patched_on(&on, "https://www.youtube.com/"), neither, "the installed scripts skip the hosts set to Lockdown when they were installed");
    assert_eq!(patched_on(&on, "https://www.netflix.com/"), neither, "the installed scripts skip the hosts set to Lockdown when they were installed");
    *on.levels.borrow_mut() = lockdown;
    for host in ["https://www.youtube.com/", "https://www.netflix.com/"] {
        let answer = patched_on(&on, host);
        assert!(answer.starts_with("error ") && answer.contains("Cannot execute JavaScript"), "not on a host set to Lockdown: {host} answered {answer}");
        assert!(!WebViewExt::settings(&on.web().unwrap()).unwrap().enables_javascript(), "JavaScript is off on {host}, so no page-world script runs");
    }

    let off = built(ShieldLevel::Strict, "burner:1123456789abcdef0123456789abcdef", false);
    assert_eq!(off.page_scripts.borrow().len(), 1, "only the Netflix script while YouTube ads are not blocked");
    assert_eq!(patched_on(&off, "https://www.youtube.com/"), neither);
    assert_eq!(patched_on(&off, "https://www.netflix.com/"), netflix);

    let locked = built(ShieldLevel::Lockdown, "private:display-page-scripts", true);
    assert!(locked.page_scripts.borrow().is_empty(), "no page script under Lockdown");

    let counts: Rc<RefCell<Vec<usize>>> = Rc::default();
    for lists in [Vec::new(), vec![shield::groups::YOUTUBE_GROUP.to_owned()]] {
        let counts = counts.clone();
        filters::with_filters(ShieldLevel::Standard, lists, move |f| counts.borrow_mut().push(f.map_or(0, |f| f.len())));
    }
    assert!(spin_until(|| counts.borrow().len() == 2, 60), "the filters compiled");
    assert_eq!(*counts.borrow(), [1, 2], "the YouTube rules join the built-in rules only when asked for");
    eprintln!("YouTube and Netflix scripts installed only where they belong");
}

fn slow_server(chunks: usize) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let mut request = [0; 2048];
                let _ = stream.read(&mut request);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"big.zip\"\r\n\r\n",
                    chunks * 1024
                );
                let _ = stream.write_all(head.as_bytes());
                for _ in 0..chunks {
                    if stream.write_all(&[b'x'; 1024]).and_then(|()| stream.flush()).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
        }
    });
    format!("http://127.0.0.1:{port}/big.zip")
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_download_outlives_its_view_and_burned_store_and_cancels_by_id() {
    gtk::init().expect("a display");
    let route = RefCell::new(session::route());
    let reports: RefCell<Vec<(String, JsonValue)>> = RefCell::default();
    let states = |id: &str| -> Vec<String> {
        while let Ok(report) = route.borrow_mut().try_recv() {
            reports.borrow_mut().push(report);
        }
        reports.borrow().iter().filter(|(_, d)| d["id"] == id).map(|(_, d)| d["state"].as_str().unwrap_or_default().to_owned()).collect()
    };
    let burner = "burner:00112233445566778899aabbccddeeff";
    let state = view(ShieldLevel::Standard, burner);
    *state.tab.borrow_mut() = "true-5".to_owned();
    VIEWS.with_borrow_mut(|views| views.push(Rc::downgrade(&state)));
    let web = open(&state);
    let url = slow_server(20);
    web.download_uri(&url);
    web.download_uri(&url);
    assert!(spin_until(|| states("1").contains(&"started".to_owned()) && states("2").contains(&"started".to_owned()), 20), "{:?}", reports.borrow());
    state.drop_view();
    drop((web, state));
    session::release(&StoreKey::parse(burner).unwrap());
    session::cancel("2");
    assert!(spin_until(|| states("1").last().is_some_and(|s| s == "finished"), 30), "{:?}", reports.borrow());
    spin_until(|| false, 1);
    assert_eq!(states("2").last().map(String::as_str), Some("failed"), "a cancelled download never reports finished");
    assert!(reports.borrow().iter().all(|(tab, _)| tab == "true-5"));
    let done = reports.borrow().iter().rev().find(|(_, d)| d["id"] == "1").map(|(_, d)| d.clone()).unwrap();
    assert_eq!(std::fs::metadata(done["path"].as_str().unwrap()).unwrap().len(), 20 * 1024);
    assert!(session::DOWNLOADS.with_borrow(HashMap::is_empty), "finished and cancelled downloads leave the map");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn the_page_joins_the_window_only_once_its_area_has_a_size() {
    gtk::init().expect("a display");
    let state = Rc::new(State { container: Some(gtk::Box::new(gtk::Orientation::Vertical, 0)), ..State::default() });
    let area = gtk::Fixed::new();
    area.put(state.container(), 0., 0.);
    let window = gtk::Window::builder().default_width(400).default_height(300).child(&area).build();
    window.present();
    assert!(spin_until(|| state.container().is_mapped(), 10), "the empty area is on screen");
    let wanted = Build { key: StoreKey::parse("private:display-size").unwrap(), level: ShieldLevel::Standard, lists: Vec::new() };
    *state.built.borrow_mut() = Some(wanted.clone());
    build(&state, &wanted, &[]);
    let web = state.web().unwrap();
    spin_until(|| false, 1);
    assert!(web.parent().is_none(), "a page mapped at no size never paints");
    state.container().set_size_request(300, 200);
    assert!(spin_until(|| web.parent().as_ref() == Some(state.container().upcast_ref()), 10), "the page joins once the area is laid out");
    state.drop_view();
    assert!(state.container().first_child().is_none());
    window.destroy();
}

fn counting_listener(hold: Duration) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let count = std::sync::Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            seen.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                std::thread::sleep(hold);
                drop(stream);
            });
        }
    });
    (port, count)
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn icon_candidates_stop_once_the_tab_leaves_the_origin() {
    use std::sync::atomic::Ordering;
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "private:display-icons");
    let web = state.web().unwrap();
    web.load_html(PAGE, Some("https://page.test/"));
    assert!(spin_until(|| event(&state, "finished").is_some(), 30), "the page loaded");
    *state.page.borrow_mut() = Url::parse("https://page.test/").ok();
    let (first, first_seen) = counting_listener(Duration::from_secs(2));
    let (second, second_seen) = counting_listener(Duration::ZERO);
    let urls = json!([format!("https://127.0.0.1:{first}/a.png"), format!("https://127.0.0.1:{second}/b.png")]);
    state.events.borrow_mut().clear();
    assert!(scripts::command(&state, "fetchIcon", &json!({ "seq": 3, "page": "https://page.test/", "urls": urls })));
    assert!(spin_until(|| first_seen.load(Ordering::SeqCst) > 0, 20), "the first candidate was asked");
    *state.page.borrow_mut() = Url::parse("https://other.test/").ok();
    assert!(spin_until(|| event(&state, "iconData").is_some(), 30), "fetchIcon answered");
    spin_until(|| false, 2);
    assert_eq!(event(&state, "iconData").unwrap()["text"], "");
    assert_eq!(second_seen.load(Ordering::SeqCst), 0, "no candidate is asked once the tab shows another origin");
}
