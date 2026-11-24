//! The agent session every shell runs: the user's approval, questions held until answered, the
//! one tool call in flight, timeouts and the bar's text. The shell opens and closes the agent's
//! window, runs each `Op` on it and reports what came back.

use std::collections::HashSet;

use engine::agent_scripts::{Act, AgentScript};
use url::Url;

use super::mcp::Outcome;
use super::{Entry, LOCKED, NO_ANSWER, PROMPT_TIMEOUT_MS, Scope, Session, Tool, Verdict};

pub const ASK_USER_TIMEOUT_MS: i64 = 10 * 60 * 1000;
pub const MAX_TEXT_CHARS: usize = 40_000;
const BUSY_ASKING: &str = "Tarn is already asking the user something; try again shortly.";

/// A tab of the agent's window as the shell sees it now.
#[derive(Clone, Debug, PartialEq)]
pub struct Page<K> {
    pub key: K,
    pub url: Option<Url>,
    pub title: String,
}

pub struct Ctx<'a, K> {
    /// Settings › AI › Let AI agents control Tarn.
    pub enabled: bool,
    /// Settings › Privacy › Open localhost links from other apps.
    pub localhost: bool,
    pub now: i64,
    pub tabs: &'a [Page<K>],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    Connect,
    Site(String),
    /// A page in the agent's window tried to open this site.
    Held(String),
    Sensitive(String),
    User(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Look,
    Act,
    Allow,
    Deny,
    Pause,
    Stop,
}

/// What the shell does on the agent's window.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Open a tab loading this address as a link; answer `Raw::Opened`.
    Open(Url),
    Close,
    /// Load this address in the tab as a link.
    Load(Url),
    Back,
    Forward,
    Reload,
    /// Run this agent script in the tab's isolated world; answer `Raw::Script`.
    Script(String),
    /// Read the text of this page, at most `MAX_TEXT_CHARS`, and nothing once the tab shows
    /// another origin; answer `Raw::Text`.
    Text(Url),
    /// A PNG of the visible page; answer `Raw::Png` with it base64-encoded.
    Screenshot,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Raw<K> {
    Opened(Option<K>),
    Script(String),
    Text(String),
    Png(Option<String>),
    /// The shell dispatched a `Load`, `Close`, `Back`, `Forward` or `Reload`.
    Done,
    /// The tab is gone or asleep and cannot run the op.
    Unavailable,
}

#[derive(Debug, PartialEq)]
pub enum Effect<K, C> {
    Reply(C, Outcome),
    OpenWindow,
    /// Close the agent's window, which wipes its profile.
    CloseWindow,
    /// Show the window and draw its bar again; `true` when a new question appeared, whose buttons
    /// wait for the layout to settle.
    Show(bool),
    Run {
        run: u64,
        tab: Option<K>,
        op: Op,
    },
    /// The sites the gate on the window's tabs lets through; `None` lifts the gate.
    Sites(Option<Vec<String>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Banner {
    pub text: String,
    /// Label, choice, primary.
    pub buttons: Vec<(&'static str, Choice, bool)>,
    /// A question: its buttons other than Stop, Deny and Pause wait for the layout to settle.
    pub asking: bool,
    /// The question's number, which an answer must name; 0 when nothing is asked.
    pub prompt: u64,
}

struct Call<C> {
    tool: Tool,
    token: C,
    /// The user allowed submitting what the agent already typed: submit only, type nothing again.
    submit_only: bool,
}

enum Then<K, C> {
    Call(Call<C>),
    Load(K, Url),
}

struct Held<K, C> {
    id: u64,
    ask: Ask,
    then: Then<K, C>,
    since: i64,
    /// The page a sensitive action was asked for; Allow Once holds only while the tab still shows it.
    page: Option<Url>,
}

struct Running<K, C> {
    run: u64,
    tab: Option<K>,
    call: Call<C>,
    confirmed: bool,
    deadline: i64,
    /// The origin the script was checked for; it returns nothing in any other document.
    origin: String,
    /// A screenshot taken, held back until the secret-field check after it passes too.
    png: Option<String>,
}

struct Active<K, C> {
    conn: u64,
    client: String,
    session: Option<Session<K>>,
    held: Option<Held<K, C>>,
    running: Option<Running<K, C>>,
    paused: Option<Call<C>>,
}

pub struct Controller<K, C> {
    active: Option<Active<K, C>>,
    refused: HashSet<u64>,
    next_run: u64,
    next_prompt: u64,
}

impl<K, C> Default for Controller<K, C> {
    fn default() -> Self {
        Controller { active: None, refused: HashSet::new(), next_run: 0, next_prompt: 0 }
    }
}

impl<K: Copy + PartialEq, C> Controller<K, C> {
    pub fn active(&self) -> bool {
        self.active.is_some()
    }

    /// A tool call from connection `conn`.
    pub fn call(&mut self, conn: u64, client: Option<&str>, tool: Tool, token: C, ctx: &Ctx<K>) -> Vec<Effect<K, C>> {
        let call = Call { tool, token, submit_only: false };
        if !ctx.enabled {
            return vec![Effect::Reply(call.token, Outcome::Error(super::OFF.into()))];
        }
        if self.refused.contains(&conn) {
            return vec![Effect::Reply(call.token, Outcome::Error(super::DENIED.into()))];
        }
        match self.active.as_mut() {
            Some(active) if active.conn != conn => vec![Effect::Reply(call.token, Outcome::Error(super::BUSY.into()))],
            Some(active) if active.session.as_ref().is_some_and(|s| s.paused) => {
                active.paused = Some(call);
                Vec::new()
            }
            Some(_) => self.run(call, None, true, ctx),
            None => {
                let client = super::client_name(client.unwrap_or_default());
                self.active = Some(Active { conn, client, session: None, held: None, running: None, paused: None });
                let mut out = vec![Effect::OpenWindow, Effect::Sites(Some(Vec::new()))];
                out.extend(self.hold(Ask::Connect, Then::Call(call), None, ctx.now));
                out
            }
        }
    }

    /// A button of the agent bar drawn for question `prompt` (`Banner::prompt`); an answer to a
    /// question no longer asked is ignored.
    pub fn answer(&mut self, choice: Choice, prompt: u64, ctx: &Ctx<K>) -> Vec<Effect<K, C>> {
        let Some(active) = self.active.as_mut() else { return Vec::new() };
        if choice == Choice::Stop {
            return self.stop(super::STOPPED, true, true);
        }
        if choice == Choice::Pause {
            let Some(session) = active.session.as_mut() else { return Vec::new() };
            session.paused = !session.paused;
            let resumed = if session.paused { None } else { active.paused.take() };
            let mut out = vec![Effect::Show(false)];
            if let Some(call) = resumed {
                out.extend(self.run(call, None, true, ctx));
            }
            return out;
        }
        let Some(held) = active.held.take_if(|h| h.id == prompt) else { return Vec::new() };
        let allowed = choice != Choice::Deny;
        let mut out = vec![Effect::Show(false)];
        if let Ask::Site(site) | Ask::Held(site) = &held.ask
            && let Some(session) = active.session.as_mut()
        {
            session.answer_site(site, allowed);
            out.push(Effect::Sites(Some(session.allowed_sites())));
        }
        if held.ask == Ask::Connect && allowed {
            let scope = if choice == Choice::Act { Scope::Act } else { Scope::Look };
            active.session = Some(Session::new(scope, active.client.clone(), ctx.now));
        }
        match (held.ask, held.then) {
            (Ask::Connect, Then::Call(call)) if allowed => out.extend(self.run(call, None, true, ctx)),
            (Ask::Connect, Then::Call(call)) => {
                out.push(Effect::Reply(call.token, Outcome::Error(super::DENIED.into())));
                out.extend(self.stop(super::DENIED, true, true));
            }
            (Ask::Site(_), Then::Call(call)) if allowed => out.extend(self.run(call, None, false, ctx)),
            (Ask::Site(site), Then::Call(call)) => {
                out.extend(self.finish(call, None, Outcome::Error(format!("The user did not allow the agent on {site}.")), ctx.now))
            }
            (Ask::Site(_) | Ask::Held(_), Then::Load(key, url)) if allowed => out.push(Effect::Run { run: 0, tab: Some(key), op: Op::Load(url) }),
            (Ask::Sensitive(_), Then::Call(call)) if allowed => out.extend(self.run(call, held.page, false, ctx)),
            (Ask::Sensitive(_), Then::Call(call)) => out.extend(self.finish(call, None, Outcome::Error(super::DENIED.into()), ctx.now)),
            (Ask::User(_), Then::Call(call)) => {
                let text = if allowed { "The user pressed Done." } else { "The user cancelled." };
                out.extend(self.finish(call, None, Outcome::Text(text.into()), ctx.now));
            }
            (_, Then::Load(..)) | (Ask::Held(_), Then::Call(_)) => {}
        }
        out
    }

    /// A page-started navigation of an agent tab to a site without a grant, which the gate held.
    pub fn held_navigation(&mut self, key: K, url: Url, now: i64) -> Vec<Effect<K, C>> {
        let Some(site) = super::agent_url(url.as_str(), false).as_ref().and_then(super::site_of) else { return Vec::new() };
        let Some(session) = self.active.as_ref().and_then(|a| a.session.as_ref()) else { return Vec::new() };
        if session.site_allowed(&site) == Some(false) {
            return Vec::new();
        }
        self.hold(Ask::Held(site), Then::Load(key, url), None, now)
    }

    /// What the shell's op `run` came back with.
    pub fn raw(&mut self, run: u64, raw: Raw<K>, ctx: &Ctx<K>) -> Vec<Effect<K, C>> {
        let Some(active) = self.active.as_mut() else { return Vec::new() };
        let Some(running) = active.running.take_if(|r| r.run == run) else { return Vec::new() };
        let page = running.tab.and_then(|k| page_of(ctx.tabs, k));
        let outcome = match (&running.call.tool, raw) {
            (_, Raw::Unavailable) => Outcome::Error(LOCKED.into()),
            (Tool::Navigate { url, .. }, Raw::Done) => Outcome::Text(format!("Loading {url}. Use wait_for, then snapshot.")),
            (_, Raw::Done) => Outcome::Text("Done. Use wait_for, then snapshot.".into()),
            (tool @ (Tool::Screenshot { .. } | Tool::WaitFor { .. }), _) if tool.touches_page() && !allowed_here(active.session.as_ref(), page.as_ref()) => {
                Outcome::Error("The tab moved to a site the agent may not use yet.".into())
            }
            (Tool::TabOpen { url }, Raw::Opened(Some(key))) => {
                let n = active.session.as_mut().map(|s| {
                    s.sync_tabs(&ctx.tabs.iter().map(|p| p.key).chain([key]).collect::<Vec<_>>());
                    s.id_of(key).unwrap_or_default()
                });
                Outcome::Text(format!("Opened tab {} at {url}. Use wait_for, then snapshot.", n.unwrap_or_default()))
            }
            (Tool::TabOpen { .. }, _) => Outcome::Error("Tarn could not open the tab.".into()),
            (Tool::WaitFor { .. }, Raw::Script(raw)) if raw.is_empty() => {
                if ctx.now < running.deadline {
                    active.running = Some(running);
                    return Vec::new();
                }
                Outcome::Text("Still not there; the wait timed out.".into())
            }
            (Tool::WaitFor { .. }, Raw::Script(_)) => Outcome::Text("Ready.".into()),
            (Tool::Snapshot { .. }, Raw::Script(raw)) | (Tool::ReadText { .. }, Raw::Text(raw)) if raw.is_empty() => Outcome::Error(LOCKED.into()),
            (Tool::Snapshot { .. }, Raw::Script(raw)) | (Tool::ReadText { .. }, Raw::Text(raw)) => Outcome::Text(super::page_answer(&raw)),
            (Tool::Screenshot { .. }, Raw::Script(raw)) if raw == "0" && running.png.is_some() => Outcome::Image(running.png.clone().unwrap_or_default()),
            (Tool::Screenshot { .. }, Raw::Script(raw)) if raw == "0" => {
                let (run, tab) = (running.run, running.tab);
                active.running = Some(Running { deadline: ctx.now + PROMPT_TIMEOUT_MS, ..running });
                return vec![Effect::Run { run, tab, op: Op::Screenshot }];
            }
            (Tool::Screenshot { .. }, Raw::Script(raw)) if raw.is_empty() => Outcome::Error(LOCKED.into()),
            (Tool::Screenshot { .. }, Raw::Script(_)) => {
                Outcome::Error("Tarn takes no screenshot while a password, payment or one-time-code field on the page holds a value.".into())
            }
            (Tool::Screenshot { .. }, Raw::Png(Some(png))) => {
                let (run, tab) = (running.run, running.tab);
                let op = Op::Script(bound(AgentScript::Sensitive.source(), &running.origin));
                active.running = Some(Running { png: Some(png), deadline: ctx.now + PROMPT_TIMEOUT_MS, ..running });
                return vec![Effect::Run { run, tab, op }];
            }
            (Tool::Screenshot { .. }, _) => Outcome::Error("Tarn could not take a screenshot of this tab.".into()),
            (_, Raw::Script(raw)) => match raw.as_str() {
                "confirm" | "confirm:typed" if !running.confirmed => {
                    let site = page.as_ref().and_then(super::site_of).unwrap_or_default();
                    let call = Call { submit_only: raw == "confirm:typed", ..running.call };
                    return self.hold(Ask::Sensitive(site), Then::Call(call), page, ctx.now);
                }
                "stale" => Outcome::Error("That element is gone; take a new snapshot.".into()),
                "refused:password" => Outcome::Error("Tarn never lets an agent type into a password field. Use ask_user so the user signs in.".into()),
                "" => Outcome::Error(LOCKED.into()),
                ok if ok.starts_with("ok") => Outcome::Text(match ok.strip_prefix("ok:") {
                    Some(info) => format!("Done. Scrolled to {info}."),
                    None => "Done. Take a new snapshot to see the result.".into(),
                }),
                error => Outcome::Error(error.strip_prefix("error:").unwrap_or(error).to_owned()),
            },
            _ => Outcome::Error("Unexpected answer from the page.".into()),
        };
        self.finish(running.call, page.as_ref(), outcome, ctx.now)
    }

    /// The connection `conn` closed.
    pub fn closed(&mut self, conn: u64) -> Vec<Effect<K, C>> {
        self.refused.remove(&conn);
        if self.active.as_ref().is_some_and(|a| a.conn == conn) { self.stop(super::STOPPED, true, false) } else { Vec::new() }
    }

    /// Called every `TICK` while `active`: times out questions and calls, ends an idle session
    /// and polls a wait.
    pub fn tick(&mut self, ctx: &Ctx<K>) -> Vec<Effect<K, C>> {
        let Some(active) = self.active.as_mut() else { return Vec::new() };
        let t = ctx.now;
        if let Some(held) = active.held.as_ref() {
            let limit = if matches!(held.ask, Ask::User(_)) { ASK_USER_TIMEOUT_MS } else { PROMPT_TIMEOUT_MS };
            if t - held.since < limit {
                return Vec::new();
            }
            let held = active.held.take().expect("checked above");
            let mut out = vec![Effect::Show(false)];
            if let Then::Call(call) = held.then {
                out.extend(self.finish(call, None, Outcome::Error(NO_ANSWER.into()), t));
            }
            if held.ask == Ask::Connect {
                out.extend(self.stop(NO_ANSWER, true, false));
            }
            return out;
        }
        if let Some(running) = active.running.as_ref() {
            if t >= running.deadline + PROMPT_TIMEOUT_MS || (t >= running.deadline && !matches!(running.call.tool, Tool::WaitFor { .. })) {
                let running = active.running.take().expect("checked above");
                return self.finish(running.call, None, Outcome::Error("The page did not answer in time.".into()), t);
            }
            if let Tool::WaitFor { text, .. } = &running.call.tool
                && (text.is_none() || allowed_here(active.session.as_ref(), running.tab.and_then(|k| page_of(ctx.tabs, k)).as_ref()))
            {
                let op = Op::Script(bound(AgentScript::Wait { text: text.as_deref() }.source(), &running.origin));
                return vec![Effect::Run { run: running.run, tab: running.tab, op }];
            }
            return Vec::new();
        }
        if active.session.as_ref().is_some_and(|s| s.idle(t)) {
            return self.stop("The agent session ended after 15 minutes without activity.", true, false);
        }
        Vec::new()
    }

    /// Ends the session: waiting calls are refused, with `close` the window closes, and with
    /// `refuse` (the user denied or stopped it) the connection may not ask again.
    pub fn stop(&mut self, why: &str, close: bool, refuse: bool) -> Vec<Effect<K, C>> {
        let Some(mut active) = self.active.take() else { return Vec::new() };
        if refuse {
            self.refused.insert(active.conn);
        }
        let held = active.held.take().and_then(|h| match h.then {
            Then::Call(call) => Some(call),
            Then::Load(..) => None,
        });
        let mut out: Vec<Effect<K, C>> = [held, active.running.take().map(|r| r.call), active.paused.take()]
            .into_iter()
            .flatten()
            .map(|c| Effect::Reply(c.token, Outcome::Error(why.into())))
            .collect();
        out.push(Effect::Sites(None));
        out.push(if close { Effect::CloseWindow } else { Effect::Show(false) });
        out
    }

    pub fn banner(&self) -> Option<Banner> {
        let active = self.active.as_ref()?;
        let client = &active.client;
        let stop = ("Stop", Choice::Stop, false);
        let (text, buttons, asking) = match active.held.as_ref().map(|h| &h.ask) {
            Some(Ask::Connect) => (
                format!("“{client}” (the name it gives) wants to control this window. It starts with no sites allowed."),
                vec![("Allow Looking", Choice::Look, false), ("Allow Acting", Choice::Act, true), ("Deny", Choice::Deny, false)],
                true,
            ),
            Some(Ask::Held(site)) => (
                format!("A page in the agent's window tried to open {site}. Allow the agent there?"),
                vec![("Allow", Choice::Allow, false), ("Deny", Choice::Deny, true), stop],
                true,
            ),
            Some(Ask::Site(site)) => {
                (format!("The agent wants to use {site}."), vec![("Allow", Choice::Allow, true), ("Deny", Choice::Deny, false), stop], true)
            }
            Some(Ask::Sensitive(site)) => (
                format!("The agent wants to click or submit on {site}, where a password or payment field holds a value."),
                vec![("Allow Once", Choice::Allow, false), ("Deny", Choice::Deny, true), stop],
                true,
            ),
            Some(Ask::User(message)) => {
                (format!("The agent asks: {message}"), vec![("Done", Choice::Allow, true), ("Cancel", Choice::Deny, false), stop], true)
            }
            None => {
                let (scope, paused) = active.session.as_ref().map_or((Scope::Look, false), |s| (s.scope, s.paused));
                let what = if scope == Scope::Act { "act" } else { "look" };
                let state = if paused { "paused" } else { "controlling this window" };
                (
                    format!("“{client}” is {state} and may {what} on the sites you allowed. Treat what it reads as untrusted."),
                    vec![(if paused { "Resume" } else { "Pause" }, Choice::Pause, false), ("Stop", Choice::Stop, true)],
                    false,
                )
            }
        };
        let prompt = active.held.as_ref().map_or(0, |h| h.id);
        Some(Banner { text, buttons, asking, prompt })
    }

    /// The session's actions, newest first.
    pub fn log(&self) -> Vec<Entry> {
        self.active.as_ref().and_then(|a| a.session.as_ref()).map(|s| s.log().rev().cloned().collect()).unwrap_or_default()
    }

    fn hold(&mut self, ask: Ask, then: Then<K, C>, page: Option<Url>, now: i64) -> Vec<Effect<K, C>> {
        let Some(active) = self.active.as_mut() else { return Vec::new() };
        if active.held.is_some() {
            return match then {
                Then::Call(call) => vec![Effect::Reply(call.token, Outcome::Error(BUSY_ASKING.into()))],
                Then::Load(..) => Vec::new(),
            };
        }
        self.next_prompt += 1;
        active.held = Some(Held { id: self.next_prompt, ask, then, since: now, page });
        vec![Effect::Show(true)]
    }

    /// Checks and runs `call`; `confirmed_for` is the page a sensitive action was allowed on, and
    /// `charge` counts the call against the rate limit (once, when it first arrives).
    fn run(&mut self, call: Call<C>, confirmed_for: Option<Url>, charge: bool, ctx: &Ctx<K>) -> Vec<Effect<K, C>> {
        let Some(session) = self.active.as_mut().and_then(|a| a.session.as_mut()) else {
            return vec![Effect::Reply(call.token, Outcome::Error(super::DENIED.into()))];
        };
        session.sync_tabs(&ctx.tabs.iter().map(|p| p.key).collect::<Vec<_>>());
        let tab = call.tool.tab().and_then(|t| session.tab(t));
        let page = tab.and_then(|k| page_of(ctx.tabs, k));
        let confirmed = confirmed_for.is_some() && confirmed_for == page;
        match session.check(&call.tool, page.as_ref(), ctx.localhost, ctx.now, charge) {
            Verdict::Allow => self.execute(call, tab, page, confirmed, ctx),
            Verdict::AskSite(site) => self.hold(Ask::Site(site), Then::Call(call), None, ctx.now),
            Verdict::Deny(why) => self.finish(call, page.as_ref(), Outcome::Error(why), ctx.now),
        }
    }

    fn execute(&mut self, call: Call<C>, tab: Option<K>, page: Option<Url>, confirmed: bool, ctx: &Ctx<K>) -> Vec<Effect<K, C>> {
        let origin = guard(page.as_ref());
        let script = |s: AgentScript| Op::Script(bound(s.source(), &origin));
        let act = |target: u32, act: Act| script(AgentScript::Act { target, act, confirmed });
        let op = match &call.tool {
            Tool::AskUser { message } => {
                let message = crate::text::bounded_title(message);
                return self.hold(Ask::User(message), Then::Call(call), None, ctx.now);
            }
            Tool::TabsList => {
                let Some(session) = self.active.as_ref().and_then(|a| a.session.as_ref()) else { return Vec::new() };
                let lines: Vec<String> = session
                    .tabs()
                    .iter()
                    .filter_map(|(n, key)| {
                        let page = ctx.tabs.iter().find(|p| p.key == *key)?;
                        let site = page.url.as_ref().and_then(super::site_of);
                        Some(match (&page.url, site) {
                            (Some(url), Some(site)) if session.site_allowed(&site) != Some(true) => {
                                format!("[{n}] a page on a site the agent may not use yet ({site})")
                            }
                            (url, _) => format!("[{n}] {} — {}", url.as_ref().map_or("about:blank", Url::as_str), crate::text::bounded_title(&page.title)),
                        })
                    })
                    .collect();
                let text = if lines.is_empty() { "No agent tabs. Open one with tab_open.".to_owned() } else { super::page_answer(&lines.join("\n")) };
                return self.finish(call, None, Outcome::Text(text), ctx.now);
            }
            Tool::TabOpen { url } => match super::agent_url(url, ctx.localhost) {
                Some(url) => Op::Open(url),
                None => return self.finish(call, None, Outcome::Error("Tarn does not let an agent open this address.".into()), ctx.now),
            },
            Tool::Navigate { url, .. } => match super::agent_url(url, ctx.localhost) {
                Some(url) => Op::Load(url),
                None => return self.finish(call, None, Outcome::Error("Tarn does not let an agent open this address.".into()), ctx.now),
            },
            Tool::TabClose { .. } => Op::Close,
            Tool::Back { .. } => Op::Back,
            Tool::Forward { .. } => Op::Forward,
            Tool::Reload { .. } => Op::Reload,
            Tool::Snapshot { .. } => script(AgentScript::Snapshot),
            Tool::ReadText { .. } => match &page {
                Some(page) => Op::Text(page.clone()),
                None => return self.finish(call, None, Outcome::Error(LOCKED.into()), ctx.now),
            },
            Tool::Screenshot { .. } => script(AgentScript::Sensitive),
            Tool::WaitFor { text, .. } => script(AgentScript::Wait { text: text.as_deref() }),
            Tool::Click { target, .. } => act(*target, Act::Click),
            Tool::Type { target, .. } if call.submit_only => act(*target, Act::Submit),
            Tool::Type { target, text, submit, .. } => act(*target, Act::Type { text, submit: *submit }),
            Tool::Select { target, value, .. } => act(*target, Act::Select(value)),
            Tool::PressKey { key, .. } => act(0, Act::Key(key.name())),
            Tool::Scroll { target, down, .. } => act(target.unwrap_or(0), Act::Scroll { down: *down }),
        };
        let deadline = match &call.tool {
            Tool::WaitFor { ms, .. } => ctx.now + i64::from(*ms),
            _ => ctx.now + PROMPT_TIMEOUT_MS,
        };
        self.next_run += 1;
        let run = self.next_run;
        let Some(active) = self.active.as_mut() else { return Vec::new() };
        active.running = Some(Running { run, tab, call, confirmed, deadline, origin, png: None });
        vec![Effect::Run { run, tab, op }]
    }

    fn finish(&mut self, call: Call<C>, page: Option<&Url>, outcome: Outcome, now: i64) -> Vec<Effect<K, C>> {
        if let Some(session) = self.active.as_mut().and_then(|a| a.session.as_mut()) {
            let summary = match &outcome {
                Outcome::Text(t) => t.lines().next().unwrap_or_default().to_owned(),
                Outcome::Image(_) => "Screenshot".to_owned(),
                Outcome::Error(e) => format!("Refused: {e}"),
            };
            session.record(now, &call.tool, page.and_then(super::site_of), summary);
        }
        vec![Effect::Reply(call.token, outcome), Effect::Show(false)]
    }
}

/// Whether the session may use the site `page` shows now; a page without a site counts as allowed.
fn allowed_here<K: Copy + PartialEq>(session: Option<&Session<K>>, page: Option<&Url>) -> bool {
    page.and_then(super::site_of).is_none_or(|site| session.is_some_and(|s| s.site_allowed(&site) == Some(true)))
}

/// The condition a document must meet to run a script checked for `page`: its origin, or for a
/// page without one ("null", shared by every opaque document) that exact address as well.
fn guard(page: Option<&Url>) -> String {
    let js = engine::handover::js_string;
    match page.map(|p| (p, p.origin().ascii_serialization())) {
        Some((_, origin)) if origin != "null" => format!("location.origin === {}", js(&origin)),
        Some((page, _)) => format!("location.origin === \"null\" && location.href === {}", js(page.as_str())),
        None => "location.origin === \"null\" && location.href === \"about:blank\"".to_owned(),
    }
}

/// `source` that runs only where `guard` holds and answers nothing elsewhere.
fn bound(source: String, guard: &str) -> String {
    format!("if (!({guard})) return '';\n{source}")
}

fn page_of<K: PartialEq>(tabs: &[Page<K>], key: K) -> Option<Url> {
    tabs.iter().find(|p| p.key == key).and_then(|p| p.url.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    type Ctl = Controller<u8, &'static str>;

    fn ctx(tabs: &[Page<u8>], now: i64) -> Ctx<'_, u8> {
        Ctx { enabled: true, localhost: false, now, tabs }
    }

    fn page(key: u8, url: &str) -> Page<u8> {
        Page { key, url: Url::parse(url).ok(), title: format!("t{key}") }
    }

    fn replies(effects: &[Effect<u8, &'static str>]) -> Vec<(&'static str, Outcome)> {
        effects.iter().filter_map(|e| if let Effect::Reply(t, o) = e { Some((*t, o.clone())) } else { None }).collect()
    }

    fn runs(effects: &[Effect<u8, &'static str>]) -> Vec<(u64, Option<u8>, Op)> {
        effects.iter().filter_map(|e| if let Effect::Run { run, tab, op } = e { Some((*run, *tab, op.clone())) } else { None }).collect()
    }

    fn ans(c: &mut Ctl, choice: Choice, ctx: &Ctx<'_, u8>) -> Vec<Effect<u8, &'static str>> {
        let prompt = c.banner().map_or(0, |b| b.prompt);
        c.answer(choice, prompt, ctx)
    }

    fn approved(scope: Choice) -> Ctl {
        let mut c = Ctl::default();
        let e = c.call(1, Some("claude-code"), Tool::TabsList, "a", &ctx(&[], 0));
        assert!(e.contains(&Effect::OpenWindow) && e.contains(&Effect::Show(true)));
        assert!(replies(&e).is_empty(), "nothing runs before the user allows it");
        assert_eq!(c.banner().unwrap().buttons.len(), 3);
        let e = ans(&mut c, scope, &ctx(&[], 10));
        assert_eq!(replies(&e), vec![("a", Outcome::Text("No agent tabs. Open one with tab_open.".into()))]);
        c
    }

    #[test]
    fn tabs_list_hides_pages_on_sites_without_a_grant() {
        let mut c = approved(Choice::Look);
        let tabs = [page(3, "https://mail.private.test/inbox"), page(4, "about:blank")];
        let e = c.call(1, None, Tool::TabsList, "l", &ctx(&tabs, 2000));
        let Outcome::Text(text) = &replies(&e)[0].1 else { panic!("{e:?}") };
        assert!(!text.contains("inbox") && !text.contains("t3") && text.contains("private.test") && text.contains("about:blank"), "{text}");
    }

    #[test]
    fn nothing_runs_while_agent_control_is_off() {
        let mut c = Ctl::default();
        let e = c.call(1, None, Tool::TabsList, "a", &Ctx { enabled: false, localhost: false, now: 0, tabs: &[] });
        assert_eq!(replies(&e), vec![("a", Outcome::Error(super::super::OFF.into()))]);
        assert!(!c.active());
    }

    #[test]
    fn a_denied_agent_is_refused_until_it_reconnects_and_others_are_busy() {
        let mut c = Ctl::default();
        c.call(1, None, Tool::TabsList, "a", &ctx(&[], 0));
        assert_eq!(replies(&c.call(2, None, Tool::TabsList, "b", &ctx(&[], 1))), vec![("b", Outcome::Error(super::super::BUSY.into()))]);
        let e = ans(&mut c, Choice::Deny, &ctx(&[], 2));
        assert!(e.contains(&Effect::CloseWindow) && e.contains(&Effect::Sites(None)));
        assert_eq!(replies(&e)[0].1, Outcome::Error(super::super::DENIED.into()));
        assert_eq!(replies(&c.call(1, None, Tool::TabsList, "c", &ctx(&[], 3)))[0].1, Outcome::Error(super::super::DENIED.into()));
        c.closed(1);
        assert!(c.call(1, None, Tool::TabsList, "d", &ctx(&[], 4)).contains(&Effect::OpenWindow));
    }

    #[test]
    fn a_new_site_is_asked_before_anything_loads() {
        let mut c = approved(Choice::Act);
        let e = c.call(1, None, Tool::TabOpen { url: "https://example.com/".into() }, "a", &ctx(&[], 2000));
        assert!(runs(&e).is_empty() && replies(&e).is_empty());
        assert!(c.banner().unwrap().text.contains("example.com"));
        let e = ans(&mut c, Choice::Allow, &ctx(&[], 2100));
        assert!(e.contains(&Effect::Sites(Some(vec!["example.com".into()]))));
        let [(run, None, Op::Open(url))] = &runs(&e)[..] else { panic!("{e:?}") };
        assert_eq!(url.as_str(), "https://example.com/");
        let tabs = [page(7, "https://example.com/")];
        let e = c.raw(*run, Raw::Opened(Some(7)), &ctx(&tabs, 2200));
        assert!(matches!(&replies(&e)[0].1, Outcome::Text(t) if t.starts_with("Opened tab 1")));
    }

    #[test]
    fn a_denied_site_fails_the_call() {
        let mut c = approved(Choice::Act);
        c.call(1, None, Tool::TabOpen { url: "https://evil.test/".into() }, "a", &ctx(&[], 2000));
        let e = ans(&mut c, Choice::Deny, &ctx(&[], 2100));
        assert!(matches!(&replies(&e)[0].1, Outcome::Error(t) if t.contains("evil.test")));
        assert!(c.active(), "denying a site keeps the session");
    }

    #[test]
    fn a_click_on_a_page_with_a_filled_secret_asks_and_then_runs_confirmed() {
        let mut c = approved(Choice::Act);
        let tabs = [page(3, "https://shop.test/pay")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&tabs, 2000));
        let site_ask = c.call(1, None, Tool::Click { tab: 1, target: 5 }, "a", &ctx(&tabs, 4000));
        assert!(site_ask.contains(&Effect::Show(true)));
        let e = ans(&mut c, Choice::Allow, &ctx(&tabs, 4100));
        let [(run, Some(3), Op::Script(src))] = &runs(&e)[..] else { panic!("{e:?}") };
        assert!(src.contains("confirmed = false"));
        let e = c.raw(*run, Raw::Script("confirm".into()), &ctx(&tabs, 4200));
        assert!(replies(&e).is_empty() && e.contains(&Effect::Show(true)));
        assert!(c.banner().unwrap().text.contains("password or payment"));
        let e = ans(&mut c, Choice::Allow, &ctx(&tabs, 4300));
        let [(run, Some(3), Op::Script(src))] = &runs(&e)[..] else { panic!("{e:?}") };
        assert!(src.contains("confirmed = true"));
        let e = c.raw(*run, Raw::Script("ok".into()), &ctx(&tabs, 4400));
        assert!(matches!(&replies(&e)[0].1, Outcome::Text(t) if t.starts_with("Done")));
    }

    #[test]
    fn look_only_refuses_actions_and_snapshots_are_marked_untrusted() {
        let mut c = approved(Choice::Look);
        let tabs = [page(3, "https://a.test/")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&tabs, 2000));
        let e = c.call(1, None, Tool::Click { tab: 1, target: 1 }, "a", &ctx(&tabs, 4000));
        assert!(matches!(&replies(&e)[0].1, Outcome::Error(t) if t.contains("look only")));
        c.call(1, None, Tool::Snapshot { tab: 1 }, "s", &ctx(&tabs, 6000));
        let e = ans(&mut c, Choice::Allow, &ctx(&tabs, 6100));
        let run = runs(&e)[0].0;
        let e = c.raw(run, Raw::Script("Address: x".into()), &ctx(&tabs, 6200));
        assert!(matches!(&replies(&e)[0].1, Outcome::Text(t) if t.starts_with(super::super::UNTRUSTED)));
        assert_eq!(c.log()[0].tool, "snapshot");
    }

    #[test]
    fn a_held_navigation_loads_once_allowed() {
        let mut c = approved(Choice::Act);
        let e = c.held_navigation(3, Url::parse("https://other.test/x").unwrap(), 2000);
        assert_eq!(e, vec![Effect::Show(true)]);
        assert!(c.banner().unwrap().text.starts_with("A page in the agent's window"), "never worded as the agent's own request");
        let e = ans(&mut c, Choice::Allow, &ctx(&[], 2100));
        assert!(runs(&e).iter().any(|(_, tab, op)| *tab == Some(3) && matches!(op, Op::Load(u) if u.as_str() == "https://other.test/x")));
    }

    #[test]
    fn a_held_navigation_to_another_scheme_is_dropped() {
        let mut c = approved(Choice::Act);
        assert!(c.held_navigation(3, Url::parse("zoommtg://zoom.us/join").unwrap(), 2000).is_empty());
        assert!(c.held_navigation(3, Url::parse("file://host/etc/passwd").unwrap(), 2000).is_empty());
    }

    #[test]
    fn a_wait_ends_once_its_tab_leaves_the_allowed_site() {
        let mut c = approved(Choice::Look);
        let allowed = [page(3, "https://a.test/")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&allowed, 2000));
        c.call(1, None, Tool::WaitFor { tab: 1, text: Some("secret".into()), ms: 5000 }, "w", &ctx(&allowed, 4000));
        let e = ans(&mut c, Choice::Allow, &ctx(&allowed, 4100));
        let run = runs(&e)[0].0;
        let moved = [page(3, "https://other.test/")];
        assert!(runs(&c.tick(&ctx(&moved, 4200))).is_empty(), "no probe of the new site");
        let e = c.raw(run, Raw::Script(String::new()), &ctx(&moved, 4300));
        assert!(matches!(&replies(&e)[0].1, Outcome::Error(t) if t.contains("moved")));
    }

    #[test]
    fn pause_holds_the_next_call_until_resumed() {
        let mut c = approved(Choice::Act);
        ans(&mut c, Choice::Pause, &ctx(&[], 20));
        assert!(c.call(1, None, Tool::TabsList, "a", &ctx(&[], 30)).is_empty());
        let e = ans(&mut c, Choice::Pause, &ctx(&[], 40));
        assert_eq!(replies(&e).len(), 1);
    }

    #[test]
    fn questions_time_out_and_an_unanswered_connect_closes_the_window() {
        let mut c = Ctl::default();
        c.call(1, None, Tool::TabsList, "a", &ctx(&[], 0));
        assert!(c.tick(&ctx(&[], PROMPT_TIMEOUT_MS - 1)).is_empty());
        let e = c.tick(&ctx(&[], PROMPT_TIMEOUT_MS));
        assert_eq!(replies(&e)[0].1, Outcome::Error(NO_ANSWER.into()));
        assert!(e.contains(&Effect::CloseWindow) && !c.active());
    }

    #[test]
    fn a_wait_polls_until_ready_or_its_deadline() {
        let mut c = approved(Choice::Look);
        let tabs = [page(3, "https://a.test/")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&tabs, 2000));
        let e = c.call(1, None, Tool::WaitFor { tab: 1, text: None, ms: 1000 }, "w", &ctx(&tabs, 4000));
        let run = runs(&e)[0].0;
        assert!(c.raw(run, Raw::Script(String::new()), &ctx(&tabs, 4100)).is_empty());
        assert_eq!(runs(&c.tick(&ctx(&tabs, 4200)))[0].0, run);
        let e = c.raw(run, Raw::Script(String::new()), &ctx(&tabs, 5001));
        assert!(matches!(&replies(&e)[0].1, Outcome::Text(t) if t.contains("timed out")));
    }

    #[test]
    fn stop_refuses_whatever_waits_and_ends_the_session() {
        let mut c = approved(Choice::Act);
        c.call(1, None, Tool::AskUser { message: "Sign in".into() }, "u", &ctx(&[], 2000));
        assert!(c.banner().unwrap().text.contains("Sign in"));
        let e = ans(&mut c, Choice::Stop, &ctx(&[], 2100));
        assert_eq!(replies(&e), vec![("u", Outcome::Error(super::super::STOPPED.into()))]);
        assert!(e.contains(&Effect::CloseWindow) && !c.active() && c.banner().is_none());
    }

    #[test]
    fn an_idle_session_closes_its_window_and_may_ask_again() {
        let mut c = approved(Choice::Act);
        let e = c.tick(&ctx(&[], super::super::IDLE_MS + 10));
        assert!(e.contains(&Effect::CloseWindow) && !c.active(), "closing the window wipes the profile");
        assert!(c.call(1, None, Tool::TabsList, "b", &ctx(&[], super::super::IDLE_MS + 20)).contains(&Effect::OpenWindow));
    }

    #[test]
    fn a_screenshot_is_refused_while_a_secret_field_holds_a_value() {
        let mut c = approved(Choice::Look);
        let tabs = [page(3, "https://a.test/")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&tabs, 2000));
        c.call(1, None, Tool::Screenshot { tab: 1 }, "s", &ctx(&tabs, 4000));
        let e = ans(&mut c, Choice::Allow, &ctx(&tabs, 4100));
        let [(run, Some(3), Op::Script(_))] = &runs(&e)[..] else { panic!("{e:?}") };
        let e = c.raw(*run, Raw::Script("1".into()), &ctx(&tabs, 4200));
        assert!(matches!(&replies(&e)[0].1, Outcome::Error(t) if t.contains("no screenshot")));
        c.call(1, None, Tool::Screenshot { tab: 1 }, "t", &ctx(&tabs, 6000));
        let e = c.raw(*run + 1, Raw::Script("0".into()), &ctx(&tabs, 6100));
        assert_eq!(runs(&e), vec![(*run + 1, Some(3), Op::Screenshot)]);
        let e = c.raw(*run + 1, Raw::Png(Some("iVBOR".into())), &ctx(&tabs, 6200));
        let [(_, Some(3), Op::Script(check))] = &runs(&e)[..] else { panic!("the capture is checked again: {e:?}") };
        assert!(check.starts_with("if (!(location.origin === \"https://a.test\"))"), "{check}");
        let e = c.raw(*run + 1, Raw::Script("0".into()), &ctx(&tabs, 6300));
        assert_eq!(replies(&e)[0].1, Outcome::Image("iVBOR".into()));
        c.call(1, None, Tool::Screenshot { tab: 1 }, "u", &ctx(&tabs, 8000));
        let e = c.raw(*run + 2, Raw::Script("0".into()), &ctx(&tabs, 8100));
        let e2 = c.raw(*run + 2, Raw::Png(Some("iVBOR".into())), &ctx(&tabs, 8200));
        assert!(runs(&e).len() == 1 && runs(&e2).len() == 1);
        let e = c.raw(*run + 2, Raw::Script("1".into()), &ctx(&tabs, 8300));
        assert!(matches!(&replies(&e)[0].1, Outcome::Error(t) if t.contains("no screenshot")), "a field filled during the capture withholds it");
        c.call(1, None, Tool::Screenshot { tab: 1 }, "v", &ctx(&tabs, 10_000));
        let e = c.raw(*run + 3, Raw::Script(String::new()), &ctx(&tabs, 10_100));
        assert_eq!(replies(&e)[0].1, Outcome::Error(LOCKED.into()), "no answer, as under Lockdown, takes no screenshot");
    }

    #[test]
    fn an_answer_to_a_question_no_longer_asked_is_ignored() {
        let mut c = approved(Choice::Act);
        c.call(1, None, Tool::TabOpen { url: "https://a.test/".into() }, "a", &ctx(&[], 2000));
        let stale = c.banner().unwrap().prompt;
        c.tick(&ctx(&[], 2000 + PROMPT_TIMEOUT_MS));
        c.call(1, None, Tool::TabOpen { url: "https://b.test/".into() }, "b", &ctx(&[], 2000 + PROMPT_TIMEOUT_MS + 10));
        assert!(c.answer(Choice::Allow, stale, &ctx(&[], 2000 + PROMPT_TIMEOUT_MS + 20)).is_empty(), "the old Allow grants nothing");
        assert!(c.banner().unwrap().text.contains("b.test"));
    }

    #[test]
    fn allow_once_holds_only_for_the_page_it_was_asked_on() {
        let mut c = approved(Choice::Act);
        let shop = [page(3, "https://shop.test/pay")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&shop, 2000));
        c.call(1, None, Tool::Click { tab: 1, target: 5 }, "a", &ctx(&shop, 4000));
        let e = ans(&mut c, Choice::Allow, &ctx(&shop, 4100));
        let run = runs(&e)[0].0;
        c.raw(run, Raw::Script("confirm".into()), &ctx(&shop, 4200));
        let moved = [page(3, "https://shop.test/other")];
        let e = ans(&mut c, Choice::Allow, &ctx(&moved, 4300));
        let [(_, Some(3), Op::Script(src))] = &runs(&e)[..] else { panic!("{e:?}") };
        assert!(src.contains("confirmed = false"), "another page gets a fresh check");
    }

    #[test]
    fn a_submit_allowed_after_typing_submits_without_typing_again() {
        let mut c = approved(Choice::Act);
        let tabs = [page(3, "https://shop.test/pay")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&tabs, 2000));
        c.call(1, None, Tool::Type { tab: 1, target: 4, text: "4111".into(), submit: true }, "t", &ctx(&tabs, 4000));
        let e = ans(&mut c, Choice::Allow, &ctx(&tabs, 4100));
        let run = runs(&e)[0].0;
        c.raw(run, Raw::Script("confirm:typed".into()), &ctx(&tabs, 4200));
        let e = ans(&mut c, Choice::Allow, &ctx(&tabs, 4300));
        let [(_, Some(3), Op::Script(src))] = &runs(&e)[..] else { panic!("{e:?}") };
        assert!(src.contains("kind: 'submit'") && src.contains("confirmed = true") && !src.contains("4111"), "{src}");
    }

    #[test]
    fn read_text_reads_only_the_page_that_was_checked() {
        let mut c = approved(Choice::Look);
        let tabs = [page(3, "https://a.test/doc")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&tabs, 2000));
        c.call(1, None, Tool::ReadText { tab: 1 }, "r", &ctx(&tabs, 4000));
        let e = ans(&mut c, Choice::Allow, &ctx(&tabs, 4100));
        let [(_, Some(3), Op::Text(url))] = &runs(&e)[..] else { panic!("{e:?}") };
        assert_eq!(url.as_str(), "https://a.test/doc");
    }

    #[test]
    fn a_script_for_a_page_without_an_origin_runs_only_in_such_a_page() {
        let mut c = approved(Choice::Look);
        let tabs = [page(3, "about:blank")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&tabs, 2000));
        let e = c.call(1, None, Tool::Snapshot { tab: 1 }, "s", &ctx(&tabs, 4000));
        let [(_, Some(3), Op::Script(src))] = &runs(&e)[..] else { panic!("{e:?}") };
        assert!(src.starts_with("if (!(location.origin === \"null\" && location.href === \"about:blank\"))"), "{src}");
    }

    #[test]
    fn history_steps_report_done_only_once_the_shell_ran_them() {
        let mut c = approved(Choice::Act);
        let tabs = [page(3, "about:blank")];
        c.call(1, None, Tool::TabsList, "l", &ctx(&tabs, 2000));
        let e = c.call(1, None, Tool::Back { tab: 1 }, "b", &ctx(&tabs, 4000));
        let [(run, Some(3), Op::Back)] = &runs(&e)[..] else { panic!("{e:?}") };
        assert!(replies(&e).is_empty());
        let e = c.raw(*run, Raw::Unavailable, &ctx(&tabs, 4100));
        assert_eq!(replies(&e)[0].1, Outcome::Error(LOCKED.into()));
        let e = c.call(1, None, Tool::Reload { tab: 1 }, "r", &ctx(&tabs, 6000));
        let e = c.raw(runs(&e)[0].0, Raw::Done, &ctx(&tabs, 6100));
        assert!(matches!(&replies(&e)[0].1, Outcome::Text(t) if t.starts_with("Done")));
    }
}
