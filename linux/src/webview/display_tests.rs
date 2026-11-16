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
    let links = ask(&state, "iconLinks", json!({ "seq": 8 }), "iconLinks");
    assert!(links["text"].as_str().unwrap().starts_with("http://page.test/i.png\t32x32\n"), "{links}");
    let icon = ask(&state, "fetchIcon", json!({ "seq": 9, "urls": ["http://page.test/i.png"], "maxBytes": 1024 }), "iconData");
    assert_eq!((icon["seq"].clone(), icon["text"].clone()), (json!(9), json!("")), "only https icons are fetched");
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
fn nothing_runs_or_applies_under_lockdown() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Lockdown, "private:display-lockdown");
    let web = open(&state);
    spin_until(|| false, 2);
    let found = ask(&state, "findText", json!({ "seq": 1, "query": "zebrafish" }), "textFound");
    assert_eq!(found["snippets"], json!([]));
    assert_eq!(ask(&state, "iconLinks", json!({ "seq": 2 }), "iconLinks")["text"], "");
    assert!(state.zap_filter.borrow().is_none(), "no zap filter under Lockdown");
    let settings = WebViewExt::settings(&web).unwrap();
    assert!(!settings.enables_javascript());
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
