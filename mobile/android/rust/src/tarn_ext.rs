//! The engine side of Tarn extensions on Android WebView (`docs/mobile-plan.md`, Android). The
//! shell registers its gate, worlds, rule lists and `tarn-extension://` handler here; the Java
//! component reaches them through `Native`. The shell runs on NSH's engine thread, not on the
//! Android main thread of the WebView callbacks, so the gate and the scripts stay on the engine
//! thread and a frame's question reaches them as the `extensionFrame` event (`frame_event`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{RwLock, RwLockReadGuard};

use engine::content_rules::{self, Request, RuleList, Verdict};
use engine::handover::js_string;
use engine::tarn_ext::{FrameNav, POST, RECEIVE, Resource, Resources, RunAt, SCHEME, World, WorldScripts};
use serde_json::Value;
use url::Url;

use crate::policy;

pub use engine::tarn_ext::MAX_MESSAGE_BYTES;

/// `(tab, store_id, world, frame)` → the indexes of the world's scripts allowed in that frame.
pub type Gate = Rc<dyn Fn(&str, &str, &World, &FrameNav<'_>) -> Vec<usize>>;

/// The name of the `addWebMessageListener` object in an extension world or page.
pub const PORT: &str = "__tarnExtPort";
const TAKE: &str = "__tarnExtTake";
/// What the loader sends first in every document: `START` followed by the document's URL.
pub const START: &str = "start ";

thread_local! {
    static GATE: RefCell<Option<Gate>> = const { RefCell::new(None) };
    static WORLDS: RefCell<HashMap<String, Vec<Rc<WorldScripts>>>> = RefCell::new(HashMap::new());
}

type Lists = HashMap<String, Vec<RuleList>>;

static NAMES: RwLock<Option<HashMap<String, Vec<String>>>> = RwLock::new(None);
static RULES: RwLock<Option<Lists>> = RwLock::new(None);
static RESOURCES: RwLock<Option<HashMap<String, Resources>>> = RwLock::new(None);
static WORLDS_SUPPORTED: AtomicBool = AtomicBool::new(false);

fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|e| e.into_inner())
}

pub fn set_gate(gate: Gate) {
    GATE.with(|g| *g.borrow_mut() = Some(gate));
}

/// Replaces the worlds of every view whose `profile` is `store_id`.
pub fn set_scripts(store_id: &str, worlds: Vec<Rc<WorldScripts>>) {
    let names: Vec<String> = worlds.iter().map(|s| s.world.name().to_owned()).collect();
    WORLDS.with(|w| {
        let mut table = w.borrow_mut();
        if worlds.is_empty() {
            table.remove(store_id);
        } else {
            table.insert(store_id.to_owned(), worlds);
        }
    });
    let mut table = NAMES.write().unwrap_or_else(|e| e.into_inner());
    let table = table.get_or_insert_with(HashMap::new);
    if names.is_empty() {
        table.remove(store_id);
    } else {
        table.insert(store_id.to_owned(), names);
    }
    crate::host::call_string("extensionScriptsChanged", store_id);
}

/// Replaces the declarativeNetRequest lists `(install id, content rule list JSON)` of `store_id`;
/// a list that does not parse is left out.
pub fn set_rule_lists(store_id: &str, lists: Vec<(String, String)>) {
    let parsed: Vec<RuleList> = lists.into_iter().filter_map(|(_, json)| RuleList::parse(&json).ok()).collect();
    let mut table = RULES.write().unwrap_or_else(|e| e.into_inner());
    let table = table.get_or_insert_with(HashMap::new);
    if parsed.is_empty() {
        table.remove(store_id);
    } else {
        table.insert(store_id.to_owned(), parsed);
    }
}

pub fn serve(store_id: &str, resources: Resources) {
    RESOURCES.write().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(store_id.to_owned(), resources);
}

pub fn worlds_supported() -> bool {
    WORLDS_SUPPORTED.load(Ordering::Relaxed)
}

pub(crate) fn set_worlds_supported(supported: bool) {
    WORLDS_SUPPORTED.store(supported, Ordering::Relaxed);
}

pub(crate) fn world_names(store_id: &str) -> Vec<String> {
    read(&NAMES).as_ref().and_then(|t| t.get(store_id).cloned()).unwrap_or_default()
}

/// The `extensionFrame` event of a `tarn.webview`, handled on the shell's thread: asks the gate
/// and hands the frame's scripts to the component (`Host.extensionFrameReady`), an empty list
/// when nothing may run.
pub fn frame_event(data: &Value) {
    let text = |key: &str| data.get(key).and_then(Value::as_str);
    let Some(request) = text("request") else { return };
    let units = match (text("tab"), text("store"), text("world"), text("url"), text("origin")) {
        (Some(tab), Some(store), Some(world), Some(url), Some(origin)) => {
            let main_frame = data.get("main").and_then(Value::as_bool).unwrap_or(false);
            frame(tab, store, world, url, text("top").filter(|t| !t.is_empty()), main_frame, origin)
        }
        _ => Vec::new(),
    };
    let flat: Vec<String> = units.into_iter().flat_map(|(phase, code)| [phase.name().to_owned(), code]).collect();
    crate::host::frame_ready(request, &flat);
}

/// Runs first in every document of an extension world: it keeps the port from the world's
/// globals, reports the document and its parsing phases, and hands the port once to `setup`.
pub fn loader() -> String {
    format!(
        "(() => {{ const port = globalThis.{PORT}; if (!port || !/^https?:$/.test(location.protocol)) return; \
        try {{ delete globalThis.{PORT}; }} catch (e) {{}} \
        Object.defineProperty(globalThis, '{TAKE}', {{ configurable: true, value: () => {{ delete globalThis.{TAKE}; return port; }} }}); \
        port.postMessage({start} + location.href); \
        const end = () => {{ port.postMessage('end'); setTimeout(() => port.postMessage('idle'), 0); }}; \
        if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', end, {{ once: true }}); else end(); }})();",
        start = js_string(START)
    )
}

fn setup(take: &str, bootstrap: &str) -> String {
    format!(
        "(() => {{ const port = {take}; if (!port) return; \
        globalThis.{POST} = s => port.postMessage(String(s)); \
        port.onmessage = e => {{ const r = globalThis.{RECEIVE}; if (typeof r === 'function' && typeof e.data === 'string') r(e.data); }}; }})();\n{bootstrap}"
    )
}

/// The document-start script of an extension page: the port of its main world, then the
/// bootstrap, only in the top document of `tarn-extension://<host>/`. WebView takes no origin rule
/// for a custom scheme, so the script is offered to every frame and checks where it is itself.
pub fn page_script(host: &str, bootstrap: &str) -> String {
    format!(
        "(() => {{ if (window !== window.top || location.protocol !== '{SCHEME}:' || location.host !== {}) return;\n{}\n}})();",
        js_string(host),
        setup(&format!("globalThis.{PORT}"), bootstrap)
    )
}

fn style(css: &str) -> String {
    format!(
        "(() => {{ const s = document.createElement('style'); s.textContent = {}; \
        const add = () => {{ const r = document.head || document.documentElement; if (r) r.append(s); return !!r; }}; \
        if (!add()) new MutationObserver((_, o) => {{ if (add()) o.disconnect(); }}).observe(document, {{ childList: true, subtree: true }}); }})();",
        js_string(css)
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Start,
    End,
    Idle,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::Start => "start",
            Phase::End => "end",
            Phase::Idle => "idle",
        }
    }
}

/// What runs in one frame of `world` after its loader reported `url` from `origin`: the setup
/// and bootstrap, then each allowed script's CSS (at document start) and JS (at its `run_at`),
/// each as its own classic script in the world. Empty unless the gate allows something.
pub fn frame(tab: &str, store_id: &str, world: &str, url: &str, top: Option<&str>, main_frame: bool, origin: &str) -> Vec<(Phase, String)> {
    let Some(world) = World::parse(world) else { return Vec::new() };
    let Ok(url) = Url::parse(url) else { return Vec::new() };
    if !matches!(url.scheme(), "http" | "https") || url.origin().ascii_serialization() != origin.trim_end_matches('/') {
        return Vec::new();
    }
    let top = if main_frame { Some(url.clone()) } else { top.and_then(|t| Url::parse(t).ok()) };
    let Some(top) = top else { return Vec::new() };
    let scripts = WORLDS.with(|w| w.borrow().get(store_id).and_then(|list| list.iter().find(|s| s.world == world).cloned()));
    let gate = GATE.with(|g| g.borrow().clone());
    let (Some(scripts), Some(gate)) = (scripts, gate) else { return Vec::new() };
    let mut allowed = gate(tab, store_id, &world, &FrameNav { url: &url, top: &top, main_frame });
    allowed.sort_unstable();
    allowed.dedup();
    let chosen: Vec<_> = allowed.iter().filter_map(|&i| scripts.scripts.get(i)).filter(|s| main_frame || s.all_frames).collect();
    if chosen.is_empty() {
        return Vec::new();
    }
    let mut units = vec![(Phase::Start, setup(&format!("globalThis.{TAKE} && globalThis.{TAKE}()"), &scripts.bootstrap))];
    units.extend(chosen.iter().flat_map(|s| s.css.iter()).map(|css| (Phase::Start, style(css))));
    for script in &chosen {
        let phase = match script.run_at {
            RunAt::DocumentStart => Phase::Start,
            RunAt::DocumentEnd => Phase::End,
            RunAt::DocumentIdle => Phase::Idle,
        };
        units.extend(script.js.iter().map(|js| (phase, js.clone())));
    }
    units
}

/// The verdict of `store_id`'s extension rule lists for one request of a page (`top`, view level);
/// extensions never act in a Lockdown-level view. While a navigation is pending the request may
/// belong to the page being left or the one coming, so a rule acts only if it acts for each.
pub fn verdict(store_id: &str, url: &str, kind: i32, pages: &[(Option<&str>, i32)]) -> i32 {
    let verdicts: Vec<i32> = pages.iter().map(|(top, level)| page_verdict(store_id, url, *top, kind, *level)).collect();
    if verdicts.is_empty() || verdicts.contains(&policy::VERDICT_ALLOW) {
        policy::VERDICT_ALLOW
    } else if verdicts.iter().all(|v| *v == policy::VERDICT_BLOCK) {
        policy::VERDICT_BLOCK
    } else {
        policy::VERDICT_MAKE_HTTPS
    }
}

fn page_verdict(store_id: &str, url: &str, top: Option<&str>, kind: i32, level: i32) -> i32 {
    if policy::level(level) == shield::ShieldLevel::Lockdown {
        return policy::VERDICT_ALLOW;
    }
    let table = read(&RULES);
    let Some(lists) = table.as_ref().and_then(|t| t.get(store_id)) else { return policy::VERDICT_ALLOW };
    let Ok(parsed) = Url::parse(url) else { return policy::VERDICT_BLOCK };
    let top = top.and_then(|t| Url::parse(t).ok());
    let request = Request::new(&parsed, top.as_ref(), policy::resource_kind(kind));
    match content_rules::evaluate(lists, &request) {
        Verdict::Allow => policy::VERDICT_ALLOW,
        Verdict::Block => policy::VERDICT_BLOCK,
        Verdict::MakeHttps => policy::VERDICT_MAKE_HTTPS,
    }
}

/// `store_id`'s `tarn-extension://` resource at `url`; 404 without a handler. WebView parses the
/// custom scheme without a host and gives its documents an opaque origin, which neither `'self'`
/// nor a host source matches, so the policy names the scheme; the page view refuses every
/// `tarn-extension://` request to another origin, so the same scripts are allowed.
pub fn resource(store_id: &str, url: &str) -> (Resource, Option<String>) {
    let resource = handled(store_id, url);
    let csp = resource.csp.map(|csp| csp.replace("'self'", &format!("{SCHEME}:")));
    (resource, csp)
}

fn handled(store_id: &str, url: &str) -> Resource {
    let missing = |status| Resource { status, mime: "text/plain", csp: Some("default-src 'none'"), body: Vec::new() };
    let Ok(url) = Url::parse(url) else { return missing(400) };
    if url.scheme() != SCHEME {
        return missing(403);
    }
    let handler = read(&RESOURCES).as_ref().and_then(|t| t.get(store_id).cloned());
    handler.map_or_else(|| missing(404), |h| h(&url))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::Arc;

    use engine::tarn_ext::Script;

    use super::*;

    const STORE: &str = "6f1c2a9e-3b7d-4c55-9a0e-1d2f3a4b5c6d";

    fn worlds() -> Rc<WorldScripts> {
        Rc::new(WorldScripts {
            world: World::for_install("abc1").unwrap(),
            bootstrap: "BOOT();".into(),
            scripts: vec![
                Script { run_at: RunAt::DocumentIdle, all_frames: false, js: vec!["IDLE();".into()], css: vec!["p{color:red}".into()] },
                Script { run_at: RunAt::DocumentStart, all_frames: true, js: vec!["A();".into(), "B();".into()], css: Vec::new() },
                Script { run_at: RunAt::DocumentEnd, all_frames: false, js: vec!["END();".into()], css: Vec::new() },
            ],
        })
    }

    fn install(store: &str, gate: Gate) {
        set_gate(gate);
        set_scripts(store, vec![worlds()]);
    }

    #[test]
    fn a_frame_gets_only_what_the_gate_allows_in_order() {
        let asked = Rc::new(Cell::new(None));
        let seen = asked.clone();
        install(
            STORE,
            Rc::new(move |tab, store, world, nav| {
                seen.set(Some((tab.to_owned(), store.to_owned(), world.name().to_owned(), nav.url.to_string(), nav.top.to_string(), nav.main_frame)));
                vec![2, 0, 0, 9]
            }),
        );
        let units = frame("n1", STORE, "tarn-ext-abc1", "https://a.test/x", None, true, "https://a.test");
        assert_eq!(asked.take(), Some(("n1".into(), STORE.into(), "tarn-ext-abc1".into(), "https://a.test/x".into(), "https://a.test/x".into(), true)));
        let phases: Vec<Phase> = units.iter().map(|u| u.0).collect();
        assert_eq!(phases, [Phase::Start, Phase::Start, Phase::Idle, Phase::End]);
        assert!(units[0].1.contains(TAKE) && units[0].1.ends_with("BOOT();"), "the bootstrap comes first, after the port");
        assert!(units[1].1.contains("p{color:red}"));
        assert_eq!((units[2].1.as_str(), units[3].1.as_str()), ("IDLE();", "END();"));
        assert_eq!(world_names(STORE), ["tarn-ext-abc1"]);
    }

    #[test]
    fn subframes_get_only_all_frames_scripts_under_their_top() {
        install("sub", Rc::new(|_, _, _, nav| if nav.top.host_str() == Some("top.test") { vec![0, 1, 2] } else { Vec::new() }));
        let units = frame("n1", "sub", "tarn-ext-abc1", "https://f.test/", Some("https://top.test/"), false, "https://f.test");
        assert_eq!(units.iter().map(|u| u.1.as_str()).skip(1).collect::<Vec<_>>(), ["A();", "B();"]);
        assert!(frame("n1", "sub", "tarn-ext-abc1", "https://f.test/", None, false, "https://f.test").is_empty(), "no top, nothing");
    }

    #[test]
    fn nothing_runs_without_a_matching_origin_world_store_or_gate() {
        const STORE: &str = "gated";
        install(STORE, Rc::new(|_, _, _, _| vec![0, 1, 2]));
        let run = |world: &str, url: &str, origin: &str, store: &str| frame("n1", store, world, url, None, true, origin);
        assert!(!run("tarn-ext-abc1", "https://a.test/", "https://a.test", STORE).is_empty());
        assert!(run("tarn-ext-abc1", "https://a.test/", "https://evil.test", STORE).is_empty(), "the URL must be the engine's origin");
        assert!(run("tarn-ext-abc1", "http://a.test/", "https://a.test", STORE).is_empty());
        assert!(run("tarn-ext-abc1", "data:text/html,x", "null", STORE).is_empty());
        assert!(run("tarn-ext-other", "https://a.test/", "https://a.test", STORE).is_empty());
        assert!(run("main", "https://a.test/", "https://a.test", STORE).is_empty());
        assert!(run("tarn-ext-abc1", "https://a.test/", "https://a.test", "other-store").is_empty());
        set_gate(Rc::new(|_, _, _, _| Vec::new()));
        assert!(run("tarn-ext-abc1", "https://a.test/", "https://a.test", STORE).is_empty(), "an empty answer runs not even the bootstrap");
        assert_eq!(world_names(STORE), ["tarn-ext-abc1"], "world names are readable from any thread");
        set_scripts(STORE, Vec::new());
        assert!(world_names(STORE).is_empty());
        frame_event(
            &serde_json::json!({"request": "1", "tab": "n1", "store": STORE, "world": "tarn-ext-abc1", "url": "https://a.test/", "main": true, "origin": "https://a.test"}),
        );
    }

    #[test]
    fn scripts_reach_the_page_only_through_the_world() {
        let loader = loader();
        assert!(loader.contains("delete globalThis.__tarnExtPort") && loader.contains("^https?:$"));
        assert!(!loader.contains("eval(") && !loader.contains("new Function"));
        let page = page_script("abc", "BOOT();");
        assert!(page.contains(POST) && page.contains(RECEIVE) && page.contains("BOOT();\n})();"));
        assert!(page.starts_with("(() => { if (window !== window.top || location.protocol !== 'tarn-extension:' || location.host !== \"abc\") return;"));
        assert!(!style("</style><script>").contains("</style>"), "CSS stays a string");
    }

    #[test]
    fn extension_rules_act_per_store_and_never_at_lockdown() {
        let rules = shield::dnr::compile(&[br#"[{"id":1,"action":{"type":"block"},"condition":{"urlFilter":"||ads.test^"}}]"#]).unwrap();
        let list = rules.content_rules(&shield::dnr::Scope::Hosts(vec!["page.test".into()])).unwrap();
        set_rule_lists("s1", vec![("i1".into(), list), ("i2".into(), "not json".into())]);
        let page = |top: &'static str, level: i32| [(Some(top), level)];
        assert_eq!(verdict("s1", "https://ads.test/x.js", 4, &page("https://page.test/", 0)), policy::VERDICT_BLOCK);
        assert_eq!(verdict("s1", "https://ads.test/x.js", 4, &page("https://page.test/", 2)), policy::VERDICT_ALLOW, "not at Lockdown");
        assert_eq!(verdict("s1", "https://ads.test/x.js", 4, &page("https://other.test/", 0)), policy::VERDICT_ALLOW, "only on granted sites");
        assert_eq!(verdict("s1", "https://ok.test/x.js", 4, &page("https://page.test/", 0)), policy::VERDICT_ALLOW);
        assert_eq!(verdict("s2", "https://ads.test/x.js", 4, &page("https://page.test/", 0)), policy::VERDICT_ALLOW);
        let leaving = (Some("https://page.test/"), 0);
        assert_eq!(
            verdict("s1", "https://ads.test/x.js", 4, &[leaving, (Some("https://other.test/"), 0)]),
            policy::VERDICT_ALLOW,
            "nor while one leaves for another site"
        );
        assert_eq!(verdict("s1", "https://ads.test/x.js", 4, &[leaving, (Some("https://page.test/b"), 2)]), policy::VERDICT_ALLOW);
        assert_eq!(verdict("s1", "https://ads.test/x.js", 4, &[leaving, (Some("https://page.test/b"), 0)]), policy::VERDICT_BLOCK);
        assert_eq!(verdict("s1", "https://ads.test/x.js", 4, &[]), policy::VERDICT_ALLOW);
        set_rule_lists("s1", Vec::new());
        assert_eq!(verdict("s1", "https://ads.test/x.js", 4, &page("https://page.test/", 0)), policy::VERDICT_ALLOW);
    }

    #[test]
    fn resources_come_only_from_the_stores_handler() {
        serve("s1", Arc::new(|u: &Url| Resource { status: 200, mime: "text/html", csp: Some("script-src 'self'"), body: u.path().as_bytes().to_vec() }));
        let (page, csp) = resource("s1", "tarn-extension://abc/popup.html");
        assert_eq!((page.status, page.body.as_slice()), (200, b"/popup.html".as_slice()));
        assert_eq!(csp.as_deref(), Some("script-src tarn-extension:"));
        assert_eq!(resource("s2", "tarn-extension://abc/popup.html").0.status, 404);
        assert_eq!(resource("s1", "https://abc/popup.html").0.status, 403);
        assert_eq!(resource("s1", "not a url").0.status, 400);
    }
}
