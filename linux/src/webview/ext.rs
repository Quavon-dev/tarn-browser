use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use engine::tarn_ext::{ExtensionView, FrameId, FrameNav, Gate, MAX_MESSAGE_BYTES, RECEIVE, Resources, RunAt, SCHEME, World, WorldScripts, page_loads};
use serde_json::json;
use shield::ShieldLevel;
use url::Url;
use webkit6::prelude::*;
use webkit6::{
    NavigationPolicyDecision, PolicyDecisionType, URISchemeRequest, URISchemeResponse, UserContentInjectedFrames, UserContentManager, UserScript,
    UserScriptInjectionTime, UserStyleLevel, UserStyleSheet, WebContext, WebView, gio, glib, soup,
};

use super::{State, VIEWS, filters};
use crate::data::StoreKey;

const PAGE_HANDLER: &str = "tarnExt";

pub struct ViewInfo<'a> {
    pub key: &'a StoreKey,
    pub tab: &'a str,
    pub level: ShieldLevel,
}

pub struct Attach {
    pub gate: Gate,
    pub worlds: Vec<Rc<WorldScripts>>,
    pub rules: Vec<(String, String)>,
}

pub type Provider = Rc<dyn Fn(&ViewInfo) -> Option<Attach>>;

thread_local! {
    static PROVIDER: RefCell<Option<Provider>> = const { RefCell::new(None) };
    static PAGES: RefCell<Vec<Weak<PageView>>> = const { RefCell::new(Vec::new()) };
    static SCHEME_READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

enum Added {
    Script(UserScript),
    Sheet(UserStyleSheet),
}

#[derive(Default)]
pub(super) struct Ext {
    gate: RefCell<Option<Gate>>,
    worlds: RefCell<BTreeMap<World, Rc<WorldScripts>>>,
    handlers: RefCell<Vec<World>>,
    allowed: RefCell<BTreeMap<(World, usize), Vec<String>>>,
    added: RefCell<Vec<Added>>,
    top: RefCell<Option<Url>>,
    pub(super) filters: RefCell<Vec<String>>,
}

pub fn set_provider(provider: Provider) {
    PROVIDER.with_borrow_mut(|p| *p = Some(provider));
}

/// WebKit user-content patterns that match exactly `url`'s document, query included, with or
/// without a fragment (WebKit's patterns name no port); `None` where a `*` in the address would
/// widen the match.
fn patterns(url: &Url) -> Option<Vec<String>> {
    let host = url.host_str()?;
    let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let base = format!("{}://{host}{}{query}", url.scheme(), url.path());
    if !matches!(url.scheme(), "http" | "https") || base.contains('*') {
        return None;
    }
    Some(vec![format!("{base}#*"), base])
}

/// Adds or takes out one document's patterns for a script; an entry left empty goes, since an
/// empty allow-list would match every page. True when the entry changed.
fn allow(allowed: &mut BTreeMap<(World, usize), Vec<String>>, key: (World, usize), list: &[String], want: bool) -> bool {
    let entry = allowed.entry(key.clone()).or_default();
    let has = list.iter().all(|p| entry.contains(p));
    if want && !has {
        entry.extend(list.iter().cloned());
    } else if !want {
        entry.retain(|p| !list.contains(p));
    }
    if entry.is_empty() {
        allowed.remove(&key);
    }
    want != has
}

fn transport(world: &World) -> String {
    let name = serde_json::to_string(world.name()).unwrap_or_default();
    format!(
        "(function () {{ const h = window.top === window && window.webkit && window.webkit.messageHandlers ? window.webkit.messageHandlers[{name}] : null; \
         globalThis.__tarnExtPost = s => {{ if (!h) throw new Error('unavailable'); h.postMessage(String(s)); }}; }})();\n"
    )
}

fn time(run_at: RunAt) -> UserScriptInjectionTime {
    match run_at {
        RunAt::DocumentStart => UserScriptInjectionTime::Start,
        RunAt::DocumentEnd | RunAt::DocumentIdle => UserScriptInjectionTime::End,
    }
}

fn frames(all: bool) -> UserContentInjectedFrames {
    if all { UserContentInjectedFrames::AllFrames } else { UserContentInjectedFrames::TopFrame }
}

fn sync(state: &State) {
    let Some(content) = state.content.borrow().clone() else { return };
    for added in state.ext.added.borrow_mut().drain(..) {
        match added {
            Added::Script(s) => content.remove_script(&s),
            Added::Sheet(s) => content.remove_style_sheet(&s),
        }
    }
    let worlds = state.ext.worlds.borrow();
    let allowed = state.ext.allowed.borrow();
    let mut added = Vec::new();
    for (world, scripts) in worlds.iter() {
        let mine: Vec<(&Rc<WorldScripts>, usize, &Vec<String>)> =
            allowed.range((world.clone(), 0)..=(world.clone(), usize::MAX)).map(|((_, ix), list)| (scripts, *ix, list)).collect();
        if mine.is_empty() {
            continue;
        }
        let mut every: Vec<&str> = mine.iter().flat_map(|(_, _, l)| l.iter().map(String::as_str)).collect();
        every.sort_unstable();
        every.dedup();
        let any_frame = mine.iter().any(|(s, ix, _)| s.scripts.get(*ix).is_some_and(|c| c.all_frames));
        let boot = format!("{}{}", transport(world), scripts.bootstrap);
        added.push(Added::Script(UserScript::for_world(&boot, frames(any_frame), UserScriptInjectionTime::Start, world.name(), &every, &[])));
        for (scripts, ix, list) in mine {
            let Some(script) = scripts.scripts.get(ix) else { continue };
            let list: Vec<&str> = list.iter().map(String::as_str).collect();
            for css in &script.css {
                added.push(Added::Sheet(UserStyleSheet::for_world(css, frames(script.all_frames), UserStyleLevel::User, world.name(), &list, &[])));
            }
            for js in &script.js {
                added.push(Added::Script(UserScript::for_world(js, frames(script.all_frames), time(script.run_at), world.name(), &list, &[])));
            }
        }
    }
    for a in &added {
        match a {
            Added::Script(s) => content.add_script(s),
            Added::Sheet(s) => content.add_style_sheet(s),
        }
    }
    *state.ext.added.borrow_mut() = added;
}

fn decide(state: &State, url: &Url, top: &Url, main_frame: bool) -> bool {
    let Some(gate) = state.ext.gate.borrow().clone() else { return false };
    let Some(list) = patterns(url) else { return false };
    let worlds: Vec<(World, Rc<WorldScripts>)> = state.ext.worlds.borrow().iter().map(|(w, s)| (w.clone(), s.clone())).collect();
    let runs: Vec<Vec<usize>> = worlds.iter().map(|(world, _)| gate(world, &FrameNav { url, top, main_frame })).collect();
    let mut allowed = state.ext.allowed.borrow_mut();
    let mut changed = false;
    for ((world, scripts), run) in worlds.into_iter().zip(runs) {
        for (ix, _) in scripts.scripts.iter().enumerate().filter(|(_, s)| main_frame || s.all_frames) {
            changed |= allow(&mut allowed, (world.clone(), ix), &list, run.contains(&ix));
        }
    }
    changed
}

pub(super) fn started(state: &State, url: Option<&Url>) {
    let had = !state.ext.allowed.borrow().is_empty();
    state.ext.allowed.borrow_mut().clear();
    *state.ext.top.borrow_mut() = url.cloned();
    let changed = url.is_some_and(|u| decide(state, u, u, true));
    if had || changed {
        sync(state);
    }
}

pub(super) fn frame(state: &State, url: &Url) {
    let top = state.ext.top.borrow().clone().or_else(|| state.page.borrow().clone()).unwrap_or_else(|| url.clone());
    if decide(state, url, &top, false) {
        sync(state);
    }
}

fn register(state: &Rc<State>, content: &UserContentManager, world: &World) {
    if state.ext.handlers.borrow().contains(world) {
        return;
    }
    content.register_script_message_handler(world.name(), Some(world.name()));
    let weak = Rc::downgrade(state);
    let name = world.name().to_owned();
    content.connect_script_message_received(Some(world.name()), move |_, value| {
        let Some(state) = weak.upgrade() else { return };
        if !value.is_string() {
            return;
        }
        let mut json = value.to_str().to_string();
        if json.len() > MAX_MESSAGE_BYTES {
            let cut = (MAX_MESSAGE_BYTES + 1..=json.len()).find(|i| json.is_char_boundary(*i)).unwrap_or(json.len());
            json.truncate(cut);
        }
        state.emit("extensionMessage", json!({ "world": name, "frame": FrameId::MAIN.0, "json": json }));
    });
    state.ext.handlers.borrow_mut().push(world.clone());
}

pub struct ViewHandle(Rc<State>);

impl ExtensionView for ViewHandle {
    fn set_extension_gate(&self, gate: Gate) {
        *self.0.ext.gate.borrow_mut() = Some(gate);
    }

    fn add_extension_scripts(&self, scripts: Rc<WorldScripts>) {
        if let Some(content) = self.0.content.borrow().clone() {
            register(&self.0, &content, &scripts.world);
        }
        self.0.ext.worlds.borrow_mut().insert(scripts.world.clone(), scripts);
    }

    fn remove_extension_scripts(&self, world: &World) {
        self.0.ext.worlds.borrow_mut().remove(world);
        self.0.ext.allowed.borrow_mut().retain(|(w, _), _| w != world);
        if let Some(content) = self.0.content.borrow().clone()
            && self.0.ext.handlers.borrow().contains(world)
        {
            content.unregister_script_message_handler(world.name(), Some(world.name()));
        }
        self.0.ext.handlers.borrow_mut().retain(|w| w != world);
        sync(&self.0);
    }

    fn post_extension_message(&self, world: &World, frame: Option<FrameId>, json: &str) {
        if frame.is_some_and(|f| f != FrameId::MAIN) || !self.0.ext.worlds.borrow().contains_key(world) {
            return;
        }
        let Some(web) = self.0.web() else { return };
        let script = format!("if (typeof globalThis.{RECEIVE} === 'function') globalThis.{RECEIVE}({});", serde_json::to_string(json).unwrap_or_default());
        web.evaluate_javascript(&script, Some(world.name()), None, None::<&gio::Cancellable>, |_| {});
    }
}

pub(super) fn attach(state: &Rc<State>) {
    let Some(content) = state.content.borrow().clone() else { return };
    let info = StoreKey::parse(&state.profile.borrow()).map(|key| (key, state.tab.borrow().clone(), state.level.get()));
    let provider = PROVIDER.with_borrow(Clone::clone);
    let attach = info.and_then(|(key, tab, level)| provider.and_then(|p| p(&ViewInfo { key: &key, tab: &tab, level })));
    let (gate, worlds, rules) = match attach {
        Some(a) => (Some(a.gate), a.worlds, a.rules),
        None => (None, Vec::new(), Vec::new()),
    };
    let keep: Vec<World> = worlds.iter().map(|w| w.world.clone()).collect();
    let gone: Vec<World> = state.ext.worlds.borrow().keys().filter(|w| !keep.contains(w)).cloned().collect();
    let handle = ViewHandle(state.clone());
    gone.iter().for_each(|w| handle.remove_extension_scripts(w));
    match gate {
        Some(gate) => handle.set_extension_gate(gate),
        None => *state.ext.gate.borrow_mut() = None,
    }
    for scripts in worlds {
        handle.add_extension_scripts(scripts);
    }
    let wanted: Vec<String> = rules.iter().map(|(id, _)| id.clone()).collect();
    let old: Vec<String> = state.ext.filters.borrow().iter().filter(|id| !wanted.contains(id)).cloned().collect();
    old.iter().for_each(|id| content.remove_filter_by_id(id));
    state.ext.filters.borrow_mut().retain(|id| wanted.contains(id));
    for (id, source) in rules {
        if state.ext.filters.borrow().contains(&id) {
            continue;
        }
        state.ext.filters.borrow_mut().push(id.clone());
        let weak = Rc::downgrade(state);
        let wanted = id.clone();
        filters::compile_one(id, source, move |filter| {
            let Some(state) = weak.upgrade() else { return };
            if state.ext.filters.borrow().contains(&wanted)
                && let Some(content) = state.content.borrow().as_ref()
            {
                content.add_filter(&filter);
            }
        });
    }
    let page = state.ext.top.borrow().clone().or_else(|| state.page.borrow().clone());
    state.ext.allowed.borrow_mut().clear();
    if let Some(page) = page {
        decide(state, &page, &page, true);
    }
    sync(state);
}

pub(super) fn detach(state: &State) {
    state.ext.handlers.borrow_mut().clear();
    state.ext.added.borrow_mut().clear();
    state.ext.filters.borrow_mut().clear();
    state.ext.allowed.borrow_mut().clear();
}

fn live_views() -> Vec<Rc<State>> {
    VIEWS.with_borrow(|views| views.iter().filter_map(Weak::upgrade).collect())
}

pub fn refresh() {
    live_views().iter().for_each(attach);
}

pub fn post(tab: Option<&str>, world: &World, frame: Option<FrameId>, json: &str) {
    for state in live_views().into_iter().filter(|s| tab.is_none_or(|t| *s.tab.borrow() == t)) {
        ViewHandle(state).post_extension_message(world, frame, json);
    }
}

/// A shell-owned view of an extension's own page (`tarn-extension://<origin>/`): hidden for the
/// background, shown for the popup. The shim runs in its main world, which only the
/// extension's own documents reach.
pub struct PageView {
    pub web: WebView,
    origin: String,
    resources: Resources,
}

impl PageView {
    pub fn post(&self, json: &str) {
        let script = format!("if (typeof globalThis.{RECEIVE} === 'function') globalThis.{RECEIVE}({});", serde_json::to_string(json).unwrap_or_default());
        self.web.evaluate_javascript(&script, None, None, None::<&gio::Cancellable>, |_| {});
    }

    pub fn close(&self) {
        self.web.stop_loading();
        self.web.try_close();
    }
}

impl Drop for PageView {
    fn drop(&mut self) {
        self.close();
    }
}

fn loads(url: &str, origin: &str, main_frame: bool) -> bool {
    Url::parse(url).is_ok_and(|u| page_loads(&u, origin, main_frame))
}

fn respond(request: &URISchemeRequest, status: u16, mime: &str, csp: Option<&str>, body: Vec<u8>) {
    let length = body.len() as i64;
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(body));
    let response = URISchemeResponse::new(&stream, length);
    response.set_status(u32::from(status), None);
    response.set_content_type(mime);
    let headers = soup::MessageHeaders::new(soup::MessageHeadersType::Response);
    headers.append("Content-Type", mime);
    if let Some(csp) = csp {
        headers.append("Content-Security-Policy", csp);
    }
    headers.append("X-Content-Type-Options", "nosniff");
    response.set_http_headers(headers);
    request.finish_with_response(&response);
}

fn serve(request: &URISchemeRequest) {
    let url = request.uri().and_then(|u| Url::parse(&u).ok());
    let viewer = request.web_view();
    let page = PAGES.with_borrow(|pages| {
        pages.iter().filter_map(Weak::upgrade).find(|p| Some(&p.web) == viewer.as_ref() && url.as_ref().and_then(Url::host_str) == Some(p.origin.as_str()))
    });
    match (page, url) {
        (Some(page), Some(url)) => {
            let r = (page.resources)(&url);
            respond(request, r.status, r.mime, r.csp, r.body);
        }
        _ => respond(request, 403, "text/plain", Some(browser_core::tarn_ext::resources::PAGE_CSP), Vec::new()),
    }
}

fn register_scheme() {
    if SCHEME_READY.replace(true) {
        return;
    }
    let Some(context) = WebContext::default() else { return };
    if let Some(security) = context.security_manager() {
        security.register_uri_scheme_as_secure(SCHEME);
        security.register_uri_scheme_as_display_isolated(SCHEME);
    }
    context.register_uri_scheme(SCHEME, serve);
}

fn page_transport() -> String {
    format!(
        "(function () {{ const h = window.top === window && window.webkit && window.webkit.messageHandlers ? window.webkit.messageHandlers.{PAGE_HANDLER} : null; \
         globalThis.__tarnExtPost = s => {{ if (!h) throw new Error('unavailable'); h.postMessage(String(s)); }}; }})();\n"
    )
}

pub struct PageSpec {
    pub key: StoreKey,
    pub level: ShieldLevel,
    pub lists: Vec<String>,
    pub origin: String,
    pub url: Url,
    pub bootstrap: String,
    pub resources: Resources,
}

pub fn open_page(spec: PageSpec, message: impl Fn(String) + 'static, ready: impl FnOnce(Rc<PageView>) + 'static) {
    register_scheme();
    let PageSpec { key, level, lists, origin, url, bootstrap, resources } = spec;
    filters::with_filters(level, lists, move |result| {
        let Ok(rules) = result else { return };
        let Some(session) = super::session::acquire(&key) else { return };
        let content = UserContentManager::new();
        rules.iter().for_each(|f| content.add_filter(f));
        let source = format!("{}{bootstrap}", page_transport());
        let own_pages = format!("{SCHEME}://{origin}/*");
        content.add_script(&UserScript::new(&source, UserContentInjectedFrames::TopFrame, UserScriptInjectionTime::Start, &[own_pages.as_str()], &[]));
        content.register_script_message_handler(PAGE_HANDLER, None);
        let web = WebView::builder().network_session(&session).user_content_manager(&content).build();
        if let Some(settings) = WebViewExt::settings(&web) {
            settings.set_enable_javascript(true);
            settings.set_javascript_can_open_windows_automatically(false);
            settings.set_enable_developer_extras(false);
        }
        let page = Rc::new(PageView { web: web.clone(), origin: origin.clone(), resources });
        PAGES.with_borrow_mut(|pages| {
            pages.retain(|p| p.strong_count() > 0);
            pages.push(Rc::downgrade(&page));
        });
        let base = origin.clone();
        web.connect_decide_policy(move |_, decision, kind| {
            let uri = decision
                .downcast_ref::<NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
                .and_then(|a| a.request())
                .and_then(|r| r.uri())
                .map(|u| u.to_string());
            match kind {
                PolicyDecisionType::NavigationAction if uri.as_deref().is_some_and(|u| loads(u, &base, false)) => false,
                PolicyDecisionType::NavigationAction | PolicyDecisionType::NewWindowAction => {
                    decision.ignore();
                    true
                }
                _ => false,
            }
        });
        let viewer = web.downgrade();
        let base = origin.clone();
        content.connect_script_message_received(Some(PAGE_HANDLER), move |_, value| {
            let on_own_page = viewer.upgrade().and_then(|w| w.uri()).is_some_and(|u| loads(&u, &base, true));
            if on_own_page && value.is_string() {
                message(value.to_str().to_string());
            }
        });
        web.load_uri(url.as_str());
        ready(page);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_document_is_allowed_by_its_exact_address_until_the_gate_says_no() {
        let url = |s: &str| Url::parse(s).unwrap();
        let ok = patterns(&url("https://a.test:8443/p?ok=1#top")).unwrap();
        assert_eq!(ok, ["https://a.test/p?ok=1#*", "https://a.test/p?ok=1"]);
        assert!(patterns(&url("https://a.test/*x")).is_none() && patterns(&url("file:///p")).is_none());
        let private = patterns(&url("https://a.test/p?private=1")).unwrap();
        let world = World::for_install("ab12").unwrap();
        let mut allowed = BTreeMap::new();
        assert!(allow(&mut allowed, (world.clone(), 0), &ok, true));
        assert!(!allow(&mut allowed, (world.clone(), 0), &private, false), "another query was never allowed");
        assert_eq!(allowed[&(world.clone(), 0)], ok);
        assert!(allow(&mut allowed, (world.clone(), 0), &ok, false));
        assert!(allowed.is_empty(), "no empty allow-list is left, which would match every page");
    }
}
