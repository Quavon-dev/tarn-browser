//! The assistant on Linux, iOS and Android: Settings › AI, the chat panel that draws
//! `browser_core::assistant::view` and Clean Up's grouping. The core decides what a request
//! carries; this module only runs a provider off the main thread and draws the answers.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use ai::markdown::{Block, Span};
use ai::{AiError, ApiToken, ClaudeApi, Ollama, Provider as _, Secrets, Transport};
use browser_core::assistant::links::{self, LinkAction};
use browser_core::assistant::settings::{self, Provider};
use browser_core::assistant::view as chat_view;
use browser_core::assistant::{ASK, ChatView, Effect, Event, Origin, Pill, ProfileKind, Role, Session, allowed, commands};
use browser_core::cleanup;
use nsh_base::{Input, input};
use nsh_core::prelude::*;
use shield::ShieldLevel;
use store::{Id, Store};
use url::Url;

use crate::controls::{accent_pill, empty_state, group, icon_button, label, pill, section, segmented, switch_row};
use crate::theme::{TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

pub const ABOUT: &str = "The assistant answers questions in a chat beside the page. It is off until you turn it on for this profile, and it never works in a private space, a burner tab or on a Lockdown page.";
pub const SENT: &str = "Nothing is sent until you press Send or a command. A send carries your question, the earlier turns of this chat and only what you attached: a page (its address without query or fragment, its title and text) or a note. Group with Assistant in Clean Up sends the id, site and title of loose tabs, only when you press it.";
pub const CLAUDE: &str =
    "Claude API sends each request over HTTPS to Anthropic (api.anthropic.com) with your API key; Anthropic's terms apply to what it receives.";
pub const OLLAMA: &str = "Ollama answers on this computer: requests go to 127.0.0.1 and never leave the device. Models that run in Ollama's cloud are refused.";
pub const OFF: &str = "Turn on the assistant for this profile in Settings › AI.";
pub const NOT_CONNECTED: &str = "Connect a Claude API key in Settings › AI first.";
pub const NOT_HERE: &str = "The assistant is not available in a private space, a burner tab or on a Lockdown page.";
const FRAME: Duration = Duration::from_millis(33);

/// What a client brings: its secret store, its transport and whether Ollama on loopback works.
pub struct Platform {
    pub secrets: &'static dyn Secrets,
    pub transport: fn() -> Arc<dyn Transport>,
    /// Where the key is kept, as Settings names it.
    pub keyring: &'static str,
    /// Why Ollama cannot answer on this client; none where it can.
    pub ollama_gap: Option<&'static str>,
}

pub trait Host: Copy + 'static {
    fn platform(self) -> &'static Platform;
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R;
    /// The active page's `Origin` (its effective Shield level, per-site setting included), or the
    /// current space's with no page; none while its profile is not open.
    fn origin(self) -> Option<Origin>;
    /// The effective Shield level of `url` in the current profile.
    fn level_of(self, url: &Url) -> ShieldLevel;
    /// Asks the active page for its text; the shell hands it to `Chat::attach` as a `Pill::Page`.
    fn read_page(self);
    /// Opens `url` in a new tab of the current profile.
    fn open(self, url: Url);
}

fn ai_on<H: Host>(host: H, profile: Id) -> bool {
    host.with_store(|s| s.profile(profile).is_ok_and(|p| p.ai_enabled))
}

/// Whether Settings lists the AI pane: a saved profile that is not at Lockdown.
pub fn offered<H: Host>(host: H) -> bool {
    host.origin()
        .is_some_and(|o| o.kind == ProfileKind::Normal && host.with_store(|s| s.profile(o.profile).is_ok_and(|p| p.shield_level != ShieldLevel::Lockdown)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Backend {
    Claude,
    Ollama(String),
}

fn backend<H: Host>(host: H) -> Result<Backend, String> {
    match host.with_store(settings::provider) {
        Provider::ClaudeApi => Ok(Backend::Claude),
        Provider::Ollama => match host.platform().ollama_gap {
            Some(why) => Err(why.to_owned()),
            None => Ok(Backend::Ollama(host.with_store(settings::ollama_model))),
        },
    }
}

/// Runs one request on the chosen provider; call off the main thread.
fn answer(
    backend: &Backend,
    secrets: &dyn Secrets,
    transport: Arc<dyn Transport>,
    req: &ai::Request,
    cancel: &AtomicBool,
    on_text: &mut dyn FnMut(&str),
) -> Result<(), AiError> {
    match backend {
        Backend::Claude => {
            let token = ai::secrets::load_token(secrets)?.ok_or_else(|| AiError::Unavailable(NOT_CONNECTED.into()))?;
            ClaudeApi::new(token, None, transport).stream(req, cancel, on_text)
        }
        Backend::Ollama(model) => ai::local::stream(&Ollama::new(model.clone(), transport), req, cancel, on_text),
    }
}

enum Msg {
    Text(String),
    End(Result<(), String>),
}

/// Everything that arrived since the last frame, as one chunk, and how the request ended if it did.
fn drain(rx: &mpsc::Receiver<Msg>) -> (String, Option<Result<(), String>>) {
    let mut text = String::new();
    loop {
        match rx.try_recv() {
            Ok(Msg::Text(t)) => text.push_str(&t),
            Ok(Msg::End(end)) => return (text, Some(end)),
            Err(mpsc::TryRecvError::Empty) => return (text, None),
            Err(mpsc::TryRecvError::Disconnected) => return (text, Some(Err("the request stopped".into()))),
        }
    }
}

/// Starts `req` on a background thread; `on_frame` gets the text of each frame (at most one call
/// per `FRAME`) and `done` how it ended.
fn run<H: Host>(host: H, req: ai::Request, cancel: Arc<AtomicBool>, on_frame: impl Fn(String) + 'static, done: impl FnOnce(Result<(), String>) + 'static) {
    let backend = match backend(host) {
        Ok(b) => b,
        Err(why) => return done(Err(why)),
    };
    let (secrets, transport) = (host.platform().secrets, (host.platform().transport)());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let sent = tx.clone();
        let ended = answer(&backend, secrets, transport, &req, &cancel, &mut |t| drop(sent.send(Msg::Text(t.to_owned()))));
        drop(tx.send(Msg::End(ended.map_err(|e| e.to_string()))));
    });
    Owner::detached().run(|| {
        drop(spawn(async move {
            loop {
                timer(FRAME).await;
                let (text, end) = drain(&rx);
                if !text.is_empty() {
                    on_frame(text);
                }
                if let Some(end) = end {
                    return done(end);
                }
            }
        }))
    });
}

thread_local! {
    static FORGOTTEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Drops every window's chat the next time it is used: call when a profile is deleted or reset or
/// its AI switch is turned off.
pub fn forget_chats() {
    FORGOTTEN.set(FORGOTTEN.get() + 1);
}

/// One window's chat: its `Session`, the request it runs and what the panel draws.
#[derive(Clone, Copy)]
pub struct Chat {
    session: Stored<Option<(u64, Session)>>,
    running: Stored<Option<(u64, Arc<AtomicBool>)>>,
    view: Signal<Option<ChatView>>,
    tail: Signal<String>,
    question: Signal<String>,
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

impl Chat {
    pub fn new() -> Self {
        Self { session: stored(None), running: stored(None), view: signal(None), tail: signal(String::new()), question: signal(String::new()) }
    }

    /// Binds the chat to the current profile, keeping its conversation when the profile is the
    /// same; false where the assistant is off or not allowed.
    pub fn open<H: Host>(self, host: H) -> bool {
        self.check();
        let Some(origin) = host.origin() else {
            self.close();
            return false;
        };
        if self.session.with(|s| s.as_ref().is_some_and(|(_, s)| s.profile() == origin.profile)) && ai_on(host, origin.profile) {
            return true;
        }
        self.close();
        let Some(session) = Session::new(origin, ai_on(host, origin.profile)) else {
            return false;
        };
        self.view.set(Some(chat_view(&session)));
        self.session.replace(Some((FORGOTTEN.get(), session)));
        true
    }

    /// Drops the conversation and stops its request.
    pub fn close(self) {
        if let Some((_, cancel)) = self.running.replace(None) {
            cancel.store(true, Ordering::Relaxed);
        }
        self.session.replace(None);
        self.view.set(None);
        self.tail.set(String::new());
        self.question.set(String::new());
    }

    fn check(self) {
        if self.session.with(|s| s.as_ref().is_some_and(|(at, _)| *at != FORGOTTEN.get())) {
            self.close();
        }
    }

    /// Attaches `pill` from `origin`, the page or note it came from now.
    pub fn attach<H: Host>(self, host: H, origin: Origin, pill: Pill) {
        self.apply(host, Event::AddPill(origin, pill));
    }

    fn apply<H: Host>(self, host: H, event: Event) {
        self.check();
        let effects = self.session.with(|s| s.as_mut().map(|(_, s)| s.apply(event))).unwrap_or_default();
        self.effects(host, effects);
    }

    pub fn send<H: Host>(self, host: H, template: &str) {
        self.check();
        let Some(profile) = self.session.with(|s| s.as_ref().map(|(_, s)| s.profile())) else { return };
        let origin = host.origin().unwrap_or(Origin { profile, kind: ProfileKind::Private, level: ShieldLevel::Lockdown });
        let on = ai_on(host, profile);
        let question = self.question.get_untracked();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
        let effects = self.session.with(|s| s.as_mut().map(|(_, s)| s.send(&question, template, now, origin, on, |u| host.level_of(u)))).unwrap_or_default();
        if effects.iter().any(|e| matches!(e, Effect::Start { .. })) {
            self.question.set(String::new());
        }
        self.effects(host, effects);
    }

    fn effects<H: Host>(self, host: H, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Start { id, request } => {
                    let cancel = Arc::new(AtomicBool::new(false));
                    self.running.replace(Some((id, cancel.clone())));
                    let req = ai::Request { system: request.system(), page: None, question: request.user_message() };
                    run(
                        host,
                        req,
                        cancel,
                        move |text| self.apply(host, Event::Chunk { id, text }),
                        move |end| {
                            if self.running.with(|r| r.as_ref().is_some_and(|(r, _)| *r == id)) {
                                self.running.replace(None);
                            }
                            self.apply(host, end.map_or_else(|message| Event::Failed { id, message }, |()| Event::Done { id }));
                        },
                    );
                }
                Effect::Append { text } => self.tail.update(|t| t.push_str(&text)),
                Effect::Stop { id } => {
                    if let Some((_, cancel)) = self.running.with(|r| r.take_if(|(r, _)| *r == id)) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                Effect::Render => {
                    self.view.set(self.session.with(|s| s.as_ref().map(|(_, s)| chat_view(s))));
                    self.tail.set(String::new());
                }
            }
        }
    }
}

fn hint(content: impl IntoReactive<SharedString>) -> Text {
    text(content).text_size(px(TEXT_SMALL)).text_color(Theme::muted).px_1()
}

fn field(name: &'static str, value: Signal<String>) -> Input {
    input()
        .value(move || value.get())
        .on_change(move |v: &str| value.set(v.to_owned()))
        .label(name)
        .placeholder(name)
        .w_full()
        .px(px(8.))
        .py(px(6.))
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .border_1()
        .border_color(Theme::hairline())
        .text_color(Theme::text())
}

fn size(bytes: usize) -> String {
    if bytes < 1024 { format!("{bytes} bytes") } else { format!("{} KB", bytes.div_ceil(1024)) }
}

/// Settings › AI for the current profile: the switch, the provider, Connect and Disconnect, and
/// what is sent where.
pub fn settings_pane<H: Host>(host: H, chat: Chat) -> AnyElement {
    let rev = signal(0u64);
    let token = signal(String::new());
    let model = signal(host.with_store(settings::ollama_model));
    let connected: Signal<Option<Result<bool, String>>> = signal(None);
    let error: Signal<Option<String>> = signal(None);
    let secrets = host.platform().secrets;
    let check = move || {
        Owner::detached().run(|| {
            drop(spawn(async move {
                let found = blocking(move || ai::secrets::load_token(secrets).map(|t| t.is_some()).map_err(|e| e.to_string())).await;
                connected.set(Some(found.unwrap_or_else(|e| Err(e.to_string()))));
            }))
        })
    };
    check();
    let keep = move |work: Box<dyn FnOnce() -> Result<(), AiError> + Send>| {
        error.set(None);
        Owner::detached().run(|| {
            drop(spawn(async move {
                match blocking(work).await {
                    Ok(Ok(())) => check(),
                    Ok(Err(e)) => error.set(Some(e.to_string())),
                    Err(e) => error.set(Some(e.to_string())),
                }
            }))
        })
    };
    dynamic(move || {
        rev.get();
        let Some(profile) = host.origin().filter(|o| o.kind == ProfileKind::Normal).map(|o| o.profile) else {
            return Some(view().gap_2().child(hint(ABOUT)).child(empty_state(NOT_HERE)));
        };
        let on = ai_on(host, profile);
        let chosen = host.with_store(settings::provider);
        let platform = host.platform();
        let toggle = move || {
            host.with_store(|s| s.set_profile_ai(profile, !on).ok());
            if on {
                forget_chats();
                chat.close();
            }
            rev.update(|r| *r += 1);
        };
        let options: Vec<crate::controls::Segment> = Provider::ALL
            .into_iter()
            .filter(|p| *p != Provider::Ollama || platform.ollama_gap.is_none())
            .map(|p| {
                let pick = move || {
                    host.with_store(|s| settings::set_provider(s, p));
                    rev.update(|r| *r += 1);
                };
                (p.name().to_owned(), Box::new(move || p == chosen) as Box<dyn Fn() -> bool>, Box::new(pick) as Box<dyn Fn()>)
            })
            .collect();
        let claude = move || {
            let status = dynamic(move || {
                Some(hint(match connected.get() {
                    None => "Checking…".to_owned(),
                    Some(Ok(true)) => format!("Connected. The key is kept in {}, never in a file or Tarn's database.", platform.keyring),
                    Some(Ok(false)) => format!("Not connected. The key you enter is kept in {} only.", platform.keyring),
                    Some(Err(e)) => format!("{} is not available: {e}", platform.keyring),
                }))
            });
            let connect = move || {
                let value = token.get_untracked().trim().to_owned();
                token.set(String::new());
                if !value.is_empty() {
                    keep(Box::new(move || ai::secrets::store_token(secrets, &ApiToken::new(value))));
                }
            };
            let disconnect = move || keep(Box::new(move || ai::secrets::delete_token(secrets)));
            group()
                .p_3()
                .gap_2()
                .child(hint(CLAUDE))
                .child(status)
                .child(field("API key", token).masked(true).auto_correct(false).on_submit(move |_: &str| connect()))
                .child(view().flex_row().gap_2().child(accent_pill("Connect", connect)).child(pill("Disconnect", disconnect)))
        };
        let ollama = move || {
            let save = move || {
                if !host.with_store(|s| settings::set_ollama_model(s, &model.get_untracked())) {
                    error.set(Some("Model names use letters, digits and . _ - : / only.".to_owned()));
                }
            };
            group()
                .p_3()
                .gap_2()
                .child(hint(OLLAMA))
                .child(field("Model", model).auto_correct(false).on_submit(move |_: &str| save()))
                .child(view().flex_row().child(pill("Save Model", save)))
        };
        Some(view().gap_2().child(hint(ABOUT)).child(group().child(switch_row("Use the assistant in this profile", move || on, toggle))).when(on, |v| {
            v.child(section("Provider"))
                .child(segmented(options))
                .when_some(platform.ollama_gap, |v, why| v.child(hint(format!("{}: unavailable. {why}", Provider::Ollama.name()))))
                .child(match chosen {
                    Provider::Ollama if platform.ollama_gap.is_none() => ollama().into_element(),
                    _ => claude().into_element(),
                })
                .child(dynamic(move || error.get().map(|e| hint(e).text_color(Theme::accent_text))))
                .child(section("What is sent"))
                .child(hint(SENT))
        }))
    })
    .into_element()
}

/// Opens `raw` after the confirmation `links::classify` asks for, showing the whole address.
fn follow<H: Host>(host: H, raw: &str) {
    let (title, url) = match links::classify(raw) {
        LinkAction::ConfirmOpen(url) => (format!("Open a page on {}?", url.host_str().unwrap_or_default()), url),
        LinkAction::ConfirmMailto(url) => ("Write an email?".to_owned(), url),
        LinkAction::Refuse => {
            drop(nsh_base::alert_dialog().title("This link cannot be opened").description(links::shown("", raw)).cancel("ok", "OK").show());
            return;
        }
    };
    let answer = nsh_base::alert_dialog().title(title).description(url.as_str().to_owned()).cancel("cancel", "Cancel").action("open", "Open").show();
    Owner::detached().run(|| {
        drop(spawn(async move {
            if answer.await.is_ok_and(|a| a.action.as_deref() == Some("open")) {
                if url.scheme() == "mailto" {
                    drop(open_url(url.as_str()));
                } else {
                    host.open(url);
                }
            }
        }))
    });
}

fn spans<H: Host>(host: H, spans: Vec<Span>) -> Text {
    text("").text_size(px(TEXT)).text_color(Theme::text()).selectable(true).children(spans.into_iter().map(move |s| {
        let shown = match &s.link {
            Some(url) => links::shown(&s.text, url),
            None => s.text.clone(),
        };
        let mut span = text(shown);
        if s.style.bold {
            span = span.font_weight(FontWeight::SEMIBOLD);
        }
        if s.style.italic {
            span = span.italic();
        }
        if s.style.strike {
            span = span.line_through();
        }
        if s.style.code {
            span = span.font_family("monospace");
        }
        match s.link {
            Some(url) => span.underline().text_color(Theme::accent_text()).on_press(move |_: &PressEvent| follow(host, &url)).into_element(),
            None => span.into_element(),
        }
    }))
}

fn block<H: Host>(host: H, block: Block) -> AnyElement {
    match block {
        Block::Heading { level, spans: s } => {
            spans(host, s).text_size(px(if level <= 2 { TEXT_TITLE } else { TEXT })).font_weight(FontWeight::SEMIBOLD).into_element()
        }
        Block::Paragraph(s) => spans(host, s).into_element(),
        Block::ListItem { depth, number, spans: s } => view()
            .flex_row()
            .gap_1()
            .pl(px(f32::from(depth.saturating_sub(1)) * 14.))
            .child(label(number.map_or_else(|| "•".to_owned(), |n| format!("{n}."))).text_color(Theme::muted))
            .child(spans(host, s).flex_1().min_w(px(0.)))
            .into_element(),
        Block::Quote(s) => view().pl_2().border_l_1().border_color(Theme::hairline()).child(spans(host, s).text_color(Theme::muted())).into_element(),
        Block::Code { code, .. } => view()
            .p_2()
            .rounded(px(Theme::radius_small()))
            .bg(Theme::raised())
            .child(text(code).font_family("monospace").text_size(px(TEXT_SMALL)).text_color(Theme::text()).selectable(true))
            .into_element(),
        Block::Rule => view().h(px(1.)).bg(Theme::separator()).into_element(),
    }
}

fn answer_view<H: Host>(host: H, markdown: &str) -> View {
    view().gap_2().children(ai::markdown::parse(markdown).into_iter().map(move |b| block(host, b)))
}

/// The chat panel: the conversation, attached pills as chips, what a send will carry with its
/// sizes, the question field and Send or Cancel.
pub fn panel<H: Host>(host: H, chat: Chat, close: impl Fn() + Copy + 'static) -> AnyElement {
    let header = view().flex_row().items_center().gap_2().child(label("Assistant").font_weight(FontWeight::SEMIBOLD).flex_1()).child(icon_button(
        crate::icons::icon(crate::icons::CLOSE, 14.),
        "Close Assistant",
        close,
    ));
    view().flex_col().gap_3().p_3().h_full().min_h(px(0.)).bg(Theme::panel()).child(header).child(body(host, chat)).into_element()
}

/// The panel without its header, for a sheet that has its own.
pub fn body<H: Host>(host: H, chat: Chat) -> AnyElement {
    let notes_open = signal(false);
    dynamic(move || {
        let Some(chat_view) = chat.view.get() else {
            let here = host.origin().is_some_and(allowed);
            return Some(view().gap_2().child(empty_state(if here { OFF } else { NOT_HERE })));
        };
        let last = chat_view.rows.len().saturating_sub(1);
        let rows: Vec<AnyElement> = chat_view
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                let bubble = view().gap_1().p_2().rounded(px(Theme::radius_row()));
                let bubble = match row.role {
                    Role::You => bubble.bg(Theme::selected()).child(label(row.text.clone()).selectable(true)),
                    Role::Assistant if ix == last && row.streaming => {
                        let base = row.text.clone();
                        bubble.child(dynamic(move || Some(answer_view(host, &format!("{base}{}", chat.tail.get())))))
                    }
                    Role::Assistant => bubble.child(answer_view(host, &row.text)),
                };
                bubble.when(row.interrupted, |b| b.opacity(0.6).child(hint("Interrupted; not sent again."))).into_element()
            })
            .collect();
        let chips: Vec<AnyElement> = chat_view
            .chips
            .iter()
            .enumerate()
            .map(|(ix, c)| {
                let name = format!("{}: {}", c.kind.name(), c.label);
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded_full()
                    .bg(Theme::control())
                    .child(label(name.clone()).text_size(px(TEXT_SMALL)).number_of_lines(1_u32))
                    .child(icon_button(crate::icons::icon(crate::icons::CLOSE, 10.), format!("Remove {name}"), move || chat.apply(host, Event::RemovePill(ix))))
                    .into_element()
            })
            .collect();
        let will_send = (!chat_view.will_send.is_empty()).then(|| {
            view()
                .gap_1()
                .child(hint(format!("Will send with your question ({}):", size(chat_view.will_send_bytes))))
                .children(chat_view.will_send.iter().map(|(name, bytes)| hint(format!("{name} · {}", size(*bytes))).into_element()))
        });
        let (can_send, can_cancel) = (chat_view.can_send, chat_view.can_cancel);
        Some(
            view()
                .gap_2()
                .flex_1()
                .min_h(px(0.))
                .child(scroll().flex_1().child(view().gap_2().children(rows)))
                .children(chat_view.notice.clone().map(|n| hint(n).text_color(Theme::accent_text)))
                .child(view().flex_row().flex_wrap().gap_1().children(chips))
                .children(will_send)
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap_1()
                        .child(pill("Attach Page", move || host.read_page()))
                        .child(pill("Attach Note", move || notes_open.update(|o| *o = !*o))),
                )
                .child(dynamic(move || notes_open.get().then(|| notes(host, chat, notes_open))))
                .child(field("Ask about the page or anything", chat.question).multiline(true).min_h(px(64.)).on_submit(move |_: &str| chat.send(host, ASK)))
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap_1()
                        .when(can_send, |v| {
                            let custom = host.origin().map(|o| host.with_store(|s| s.ai_commands(o.profile).unwrap_or_default())).unwrap_or_default();
                            v.child(accent_pill("Send", move || chat.send(host, ASK))).children(
                                commands::all(&custom).into_iter().map(|(name, template)| pill(name, move || chat.send(host, &template)).into_element()),
                            )
                        })
                        .when(can_cancel, |v| v.child(pill("Cancel", move || chat.apply(host, Event::Cancel)))),
                ),
        )
    })
}

fn notes<H: Host>(host: H, chat: Chat, open: Signal<bool>) -> View {
    let Some(origin) = host.origin() else { return view() };
    let saved = host.with_store(|s| s.notes(origin.profile).unwrap_or_default());
    if saved.is_empty() {
        return view().child(hint(browser_core::notes::EMPTY));
    }
    group().children(saved.into_iter().map(move |n| {
        let name = browser_core::notes::title(&n);
        pill(name, move || {
            open.set(false);
            if let Some(origin) = host.origin() {
                chat.attach(host, origin, browser_core::notes::as_pill(&n));
            }
        })
        .into_element()
    }))
}

/// Clean Up with the assistant: sends `request`, checks the proposal against the tabs it named,
/// asks before anything moves and hands the checked groups to `apply`; `failed` gets the reason.
pub fn group_tabs<H: Host>(
    host: H,
    request: cleanup::ai::Request,
    apply: impl FnOnce(HashSet<Id>, Vec<(String, Vec<Id>)>) + 'static,
    failed: impl FnOnce(String) + 'static,
) {
    let req = ai::Request { system: request.system.to_owned(), page: None, question: request.prompt.clone() };
    let output = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let kept = output.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let sent = request.sent;
    run(
        host,
        req,
        cancel,
        move |text| {
            let mut out = kept.borrow_mut();
            out.push_str(&text);
            if out.len() > ai::grouping::MAX_OUTPUT_BYTES {
                stop.store(true, Ordering::Relaxed);
            }
        },
        move |end| {
            let proposal = end.and_then(|()| proposal(&output.borrow(), &sent));
            match proposal {
                Ok(groups) => confirm_groups(sent, groups, apply),
                Err(why) => failed(why),
            }
        },
    );
}

fn proposal(output: &str, sent: &HashSet<Id>) -> Result<Vec<(String, Vec<Id>)>, String> {
    let groups = ai::grouping::parse(output, sent).map_err(|e| format!("The assistant's answer was not a usable grouping: {e}"))?;
    Ok(groups.into_iter().map(|g| (g.name, g.tab_ids)).collect())
}

fn confirm_groups(sent: HashSet<Id>, groups: Vec<(String, Vec<Id>)>, apply: impl FnOnce(HashSet<Id>, Vec<(String, Vec<Id>)>) + 'static) {
    let names = groups.iter().map(|(name, ids)| format!("{name} ({})", ids.len())).collect::<Vec<_>>().join(", ");
    let answer = nsh_base::alert_dialog()
        .title("Group tabs as the assistant suggests?")
        .description(format!("{names}. You can undo this."))
        .cancel("cancel", "Cancel")
        .action("group", "Group Tabs")
        .show();
    Owner::detached().run(|| {
        drop(spawn(async move {
            if answer.await.is_ok_and(|a| a.action.as_deref() == Some("group")) {
                apply(sent, groups);
            }
        }))
    });
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::sync::Mutex;

    use ai::transport::Response;

    use super::*;

    #[derive(Default)]
    struct FakeSecrets(Mutex<Option<String>>);

    impl Secrets for FakeSecrets {
        fn get(&self, _: &str, _: &str) -> Result<Option<String>, AiError> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn set(&self, _: &str, _: &str, secret: &str) -> Result<(), AiError> {
            *self.0.lock().unwrap() = Some(secret.to_owned());
            Ok(())
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), AiError> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FakeTransport(Mutex<Vec<(String, String)>>);

    impl Transport for FakeTransport {
        fn post<'a>(&self, url: &str, headers: &[(&str, &str)], body: &[u8], _: u64, _: &'a AtomicBool) -> Result<Response<'a>, AiError> {
            let key = headers.iter().find(|(n, _)| *n == "x-api-key").map(|(_, v)| (*v).to_owned()).unwrap_or_default();
            self.0.lock().unwrap().push((url.to_owned(), format!("{key} {}", String::from_utf8_lossy(body))));
            let reply: &'static [u8] = if url.ends_with("/v1/messages") {
                b"data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\ndata: {\"type\":\"message_stop\"}\n\n"
            } else if url.ends_with("/api/show") {
                b"{}"
            } else {
                b"{\"message\":{\"content\":\"local\"},\"done\":true}\n"
            };
            Ok(Response { status: 200, body: Box::new(std::io::BufReader::new(reply.chain(&b""[..]))) })
        }
    }

    fn ask(backend: &Backend, secrets: &FakeSecrets) -> (Result<String, AiError>, Vec<(String, String)>) {
        let transport = Arc::new(FakeTransport::default());
        let req = ai::Request { system: "s".into(), page: None, question: "q".into() };
        let mut out = String::new();
        let result = answer(backend, secrets, transport.clone(), &req, &AtomicBool::new(false), &mut |t| out.push_str(t));
        let asked = transport.0.lock().unwrap().clone();
        (result.map(|()| out), asked)
    }

    #[test]
    fn claude_needs_a_stored_key_and_sends_it_only_to_anthropic() {
        let secrets = FakeSecrets::default();
        let (result, asked) = ask(&Backend::Claude, &secrets);
        assert!(matches!(result, Err(AiError::Unavailable(ref m)) if m == NOT_CONNECTED));
        assert!(asked.is_empty(), "nothing is sent without a key");
        ai::secrets::store_token(&secrets, &ApiToken::new("test-key".into())).unwrap();
        let (result, asked) = ask(&Backend::Claude, &secrets);
        assert_eq!(result.unwrap(), "Hi");
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].0, "https://api.anthropic.com/v1/messages");
        assert!(asked[0].1.starts_with("test-key ") && asked[0].1.contains("\"q\""));
    }

    #[test]
    fn ollama_answers_on_loopback_without_the_key() {
        let secrets = FakeSecrets(Mutex::new(Some("test-key".into())));
        let (result, asked) = ask(&Backend::Ollama("llama3.2".into()), &secrets);
        assert_eq!(result.unwrap(), "local");
        assert!(asked.iter().all(|(url, body)| url.starts_with("http://127.0.0.1:11434/") && !body.contains("test-key")));
        assert!(asked.iter().any(|(url, _)| url.ends_with("/api/chat")));
    }

    #[test]
    fn a_frame_takes_every_chunk_so_far_as_one() {
        let (tx, rx) = mpsc::channel();
        assert_eq!(drain(&rx), (String::new(), None));
        for t in ["a", "b", "c"] {
            tx.send(Msg::Text(t.into())).unwrap();
        }
        assert_eq!(drain(&rx), ("abc".into(), None));
        tx.send(Msg::Text("d".into())).unwrap();
        tx.send(Msg::End(Ok(()))).unwrap();
        assert_eq!(drain(&rx), ("d".into(), Some(Ok(()))));
        drop(tx);
        assert!(matches!(drain(&rx), (t, Some(Err(_))) if t.is_empty()), "a thread that died ends the request");
    }

    #[test]
    fn a_grouping_answer_is_checked_against_the_tabs_sent() {
        let sent: HashSet<Id> = [1, 2, 3].into();
        assert_eq!(proposal("{\"groups\":[{\"name\":\"News\",\"tab_ids\":[1,3]}]}", &sent).unwrap(), [("News".to_owned(), vec![1, 3])]);
        assert!(proposal("{\"groups\":[{\"name\":\"News\",\"tab_ids\":[1,9]}]}", &sent).is_err());
        assert!(proposal("not json", &sent).is_err());
    }

    #[test]
    fn sizes_read_as_bytes_or_kilobytes() {
        assert_eq!(size(512), "512 bytes");
        assert_eq!(size(1025), "2 KB");
    }
}
