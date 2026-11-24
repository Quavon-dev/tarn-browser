//! Agent control (S31) in the Linux shell: listens for agents, runs the core's `agent::control`
//! effects on the agent's window and draws its bar.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::BufReader;
use std::sync::mpsc as std_mpsc;
use std::time::{Duration, Instant};

use browser_core::agent::control::{Choice, Controller, Ctx, Effect, MAX_TEXT_CHARS, Op, Page, Raw};
use browser_core::agent::mcp::{self, Outcome};
use browser_core::agent::{self, Tool};
use browser_core::incoming::LOCALHOST_FROM_APPS;
use browser_core::prefs::Switch;
use browser_core::tabs::TabKey;
use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use nsh_core::JsonValue;
use nsh_core::prelude::*;
use serde_json::json;
use tarn_nsh_shared::controls::{described, list_row, section_with, secure_button, secure_surface};
use url::Url;

use super::tools::note;
use super::{REFS, Ui, WINDOW_REFS, act, act_quietly, read};
use crate::browser::Browser;
use crate::controls::{body, group, switch};
use crate::theme::{Secure, Theme};

pub const SWITCH: Switch = Switch { key: agent::SETTING, default: false };
const TICK: Duration = Duration::from_millis(250);

type Reply = std_mpsc::Sender<Outcome>;

enum Msg {
    Call { conn: u64, client: Option<String>, tool: Tool, reply: Reply },
    Closed(u64),
}

enum Wait {
    Script,
    Text,
    Shot,
}

#[derive(Default)]
struct Agent {
    tx: Option<UnboundedSender<Msg>>,
    /// Set to stop the listener, whose socket exists only while agent control is on.
    listening: Option<engine::agent_socket::Server>,
    ticking: bool,
    enabled: bool,
    /// Settings › Privacy › Open localhost links from other apps, read again for each tool call.
    localhost: bool,
    ctl: Controller<TabKey, Reply>,
    window: Option<Ui>,
    /// Counts the agent windows asked for; one that mounts for an older count closes at once.
    opening: u64,
    /// What the session last said the gate lets through, for a window that opens after it.
    sites: Option<Option<Vec<String>>>,
    scripts: HashMap<u64, Wait>,
}

thread_local! {
    static AGENT: RefCell<Agent> = RefCell::default();
}

fn with<R>(f: impl FnOnce(&mut Agent) -> R) -> R {
    AGENT.with_borrow_mut(f)
}

pub(super) fn init() {
    let (tx, mut rx) = unbounded();
    with(|a| a.tx = Some(tx));
    Owner::detached().run(|| {
        drop(spawn(async move {
            while let Some(msg) = rx.next().await {
                on_msg(msg);
            }
        }))
    });
    let on = SWITCH.on(&super::core().disk);
    with(|a| a.enabled = on);
    if on {
        listen();
    }
}

/// Settings › AI turned agent control on or off. Off ends a session and closes the socket.
fn set_enabled(on: bool) {
    with(|a| a.enabled = on);
    if on {
        return listen();
    }
    let effects = with(|a| a.ctl.stop(agent::OFF, true, false));
    apply(effects);
    if let Some(server) = with(|a| a.listening.take()) {
        server.stop();
    }
}

fn listen() {
    if with(|a| a.listening.is_some()) {
        return;
    }
    let Some(tx) = with(|a| a.tx.clone()) else { return };
    match engine::agent_socket::Server::start(&crate::data::root(), agent::BUSY, move |conn, stream| serve(conn, stream, tx.clone())) {
        Ok(server) => {
            with(|a| a.listening = Some(server));
        }
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

fn on_msg(msg: Msg) {
    match msg {
        Msg::Call { conn, client, tool, reply } => {
            let localhost =
                super::WINDOWS.with_borrow(|windows| windows.first().copied()).and_then(|ui| read(ui, |b| b.setting_on(LOCALHOST_FROM_APPS))).unwrap_or(false);
            with(|a| a.localhost = localhost);
            with_ctx(|ctl, ctx| ctl.call(conn, client.as_deref(), tool, reply, ctx));
        }
        Msg::Closed(conn) => {
            let effects = with(|a| a.ctl.closed(conn));
            apply(effects);
        }
    }
}

fn pages(b: &Browser) -> Vec<Page<TabKey>> {
    b.model
        .tabs
        .iter()
        .map(|t| {
            let key = b.model.key(t);
            let page = b.pages.get(&key);
            let url = page.map(|p| p.url.as_str()).filter(|u| !u.is_empty()).unwrap_or(&t.url);
            let title = page.map(|p| p.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| t.title.clone());
            Page { key, url: Url::parse(url).ok(), title }
        })
        .collect()
}

/// Runs `f` with what the core needs to know now: the switches and the agent window's tabs.
fn with_ctx(f: impl FnOnce(&mut Controller<TabKey, Reply>, &Ctx<TabKey>) -> Vec<Effect<TabKey, Reply>>) {
    let (enabled, localhost) = with(|a| (a.enabled, a.localhost));
    let tabs = with(|a| a.window).and_then(|ui| read(ui, pages)).unwrap_or_default();
    let effects = with(|a| {
        f(&mut a.ctl, &Ctx { enabled, localhost, now: browser_core::now(), tabs: &tabs, profile_on: false, profile: None, tabs_on: false, yours: &[] })
    });
    apply(effects);
}

fn apply(effects: Vec<Effect<TabKey, Reply>>) {
    for effect in effects {
        match effect {
            Effect::Reply(reply, outcome) => drop(reply.send(outcome)),
            Effect::OpenWindow => {
                let opening = with(|a| {
                    a.opening += 1;
                    a.opening
                });
                if !super::open_agent_window(opening) {
                    let effects = with(|a| a.ctl.stop("Tarn could not open a window for the agent.", false, false));
                    apply(effects);
                }
            }
            Effect::Switch(_) => {
                let effects = with(|a| a.ctl.stop(browser_core::agent::OFF, true, false));
                apply(effects);
            }
            Effect::CloseWindow => {
                with(|a| a.opening += 1);
                if let Some(ui) = with(|a| a.window.take().inspect(|_| forget_runs(a)))
                    && let Some(here) = WINDOW_REFS.with_borrow(|refs| refs.get(&ui.window).cloned())
                {
                    super::close_agent_window(ui, &here);
                }
            }
            Effect::Show(asking) => {
                if let Some(ui) = with(|a| a.window).filter(|_| asking) {
                    ask(ui);
                }
            }
            Effect::Sites(sites) => {
                if let Some(ui) = with(|a| {
                    a.sites = Some(sites.clone());
                    a.window
                }) {
                    act_quietly(ui, |b| b.agent_sites = sites);
                }
            }
            Effect::Run { run, tab, op } => run_op(run, tab, op),
        }
    }
    sync_banner();
    if with(|a| a.ctl.active()) {
        tick_while_active();
    } else {
        with(|a| a.window.take().map(|_| forget_runs(a)));
    }
}

/// A new question: the window comes to the front and its buttons wait for the layout to settle.
fn ask(ui: Ui) {
    act_quietly(ui, |b| browser_core::prompts::restart(&mut b.guard_since, Instant::now()));
    if let Some(here) = WINDOW_REFS.with_borrow(|refs| refs.get(&ui.window).cloned()) {
        here.show();
        here.focus();
    }
}

fn sync_banner() {
    let (window, banner) = with(|a| (a.window, a.ctl.banner()));
    let Some(ui) = window else { return };
    if read(ui, |b| b.agent_banner != banner) == Some(true) {
        act_quietly(ui, |b| b.agent_banner = banner);
    }
}

/// The agent's window has opened (`open_agent_window`).
pub(super) fn opened(ui: Ui, opening: u64) {
    if !with(|a| a.ctl.active() && a.opening == opening) {
        if let Some(here) = WINDOW_REFS.with_borrow(|refs| refs.get(&ui.window).cloned()) {
            super::close_agent_window(ui, &here);
        }
        return;
    }
    let sites = with(|a| {
        a.window = Some(ui);
        a.sites.clone()
    });
    if let Some(sites) = sites {
        act_quietly(ui, |b| b.agent_sites = sites);
    }
    ask(ui);
    sync_banner();
}

/// A window is closing; the agent's ends its session.
pub(super) fn window_closed(ui: Ui) {
    if with(|a| a.window.take_if(|w| w.window == ui.window).inspect(|_| forget_runs(a)).is_some()) {
        let effects = with(|a| a.ctl.stop(agent::STOPPED, false, true));
        apply(effects);
    }
}

/// Drops the answers still awaited from the agent's window, so none takes another view's event.
fn forget_runs(a: &mut Agent) {
    a.scripts.clear();
}

fn agent_window(ui: Ui) -> bool {
    with(|a| a.window.is_some_and(|w| w.window == ui.window))
}

fn run_op(run: u64, tab: Option<TabKey>, op: Op) {
    let Some(ui) = with(|a| a.window) else { return };
    let localhost = with(|a| a.localhost);
    let typed = |url: &Url| engine::nav_policy::app_link_typed(url, localhost);
    let follow = matches!(op, Op::Follow(_));
    let immediate = match (op, tab) {
        (Op::Open(url), _) => {
            let mut key = None;
            act(ui, |b| key = b.open_link_tab(url.as_str(), typed(&url)));
            Some(Raw::Opened(key))
        }
        (_, None) => Some(Raw::Unavailable),
        (Op::Load(url) | Op::Follow(url), Some(key)) => {
            let typed = !follow && typed(&url);
            let mut loaded = false;
            act(ui, |b| loaded = b.load_link_in(key, url.as_str(), typed));
            Some(if loaded { Raw::Done } else { Raw::Unavailable })
        }
        (Op::Close, Some(key)) => {
            let mut open = false;
            act(ui, |b| {
                open = b.model.tab(key).is_some();
                b.close_tab(key);
            });
            Some(if open { Raw::Done } else { Raw::Unavailable })
        }
        (op, Some(key)) => page_op(ui, run, key, op),
    };
    if let Some(raw) = immediate {
        with_ctx(|ctl, ctx| ctl.raw(run, raw, ctx));
    }
}

/// Runs `op` on the live view of `key`; a script's, text's or snapshot's answer comes back as a page event.
fn page_op(ui: Ui, run: u64, key: TabKey, op: Op) -> Option<Raw<TabKey>> {
    with(|a| match &op {
        Op::Script(..) => drop(a.scripts.insert(run, Wait::Script)),
        Op::Text(_) => drop(a.scripts.insert(run, Wait::Text)),
        Op::Screenshot => drop(a.scripts.insert(run, Wait::Shot)),
        _ => {}
    });
    let steps = matches!(op, Op::Back | Op::Forward | Op::Reload);
    let mut shown = false;
    act(ui, |b| {
        shown = b.agent_show(key);
        if !shown {
            return;
        }
        match op {
            Op::Back => b.back(),
            Op::Forward => b.forward(),
            Op::Reload => b.reload(),
            Op::Script(body) => b.outbox.push((key, "agent", json!({ "seq": run, "body": body }))),
            Op::Text(page) => {
                let body = engine::page_scripts::page_text_script(&page, MAX_TEXT_CHARS);
                b.outbox.push((key, "agent", json!({ "seq": run, "body": body })));
            }
            Op::Screenshot => b.outbox.push((key, "snapshot", json!({ "seq": run }))),
            Op::Open(_) | Op::Load(_) | Op::Follow(_) | Op::Close => {}
        }
    });
    if shown && REFS.with_borrow(|refs| refs.contains_key(&(ui.window, key))) {
        return steps.then_some(Raw::Done);
    }
    with(|a| a.scripts.remove(&run));
    Some(Raw::Unavailable)
}

/// The answer of an agent script (`agentAnswer`).
pub(super) fn answered(ui: Ui, data: &JsonValue) {
    if !agent_window(ui) {
        return;
    }
    let Some(run) = data["seq"].as_u64() else { return };
    let Some(wait) = with(|a| a.scripts.get(&run).filter(|w| !matches!(w, Wait::Shot)).is_some().then(|| a.scripts.remove(&run)).flatten()) else { return };
    let text = data["text"].as_str().unwrap_or_default().to_owned();
    let raw = match wait {
        Wait::Text => Raw::Text(text),
        Wait::Script | Wait::Shot => Raw::Script(text),
    };
    with_ctx(|ctl, ctx| ctl.raw(run, raw, ctx));
}

/// A tab's snapshot (`snapshot`); true when the agent asked for it, which its seq says.
pub(super) fn shot(ui: Ui, data: &JsonValue) -> bool {
    let Some(run) = data["seq"].as_u64().filter(|_| agent_window(ui)) else { return false };
    if with(|a| !matches!(a.scripts.get(&run), Some(Wait::Shot))) {
        return false;
    }
    with(|a| a.scripts.remove(&run));
    let png = data["png"].as_str().and_then(|p| p.strip_prefix("data:image/png;base64,")).map(str::to_owned);
    with_ctx(|ctl, ctx| ctl.raw(run, Raw::Png(png), ctx));
    true
}

/// A page-started navigation in the agent's window to a site without a grant (`agentHeld`).
pub(super) fn held_navigation(ui: Ui, key: TabKey, data: &JsonValue) {
    if !agent_window(ui) {
        return;
    }
    let Some(url) = data["url"].as_str().and_then(|u| Url::parse(u).ok()) else { return };
    let effects = with(|a| a.ctl.held_navigation(key, url, browser_core::now()));
    apply(effects);
}

fn tick_while_active() {
    if !with(|a| std::mem::replace(&mut a.ticking, true)) {
        later_tick();
    }
}

fn later_tick() {
    Owner::detached().run(|| drop(set_timeout(TICK, tick)));
}

fn tick() {
    if !with(|a| a.ctl.active()) {
        return with(|a| a.ticking = false);
    }
    with_ctx(|ctl, ctx| ctl.tick(ctx));
    later_tick();
}

fn press(ui: Ui, choice: Choice, prompt: u64, waits: bool) {
    if !agent_window(ui) {
        return;
    }
    if waits && read(ui, |b| browser_core::prompts::stable(b.guard_since, Instant::now())) != Some(true) {
        return;
    }
    with_ctx(|ctl, ctx| ctl.answer(choice, prompt, ctx));
}

/// The agent bar at the top of the agent's window: its question, or who controls the window.
pub(super) fn bar(ui: Ui) -> AnyElement {
    dynamic(move || {
        let banner = ui.snap.with(|s| s.agent.clone())?;
        let (asking, prompt) = (banner.asking, banner.prompt);
        let buttons: Vec<AnyElement> = banner
            .buttons
            .into_iter()
            .map(|(label, choice, primary)| {
                let waits = asking && !matches!(choice, Choice::Stop | Choice::Deny | Choice::Pause);
                secure_button(label, move || press(ui, choice, prompt, waits), primary).into_element()
            })
            .collect();
        let surface = secure_surface(Theme::radius_panel()).flex_row().items_center().gap_2().min_h(px(42.)).pl(px(14.)).pr_2().py_1().mb_2();
        let text = browser_core::text::bounded_title(&banner.text);
        Some(surface.shadow(Theme::elevation(4., 12.)).child(body(text).text_color(Secure::text()).flex_1().truncate()).children(buttons))
    })
    .into_element()
}

/// Settings › AI › Agent control, for every profile: the switch, how to connect an agent and what it did.
pub(super) fn settings(ui: Ui) -> View {
    let on = SWITCH.on(&super::core().disk);
    let toggle = move || {
        let on = !SWITCH.on(&super::core().disk);
        act(ui, |b| b.set_pref(SWITCH, on));
        set_enabled(on);
    };
    let mut out =
        view().child(section_with("Agent control", "")).child(group().child(described(switch(on, "Let AI agents control Tarn", toggle), agent::DETAIL)));
    if on {
        out = out.child(note(setup()));
        let entries = with(|a| a.ctl.log());
        if !entries.is_empty() {
            let rows = entries.into_iter().take(20).enumerate().map(|(ix, e)| {
                let site = e.site.map(|s| format!(" · {}", browser_core::text::bounded_origin(&s))).unwrap_or_default();
                list_row(ix, format!("{}{site}", e.tool), e.outcome)
            });
            out = out.child(section_with("This session", "")).child(group().children(rows));
        }
    }
    out
}

/// How to connect an agent; in the Flatpak, through `flatpak run`, since the binary's path is the sandbox's.
fn setup() -> String {
    if !std::path::Path::new("/.flatpak-info").exists() {
        return agent::setup(&std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "tarn".into()));
    }
    let app = crate::APP_ID;
    format!(
        "Claude Code: claude mcp add tarn -- flatpak run --command=tarn {app} --mcp\nCodex, in ~/.codex/config.toml:\n[mcp_servers.tarn]\ncommand = 'flatpak'\nargs = [\"run\", \"--command=tarn\", \"{app}\", \"--mcp\"]"
    )
}
