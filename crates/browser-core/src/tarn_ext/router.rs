//! Messages between an extension's content-script worlds, its pages and the shell. The sender is
//! what the engine and shell say it is (`Sender`), never what a message claims; every decision on
//! who receives what is made here so all clients behave alike.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use engine::tarn_ext::{FrameId, MAX_MESSAGE_BYTES, PageKind};
use serde_json::{Map, Value, json};
use store::{Id, Store};
use url::Url;

use super::patterns::Pattern;
use super::{Access, Package};

pub const MAX_RATE: usize = 200;
pub const QUOTA_BYTES: usize = 5 << 20;
pub const MAX_KEY_BYTES: usize = 1024;
pub const IDLE: Duration = Duration::from_secs(30);
pub const MAX_OPENED: usize = 3;
pub const OPENED_WINDOW: Duration = Duration::from_secs(10);
pub const MAX_BADGE_CHARS: usize = 4;
pub const MAX_QUEUED: usize = 64;
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
pub const MAX_DELIVERIES: usize = 256;
const SETTLE: Duration = Duration::from_secs(1);
const MAX_TITLE_CHARS: usize = 256;
const NO_RECEIVER: &str = "Could not establish connection. Receiving end does not exist.";
const NO_ANSWER: &str = "The message port closed before a response was received.";
const NOT_STARTED: &str = "The extension's background page did not start.";
const QUEUE_FULL: &str = "Too many messages are waiting for the extension's page.";
const TOO_MANY: &str = "Too many messages are waiting for an answer.";

pub type TabId = u64;

/// Where a message came from, as the engine reported it: a frame of a tab's view, where the
/// install's world runs, or one of the install's own pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum From {
    Tab { tab: TabId, frame: FrameId },
    Page(PageKind),
}

/// `origin` is a subframe document's web origin as the engine reported it; a subframe message
/// without one is dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sender {
    pub install_id: String,
    pub from: From,
    pub origin: Option<Url>,
}

/// A tab of the profile's persistent spaces; burner and private tabs are never listed.
#[derive(Clone, Debug, PartialEq)]
pub struct TabInfo {
    pub id: TabId,
    pub url: Option<Url>,
    pub title: String,
    pub active: bool,
    pub current_window: bool,
}

/// What the shell knows at the moment of a call. `store` is `None` for a private profile, whose
/// storage stays in memory.
pub struct Env<'a> {
    pub tabs: &'a [TabInfo],
    pub access: &'a dyn Fn(&str, &str) -> Access,
    pub store: Option<&'a Store>,
    pub now: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Frame {
        tab: TabId,
        frame: FrameId,
    },
    /// Every frame of the tab where the install's world runs.
    Tab(TabId),
    /// Every frame of every tab of the profile where the install's world runs.
    AllTabs,
    Page(PageKind),
}

/// What the shell carries out: `Post` is `ExtensionView::post_extension_message` into the
/// install's world (or its page's main world).
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Post { install_id: String, target: Target, json: String },
    StartBackground(String),
    StopBackground(String),
    OpenTab { url: Url, active: bool },
    ActionChanged(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionState {
    pub title: String,
    pub badge_text: String,
    pub badge_color: Option<[u8; 4]>,
}

struct Installed {
    origin_id: String,
    title: String,
    storage: bool,
    active_tab: bool,
    background: bool,
    popup: bool,
    sites: Vec<Pattern>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Waiter {
    Frame(TabId, FrameId),
    Tab(TabId),
    Page(PageKind),
}

impl Waiter {
    fn answers(self, from: From) -> bool {
        match (self, from) {
            (Waiter::Frame(t, f), From::Tab { tab, frame }) => t == tab && f == frame,
            (Waiter::Tab(t), From::Tab { tab, .. }) => t == tab,
            (Waiter::Page(k), From::Page(kind)) => k == kind,
            _ => false,
        }
    }
}

/// A message waiting for its answer until `deadline`; `answered` once a receiver replied without
/// a value.
struct Delivery {
    install_id: String,
    reply_to: From,
    request: u64,
    waiting: Vec<Waiter>,
    deadline: Instant,
    answered: bool,
}

struct PageState {
    ready: bool,
    queue: Vec<String>,
    started: Instant,
    last: Instant,
}

#[derive(Default)]
pub struct Router {
    profile: Id,
    installs: HashMap<String, Installed>,
    rate: HashMap<(String, Option<TabId>), VecDeque<Instant>>,
    opened: HashMap<String, VecDeque<Instant>>,
    deliveries: HashMap<u64, Delivery>,
    next_delivery: u64,
    storage: HashMap<String, BTreeMap<String, String>>,
    actions: HashMap<(String, Option<TabId>), ActionState>,
    active_tab: HashMap<String, TabId>,
    pages: HashMap<(String, PageKind), PageState>,
}

fn target(from: From) -> Target {
    match from {
        From::Tab { tab, frame } => Target::Frame { tab, frame },
        From::Page(kind) => Target::Page(kind),
    }
}

fn web_host(url: &Url) -> Option<&str> {
    matches!(url.scheme(), "http" | "https").then(|| url.host_str()).flatten()
}

/// The request id of a message too large to parse, read from the shim's `{"id":<n>,` prefix.
fn leading_id(json: &str) -> Option<u64> {
    let digits: String = json.strip_prefix("{\"id\":")?.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

impl Router {
    pub fn new(profile: Id) -> Self {
        Router { profile, ..Router::default() }
    }

    pub fn add(&mut self, package: &Package) {
        let m = &package.manifest;
        let title = m.action.as_ref().and_then(|a| a.title.clone()).map_or_else(|| package.name.clone(), |t| super::i18n::localize(&t, &package.messages));
        self.installs.insert(
            package.install_id.clone(),
            Installed {
                origin_id: package.origin_id.clone(),
                title: crate::theme_fetch::shown(&title),
                storage: m.storage,
                active_tab: m.active_tab,
                background: m.background.is_some(),
                popup: m.action.as_ref().is_some_and(|a| a.popup.is_some()),
                sites: m.wanted().into_iter().cloned().collect(),
            },
        );
    }

    pub fn remove(&mut self, install_id: &str) -> Vec<Out> {
        let mut out = Vec::new();
        if self.pages.remove(&(install_id.to_owned(), PageKind::Background)).is_some() {
            out.push(Out::StopBackground(install_id.to_owned()));
        }
        self.pages.remove(&(install_id.to_owned(), PageKind::Popup));
        self.installs.remove(install_id);
        self.deliveries.retain(|_, d| d.install_id != install_id);
        self.storage.remove(install_id);
        self.actions.retain(|(i, _), _| i != install_id);
        self.active_tab.remove(install_id);
        self.rate.retain(|(i, _), _| i != install_id);
        self.opened.remove(install_id);
        out
    }

    pub fn action(&self, install_id: &str, tab: Option<TabId>) -> ActionState {
        let base = self.actions.get(&(install_id.to_owned(), None)).cloned().unwrap_or_default();
        let own = tab.and_then(|t| self.actions.get(&(install_id.to_owned(), Some(t))));
        let title = self.installs.get(install_id).map(|i| i.title.clone()).unwrap_or_default();
        let mut state = own.cloned().unwrap_or(base);
        if state.title.is_empty() {
            state.title = title;
        }
        state
    }

    /// The shell created the install's page view (after `StartBackground`, or a popup).
    pub fn page_opened(&mut self, install_id: &str, kind: PageKind, now: Instant) {
        self.pages.entry((install_id.to_owned(), kind)).or_insert(PageState { ready: false, queue: Vec::new(), started: now, last: now });
    }

    pub fn page_closed(&mut self, install_id: &str, kind: PageKind) -> Vec<Out> {
        self.pages.remove(&(install_id.to_owned(), kind));
        self.unanswered(|w| w == Waiter::Page(kind), Some(install_id), Ok(Value::Null))
    }

    pub fn tab_closed(&mut self, tab: TabId) -> Vec<Out> {
        self.rate.retain(|(_, t), _| *t != Some(tab));
        self.active_tab.retain(|_, t| *t != tab);
        self.actions.retain(|(_, t), _| *t != Some(tab));
        self.unanswered(|w| matches!(w, Waiter::Tab(t) | Waiter::Frame(t, _) if t == tab), None, Ok(Value::Null))
    }

    /// The tab's main frame committed a new document: an `activeTab` grant ends.
    pub fn navigated(&mut self, tab: TabId) {
        self.active_tab.retain(|_, t| *t != tab);
    }

    /// The user clicked the install's toolbar button (or menu entry) with `tab` in front. With a
    /// popup the shell opens it; without, the background hears `action.onClicked`.
    pub fn clicked(&mut self, install_id: &str, tab: &TabInfo, env: &Env) -> Vec<Out> {
        let Some(installed) = self.installs.get(install_id) else { return Vec::new() };
        if installed.active_tab {
            self.active_tab.insert(install_id.to_owned(), tab.id);
        }
        if installed.popup || !installed.background {
            return Vec::new();
        }
        let info = self.tab_json(install_id, tab, env);
        self.deliver_to_page(install_id, PageKind::Background, json!({"event": "action.onClicked", "tab": info}).to_string(), env.now)
    }

    /// Stops background pages idle for `IDLE`, and those that did not get ready within it: the
    /// calls waiting for such a page get an error. Only events and messages to a page count as
    /// activity, not its own calls. Calls unanswered after `REPLY_TIMEOUT` end.
    pub fn tick(&mut self, now: Instant) -> Vec<Out> {
        let waited: Vec<String> =
            self.deliveries.values().filter(|d| d.waiting.contains(&Waiter::Page(PageKind::Background))).map(|d| d.install_id.clone()).collect();
        let stopped: Vec<(String, bool)> = self
            .pages
            .iter()
            .filter(|((_, kind), _)| *kind == PageKind::Background)
            .filter_map(|((i, _), p)| {
                let failed = !p.ready && now.duration_since(p.started) >= IDLE;
                let idle = p.queue.is_empty() && now.duration_since(p.last) >= IDLE && !waited.contains(i);
                (failed || idle).then(|| (i.clone(), failed))
            })
            .collect();
        let mut out = Vec::new();
        for (install_id, failed) in stopped {
            self.pages.remove(&(install_id.clone(), PageKind::Background));
            out.push(Out::StopBackground(install_id.clone()));
            if failed {
                out.extend(self.unanswered(|w| w == Waiter::Page(PageKind::Background), Some(&install_id), Err(NOT_STARTED.to_owned())));
            }
        }
        let expired: Vec<u64> = self.deliveries.iter().filter(|(_, d)| d.deadline <= now).map(|(id, _)| *id).collect();
        for id in expired {
            let Some(d) = self.deliveries.remove(&id) else { continue };
            let result = if d.answered { Ok(Value::Null) } else { Err(NO_ANSWER.to_owned()) };
            let to = Sender { install_id: d.install_id, from: d.reply_to, origin: None };
            out.push(self.reply(&to, d.request, result));
        }
        out
    }

    pub fn handle(&mut self, sender: &Sender, json: &str, env: &Env) -> Vec<Out> {
        let Some(installed) = self.installs.get(&sender.install_id) else { return Vec::new() };
        let storage_allowed = installed.storage;
        if let From::Tab { tab, frame } = sender.from {
            let Some(info) = env.tabs.iter().find(|t| t.id == tab) else { return Vec::new() };
            let document = match frame {
                FrameId::MAIN => info.url.as_ref().filter(|u| sender.origin.as_ref().is_none_or(|o| o.origin() == u.origin())),
                _ => sender.origin.as_ref(),
            };
            if !document.is_some_and(|d| self.granted(&sender.install_id, d, env)) {
                return Vec::new();
            }
        }
        let tab_key = match sender.from {
            From::Tab { tab, .. } => Some(tab),
            From::Page(_) => None,
        };
        let window = self.rate.entry((sender.install_id.clone(), tab_key)).or_default();
        while window.front().is_some_and(|t| env.now.duration_since(*t) >= Duration::from_secs(1)) {
            window.pop_front();
        }
        if json.len() > MAX_MESSAGE_BYTES {
            return leading_id(json).map(|id| vec![self.reply(sender, id, Err("The message is larger than 1 MiB.".to_owned()))]).unwrap_or_default();
        }
        if window.len() >= MAX_RATE {
            let id = serde_json::from_str::<Value>(json).ok().and_then(|m| m.get("id")?.as_u64());
            return id.map(|id| vec![self.reply(sender, id, Err("Too many messages.".to_owned()))]).unwrap_or_default();
        }
        window.push_back(env.now);
        let Ok(Value::Object(message)) = serde_json::from_str::<Value>(json) else { return Vec::new() };
        let (Some(call), Some(Value::Array(args))) = (message.get("call").and_then(Value::as_str), message.get("args")) else { return Vec::new() };
        let id = message.get("id").and_then(Value::as_u64);
        let page = matches!(sender.from, From::Page(_));
        let arg = |n: usize| args.get(n).cloned().unwrap_or(Value::Null);
        let result: Result<Value, String> = match call {
            "ready" if page => return self.ready(sender),
            "reply" => return self.answered(sender, arg(0).as_u64(), arg(1), env.now),
            "runtime.sendMessage" => return self.send_to_pages(sender, id, arg(0), env),
            "tabs.sendMessage" if page => return self.send_to_tab(sender, id, &arg(0), arg(1), &arg(2), env),
            "tabs.query" if page => self.query(&sender.install_id, &arg(0), env),
            "tabs.create" if page => {
                let (result, mut out) = match self.create(&sender.install_id, &arg(0), env.now) {
                    Ok((url, active)) => (Ok(json!({"url": url.as_str(), "active": active})), vec![Out::OpenTab { url, active }]),
                    Err(e) => (Err(e), Vec::new()),
                };
                out.extend(id.map(|id| self.reply(sender, id, result)));
                return out;
            }
            "action.setBadgeText" | "action.setBadgeBackgroundColor" | "action.setTitle" if page => self.set_action(&sender.install_id, call, &arg(0), env),
            c if c.starts_with("storage.") && storage_allowed => return self.storage_call(sender, id, c, &arg(0), env),
            _ => Err(format!("{call} is not available here.")),
        };
        let mut out = Vec::new();
        if result.is_ok() && call.starts_with("action.") {
            out.push(Out::ActionChanged(sender.install_id.clone()));
        }
        if let Some(id) = id {
            out.push(self.reply(sender, id, result));
        }
        out
    }

    fn reply(&self, sender: &Sender, id: u64, result: Result<Value, String>) -> Out {
        let body = match result {
            Ok(value) => json!({"id": id, "result": value}),
            Err(error) => json!({"id": id, "error": error}),
        };
        Out::Post { install_id: sender.install_id.clone(), target: target(sender.from), json: body.to_string() }
    }

    fn ready(&mut self, sender: &Sender) -> Vec<Out> {
        let From::Page(kind) = sender.from else { return Vec::new() };
        let Some(page) = self.pages.get_mut(&(sender.install_id.clone(), kind)) else { return Vec::new() };
        page.ready = true;
        std::mem::take(&mut page.queue).into_iter().map(|json| Out::Post { install_id: sender.install_id.clone(), target: Target::Page(kind), json }).collect()
    }

    fn deliver_to_page(&mut self, install_id: &str, kind: PageKind, json: String, now: Instant) -> Vec<Out> {
        let key = (install_id.to_owned(), kind);
        match self.pages.get_mut(&key) {
            Some(page) if page.ready => {
                page.last = now;
                vec![Out::Post { install_id: install_id.to_owned(), target: Target::Page(kind), json }]
            }
            Some(page) => {
                page.last = now;
                if page.queue.len() < MAX_QUEUED {
                    page.queue.push(json);
                }
                Vec::new()
            }
            None if kind == PageKind::Background => {
                self.pages.insert(key, PageState { ready: false, queue: vec![json], started: now, last: now });
                vec![Out::StartBackground(install_id.to_owned())]
            }
            None => Vec::new(),
        }
    }

    fn sender_json(&self, sender: &Sender, env: &Env) -> Value {
        let Some(installed) = self.installs.get(&sender.install_id) else { return Value::Null };
        match sender.from {
            From::Tab { tab, frame } => {
                let info = env.tabs.iter().find(|t| t.id == tab);
                let url = info.and_then(|t| t.url.as_ref());
                json!({
                    "id": installed.origin_id,
                    "tab": info.map(|t| self.tab_json(&sender.install_id, t, env)),
                    "frameId": frame.0,
                    "url": if frame == FrameId::MAIN { url.map(Url::as_str) } else { None },
                    "origin": if frame == FrameId::MAIN { url.map(|u| u.origin().ascii_serialization()) } else { None },
                })
            }
            From::Page(_) => json!({"id": installed.origin_id, "url": super::resources::base_url(&installed.origin_id)}),
        }
    }

    fn start_delivery(&mut self, sender: &Sender, id: Option<u64>, waiting: Vec<Waiter>, message: Value, env: &Env) -> String {
        self.next_delivery += 1;
        let delivery = self.next_delivery;
        let body = json!({"event": "runtime.onMessage", "delivery": delivery, "message": message, "sender": self.sender_json(sender, env)}).to_string();
        if let Some(request) = id {
            let (install_id, reply_to, deadline) = (sender.install_id.clone(), sender.from, env.now + REPLY_TIMEOUT);
            self.deliveries.insert(delivery, Delivery { install_id, reply_to, request, waiting, deadline, answered: false });
        }
        body
    }

    fn deliveries_full(&self, install_id: &str) -> bool {
        self.deliveries.values().filter(|d| d.install_id == install_id).count() >= MAX_DELIVERIES
    }

    /// Whether the install holds a grant for `url`'s host; a plain `http` document also needs a
    /// site pattern of the install that names `http`, so a grant made on `https` does not reach it.
    fn granted(&self, install_id: &str, url: &Url, env: &Env) -> bool {
        let Some(host) = web_host(url) else { return false };
        let scheme_wanted = url.scheme() == "https" || self.installs.get(install_id).is_some_and(|i| i.sites.iter().any(|p| p.covers(url)));
        scheme_wanted && (env.access)(install_id, host) == Access::Granted
    }

    fn send_to_pages(&mut self, sender: &Sender, id: Option<u64>, message: Value, env: &Env) -> Vec<Out> {
        let Some(installed) = self.installs.get(&sender.install_id) else { return Vec::new() };
        let mut kinds = Vec::new();
        if installed.background && sender.from != From::Page(PageKind::Background) {
            kinds.push(PageKind::Background);
        }
        if self.pages.contains_key(&(sender.install_id.clone(), PageKind::Popup)) && sender.from != From::Page(PageKind::Popup) {
            kinds.push(PageKind::Popup);
        }
        if kinds.is_empty() {
            return id.map(|id| vec![self.reply(sender, id, Err(NO_RECEIVER.to_owned()))]).unwrap_or_default();
        }
        kinds.retain(|k| self.pages.get(&(sender.install_id.clone(), *k)).is_none_or(|p| p.ready || p.queue.len() < MAX_QUEUED));
        if kinds.is_empty() {
            return id.map(|id| vec![self.reply(sender, id, Err(QUEUE_FULL.to_owned()))]).unwrap_or_default();
        }
        if id.is_some() && self.deliveries_full(&sender.install_id) {
            return id.map(|id| vec![self.reply(sender, id, Err(TOO_MANY.to_owned()))]).unwrap_or_default();
        }
        let body = self.start_delivery(sender, id, kinds.iter().map(|k| Waiter::Page(*k)).collect(), message, env);
        kinds.into_iter().flat_map(|kind| self.deliver_to_page(&sender.install_id, kind, body.clone(), env.now)).collect()
    }

    fn send_to_tab(&mut self, sender: &Sender, id: Option<u64>, tab: &Value, message: Value, options: &Value, env: &Env) -> Vec<Out> {
        let reachable =
            tab.as_u64().and_then(|t| env.tabs.iter().find(|i| i.id == t)).filter(|t| t.url.as_ref().is_some_and(|u| self.granted(&sender.install_id, u, env)));
        let Some(tab) = reachable else {
            return id.map(|id| vec![self.reply(sender, id, Err(NO_RECEIVER.to_owned()))]).unwrap_or_default();
        };
        if let Some(id) = id.filter(|_| self.deliveries_full(&sender.install_id)) {
            return vec![self.reply(sender, id, Err(TOO_MANY.to_owned()))];
        }
        let (waiter, to) = match options.get("frameId").and_then(Value::as_u64) {
            Some(frame) => (Waiter::Frame(tab.id, FrameId(frame)), Target::Frame { tab: tab.id, frame: FrameId(frame) }),
            None => (Waiter::Tab(tab.id), Target::Tab(tab.id)),
        };
        let body = self.start_delivery(sender, id, vec![waiter], message, env);
        vec![Out::Post { install_id: sender.install_id.clone(), target: to, json: body }]
    }

    /// A receiver's answer; only one the delivery went to counts. The first answer with a value
    /// wins, else the last one, as with several pages listening. A whole tab has an unknown number
    /// of frames, so an answer without a value from one of them settles only `SETTLE` later.
    fn answered(&mut self, sender: &Sender, delivery: Option<u64>, response: Value, now: Instant) -> Vec<Out> {
        let Some(delivery) = delivery else { return Vec::new() };
        let Some(d) = self.deliveries.get_mut(&delivery).filter(|d| d.install_id == sender.install_id) else { return Vec::new() };
        let Some(at) = d.waiting.iter().position(|w| w.answers(sender.from)) else { return Vec::new() };
        if response.is_null() {
            d.answered = true;
            if matches!(d.waiting[at], Waiter::Tab(_)) {
                d.deadline = d.deadline.min(now + SETTLE);
                return Vec::new();
            }
            d.waiting.remove(at);
            if !d.waiting.is_empty() {
                return Vec::new();
            }
        }
        let Some(d) = self.deliveries.remove(&delivery) else { return Vec::new() };
        let to = Sender { install_id: d.install_id, from: d.reply_to, origin: None };
        vec![self.reply(&to, d.request, Ok(response))]
    }

    fn unanswered(&mut self, gone: impl Fn(Waiter) -> bool, install_id: Option<&str>, result: Result<Value, String>) -> Vec<Out> {
        let ids: Vec<u64> = self.deliveries.iter().filter(|(_, d)| install_id.is_none_or(|i| i == d.install_id)).map(|(id, _)| *id).collect();
        let mut out = Vec::new();
        for id in ids {
            let Some(d) = self.deliveries.get_mut(&id) else { continue };
            d.waiting.retain(|w| !gone(*w));
            if d.waiting.is_empty()
                && let Some(d) = self.deliveries.remove(&id)
            {
                let to = Sender { install_id: d.install_id, from: d.reply_to, origin: None };
                out.push(self.reply(&to, d.request, result.clone()));
            }
        }
        out
    }

    /// `url` and `title` only for tabs whose site the install holds a grant for, or the tab its
    /// toolbar button was clicked in (`activeTab`).
    fn visible(&self, install_id: &str, tab: &TabInfo, env: &Env) -> bool {
        self.active_tab.get(install_id) == Some(&tab.id) || tab.url.as_ref().is_some_and(|u| self.granted(install_id, u, env))
    }

    fn tab_json(&self, install_id: &str, tab: &TabInfo, env: &Env) -> Value {
        let mut out = json!({"id": tab.id, "active": tab.active});
        if self.visible(install_id, tab, env) {
            out["url"] = tab.url.as_ref().map_or(Value::Null, |u| u.as_str().into());
            out["title"] = tab.title.clone().into();
        }
        out
    }

    fn query(&self, install_id: &str, info: &Value, env: &Env) -> Result<Value, String> {
        let flag = |key: &str| info.get(key).and_then(Value::as_bool);
        let patterns: Option<Vec<Pattern>> = match info.get("url") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(vec![Pattern::parse(s)?]),
            Some(Value::Array(list)) => Some(list.iter().map(|p| Pattern::parse(p.as_str().unwrap_or_default())).collect::<Result<_, _>>()?),
            Some(_) => return Err("tabs.query: \"url\" must be a match pattern.".to_owned()),
        };
        let tabs = env
            .tabs
            .iter()
            .filter(|t| flag("active").is_none_or(|a| a == t.active))
            .filter(|t| flag("currentWindow").is_none_or(|c| c == t.current_window))
            .filter(|t| patterns.as_ref().is_none_or(|ps| self.visible(install_id, t, env) && t.url.as_ref().is_some_and(|u| ps.iter().any(|p| p.matches(u)))))
            .map(|t| self.tab_json(install_id, t, env))
            .collect();
        Ok(Value::Array(tabs))
    }

    /// Web pages only, through `nav_policy`, never a private-network address (S10), at most
    /// `MAX_OPENED` per `OPENED_WINDOW` per install.
    fn create(&mut self, install_id: &str, props: &Value, now: Instant) -> Result<(Url, bool), String> {
        let url = props.get("url").and_then(Value::as_str).and_then(|u| Url::parse(u).ok()).ok_or("tabs.create: \"url\" must be an absolute web address.")?;
        if !engine::tarn_ext::opens_as_tab(&url) {
            return Err("tabs.create: this address cannot be opened.".to_owned());
        }
        let opened = self.opened.entry(install_id.to_owned()).or_default();
        while opened.front().is_some_and(|t| now.duration_since(*t) >= OPENED_WINDOW) {
            opened.pop_front();
        }
        if opened.len() >= MAX_OPENED {
            return Err("tabs.create: too many tabs opened.".to_owned());
        }
        opened.push_back(now);
        let active = props.get("active").and_then(Value::as_bool).unwrap_or(true);
        Ok((url, active))
    }

    fn set_action(&mut self, install_id: &str, call: &str, details: &Value, env: &Env) -> Result<Value, String> {
        let tab = match details.get("tabId") {
            None | Some(Value::Null) => None,
            Some(t) => Some(t.as_u64().filter(|t| env.tabs.iter().any(|i| i.id == *t)).ok_or("action: no tab with this \"tabId\".")?),
        };
        let base = self.action(install_id, tab);
        let state = self.actions.entry((install_id.to_owned(), tab)).or_insert(ActionState { title: String::new(), ..base });
        match call {
            "action.setBadgeText" => {
                let text = details.get("text").and_then(Value::as_str).ok_or("action.setBadgeText: \"text\" is missing.")?;
                let text = crate::theme_fetch::shown(text);
                if text.chars().count() > MAX_BADGE_CHARS {
                    return Err("action.setBadgeText: at most 4 characters.".to_owned());
                }
                state.badge_text = text;
            }
            "action.setBadgeBackgroundColor" => state.badge_color = Some(color(details.get("color")).ok_or("action.setBadgeBackgroundColor: invalid color.")?),
            _ => {
                let title = details.get("title").and_then(Value::as_str).ok_or("action.setTitle: \"title\" is missing.")?;
                state.title = crate::theme_fetch::shown(&title.chars().take(MAX_TITLE_CHARS).collect::<String>());
            }
        }
        Ok(Value::Null)
    }

    fn items(&mut self, install_id: &str, env: &Env) -> &mut BTreeMap<String, String> {
        let profile = self.profile;
        self.storage
            .entry(install_id.to_owned())
            .or_insert_with(|| env.store.and_then(|s| s.extension_storage(profile, install_id).ok()).unwrap_or_default().into_iter().collect())
    }

    fn storage_call(&mut self, sender: &Sender, id: Option<u64>, call: &str, arg: &Value, env: &Env) -> Vec<Out> {
        let install_id = sender.install_id.clone();
        let profile = self.profile;
        let wanted = |k: &str| match arg {
            Value::Null => true,
            Value::String(s) => s == k,
            Value::Array(a) => a.iter().any(|x| x.as_str() == Some(k)),
            Value::Object(o) => o.contains_key(k),
            _ => false,
        };
        let parse = |s: &str| serde_json::from_str::<Value>(s).unwrap_or(Value::Null);
        let items = self.items(&install_id, env);
        let mut changes: Vec<(String, Option<String>)> = Vec::new();
        let result: Result<Value, String> = match (call, arg) {
            ("storage.get", _) => {
                let mut out: Map<String, Value> = arg.as_object().cloned().unwrap_or_default();
                out.extend(items.iter().filter(|(k, _)| wanted(k)).map(|(k, v)| (k.clone(), parse(v))));
                Ok(Value::Object(out))
            }
            ("storage.getBytesInUse", _) => Ok(items.iter().filter(|(k, _)| wanted(k)).map(|(k, v)| k.len() + v.len()).sum::<usize>().into()),
            ("storage.set", Value::Object(new)) if new.keys().any(|k| k.len() > MAX_KEY_BYTES) => {
                Err("storage.local.set: a key is longer than 1 KiB.".to_owned())
            }
            ("storage.set", Value::Object(new)) => {
                let total: usize = items.iter().map(|(k, v)| k.len() + v.len()).sum();
                changes = new.iter().map(|(k, v)| (k.clone(), Some(v.to_string()))).collect();
                let after = changes.iter().fold(total, |sum, (k, v)| {
                    let old = items.get(k).map_or(0, |o| k.len() + o.len());
                    sum - old + k.len() + v.as_ref().map_or(0, String::len)
                });
                if after > QUOTA_BYTES {
                    changes.clear();
                    Err("QUOTA_BYTES quota exceeded".to_owned())
                } else {
                    Ok(Value::Null)
                }
            }
            ("storage.set", _) => Err("storage.local.set: items must be an object.".to_owned()),
            ("storage.remove", Value::String(_) | Value::Array(_)) | ("storage.clear", _) => {
                changes = items.keys().filter(|k| call == "storage.clear" || wanted(k)).map(|k| (k.clone(), None)).collect();
                Ok(Value::Null)
            }
            _ => Err(format!("{call} is not available here.")),
        };
        let saved = changes.is_empty() || env.store.is_none_or(|s| s.change_extension_storage(profile, &install_id, &changes).is_ok());
        let mut diff = Map::new();
        if saved {
            for (k, v) in &changes {
                let old = match v {
                    Some(v) => items.insert(k.clone(), v.clone()),
                    None => items.remove(k),
                };
                if old.as_ref() == v.as_ref() {
                    continue;
                }
                let mut change = Map::new();
                change.extend(old.map(|o| ("oldValue".to_owned(), parse(&o))));
                change.extend(v.as_ref().map(|n| ("newValue".to_owned(), parse(n))));
                diff.insert(k.clone(), Value::Object(change));
            }
        }
        let result = if saved { result } else { Err("storage.local: the change could not be saved.".to_owned()) };
        let mut out = Vec::new();
        if !diff.is_empty() {
            let body = json!({"event": "storage.onChanged", "changes": diff}).to_string();
            out.push(Out::Post { install_id: install_id.clone(), target: Target::AllTabs, json: body.clone() });
            for kind in [PageKind::Background, PageKind::Popup] {
                if self.pages.get(&(install_id.clone(), kind)).is_some_and(|p| p.ready) {
                    out.push(Out::Post { install_id: install_id.clone(), target: Target::Page(kind), json: body.clone() });
                }
            }
        }
        out.extend(id.map(|id| self.reply(sender, id, result)));
        out
    }
}

fn color(value: Option<&Value>) -> Option<[u8; 4]> {
    match value? {
        Value::String(s) => {
            let hex = s.strip_prefix('#')?;
            let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
            match digits.as_slice() {
                [r, g, b] => Some([r * 17, g * 17, b * 17, 255]),
                [r1, r2, g1, g2, b1, b2] => Some([r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2, 255]),
                _ => None,
            }
        }
        Value::Array(parts) if parts.len() == 4 => {
            let v: Vec<u8> = parts.iter().map(|p| p.as_u64().filter(|n| *n <= 255).map(|n| n as u8)).collect::<Option<_>>()?;
            Some([v[0], v[1], v[2], v[3]])
        }
        _ => None,
    }
}
