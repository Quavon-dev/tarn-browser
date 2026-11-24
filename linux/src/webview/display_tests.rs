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
    let wanted = Build { key: StoreKey::parse(profile).unwrap(), level, lists: Vec::new(), inspectable: false };
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
    state.events.borrow_mut().clear();
    assert!(scripts::command(&state, "pick", &json!({ "seq": 8, "rect": true })));
    spin_until(|| false, 1);
    assert!(scripts::command(&state, "pickCancel", &JsonValue::Null));
    assert!(spin_until(|| event(&state, "picked").is_some(), 20), "a cancelled rectangle pick answers");
    let picked = event(&state, "picked").unwrap();
    assert_eq!((picked["seq"].clone(), picked["rect"].clone(), picked.get("selector")), (json!(8), json!(""), None), "{picked}");
    eprintln!("v3 scripts, styles and zaps verified on {}", web.uri().unwrap_or_default());
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn insert_text_types_into_the_focused_field_only() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    let none = ask(&state, "insertText", json!({ "seq": 1, "text": "hello" }), "textInserted");
    assert_eq!(none, json!({ "seq": 1, "inserted": false }), "no field has the focus");
    page_eval(&web, "const t = document.createElement('textarea'); t.id = 'note'; document.body.append(t); t.focus(); 'ok'");
    let typed = ask(&state, "insertText", json!({ "seq": 2, "text": "Best,\nAda" }), "textInserted");
    assert_eq!(typed, json!({ "seq": 2, "inserted": true }));
    assert_eq!(page_eval(&web, "document.getElementById('note').value"), "Best,\nAda");
    let long = "a".repeat(browser_core::snippets::MAX_SNIPPET + 1);
    assert_eq!(ask(&state, "insertText", json!({ "seq": 3, "text": long }), "textInserted")["inserted"], false, "a longer text is refused");
    assert_eq!(page_eval(&web, "document.getElementById('note').value"), "Best,\nAda");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn an_agent_script_answers_and_its_gate_stops_a_page_started_navigation() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    let answer = ask(&state, "agent", json!({ "seq": 3, "body": "return document.querySelector('h1').innerText;" }), "agentAnswer");
    assert_eq!(answer, json!({ "seq": 3, "url": "http://page.test/", "text": "Title" }));
    let guarded = |guard: &str| format!("if (!({guard})) return '';\nreturn document.querySelector('h1').innerText;");
    let here = ask(&state, "agent", json!({ "seq": 4, "body": guarded("location.origin === \"http://page.test\"") }), "agentAnswer");
    assert_eq!(here["text"], "Title", "a script bound to the page's origin runs");
    for (seq, guard) in [(5, "location.origin === \"http://other.test\""), (6, "location.origin === \"null\" && location.href === \"about:blank\"")] {
        let elsewhere = ask(&state, "agent", json!({ "seq": seq, "body": guarded(guard) }), "agentAnswer");
        assert_eq!(elsewhere["text"], "", "a script checked for another page answers nothing here: {guard}");
    }
    let site = browser_core::agent::site_of(&Url::parse("http://page.test/").unwrap()).unwrap();
    *state.agent_sites.borrow_mut() = Some(HashSet::from([site]));
    let click = |href: &str| format!("const a = document.createElement('a'); a.href = '{href}'; document.body.append(a); a.click(); 'ok'");
    let decided = |state: &State, url: &str| state.events.borrow().iter().any(|(n, d)| n == "decided" && d["url"] == url);
    state.events.borrow_mut().clear();
    page_eval(&web, &click("http://page.test/next"));
    assert!(spin_until(|| decided(&state, "http://page.test/next"), 20), "the allowed link reached the policy");
    assert!(event(&state, "agentHeld").is_none(), "a link to an allowed site goes through: {:?}", state.events.borrow());
    spin_until(|| event(&state, "finished").is_some() || event(&state, "failed").is_some(), 30);
    state.events.borrow_mut().clear();
    web.load_html(PAGE, Some("http://page.test/"));
    assert!(spin_until(|| event(&state, "finished").is_some(), 30), "the page loaded");
    state.events.borrow_mut().clear();
    page_eval(&web, &click("http://other.test/away"));
    assert!(spin_until(|| event(&state, "agentHeld").is_some(), 20), "a click run from outside the page is held: {:?}", state.events.borrow());
    assert_eq!(event(&state, "agentHeld").unwrap(), json!({ "url": "http://other.test/away" }));
    spin_until(|| false, 1);
    assert_eq!(web.uri().unwrap_or_default(), "http://page.test/", "the held navigation never loaded");
    assert!(event(&state, "started").is_none(), "{:?}", state.events.borrow());
    state.events.borrow_mut().clear();
    web.load_html("<html><body><script>location.href = 'http://other.test/script'</script></body></html>", Some("http://page.test/"));
    assert!(spin_until(|| decided(&state, "http://other.test/script"), 20), "the page's own navigation reached the policy");
    spin_until(|| false, 1);
    assert_eq!(web.uri().unwrap_or_default(), "http://page.test/", "the refused navigation never loaded");
    assert!(event(&state, "agentHeld").is_none(), "a navigation that may be a frame's is cancelled without asking");
}

#[test]
fn the_agent_gate_reports_only_a_known_main_frame_navigation() {
    let state = State::default();
    let away = Url::parse("https://other.test/away").unwrap();
    assert!(!agent_held(&state, &away, true, true), "no gate outside an agent's window");
    *state.agent_sites.borrow_mut() = Some(HashSet::from(["page.test".to_owned()]));
    assert!(!agent_held(&state, &Url::parse("https://www.page.test/").unwrap(), true, true), "an allowed site goes through");
    assert!(!agent_held(&state, &away, false, true), "the shell's own load goes through");
    assert!(agent_held(&state, &away, true, false) && event(&state, "agentHeld").is_none(), "a frame's or a script's is only cancelled");
    assert!(agent_held(&state, &away, true, true));
    assert_eq!(event(&state, "agentHeld").unwrap(), json!({ "url": "https://other.test/away" }));
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn ask_assistant_reads_the_selection_with_its_command_but_never_a_secret_field() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    let asked = |command: Option<&str>| {
        state.events.borrow_mut().clear();
        scripts::ask_selection(&state, command.map(str::to_owned));
        assert!(spin_until(|| event(&state, "assistantSelection").is_some(), 20), "the selection was read");
        event(&state, "assistantSelection").unwrap()
    };
    page_eval(&web, "getSelection().selectAllChildren(document.getElementById('p')); 'ok'");
    let read = asked(Some("Explain"));
    assert!(read["text"].as_str().unwrap().starts_with("The quick zebrafish"), "{read}");
    assert_eq!(read["command"], "Explain");
    assert!(read["url"].as_str().is_some_and(|u| !u.is_empty()));
    assert_eq!(asked(None)["command"], JsonValue::Null);
    page_eval(
        &web,
        "const f = document.createElement('input'); f.type = 'password'; f.value = 'typed value'; document.body.append(f); f.focus(); f.select(); 'ok'",
    );
    assert_eq!(asked(None)["text"], "", "a password field's text never reaches the assistant");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn the_selection_is_read_but_never_from_a_secret_field() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    page_eval(&web, "getSelection().selectAllChildren(document.getElementById('p')); 'ok'");
    let read = ask(&state, "readSelection", json!({ "seq": 1 }), "selection");
    assert!(read["text"].as_str().unwrap().starts_with("The quick zebrafish"), "{read}");
    for (field, autocomplete) in [("input", "one-time-code"), ("input", "section-a cc-number"), ("textarea", "new-password")] {
        page_eval(
            &web,
            &format!(
                "(() => {{ const host = document.createElement('div'); document.body.append(host);
                 const f = host.attachShadow({{ mode: 'open' }}).appendChild(document.createElement('{field}'));
                 f.setAttribute('autocomplete', '{autocomplete}'); f.value = '4111111111111111'; f.focus(); f.select(); return 'ok'; }})()"
            ),
        );
        assert_eq!(ask(&state, "readSelection", json!({ "seq": 2 }), "selection")["text"], "", "{field} {autocomplete}");
    }
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn only_the_users_own_navigation_to_a_store_link_asks_to_install() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    let block: Rc<RefCell<Option<UserContentFilter>>> = Rc::default();
    let slot = block.clone();
    let rules = r#"[{"trigger":{"url-filter":"^https://extensions\\.tarnbrowser\\.com/"},"action":{"type":"block"}},
        {"trigger":{"url-filter":"^https://rules-ready\\.invalid/"},"action":{"type":"block"}}]"#;
    filters::compile_one("store-block.test".into(), rules.into(), move |f| *slot.borrow_mut() = Some(f));
    assert!(spin_until(|| block.borrow().is_some(), 20), "the test's rule compiled");
    state.content.borrow().as_ref().unwrap().add_filter(block.borrow().as_ref().unwrap());
    let failed = |state: &State, url: &str, why: &str| {
        state
            .events
            .borrow()
            .iter()
            .any(|(n, d)| n == "failed" && d["url"].as_str().is_some_and(|u| u.starts_with(url)) && d["message"].as_str().is_some_and(|m| m.contains(why)))
    };
    let ready = (0..10).any(|_| {
        state.events.borrow_mut().clear();
        web.load_html("<html><body><script>location.href = 'https://rules-ready.invalid/'</script></body></html>", Some("http://page.test/"));
        spin_until(|| failed(&state, "https://rules-ready.invalid/", "content blocker"), 3)
    });
    assert!(ready, "the test's rule is in effect (WebKit's English error text; run under the C locale as display-tests.sh does): {:?}", state.events.borrow());
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
        if !script.contains("iframe") {
            assert!(spin_until(|| failed(&state, link, "content blocker"), 20), "{script}: the test's rule stopped the request: {:?}", state.events.borrow());
        }
        spin_until(|| false, 1);
        assert!(event(&state, "committed").is_some(), "{script}: the page loaded");
        assert!(event(&state, engine::tarn_ext::STORE_EVENT).is_none(), "{script}: {:?}", state.events.borrow());
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
fn only_the_shells_open_file_load_shows_a_local_file() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("page.html");
    std::fs::write(&path, "<p id=\"p\">local file</p>").unwrap();
    let url = browser_core::open_file::url_for(&path).unwrap();
    state.events.borrow_mut().clear();
    *state.load.borrow_mut() = Some((url.to_string(), 1));
    load(&state, None);
    assert!(spin_until(|| event(&state, "blocked").is_some(), 20), "a plain load: {:?}", state.events.borrow());
    assert_eq!(event(&state, "blocked").unwrap(), json!({ "url": url.as_str(), "reason": "nav_policy" }));
    state.events.borrow_mut().clear();
    *state.load.borrow_mut() = Some((url.to_string(), 2));
    state.load_file.set(true);
    load(&state, None);
    assert!(spin_until(|| event(&state, "finished").is_some(), 30), "the Open File load: {:?}", state.events.borrow());
    assert_eq!(page_eval(&web, "document.getElementById('p').textContent"), "local file");
    state.events.borrow_mut().clear();
    page_eval(&web, "location.href = 'file:///etc/hostname'; 1");
    assert!(spin_until(|| event(&state, "blocked").is_some_and(|d| d["url"] == "file:///etc/hostname"), 20), "the file page's own navigation");
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
        let wanted = Build { key: StoreKey::parse(profile).unwrap(), level, lists, inspectable: false };
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
    session::cancel("2");
    assert!(spin_until(|| states("1").last().is_some_and(|s| s == "finished"), 30), "{:?}", reports.borrow());
    assert!(spin_until(|| states("2").last().is_some_and(|s| s == "failed"), 10), "a cancelled download never reports finished: {:?}", reports.borrow());
    assert!(!states("2").contains(&"finished".to_owned()), "{:?}", reports.borrow());
    assert_eq!(states("2").iter().filter(|s| *s == "failed").count(), 1, "a cancel reports once: {:?}", reports.borrow());
    let path = |id: &str| reports.borrow().iter().find(|(_, d)| d["id"] == id && d["state"] == "started").map(|(_, d)| d["path"].clone());
    assert_ne!(path("1"), path("2"), "two downloads of one name never share a file");
    assert!(!std::path::Path::new(path("2").unwrap().as_str().unwrap()).exists(), "a cancelled download leaves no file");
    assert!(reports.borrow().iter().all(|(tab, _)| tab == "true-5"));
    let done = reports.borrow().iter().rev().find(|(_, d)| d["id"] == "1").map(|(_, d)| d.clone()).unwrap();
    assert_eq!(std::fs::metadata(done["path"].as_str().unwrap()).unwrap().len(), 20 * 1024);
    assert!(session::DOWNLOADS.with_borrow(HashMap::is_empty), "finished and cancelled downloads leave the map");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_download_from_a_malware_site_waits_unsaved_until_kept() {
    gtk::init().expect("a display");
    let route = RefCell::new(session::route());
    let reports: RefCell<Vec<JsonValue>> = RefCell::default();
    let all = || {
        while let Ok((_, data)) = route.borrow_mut().try_recv() {
            reports.borrow_mut().push(data);
        }
        reports.borrow().clone()
    };
    let states = |id: &str| all().iter().filter(|d| d["id"] == id).map(|d| d["state"].as_str().unwrap_or_default().to_owned()).collect::<Vec<_>>();
    let flagged = || all().into_iter().filter(|d| d["state"] == "flagged").collect::<Vec<_>>();
    let state = view(ShieldLevel::Standard, "private:display-flagged");
    *state.lists.borrow_mut() = vec!["malware".to_owned()];
    *state.tab.borrow_mut() = "true-6".to_owned();
    VIEWS.with_borrow_mut(|views| views.push(Rc::downgrade(&state)));
    let web = open(&state);
    list_malware(&["page.test"]);
    assert!(filters::malware().is_some_and(|s| s.contains("page.test")));
    let url = slow_server(5);
    web.download_uri(&url);
    web.download_uri(&url);
    assert!(spin_until(|| flagged().len() == 2, 20), "{:?}", reports.borrow());
    let held = flagged();
    assert!(held.iter().all(|d| d["host"] == "page.test" && d["name"] == "big.zip"), "{held:?}");
    let (kept, declined) = (held[0]["id"].as_str().unwrap().to_owned(), held[1]["id"].as_str().unwrap().to_owned());
    spin_until(|| false, 2);
    let saved = || std::fs::read_dir(session::downloads_dir()).map(|d| d.count()).unwrap_or(0);
    assert_eq!(saved(), 0, "nothing is written before the user keeps it");
    session::keep(&kept, true);
    session::keep(&declined, false);
    assert!(spin_until(|| states(&kept).last().is_some_and(|s| s == "finished"), 30), "{:?}", reports.borrow());
    spin_until(|| false, 1);
    assert_eq!(states(&declined), ["started", "flagged"], "a declined download ends without a failure");
    let done = all().into_iter().rev().find(|d| d["id"] == kept.as_str()).unwrap();
    assert_eq!(std::fs::metadata(done["path"].as_str().unwrap()).unwrap().len(), 5 * 1024);
    assert_eq!(saved(), 1, "only the kept file is on disk");
    assert!(session::DOWNLOADS.with_borrow(HashMap::is_empty));
    state.lists.borrow_mut().clear();
    web.download_uri(&url);
    assert!(spin_until(|| all().iter().any(|d| d["state"] == "finished" && d["id"] != kept.as_str()), 30), "{:?}", reports.borrow());
    assert_eq!(flagged().len(), 2, "with the malware switch off nothing is flagged");
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
    let wanted = Build { key: StoreKey::parse("private:display-size").unwrap(), level: ShieldLevel::Standard, lists: Vec::new(), inspectable: false };
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

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn save_pdf_prints_the_page_to_the_chosen_file_and_lists_it_as_a_download() {
    gtk::init().expect("a display");
    let route = RefCell::new(session::route());
    let state = view(ShieldLevel::Standard, "private:display-pdf");
    *state.tab.borrow_mut() = "tab-pdf".to_owned();
    let web = open(&state);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Title.pdf");
    save_pdf(&state, &web, &json!({ "path": "relative.pdf" }));
    save_pdf(&state, &web, &json!({ "path": path }));
    let reported = RefCell::new(None);
    let arrived = || {
        if reported.borrow().is_none() {
            *reported.borrow_mut() = route.borrow_mut().try_recv().ok();
        }
        reported.borrow().is_some()
    };
    assert!(spin_until(arrived, 30), "the save was reported");
    let (tab, data) = reported.take().unwrap();
    assert_eq!(tab, "tab-pdf");
    assert_eq!(
        (data["state"].as_str(), data["name"].as_str(), data["url"].as_str()),
        (Some("finished"), Some("Title.pdf"), Some("http://page.test/")),
        "{data}"
    );
    assert_eq!(data["path"], json!(path));
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"%PDF-"), "a PDF was written");
    assert!(!dir.path().join("relative.pdf").exists() && std::fs::read_dir(dir.path()).unwrap().count() == 1, "a relative path is refused");
}

fn list_malware(hosts: &[&str]) {
    filters::want_sets(vec![Category::Malware]);
    let set = shield::domains::DomainSet::from_bytes(&shield::domains::DomainSet::encode(hosts.iter().copied())).map(std::sync::Arc::new);
    filters::install_sets(browser_core::site::Sets { malware: set, ..Default::default() });
}

fn listed(state: &State) -> Option<JsonValue> {
    state.events.borrow().iter().find(|(n, d)| n == "blocked" && d["reason"] == "listed").map(|(_, d)| d.clone())
}

fn redirect_server(to: String) -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut request = [0; 2048];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
        }
    });
    port
}

#[test]
fn a_snapshot_is_scaled_down_to_the_card_width() {
    let pixels: Vec<u8> = (0..4u8).flat_map(|p| [p, p, p, 255]).collect();
    let (scaled, w, h) = scale_down(&pixels, 4, 1, 2);
    assert_eq!((w, h), (2, 1));
    assert_eq!(scaled, vec![0, 0, 0, 255, 2, 2, 2, 255]);
    assert_eq!(scale_down(&pixels, 2, 2, 8).1, 2, "never scaled up");
}

#[test]
fn a_full_page_texture_larger_than_the_checked_document_is_refused() {
    assert!(as_checked((800, 2000), [800., 2000.], 1.));
    assert!(as_checked((1601, 4001), [800.4, 2000.4], 2.), "a fraction rounds up");
    assert!(!as_checked((800, 2001), [800., 2000.], 1.), "the page grew after it was measured");
    assert!(!as_checked((1601, 4000), [800., 2000.], 2.));
}

#[test]
fn a_snapshot_area_is_cut_inside_the_texture() {
    assert_eq!(pixel_rect([10., 5., 20., 10.], 2., 1000, 1000), Some([20, 10, 60, 30]));
    assert_eq!(pixel_rect([90., 0., 50., 10.], 1., 100, 100), Some([90, 0, 100, 10]), "cut at the edge");
    assert_eq!(pixel_rect([200., 0., 10., 10.], 1., 100, 100), None, "outside the visible page");
    assert_eq!(pixel_rect([0., 0., 10., 10.], f64::NAN, 100, 100), None);
    let pixels: Vec<u8> = (0..9u8).flat_map(|p| [p, p, p, 255]).collect();
    assert_eq!(crop(&pixels, 3, [1, 1, 3, 2]), vec![4, 4, 4, 255, 5, 5, 5, 255]);
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_muted_view_stays_muted_when_rebuilt_and_unmutes() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdee");
    assert!(!state.web().unwrap().is_muted());
    mute(&state, true);
    assert!(state.web().unwrap().is_muted());
    let wanted =
        Build { key: StoreKey::parse("burner:0123456789abcdef0123456789abcdee").unwrap(), level: ShieldLevel::Strict, lists: Vec::new(), inspectable: false };
    state.drop_view();
    build(&state, &wanted, &[]);
    assert!(state.web().unwrap().is_muted(), "a rebuilt view keeps the mute");
    mute(&state, false);
    assert!(!state.web().unwrap().is_muted());
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn the_gate_stops_a_listed_host_in_every_frame_and_redirect_before_any_request() {
    use std::sync::atomic::Ordering;
    gtk::init().expect("a display");
    let (port, seen) = counting_listener(Duration::ZERO);
    let target = format!("http://127.0.0.1:{port}/page");
    list_malware(&["127.0.0.1"]);
    for (level, profile) in [(ShieldLevel::Standard, "private:display-gate"), (ShieldLevel::Lockdown, "private:display-gate-lockdown")] {
        let state = view(level, profile);
        VIEWS.with_borrow_mut(|views| views.push(Rc::downgrade(&state)));
        let web = open(&state);
        state.events.borrow_mut().clear();
        *state.load.borrow_mut() = Some((target.clone(), 1));
        load(&state, None);
        assert!(spin_until(|| listed(&state).is_some(), 20), "{level:?}: the typed load is stopped: {:?}", state.events.borrow());
        assert_eq!(listed(&state).unwrap(), json!({ "url": target, "reason": "listed", "category": "malware" }));
        assert_eq!(web.uri().as_deref(), Some("http://page.test/"), "{level:?}: the tab keeps its page");
        spin_until(|| false, 1);
        assert_eq!(seen.load(Ordering::SeqCst), 0, "{level:?}: the typed load never connects to the listed host");
        state.events.borrow_mut().clear();
        web.load_html(&format!("<html><body><iframe src=\"{target}\"></iframe></body></html>"), Some("http://page.test/"));
        let decided = |state: &State| state.events.borrow().iter().any(|(n, d)| n == "decided" && d["url"] == target.as_str());
        assert!(spin_until(|| decided(&state), 20), "{level:?}: the frame reached the gate: {:?}", state.events.borrow());
        spin_until(|| false, 1);
        assert!(listed(&state).is_none(), "{level:?}: a frame is stopped without a warning page: {:?}", state.events.borrow());
        assert_eq!(seen.load(Ordering::SeqCst), 0, "{level:?}: the frame never connects to the listed host");
        if level == ShieldLevel::Standard {
            state.events.borrow_mut().clear();
            let redirect = format!("http://localhost:{}/go", redirect_server(target.clone()));
            *state.load.borrow_mut() = Some((redirect, 2));
            load(&state, None);
            assert!(spin_until(|| listed(&state).is_some(), 20), "the redirect hop is stopped: {:?}", state.events.borrow());
            spin_until(|| false, 1);
            assert_eq!(seen.load(Ordering::SeqCst), 0, "the redirect hop never connects to the listed host");
        }
        state.drop_view();
    }
    assert_eq!(seen.load(Ordering::SeqCst), 0, "no request ever reached the listed host");
    let state = view(ShieldLevel::Standard, "private:display-gate-script");
    let web = open(&state);
    state.events.borrow_mut().clear();
    web.load_html(&format!("<html><body><script>location.href = '{target}'</script></body></html>"), Some("http://page.test/"));
    assert!(
        spin_until(|| state.events.borrow().iter().any(|(n, d)| n == "decided" && d["url"] == target.as_str()), 20),
        "the script's navigation reached the gate"
    );
    spin_until(|| false, 1);
    assert_eq!(seen.load(Ordering::SeqCst), 0, "a page's own navigation never reaches the listed host either");
    *state.rules.exceptions.borrow_mut() = ["127.0.0.1".to_owned()].into();
    *state.load.borrow_mut() = Some((target.clone(), 3));
    load(&state, None);
    assert!(spin_until(|| seen.load(Ordering::SeqCst) > 0, 20), "Visit Anyway's exception lets the host through");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_page_already_open_gets_the_warning_once_its_set_arrives() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "private:display-gate-late");
    VIEWS.with_borrow_mut(|views| views.push(Rc::downgrade(&state)));
    open(&state);
    *state.page.borrow_mut() = Url::parse("http://page.test/").ok();
    state.events.borrow_mut().clear();
    list_malware(&["page.test"]);
    assert_eq!(listed(&state).map(|d| d["category"].clone()), Some(json!("malware")));
    state.events.borrow_mut().clear();
    filters::want_sets(Vec::new());
    list_malware(&[]);
    assert!(listed(&state).is_none(), "a switched-off set stops nothing");
}

#[test]
fn a_sites_allow_never_covers_the_pop_ups_of_frames_it_embeds() {
    let state = State::default();
    state.rules.popups.borrow_mut().insert("https://a.test".into(), PopupRule::Allow);
    state.rules.popups.borrow_mut().insert("https://b.test".into(), PopupRule::Block);
    let page = |u: &str| Url::parse(u).ok();
    assert_eq!(opener(&state, page("https://a.test/x").as_ref()).popups, PopupRule::Default);
    assert_eq!(opener(&state, page("https://b.test/x").as_ref()).popups, PopupRule::Block);
    assert_eq!(opener(&state, None).popups, PopupRule::Default);
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn blocking_a_pages_requests_declines_them_until_the_next_navigation() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    state.events.borrow_mut().clear();
    page_eval(&web, "setTimeout(() => { alert('one'); window.second = confirm('two'); }, 0); 'ok'");
    assert!(spin_until(|| event(&state, "prompt").is_some(), 20), "the alert asks");
    let id = event(&state, "prompt").unwrap()["id"].clone();
    answer(&state, &json!({ "id": id, "accept": false, "silence": true }));
    assert!(spin_until(|| page_eval(&web, "String(window.second)") == "false", 20), "the confirm is declined unasked");
    assert_eq!(state.events.borrow().iter().filter(|(n, _)| n == "prompt").count(), 1, "the page asks no more");
    open(&state);
    state.events.borrow_mut().clear();
    page_eval(&web, "setTimeout(() => alert('three'), 0); 'ok'");
    assert!(spin_until(|| event(&state, "prompt").is_some(), 20), "the next page may ask again");
    answer(&state, &json!({ "id": event(&state, "prompt").unwrap()["id"], "accept": false }));
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn only_a_page_without_frames_names_itself_as_the_top_origin() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = state.web().unwrap();
    state.container().set_size_request(300, 200);
    let window = gtk::Window::builder().default_width(400).default_height(300).child(state.container()).build();
    window.present();
    assert!(spin_until(|| web.is_mapped(), 10), "the page is on screen, where WebKit asks for the location");
    web.load_html("<p>where</p>", Some("https://geo.test/"));
    assert!(spin_until(|| event(&state, "finished").is_some(), 30), "the page loaded");
    let locate = "navigator.geolocation.getCurrentPosition(() => {}, () => {}); 'ok'";
    state.events.borrow_mut().clear();
    page_eval(&web, locate);
    assert!(spin_until(|| event(&state, "prompt").is_some(), 20), "the page asks for the location");
    let asked = event(&state, "prompt").unwrap();
    assert_eq!((asked["origin"].as_str(), asked["topOrigin"].as_str()), (Some("https://geo.test"), Some("https://geo.test")));
    answer(&state, &json!({ "id": asked["id"], "accept": false }));
    state.events.borrow_mut().clear();
    web.load_html("<p>where</p><iframe></iframe>", Some("https://geo.test/"));
    assert!(spin_until(|| event(&state, "finished").is_some(), 30), "the page with a frame loaded");
    state.events.borrow_mut().clear();
    page_eval(&web, locate);
    assert!(spin_until(|| event(&state, "prompt").is_some(), 20), "the page asks again");
    let asked = event(&state, "prompt").unwrap();
    assert_eq!(asked["topOrigin"], "", "a frame could be the one asking");
    answer(&state, &json!({ "id": asked["id"], "accept": false }));
    window.destroy();
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_popup_shares_its_openers_prompt_and_popup_limits() {
    gtk::init().expect("a display");
    let opener = Rc::new(State::default());
    *opener.tab.borrow_mut() = "false-1".to_owned();
    let mut popup = TarnWebView(Rc::new(State::default()));
    VIEWS.with_borrow_mut(|views| views.extend([Rc::downgrade(&opener), Rc::downgrade(&popup.0)]));
    popup.set_prop("tab", Some(&json!("false-2")));
    popup.set_prop("burstsOf", Some(&json!("false-1")));
    assert!((0..policy::POPUP_BURST).all(|_| opener.admit_popup()));
    assert!(!popup.0.admit_popup(), "the opener used the popups");
    assert!((0..policy::PROMPT_BURST).all(|_| popup.0.admit_prompt()));
    assert!(!opener.admit_prompt(), "the popup used the prompts");
    let alone = TarnWebView(Rc::new(State::default()));
    assert!(alone.0.admit_popup());
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_snapshot_answers_and_a_scripted_shift_click_does_not_peek() {
    gtk::init().expect("a display");
    let window = gtk::Window::new();
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    window.set_child(Some(state.container()));
    window.set_default_size(800, 600);
    window.present();
    let web = open(&state);
    state.events.borrow_mut().clear();
    snapshot(&state, &web, json!(5));
    assert!(spin_until(|| event(&state, "snapshot").is_some(), 20), "the snapshot answered");
    assert_eq!(event(&state, "snapshot").unwrap()["seq"], 5, "the answer names its request");
    let png = event(&state, "snapshot").unwrap()["png"].clone();
    assert!(png.as_str().is_some_and(|p| p.starts_with("data:image/png;base64,")), "a PNG of the page");
    state.events.borrow_mut().clear();
    page_eval(
        &web,
        "const a = document.createElement('a'); a.href = 'https://peek.test/'; document.body.append(a); a.dispatchEvent(new MouseEvent('click', { shiftKey: true, bubbles: true })); 1",
    );
    spin_until(|| false, 1);
    assert!(event(&state, "peek").is_none(), "only the user's Shift-click peeks");
    window.close();
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_screenshot_saves_a_new_png_under_the_given_name_only() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "private:display-snapshot");
    let window = gtk::Window::builder().default_width(400).default_height(300).child(state.container()).build();
    window.present();
    let web = open(&state);
    assert!(
        spin_until(|| web.is_mapped() && WebViewExt::settings(&web).unwrap().enables_back_forward_navigation_gestures(), 10),
        "touchpad swipes step through history on a page on screen"
    );
    window.set_child(None::<&gtk::Widget>);
    assert!(spin_until(|| !WebViewExt::settings(&web).unwrap().enables_back_forward_navigation_gestures(), 10), "and not once the page is off screen");
    window.set_child(Some(state.container()));
    assert!(spin_until(|| web.is_mapped(), 10), "the page is back on screen");
    state.events.borrow_mut().clear();
    screenshot(&state, &web, &json!({ "seq": 3, "name": "../tarn-display-snapshot.png" }));
    assert!(spin_until(|| event(&state, "screenshot").is_some(), 20), "the screenshot answered");
    let answer = event(&state, "screenshot").unwrap();
    assert_eq!(answer["seq"], 3, "{answer}");
    let path = std::path::PathBuf::from(answer["path"].as_str().unwrap_or_default());
    assert_eq!(path.parent(), Some(session::downloads_dir().as_path()), "{answer}");
    assert!(path.file_name().unwrap().to_string_lossy().starts_with("tarn-display-snapshot"), "{answer}");
    let png = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n") && png.len() > 100);
    state.events.borrow_mut().clear();
    screenshot(&state, &web, &json!({ "seq": 4, "name": "tarn-display-area.png", "x": 10, "y": 10, "width": 50, "height": 40 }));
    assert!(spin_until(|| event(&state, "screenshot").is_some(), 20), "the area screenshot answered");
    let answer = event(&state, "screenshot").unwrap();
    let path = std::path::PathBuf::from(answer["path"].as_str().unwrap_or_default());
    let png = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let side = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
    assert_eq!((side(16), side(20)), (50, 40), "only the area is saved");
    state.events.borrow_mut().clear();
    screenshot(&state, &web, &json!({ "seq": 6, "name": "tarn-display-full.png", "fullPage": true }));
    assert!(spin_until(|| event(&state, "screenshot").is_some(), 20), "the full-page screenshot answered");
    let answer = event(&state, "screenshot").unwrap();
    let path = std::path::PathBuf::from(answer["path"].as_str().unwrap_or_default());
    assert!(std::fs::read(&path).is_ok_and(|png| png.starts_with(b"\x89PNG")), "a full page as measured is saved: {answer}");
    std::fs::remove_file(&path).unwrap();
    state.events.borrow_mut().clear();
    screenshot(&state, &web, &json!({ "seq": 5, "name": "tarn-display-none.png", "x": -1, "y": 0, "width": 50, "height": 40 }));
    assert!(spin_until(|| event(&state, "screenshot").is_some(), 20));
    assert!(event(&state, "screenshot").unwrap()["message"].is_string(), "a malformed area saves nothing");
    window.destroy();
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn imported_cookies_land_in_the_profile_jar_with_every_attribute() {
    use engine::cookies::{Cookie, SameSite};
    gtk::init().expect("a display");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let cookie = |name: &str, domain: &str, host_only: bool, expires: Option<i64>| Cookie {
        name: name.into(),
        value: format!("{name}-value"),
        domain: domain.into(),
        host_only,
        path: "/".into(),
        secure: true,
        http_only: true,
        same_site: SameSite::Strict,
        expires,
        partitioned: false,
    };
    let key = StoreKey::parse("import-display-test").unwrap();
    let run = |cookies: Vec<Cookie>| {
        let rx = RefCell::new(session::import_cookies(&key, cookies, now));
        let answer = Rc::new(Cell::new(None));
        let slot = answer.clone();
        assert!(
            spin_until(
                || {
                    if let Ok(Some(a)) = rx.borrow_mut().try_recv() {
                        slot.set(Some(a));
                    }
                    slot.get().is_some()
                },
                20
            ),
            "the jar answered"
        );
        answer.get().unwrap()
    };
    let host = cookie("host", "a.test", true, Some(now + 3600));
    let domain = cookie("dom", "a.test", false, Some(now + 3600));
    let expired = cookie("old", "a.test", true, Some(now - 10));
    assert_eq!(run(vec![host.clone(), domain.clone(), expired]), (2, 1), "the expired cookie is refused and counted");
    let manager = session::acquire(&key).and_then(|s| s.cookie_manager()).unwrap();
    let jar = Rc::new(RefCell::new(None));
    let slot = jar.clone();
    manager.all_cookies(None::<&webkit6::gio::Cancellable>, move |r| *slot.borrow_mut() = Some(r.unwrap()));
    assert!(spin_until(|| jar.borrow().is_some(), 20), "the jar was read");
    let mut stored: Vec<Cookie> = jar.take().unwrap().into_iter().filter_map(|mut c| session::from_soup(&mut c)).collect();
    stored.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(stored, [domain.clone(), host.clone()], "both kept every attribute");
    assert_eq!(run(vec![Cookie { value: "replaced".into(), ..host }]), (0, 1), "an import never replaces a cookie the jar holds");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn a_filter_update_with_a_group_that_does_not_compile_swaps_no_group() {
    gtk::init().expect("a display");
    let ready = Rc::new(Cell::new(false));
    let flag = ready.clone();
    filters::when_groups_ready(Box::new(move || flag.set(true)));
    assert!(spin_until(|| ready.get(), 60), "the installed lists loaded");
    let rule = r#"[{"trigger":{"url-filter":"replaced\\.test"},"action":{"type":"block"}}]"#;
    let group = |name, key: &str, chunk: &str| shield::groups::Group { name, key: key.repeat(16), chunks: vec![chunk.to_owned()] };
    let replace = |groups| {
        let out = Rc::new(RefCell::new(None));
        let slot = out.clone();
        filters::replace(groups, move |r| *slot.borrow_mut() = Some(r));
        assert!(spin_until(|| out.borrow().is_some(), 60), "the update finished");
        out.take().unwrap()
    };
    let ads = || {
        let out = Rc::new(RefCell::new(None));
        let slot = out.clone();
        filters::with_filters(ShieldLevel::Standard, vec!["ads".into()], move |f| *slot.borrow_mut() = Some(f));
        assert!(spin_until(|| out.borrow().is_some(), 60), "the filters compiled");
        out.take().unwrap().unwrap().iter().skip(1).filter_map(UserContentFilter::identifier).map(|id| id.to_string()).collect::<Vec<_>>()
    };

    assert_eq!(replace(vec![group("ads", "a", rule), group("trackers", "b", rule)]), Ok(()));
    assert_eq!(ads(), ["tarn-ads-aaaaaaaaaaaaaaaa-0"]);
    assert!(replace(vec![group("ads", "c", rule), group("trackers", "d", "not json")]).is_err(), "a list that does not compile fails the update");
    assert_eq!(ads(), ["tarn-ads-aaaaaaaaaaaaaaaa-0"], "no list of a failed update replaces the installed one");
    assert!(!filters::stored("tarn-ads-cccccccccccccccc-0"), "and the list it compiled is not kept");
    assert!(filters::stored("tarn-ads-aaaaaaaaaaaaaaaa-0"), "while the installed one stays");
    let split = shield::groups::Group { name: "trackers", key: "e".repeat(16), chunks: vec![rule.to_owned(), "not json".to_owned()] };
    assert!(replace(vec![split]).is_err());
    assert!(!filters::stored("tarn-trackers-eeeeeeeeeeeeeeee-0"), "nor the chunks of a list that failed partway");
}

#[test]
#[ignore = "needs a display and WebKitGTK; run scripts/test-display.sh"]
fn closing_find_keeps_the_last_match_selected() {
    gtk::init().expect("a display");
    let state = view(ShieldLevel::Standard, "burner:0123456789abcdef0123456789abcdef");
    let web = open(&state);
    state.events.borrow_mut().clear();
    find(&state, &json!({ "text": "zebrafish" }));
    assert!(spin_until(|| event(&state, "found").is_some(), 20), "find counted");
    assert_eq!(event(&state, "found").unwrap()["matched"], 1);
    assert!(spin_until(|| page_eval(&web, "String(getSelection())") == "zebrafish", 5), "find selected the match");
    clear_find(&state);
    assert!(!spin_until(|| page_eval(&web, "String(getSelection())") != "zebrafish", 5), "the last match stays selected");
    assert!(state.find_text.borrow().is_empty(), "the next find starts afresh");
}
