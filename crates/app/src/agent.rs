//! Agent control (S31) in the macOS and Windows shell: listens for agents, runs the core's
//! `agent::control` effects on the agent's window, or on the user's tabs in `Mode::Tabs`, and
//! draws its bar.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::BufReader;
use std::rc::Rc;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use browser_core::agent::control::{Choice, Controller, Ctx, Effect, MAX_TEXT_CHARS, Op, Page, Raw};
use browser_core::agent::mcp::{self, Outcome};
use browser_core::agent::{self, Mode, Tool};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, Context, Global, ParentElement, Rgba, StatefulInteractiveElement, Window, WindowHandle};
use ui::controls::secure_button;
use ui::theme::Secure;
use url::Url;

use crate::browser::{Browser, TabKey, WindowStart, now};
use crate::prefs::Pref;
use store::Id;

const TICK: Duration = Duration::from_millis(250);

/// Whether the agent may use the user's profile or tabs here: not on Windows, whose views cannot
/// turn passkeys off yet (`docs/port-backlog.md`).
pub(crate) const SIGNED_IN: bool = cfg!(target_os = "macos");

type Reply = std_mpsc::Sender<Outcome>;

enum Msg {
    Call { conn: u64, client: Option<String>, tool: Tool, reply: Reply },
    Closed(u64),
    Raw(u64, Raw<TabKey>),
}

pub(crate) struct Agent {
    tx: UnboundedSender<Msg>,
    /// Set to stop the listener, whose socket exists only while agent control is on.
    listening: Option<engine::agent_socket::Server>,
    ticking: bool,
    ctl: Controller<TabKey, Reply>,
    window: Option<WindowHandle<Browser>>,
    /// The sites the gate on the window's tabs lets through; `None` lets everything through.
    sites: Rc<RefCell<Option<HashSet<String>>>>,
    offered: Option<Id>,
    gate: Option<engine::agent_scripts::Gate>,
    touched: HashSet<TabKey>,
}

impl Global for Agent {}

pub(crate) fn init(cx: &mut App) {
    let (tx, mut rx) = unbounded();
    cx.set_global(Agent {
        tx,
        listening: None,
        ticking: false,
        ctl: Controller::default(),
        window: None,
        sites: Rc::default(),
        offered: None,
        gate: None,
        touched: HashSet::new(),
    });
    cx.spawn(async move |cx| {
        while let Some(msg) = rx.next().await {
            if cx.update(|cx| on_msg(msg, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
    if crate::shared::core(cx).pref(Pref::AgentControl) {
        listen(cx);
    }
}

/// Settings › AI turned agent control on or off. Off ends a session and closes the socket.
pub(crate) fn set_enabled(on: bool, cx: &mut App) {
    cx.defer(move |cx| {
        if on {
            listen(cx);
        } else {
            let effects = cx.global_mut::<Agent>().ctl.stop(agent::OFF, true, false);
            apply(effects, cx);
            if let Some(server) = cx.global_mut::<Agent>().listening.take() {
                server.stop();
            }
        }
    });
}

pub(crate) fn recheck(cx: &mut App) {
    cx.defer(|cx| {
        if cx.global::<Agent>().ctl.active() {
            with_ctx(cx, |ctl, ctx| ctl.tick(ctx));
        }
    });
}

pub(crate) fn gate_view(key: TabKey, view: &engine::WebView, cx: &App) {
    if let Some(agent) = cx.try_global::<Agent>()
        && agent.touched.contains(&key)
    {
        view.set_agent_gate(agent.gate.clone());
        view.set_passkeys(false);
    }
}

pub(crate) fn gated(key: TabKey, cx: &App) -> Option<engine::agent_scripts::Gate> {
    cx.try_global::<Agent>().filter(|a| a.touched.contains(&key)).and_then(|a| a.gate.clone())
}

pub(crate) fn may_open(gate: &engine::agent_scripts::Gate, url: &Url, cx: &App) -> bool {
    gate(url) && cx.global::<Agent>().ctl.room()
}

pub(crate) fn touch(key: TabKey, cx: &mut App) {
    let agent = cx.global_mut::<Agent>();
    agent.touched.insert(key);
    agent.ctl.opened(key);
}

/// A button of the agent bar, run after the window's own update, which the agent's work needs.
pub(crate) fn answer(choice: Choice, prompt: u64, cx: &mut App) {
    cx.defer(move |cx| with_ctx(cx, |ctl, ctx| ctl.answer(choice, prompt, ctx)));
}

/// A page-started navigation in the agent's window to a site without a grant (`Event::AgentHeld`).
pub(crate) fn held_navigation(key: TabKey, url: Url, cx: &mut App) {
    cx.defer(move |cx| {
        let effects = cx.global_mut::<Agent>().ctl.held_navigation(key, url, now());
        apply(effects, cx);
    });
}

/// The agent's actions in this session, newest first.
pub(crate) fn log(cx: &App) -> Vec<agent::Entry> {
    cx.try_global::<Agent>().map(|a| a.ctl.log()).unwrap_or_default()
}

fn listen(cx: &mut App) {
    let agent = cx.global_mut::<Agent>();
    if agent.listening.is_some() {
        return;
    }
    let tx = agent.tx.clone();
    match engine::agent_socket::Server::start(&crate::browser::data_dir(), agent::BUSY, move |conn, stream| serve(conn, stream, tx.clone())) {
        Ok(server) => agent.listening = Some(server),
        Err(e) => eprintln!("tarn: agent control cannot listen: {e}"),
    }
}

fn serve(conn: u64, stream: engine::agent_socket::Stream, tx: UnboundedSender<Msg>) {
    let Ok(reader) = stream.try_clone() else { return };
    let _ = mcp::serve(BufReader::new(reader), &stream, |client, tool| {
        let (reply, answer) = std_mpsc::channel();
        if tx.unbounded_send(Msg::Call { conn, client: client.map(str::to_owned), tool, reply }).is_err() {
            return Outcome::Error(agent::STOPPED.into());
        }
        engine::agent_socket::await_answer(&answer, &stream).unwrap_or_else(|| {
            let _ = tx.unbounded_send(Msg::Closed(conn));
            Outcome::Error(agent::STOPPED.into())
        })
    });
    let _ = tx.unbounded_send(Msg::Closed(conn));
}

fn on_msg(msg: Msg, cx: &mut App) {
    match msg {
        Msg::Call { conn, client, tool, reply } => with_ctx(cx, |ctl, ctx| ctl.call(conn, client.as_deref(), tool, reply, ctx)),
        Msg::Closed(conn) => {
            let effects = cx.global_mut::<Agent>().ctl.closed(conn);
            apply(effects, cx);
        }
        Msg::Raw(run, raw) => with_ctx(cx, |ctl, ctx| ctl.raw(run, raw, ctx)),
    }
}

fn users(b: &Browser) -> bool {
    b.model.agent_profile.is_none() && !b.model.burner_shown()
}

fn open_here(b: &Browser) -> bool {
    b.model.profile().is_some_and(|(private, _)| !private)
}

fn user_windows(cx: &App) -> Vec<WindowHandle<Browser>> {
    let all = cx.window_stack().unwrap_or_else(|| cx.windows());
    all.into_iter().filter_map(|w| w.downcast::<Browser>()).filter(|h| h.read(cx).is_ok_and(users)).collect()
}

fn front_profile(cx: &App) -> Option<(Id, String)> {
    let front = user_windows(cx).into_iter().next()?;
    let b = front.read(cx).ok()?;
    let (_, id) = b.model.profile().filter(|_| open_here(b))?;
    Some((id, b.core.store(false).profile(id).ok()?.name))
}

fn user_tabs(cx: &App) -> Vec<Page<TabKey>> {
    let mut out = Vec::new();
    for h in user_windows(cx) {
        let Ok(b) = h.read(cx) else { continue };
        for t in &b.model.tabs {
            let key = b.model.key(t);
            if !key.private {
                out.push(Page { key, url: b.tab_url(key).and_then(|u| Url::parse(&u).ok()), title: crate::tabs::display_title(t) });
            }
        }
    }
    out
}

fn window_of(key: Option<TabKey>, cx: &App) -> Option<WindowHandle<Browser>> {
    let agent = cx.global::<Agent>();
    if agent.ctl.mode() != Some(Mode::Tabs) {
        return agent.window;
    }
    let windows = user_windows(cx);
    match key {
        Some(key) => windows.into_iter().find(|h| h.read(cx).is_ok_and(|b| b.model.tab(key).is_some())),
        None => windows.into_iter().find(|h| h.read(cx).is_ok_and(open_here)),
    }
}

/// Runs `f` with what the core needs to know now: the switches and the agent window's tabs.
fn with_ctx(cx: &mut App, f: impl FnOnce(&mut Controller<TabKey, Reply>, &Ctx<TabKey>) -> Vec<Effect<TabKey, Reply>>) {
    let core = crate::shared::core(cx);
    let (enabled, localhost) = (core.pref(Pref::AgentControl), core.pref(Pref::LocalhostFromApps));
    let (profile_on, tabs_on) = (SIGNED_IN && core.pref(Pref::AgentProfile), SIGNED_IN && core.pref(Pref::AgentTabs));
    let (active, mode) = (cx.global::<Agent>().ctl.active(), cx.global::<Agent>().ctl.mode());
    let profile = (profile_on && !active).then(|| front_profile(cx)).flatten();
    if !active {
        cx.global_mut::<Agent>().offered = profile.as_ref().map(|p| p.0);
    }
    let yours = if tabs_on && matches!(mode, None | Some(Mode::Tabs)) { user_tabs(cx) } else { Vec::new() };
    let tabs = cx
        .global::<Agent>()
        .window
        .and_then(|w| {
            w.update(cx, |b, _, _| {
                b.model
                    .tabs
                    .iter()
                    .map(|t| {
                        let key = b.model.key(t);
                        Page { key, url: b.tab_url(key).and_then(|u| Url::parse(&u).ok()), title: crate::tabs::display_title(t) }
                    })
                    .collect::<Vec<_>>()
            })
            .ok()
        })
        .unwrap_or_default();
    let ctx = Ctx { enabled, localhost, now: now(), tabs: &tabs, profile_on, profile: profile.map(|p| p.1), tabs_on, yours: &yours };
    let effects = f(&mut cx.global_mut::<Agent>().ctl, &ctx);
    apply(effects, cx);
}

fn apply(effects: Vec<Effect<TabKey, Reply>>, cx: &mut App) {
    for effect in effects {
        match effect {
            Effect::Reply(reply, outcome) => drop(reply.send(outcome)),
            Effect::OpenWindow => open_window(None, cx),
            Effect::Switch(mode) => {
                close_window(cx);
                match (mode, cx.global::<Agent>().offered) {
                    (Mode::Profile, Some(profile)) => open_window(Some(profile), cx),
                    (Mode::Tabs, _) => {
                        let gate = new_gate(cx);
                        cx.global_mut::<Agent>().gate = Some(gate);
                        if let Some(front) = user_windows(cx).into_iter().next() {
                            front.update(cx, |_, window, _| window.activate_window()).ok();
                        }
                    }
                    _ => {
                        let effects = cx.global_mut::<Agent>().ctl.stop("Tarn could not open a window for the agent.", false, false);
                        apply(effects, cx);
                    }
                }
            }
            Effect::CloseWindow => close_window(cx),
            Effect::Show(asking) => {
                let windows = match cx.global::<Agent>().ctl.mode() {
                    Some(Mode::Tabs) => user_windows(cx),
                    _ => cx.global::<Agent>().window.into_iter().collect(),
                };
                for (ix, handle) in windows.into_iter().enumerate() {
                    handle
                        .update(cx, |b, window, cx| {
                            if asking {
                                b.layout_stable_since = std::time::Instant::now();
                                if ix == 0 {
                                    window.activate_window();
                                }
                            }
                            cx.notify();
                        })
                        .ok();
                }
            }
            Effect::Sites(sites) => {
                let agent = cx.global_mut::<Agent>();
                let ended = sites.is_none() && agent.gate.take().is_some();
                if sites.is_none() {
                    agent.touched.clear();
                }
                *agent.sites.borrow_mut() = sites.map(|s| s.into_iter().collect());
                if ended {
                    crate::shared::each(cx, |b, _, cx| {
                        b.apply_view_prefs(cx);
                        cx.notify();
                    });
                }
            }
            Effect::Run { run, tab, op } => run_op(run, tab, op, cx),
        }
    }
    if cx.global::<Agent>().ctl.active() {
        tick_while_active(cx);
    }
}

fn new_gate(cx: &mut App) -> engine::agent_scripts::Gate {
    let sites: Rc<RefCell<Option<HashSet<String>>>> = Rc::new(RefCell::new(Some(HashSet::new())));
    cx.global_mut::<Agent>().sites = sites.clone();
    Rc::new(move |url: &Url| sites.borrow().as_ref().is_none_or(|allowed| agent::site_of(url).is_none_or(|site| allowed.contains(&site))))
}

fn open_window(profile: Option<Id>, cx: &mut App) {
    let Some(window) = crate::shared::open_new_window(WindowStart::Agent(profile), cx) else {
        let effects = cx.global_mut::<Agent>().ctl.stop("Tarn could not open a window for the agent.", false, false);
        return apply(effects, cx);
    };
    let gate = new_gate(cx);
    window.update(cx, |b, _, _| b.agent_gate = Some(gate)).ok();
    cx.global_mut::<Agent>().window = Some(window);
}

fn close_window(cx: &mut App) {
    if let Some(window) = cx.global_mut::<Agent>().window.take() {
        window
            .update(cx, |b, window, cx| {
                b.closing(cx);
                window.remove_window();
            })
            .ok();
    }
}

fn run_op(run: u64, tab: Option<TabKey>, op: Op, cx: &mut App) {
    let Some(window) = window_of(tab, cx) else {
        return with_ctx(cx, |ctl, ctx| ctl.raw(run, Raw::Unavailable, ctx));
    };
    let agent = cx.global::<Agent>();
    let tx = agent.tx.clone();
    let gate = agent.gate.clone();
    let send = move |raw: Raw<TabKey>| drop(tx.unbounded_send(Msg::Raw(run, raw)));
    let localhost = crate::shared::core(cx).pref(Pref::LocalhostFromApps);
    let typed = |url: &Url| engine::nav_policy::app_link_typed(url, localhost);
    let opened = window
        .update(cx, |b, window, cx| {
            if let Op::Open(url) = &op {
                let key = b.open_link_tab(url, typed(url), window, cx);
                if let (Some(key), Some(gate)) = (key, &gate) {
                    b.views.get(&key).inspect(|v| {
                        v.set_agent_gate(Some(gate.clone()));
                        v.set_passkeys(false);
                    });
                }
                return Some(Raw::Opened(key));
            }
            let Some(key) = tab.filter(|k| b.model.tab(*k).is_some()) else { return Some(Raw::Unavailable) };
            if gate.is_some() {
                if chromium(b, key) {
                    return Some(Raw::Unavailable);
                }
                cx.global_mut::<Agent>().touched.insert(key);
            }
            if let Some(gate) = &gate {
                b.views.get(&key).inspect(|v| {
                    v.set_agent_gate(Some(gate.clone()));
                    v.set_passkeys(false);
                });
            }
            if let Op::Load(url) | Op::Follow(url) = &op {
                b.load_link_in(key, url, matches!(op, Op::Load(_)) && typed(url), window, cx);
                return Some(Raw::Done);
            }
            if op == Op::Close {
                b.close_tab(key, window, cx);
                return Some(Raw::Done);
            }
            let Some(view) = b.agent_view(key, window, cx).filter(|v| gate.is_none() || !v.is_chromium()) else { return Some(Raw::Unavailable) };
            match op {
                Op::Back | Op::Forward | Op::Reload => {
                    match op {
                        Op::Back => view.back(),
                        Op::Forward => view.forward(),
                        _ => view.reload(),
                    }
                    return Some(Raw::Done);
                }
                Op::Script(source) => view.agent(&source, move |raw| send(Raw::Script(raw))),
                Op::Text(page) => view.page_text(&page, MAX_TEXT_CHARS, move |_, text| send(Raw::Text(text))),
                Op::Screenshot => view.snapshot_png(Some(agent::SCREENSHOT_WIDTH), move |png| send(Raw::Png(png.map(|p| engine::agent_scripts::base64(&p))))),
                Op::Open(_) | Op::Load(_) | Op::Follow(_) | Op::Close => {}
            }
            None
        })
        .ok();
    let immediate = opened.unwrap_or(Some(Raw::Unavailable));
    if let Some(Raw::Opened(Some(key))) = &immediate
        && cx.global::<Agent>().gate.is_some()
    {
        cx.global_mut::<Agent>().touched.insert(*key);
    }
    if let Some(raw) = immediate {
        with_ctx(cx, |ctl, ctx| ctl.raw(run, raw, ctx));
    }
}

fn chromium(b: &mut Browser, key: TabKey) -> bool {
    #[cfg(feature = "chromium")]
    if let Some(url) = b.tab_url(key)
        && b.web_engine_for(key, &url) == store::WebEngine::Chromium
    {
        return true;
    }
    b.views.get(&key).is_some_and(|v| v.is_chromium())
}

fn tick_while_active(cx: &mut App) {
    let agent = cx.global_mut::<Agent>();
    if agent.ticking {
        return;
    }
    agent.ticking = true;
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            let more = cx.update(tick).unwrap_or(false);
            if !more {
                break;
            }
        }
    })
    .detach();
}

fn tick(cx: &mut App) -> bool {
    let agent = cx.global_mut::<Agent>();
    if !agent.ctl.active() {
        agent.ticking = false;
        return false;
    }
    if agent.window.is_some_and(|w| w.update(cx, |_, _, _| ()).is_err()) {
        let agent = cx.global_mut::<Agent>();
        agent.window = None;
        let effects = agent.ctl.stop(agent::STOPPED, false, true);
        apply(effects, cx);
    } else {
        with_ctx(cx, |ctl, ctx| ctl.tick(ctx));
    }
    true
}

/// Whether an agent controls this window, and whether it waits for an answer.
pub(crate) fn status(window: &Window, cx: &App) -> Option<(String, bool)> {
    let agent = cx.try_global::<Agent>()?;
    agent.window.filter(|w| w.window_id() == window.window_handle().window_id())?;
    agent.ctl.banner().map(|b| (b.text, b.asking))
}

/// The agent bar at the top of the agent's window: its question, or who controls the window.
pub(crate) fn banner(b: &Browser, window: &Window, cx: &mut Context<Browser>, push: &mut impl FnMut(&'static str, String, Rgba, gpui::Div)) {
    let Some(agent) = cx.try_global::<Agent>() else { return };
    let here = agent.window.is_some_and(|w| w.window_id() == window.window_handle().window_id());
    let theirs = agent.ctl.mode() == Some(Mode::Tabs) && users(b);
    if !here && !theirs {
        return;
    }
    let Some(banner) = agent.ctl.banner() else { return };
    let text = ui::text::bounded_title(&banner.text);
    let prompt = banner.prompt;
    let buttons = banner.buttons.into_iter().map(|(label, choice, primary)| {
        let waits = banner.asking && !matches!(choice, Choice::Stop | Choice::Deny | Choice::Pause);
        secure_button(label, label, primary).on_click(cx.listener(move |b, _, _, cx| {
            if !waits || b.layout_stable() {
                answer(choice, prompt, cx);
            }
        }))
    });
    push("banner-agent", text.clone(), Secure::text(), crate::content::banner(text).children(buttons));
}
