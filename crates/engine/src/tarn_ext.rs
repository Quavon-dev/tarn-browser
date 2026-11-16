//! The web view contract for Tarn extensions (`docs/shared-core.md` §1). Every engine supplies
//! only the transport: a `__tarnExtPost(string)` function in the extension's isolated world (or
//! in the main world of its own pages) that delivers to the shell as `Event::ExtensionMessage`,
//! and `post_extension_message`, which calls `__tarnExtReceive(string)` in that world. The
//! sender's identity is where the message arrived, never its body.

use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::Value;
use url::Url;

pub const SCHEME: &str = "tarn-extension";
pub const WORLD_PREFIX: &str = "tarn-ext-";
pub const POST: &str = "__tarnExtPost";
pub const RECEIVE: &str = "__tarnExtReceive";
pub const MAX_MESSAGE_BYTES: usize = 1 << 20;
/// Starts the messages an engine's own setup code sends; the shim only sends JSON objects.
pub const HELLO: char = '\u{1}';
const SHIM: &str = include_str!("tarn_ext_shim.js");
const RUN: &str = "__tarnExtRun";
const MAX_KEY_BYTES: usize = 8 << 10;
pub(crate) const MAX_DECISIONS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct World(String);

impl World {
    pub fn for_install(install_id: &str) -> Option<Self> {
        let valid = !install_id.is_empty() && install_id.len() <= 64 && install_id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        valid.then(|| World(format!("{WORLD_PREFIX}{install_id}")))
    }

    pub fn parse(name: &str) -> Option<Self> {
        World::for_install(name.strip_prefix(WORLD_PREFIX)?)
    }

    pub fn name(&self) -> &str {
        &self.0
    }

    pub fn install_id(&self) -> &str {
        &self.0[WORLD_PREFIX.len()..]
    }
}

/// A frame of one view, numbered by the engine; the main frame is `MAIN`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FrameId(pub u64);

impl FrameId {
    pub const MAIN: FrameId = FrameId(0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunAt {
    DocumentStart,
    DocumentEnd,
    DocumentIdle,
}

/// One `content_scripts` entry: its sources, injected in this order after the bootstrap.
#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub run_at: RunAt,
    pub all_frames: bool,
    pub js: Vec<String>,
    pub css: Vec<String>,
}

/// Everything one install injects; `bootstrap` runs first in the world, at document start, in
/// every frame that gets any of `scripts`.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldScripts {
    pub world: World,
    pub bootstrap: String,
    pub scripts: Vec<Script>,
}

/// A frame about to get a document. `top` is the main frame's URL (the frame's own for it).
#[derive(Clone, Copy, Debug)]
pub struct FrameNav<'a> {
    pub url: &'a Url,
    pub top: &'a Url,
    pub main_frame: bool,
}

/// Asked synchronously for every frame navigation: the indexes of the world's `scripts` that may
/// run there. An engine never injects anything this has not returned.
pub type Gate = Rc<dyn Fn(&World, &FrameNav) -> Vec<usize>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageKind {
    Background,
    Popup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub status: u16,
    pub mime: &'static str,
    pub csp: Option<&'static str>,
    pub body: Vec<u8>,
}

/// The profile's `tarn-extension://` handler; engines may call it off the main thread.
pub type Resources = Arc<dyn Fn(&Url) -> Resource + Send + Sync>;

/// What an extension page view (`PageKind`) is made with: the install's world and origin, the
/// bootstrap for its main world (`bootstrap(true)`) and the profile's handler, which the view
/// limits to `origin`.
#[derive(Clone)]
pub struct PageSpec {
    pub world: World,
    pub origin: String,
    pub bootstrap: String,
    pub resources: Resources,
    pub inspectable: bool,
}

/// What every web view implements for Tarn extensions, beside `Event::ExtensionMessage`.
pub trait ExtensionView {
    fn set_extension_gate(&self, gate: Gate);
    fn add_extension_scripts(&self, scripts: Rc<WorldScripts>);
    fn remove_extension_scripts(&self, world: &World);
    /// `None`: every frame of the view where the world runs.
    fn post_extension_message(&self, world: &World, frame: Option<FrameId>, json: &str);
}

/// Whether an extension page view of `origin` may load `url` into a frame: its own origin, or a
/// blank subframe; never a web document.
pub fn page_loads(url: &Url, origin: &str, main_frame: bool) -> bool {
    (url.scheme() == SCHEME && url.host_str() == Some(origin)) || (!main_frame && url.scheme() == "about")
}

/// Whether a link or `tabs.create` of an extension opens as a tab: a web page `nav_policy` allows,
/// never a private-network address (S10).
pub fn opens_as_tab(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && crate::nav_policy::decide(url, crate::nav_policy::Source::Link) == crate::nav_policy::Decision::Allow
        && !shield::rules::is_local(url, true)
}

/// The shim that defines `chrome.*` and `browser.*` from `config`, made by the core.
pub fn bootstrap(config: &Value) -> String {
    format!("{SHIM}({config});\n")
}

/// What a document of `url` looks up in the world's decisions, as its script computes it.
pub fn frame_key(url: &Url, main_frame: bool) -> Option<String> {
    let mut url = url.clone();
    url.set_fragment(None);
    let key = format!("{} {}", if main_frame { 'm' } else { 's' }, url.as_str());
    (key.len() <= MAX_KEY_BYTES).then_some(key)
}

/// The gate's latest answers for one world, by frame key. An engine asks the gate as a frame
/// navigates and carries the answers into the world's scripts before the document starts; a
/// document whose key is missing runs nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decisions(VecDeque<(String, Vec<usize>)>);

impl Decisions {
    /// Records the gate's answer for `frame`; true when the world's scripts must be replaced.
    pub fn record(&mut self, frame: &FrameNav, run: Vec<usize>) -> bool {
        let Some(key) = frame_key(frame.url, frame.main_frame) else { return false };
        match (self.0.iter().position(|(k, _)| *k == key), run.is_empty()) {
            (Some(at), _) if self.0[at].1 == run => false,
            (Some(at), true) => {
                self.0.remove(at);
                true
            }
            (Some(at), false) => {
                self.0[at].1 = run;
                true
            }
            (None, true) => false,
            (None, false) => {
                if self.0.len() >= MAX_DECISIONS {
                    self.0.pop_front();
                }
                self.0.push_back((key, run));
                true
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// One script an engine registers in the world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Injection {
    pub source: String,
    pub run_at: RunAt,
    pub all_frames: bool,
}

/// The world's scripts in the order an engine registers them: the gate, then each content script
/// file.
pub fn injections(scripts: &WorldScripts, decisions: &Decisions, setup: &str) -> Vec<Injection> {
    let mut out = vec![gate_injection(scripts, decisions, setup)];
    out.extend(content_injections(scripts));
    out
}

/// The gate, at document start in every frame: it looks the document up in `decisions`, runs the
/// engine's `setup` (it must define `POST`, or `return` to run nothing), the bootstrap and the
/// style sheets.
pub fn gate_injection(scripts: &WorldScripts, decisions: &Decisions, setup: &str) -> Injection {
    let runs: Vec<(&str, &[usize])> = decisions.0.iter().map(|(k, run)| (k.as_str(), run.as_slice())).collect();
    let css: Vec<&[String]> = scripts.scripts.iter().map(|s| s.css.as_slice()).collect();
    let gate = format!(
        "(() => {{\n\
         const run = new Map({runs}).get((window === window.top ? 'm ' : 's ') + location.href.split('#')[0]);\n\
         if (!run) return;\n\
         {setup}\n\
         Object.defineProperty(globalThis, '{RUN}', {{ value: Object.freeze(run) }});\n\
         {bootstrap}\
         const css = {css};\n\
         const style = () => {{ for (const i of run) for (const text of css[i] || []) {{ const s = document.createElement('style'); s.textContent = text; (document.head || document.documentElement).append(s); }} }};\n\
         if (document.documentElement) style(); else new MutationObserver((_, o) => {{ if (document.documentElement) {{ o.disconnect(); style(); }} }}).observe(document, {{ childList: true }});\n\
         }})();\n",
        runs = serde_json::to_string(&runs).unwrap_or_else(|_| "[]".into()),
        bootstrap = scripts.bootstrap,
        css = serde_json::to_string(&css).unwrap_or_else(|_| "[]".into()),
    );
    Injection { source: gate, run_at: RunAt::DocumentStart, all_frames: true }
}

/// Each content script file, which runs only where the gate let its entry through.
pub fn content_injections(scripts: &WorldScripts) -> Vec<Injection> {
    scripts
        .scripts
        .iter()
        .enumerate()
        .flat_map(|(ix, script)| script.js.iter().map(move |js| Injection { source: guarded(ix, js), run_at: script.run_at, all_frames: script.all_frames }))
        .collect()
}

/// `source` with a first statement that ends the script unless the gate let entry `ix` run. A
/// script that throws runs nothing after the throw; its file keeps its own top level, so it shares
/// globals with the entry's other files as in Chrome. A leading `'use strict'` stays a directive.
fn guarded(ix: usize, source: &str) -> String {
    let guard = format!("if (!(globalThis.{RUN} || []).includes({ix})) throw new Error('Tarn: not granted here.');");
    let start = skip_comments(source);
    let rest = &source[start..];
    match ["'use strict'", "\"use strict\""].iter().find(|d| rest.starts_with(**d)) {
        Some(directive) => {
            let end = start + directive.len();
            let end = if source[end..].starts_with(';') { end + 1 } else { end };
            let separator = if source[..end].ends_with(';') { "" } else { ";" };
            format!("{}{separator}{guard}{}", &source[..end], &source[end..])
        }
        None => format!("{guard}{source}"),
    }
}

fn skip_comments(source: &str) -> usize {
    let mut at = 0;
    loop {
        let rest = &source[at..];
        let trimmed = rest.trim_start();
        at += rest.len() - trimmed.len();
        if let Some(line) = trimmed.strip_prefix("//") {
            at += 2 + line.find('\n').unwrap_or(line.len());
        } else if let Some(block) = trimmed.strip_prefix("/*") {
            match block.find("*/") {
                Some(end) => at += 2 + end + 2,
                None => return 0,
            }
        } else {
            return at;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worlds_name_one_install() {
        let world = World::for_install("0123abcd").unwrap();
        assert_eq!(world.name(), "tarn-ext-0123abcd");
        assert_eq!(world.install_id(), "0123abcd");
        assert_eq!(World::parse("tarn-ext-0123abcd"), Some(world));
        for bad in ["", "A1", "a/b", "a-b", "ab\n"] {
            assert!(World::for_install(bad).is_none(), "{bad:?}");
        }
        assert!(World::parse("other-0123").is_none());
    }

    fn scripts() -> WorldScripts {
        WorldScripts {
            world: World::for_install("ab12").unwrap(),
            bootstrap: "BOOT;\n".into(),
            scripts: vec![
                Script {
                    run_at: RunAt::DocumentStart,
                    all_frames: false,
                    js: vec!["'use strict';\nlet a = 1;".into(), "// c\nwindow.b = 2;".into()],
                    css: vec!["p{}".into()],
                },
                Script { run_at: RunAt::DocumentIdle, all_frames: true, js: vec!["/* x */ \"use strict\"\nwindow.c = 3;".into()], css: vec![] },
            ],
        }
    }

    #[test]
    fn decisions_follow_the_gate_and_stay_bounded() {
        let (url, top) = (Url::parse("https://a.test/p?q#frag").unwrap(), Url::parse("https://a.test/").unwrap());
        let main = FrameNav { url: &url, top: &top, main_frame: true };
        let mut d = Decisions::default();
        assert!(!d.record(&main, vec![]));
        assert!(d.record(&main, vec![0]));
        assert!(!d.record(&main, vec![0]));
        assert_eq!(frame_key(&url, true).as_deref(), Some("m https://a.test/p?q"));
        assert!(d.record(&FrameNav { main_frame: false, ..main }, vec![1]));
        assert!(d.record(&main, vec![]));
        assert_eq!(d.0.len(), 1);
        for n in 0..100 {
            let u = Url::parse(&format!("https://a.test/{n}")).unwrap();
            d.record(&FrameNav { url: &u, top: &top, main_frame: true }, vec![0]);
        }
        assert_eq!(d.0.len(), MAX_DECISIONS);
        let long = Url::parse(&format!("https://a.test/{}", "x".repeat(MAX_KEY_BYTES))).unwrap();
        assert!(!d.record(&FrameNav { url: &long, top: &top, main_frame: true }, vec![0]));
    }

    #[test]
    fn content_scripts_run_only_behind_the_gate() {
        let url = Url::parse("https://a.test/").unwrap();
        let mut d = Decisions::default();
        d.record(&FrameNav { url: &url, top: &url, main_frame: true }, vec![0, 1]);
        let out = injections(&scripts(), &d, "SETUP;");
        assert_eq!(out.len(), 4);
        let gate = &out[0];
        assert_eq!((gate.run_at, gate.all_frames), (RunAt::DocumentStart, true));
        assert!(gate.source.contains(r#"new Map([["m https://a.test/",[0,1]]])"#));
        let (setup, boot, run) = (gate.source.find("SETUP;").unwrap(), gate.source.find("BOOT;").unwrap(), gate.source.find("if (!run) return;").unwrap());
        assert!(run < setup && setup < boot);
        assert!(gate.source.contains(r#"const css = [["p{}"],[]]"#));
        assert_eq!(out[1].source, "'use strict';if (!(globalThis.__tarnExtRun || []).includes(0)) throw new Error('Tarn: not granted here.');\nlet a = 1;");
        assert!(out[2].source.starts_with("if (!(globalThis.__tarnExtRun || []).includes(0)) throw") && out[2].source.ends_with("// c\nwindow.b = 2;"));
        assert!(out[3].source.starts_with("/* x */ \"use strict\";if (!(globalThis.__tarnExtRun || []).includes(1)) throw"));
        assert_eq!((out[3].run_at, out[3].all_frames), (RunAt::DocumentIdle, true));
        assert_eq!((out[1].run_at, out[1].all_frames), (RunAt::DocumentStart, false));
        assert!(injections(&scripts(), &Decisions::default(), "").first().unwrap().source.contains("new Map([])"));
        assert_eq!(skip_comments("/* open"), 0);
    }

    #[test]
    fn extension_pages_load_only_their_own_origin_and_blank_frames() {
        let origin = "0123456789abcdef0123456789abcdef";
        let load = |url: &str, main| page_loads(&Url::parse(url).unwrap(), origin, main);
        assert!(load(&format!("tarn-extension://{origin}/popup.html"), true));
        assert!(load(&format!("tarn-extension://{origin}/frame.html"), false));
        assert!(load("about:blank", false) && load("about:srcdoc", false));
        assert!(!load("about:blank", true));
        assert!(!load("https://example.com/", false) && !load("http://example.com/", true));
        assert!(!load("tarn-extension://ffffffffffffffffffffffffffffffff/x.html", false));
    }

    #[test]
    fn the_bootstrap_calls_the_shim_with_its_config() {
        let script = bootstrap(&serde_json::json!({"id": "x"}));
        assert!(script.starts_with("(function (config)"));
        assert!(script.ends_with("({\"id\":\"x\"});\n"));
        assert!(script.contains(POST) && script.contains(RECEIVE));
        assert!(!script.contains("eval(") && !script.contains("new Function"));
    }
}
