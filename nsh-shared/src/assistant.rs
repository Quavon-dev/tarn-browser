//! The assistant on Linux, iOS and Android: Settings › AI, the chat panel that draws
//! `browser_core::assistant::view` and Clean Up's grouping. The core decides what a request
//! carries; this module only runs a provider off the main thread and draws the answers.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use ai::local::Model;
use ai::markdown::{Block, Span};
use ai::{AiError, ApiToken, ClaudeApi, Ollama, Secrets, Transport};
use browser_core::assistant::links::{self, LinkAction};
use browser_core::assistant::settings::{self, Provider};
use browser_core::assistant::view as chat_view;
use browser_core::assistant::{ASK, ChatView, Effect, Event, Origin, Pill, ProfileKind, Role, Session, allowed, commands, on_device_task};
use browser_core::cleanup;
use browser_core::tabs::{TabChoice, TabKey};
use nsh_core::prelude::*;
use shield::ShieldLevel;
use store::{Id, Store};
use url::Url;

use crate::controls::{
    accent_pill, add_button, confirm, described, editor, editor_actions, empty, field, group, hint, label, list_row, open_row, pill, pressable, row_action,
    section_with, segmented, signal_field, switch_row,
};
use crate::icons;
use crate::theme::{TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

pub const ABOUT: &str = "Answers questions in a chat beside the page. Never in a private space, a burner tab or on a Lockdown page.";
pub use browser_core::assistant::{NOT_CONNECTED, NOT_HERE, SENT};
pub const CLAUDE: &str =
    "Claude API sends each request over HTTPS to Anthropic (api.anthropic.com) with your API key; Anthropic's terms apply to what it receives.";
pub const OLLAMA: &str = "Ollama answers on this computer: requests go to 127.0.0.1 and never leave the device. Models that run in Ollama's cloud are refused.";
pub const OFF: &str = "Turn on the assistant for this profile in Settings › AI.";
const FRAME: Duration = Duration::from_millis(33);

/// What a client brings: its secret store, its transport and whether Ollama on loopback works.
pub struct Platform {
    pub secrets: &'static dyn Secrets,
    pub transport: fn() -> Arc<dyn Transport>,
    /// Where the key is kept, as Settings names it.
    pub keyring: &'static str,
    /// Why Ollama cannot answer on this client; none where it can.
    pub ollama_gap: Option<&'static str>,
    /// The system's on-device model and its name, which answers commands in place of the Claude
    /// API where it can; none where the client has none.
    pub on_device: Option<(&'static dyn ai::LocalModel, &'static str)>,
}

pub trait Host: Copy + 'static {
    fn platform(self) -> &'static Platform;
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R;
    /// The active page's `Origin` (its effective Shield level, per-site setting included), or the
    /// current space's with no page; none while its profile is not open.
    fn origin(self) -> Option<Origin>;
    /// The effective Shield level of `url` in the current profile.
    fn level_of(self, url: &Url) -> ShieldLevel;
    /// The effective Shield level of `url` in the saved `profile`, whatever the window shows now.
    fn level_in(self, profile: Id, url: &Url) -> ShieldLevel;
    /// Asks the active page for its text; the shell hands it to `Chat::attach` as a `Pill::Page`.
    fn read_page(self);
    /// The tab picker's tabs (`tabs::Model::assistant_tabs`).
    fn tabs(self) -> Vec<TabChoice>;
    /// Asks a loaded tab for its page text, never waking a sleeping one; the shell hands it to
    /// `Chat::attach` as a `Pill::Tab`.
    fn read_tab(self, key: TabKey);
    /// Opens `url` in a new tab of the current profile.
    fn open(self, url: Url);
    /// The current profile's AI switch changed, so its pages' selection menus follow.
    fn changed(self);
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
    Claude(String),
    Ollama(String),
}

fn backend<H: Host>(host: H) -> Result<Backend, String> {
    match host.with_store(settings::provider) {
        Provider::ClaudeApi => Ok(Backend::Claude(host.with_store(settings::claude_model_id))),
        Provider::Ollama => match host.platform().ollama_gap {
            Some(why) => Err(why.to_owned()),
            None => Ok(Backend::Ollama(host.with_store(settings::ollama_model))),
        },
    }
}

/// Blocks on the network: call off the main thread.
fn answer(
    backend: &Backend,
    platform: &Platform,
    transport: Arc<dyn Transport>,
    on_device: bool,
    req: &ai::Request,
    cancel: &AtomicBool,
    out: &mut dyn FnMut(Msg),
) -> Result<(), AiError> {
    let out = std::cell::RefCell::new(out);
    let named = &mut |m: &str| (out.borrow_mut())(Msg::Model(m.to_owned()));
    let on_text = &mut |t: &str| (out.borrow_mut())(Msg::Text(t.to_owned()));
    match backend {
        Backend::Claude(model) => {
            let token = ai::secrets::load_token(platform.secrets)?.ok_or_else(|| AiError::Unavailable(NOT_CONNECTED.into()))?;
            let api = ClaudeApi::new(token, Some(model.clone()), transport);
            ai::local::answer(Model::Cloud(&api, Provider::ClaudeApi.name()), platform.on_device, on_device, req, cancel, named, on_text)
        }
        Backend::Ollama(model) => {
            let name = format!("{} ({model})", Provider::Ollama.name());
            ai::local::answer(Model::Local(&Ollama::new(model.clone(), transport), &name), None, on_device, req, cancel, named, on_text)
        }
    }
}

enum Msg {
    Model(String),
    Text(String),
    End(Result<(), String>),
}

fn drain(rx: &mpsc::Receiver<Msg>) -> (Option<String>, String, Option<Result<(), String>>) {
    let (mut model, mut text) = (None, String::new());
    loop {
        match rx.try_recv() {
            Ok(Msg::Model(m)) => model = Some(m),
            Ok(Msg::Text(t)) => text.push_str(&t),
            Ok(Msg::End(end)) => return (model, text, Some(end)),
            Err(mpsc::TryRecvError::Empty) => return (model, text, None),
            Err(mpsc::TryRecvError::Disconnected) => return (model, text, Some(Err("the request stopped".into()))),
        }
    }
}

fn run<H: Host>(
    host: H,
    req: ai::Request,
    on_device: bool,
    cancel: Arc<AtomicBool>,
    on_frame: impl Fn(Option<String>, String) + 'static,
    done: impl FnOnce(Result<(), String>) + 'static,
) {
    let backend = match backend(host) {
        Ok(b) => b,
        Err(why) => return done(Err(why)),
    };
    let (platform, transport) = (host.platform(), (host.platform().transport)());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let sent = tx.clone();
        let ended = answer(&backend, platform, transport, on_device, &req, &cancel, &mut |m| drop(sent.send(m)));
        drop(tx.send(Msg::End(ended.map_err(|e| e.to_string()))));
    });
    Owner::detached().run(|| {
        drop(spawn(async move {
            loop {
                timer(FRAME).await;
                let (model, text, end) = drain(&rx);
                if model.is_some() || !text.is_empty() {
                    on_frame(model, text);
                }
                if let Some(end) = end {
                    return done(end);
                }
            }
        }))
    });
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

thread_local! {
    static FORGOTTEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static RUNS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Each `forget_chats_of` since the start, by the generation it made.
    static FORGOTTEN_OF: std::cell::RefCell<Vec<(u64, Id)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Drops every window's chat the next time it is used: call when a profile is deleted or reset or
/// its AI switch is turned off.
pub fn forget_chats() {
    FORGOTTEN.set(FORGOTTEN.get() + 1);
    FORGOTTEN_OF.with_borrow_mut(Vec::clear);
}

/// Drops every window's chat of `profile` the next time it is used, once its saved chats are deleted.
fn forget_chats_of(profile: Id) {
    FORGOTTEN.set(FORGOTTEN.get() + 1);
    FORGOTTEN_OF.with_borrow_mut(|f| f.push((FORGOTTEN.get(), profile)));
}

/// Whether a chat of `profile` made at generation `at` was forgotten since.
fn forgotten(at: u64, profile: Id) -> bool {
    let now = FORGOTTEN.get();
    at != now && FORGOTTEN_OF.with_borrow(|f| f.first().is_none_or(|(first, _)| *first > at + 1) || f.iter().any(|(made, p)| *made > at && *p == profile))
}

/// One window's chat: its `Session`, the request it runs and what the panel draws.
#[derive(Clone, Copy)]
pub struct Chat {
    session: Stored<Option<(u64, Session)>>,
    /// The request running now: its run, unique for the process, its id in the session, and its cancel flag.
    running: Stored<Option<(u64, u64, Arc<AtomicBool>)>>,
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
        if self.here(host) {
            return true;
        }
        let Some(session) = host.origin().and_then(|o| Session::new(o, ai_on(host, o.profile))) else {
            return false;
        };
        self.view.set(Some(chat_view(&session)));
        self.session.replace(Some((FORGOTTEN.get(), session)));
        true
    }

    /// Drops the conversation and stops its request.
    pub fn close(self) {
        if let Some((_, _, cancel)) = self.running.replace(None) {
            cancel.store(true, Ordering::Relaxed);
        }
        self.session.replace(None);
        self.view.set(None);
        self.tail.set(String::new());
        self.question.set(String::new());
    }

    /// Ends the conversation once the current page is another profile's, in a space or at a level
    /// the assistant may not see: call whenever the window changes. Its AI switch turned off ends
    /// it through `forget_chats`, so a page event reads no store.
    pub fn sync<H: Host>(self, host: H) {
        self.check();
        let stays = |profile: Id| host.origin().is_some_and(|o| allowed(o) && o.profile == profile);
        if self.session.with(|s| s.as_ref().is_some_and(|(_, s)| !stays(s.profile()))) {
            self.close();
        }
    }

    /// Whether the conversation still belongs where the window is now, after `sync`.
    fn here<H: Host>(self, host: H) -> bool {
        self.sync(host);
        self.session.with(|s| s.is_some())
    }

    fn check(self) {
        match self.session.with(|s| s.as_ref().map(|(at, s)| forgotten(*at, s.profile()))) {
            Some(true) => self.close(),
            Some(false) => self.session.with(|s| s.iter_mut().for_each(|(at, _)| *at = FORGOTTEN.get())),
            None => {}
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

    /// Sends the question field through `template`.
    pub fn send<H: Host>(self, host: H, template: &str) {
        self.send_with(host, &self.question.get_untracked(), template, true);
    }

    /// Sends the question field; `/name rest` runs that command with `rest` as its question.
    pub fn submit<H: Host>(self, host: H) {
        let typed = self.question.get_untracked();
        let all = all_commands(host);
        match commands::slash(&typed, &all) {
            Some((ix, rest)) => self.send_with(host, rest, &all[ix].1, true),
            None => self.send(host, ASK),
        }
    }

    /// Attaches the page's selection from the selection menu and, for a command, sends it with
    /// that command's template and no question: one choice, one send.
    pub fn ask_selection<H: Host>(self, host: H, origin: Origin, pill: Pill, command: Option<&str>) {
        self.attach(host, origin, pill.clone());
        if command.is_none() || !self.session.with(|s| s.as_ref().is_some_and(|(_, s)| s.holds(&pill))) {
            return;
        }
        let custom = host.with_store(|s| s.ai_commands(origin.profile).unwrap_or_default());
        if let Some(template) = command.and_then(|name| commands::for_selection(&custom, name)) {
            self.send_with(host, "", &template, false);
        }
    }

    /// Where a send comes from now and whether the profile's AI switch is on; none without a chat.
    fn context<H: Host>(self, host: H) -> Option<(Origin, bool)> {
        self.check();
        let profile = self.session.with(|s| s.as_ref().map(|(_, s)| s.profile()))?;
        let origin = host.origin().unwrap_or(Origin { profile, kind: ProfileKind::Private, level: ShieldLevel::Lockdown });
        Some((origin, ai_on(host, profile)))
    }

    /// `from_input`: the question came from the field, which empties once it is sent.
    fn send_with<H: Host>(self, host: H, question: &str, template: &str, from_input: bool) {
        let Some((origin, on)) = self.context(host) else { return };
        let effects = self.session.with(|s| s.as_mut().map(|(_, s)| s.send(question, template, now(), origin, on, |u| host.level_of(u)))).unwrap_or_default();
        if from_input && effects.iter().any(|e| matches!(e, Effect::Start { .. })) {
            self.question.set(String::new());
        }
        self.effects_for(host, effects, on_device_task(template));
        self.save(host);
    }

    /// Asks the last question again in place of its answer.
    pub fn retry<H: Host>(self, host: H) {
        let Some((origin, on)) = self.context(host) else { return };
        let (effects, on_device) =
            self.session.with(|s| s.as_mut().map(|(_, s)| (s.retry(now(), origin, on, |u| host.level_of(u)), s.last_was_command()))).unwrap_or_default();
        self.effects_for(host, effects, on_device);
        self.save(host);
    }

    /// Takes the last question and its answer out of the chat and puts the question in the field.
    pub fn edit_last<H: Host>(self, host: H) {
        self.check();
        if let Some(question) = self.session.with(|s| s.as_mut().and_then(|(_, s)| s.edit_last())) {
            self.save(host);
            self.question.set(question);
            self.render();
        }
    }

    /// Keeps the chat once no answer streams, without what a site in Lockdown now gave it, and
    /// shows a notice the save gave.
    fn save<H: Host>(self, host: H) {
        self.check();
        let shown = self.session.with(|s| {
            s.as_mut().is_some_and(|(_, s)| {
                let before = chat_view(s).notice;
                let profile = s.profile();
                host.with_store(|store| s.save_with(store, |u| host.level_in(profile, u)));
                chat_view(s).notice != before
            })
        });
        if shown {
            self.render();
        }
    }

    fn runs(self, run: u64) -> bool {
        self.running.with(|r| r.as_ref().is_some_and(|(r, _, _)| *r == run))
    }

    /// The saved chat this conversation continues.
    pub fn saved(self) -> Option<Id> {
        self.session.with(|s| s.as_ref().and_then(|(_, s)| s.chat()))
    }

    /// Continues the current profile's saved chat `id` in place of this conversation.
    pub fn restore<H: Host>(self, host: H, id: Id) -> bool {
        let Some(origin) = host.origin() else { return false };
        if self.saved() == Some(id) && allowed(origin) && ai_on(host, origin.profile) {
            return true;
        }
        self.finish(host);
        let Some(saved) = host.with_store(|s| s.chat(origin.profile, id).ok().flatten()) else { return false };
        let Some(session) = Session::restore(origin, ai_on(host, origin.profile), &saved, |u| host.level_of(u)) else { return false };
        self.close();
        self.view.set(Some(chat_view(&session)));
        self.session.replace(Some((FORGOTTEN.get(), session)));
        self.save(host);
        true
    }

    /// Attaches `note` from the page or space shown now.
    pub fn attach_note<H: Host>(self, host: H, note: &store::Note) {
        if let Some(origin) = host.origin()
            && let Some(pill) = browser_core::notes::as_pill(note, origin.profile)
        {
            self.attach(host, origin, pill);
        }
    }

    /// Stops an answer that is streaming and keeps the chat as it stands.
    pub fn finish<H: Host>(self, host: H) {
        self.apply(host, Event::Cancel);
        self.save(host);
    }

    fn render(self) {
        self.view.set(self.session.with(|s| s.as_ref().map(|(_, s)| chat_view(s))));
        self.tail.set(String::new());
    }

    fn effects<H: Host>(self, host: H, effects: Vec<Effect>) {
        self.effects_for(host, effects, false);
    }

    fn effects_for<H: Host>(self, host: H, effects: Vec<Effect>, on_device: bool) {
        for effect in effects {
            match effect {
                Effect::Start { id, request } => {
                    let cancel = Arc::new(AtomicBool::new(false));
                    let ticket = RUNS.get() + 1;
                    RUNS.set(ticket);
                    self.running.replace(Some((ticket, id, cancel.clone())));
                    let req = ai::Request { system: request.system(), page: None, question: request.user_message() };
                    run(
                        host,
                        req,
                        on_device,
                        cancel,
                        move |model, text| {
                            if !self.runs(ticket) {
                                return;
                            }
                            if let Some(model) = model {
                                self.apply(host, Event::AnsweredBy { id, model });
                            }
                            if !text.is_empty() {
                                self.apply(host, Event::Chunk { id, text });
                            }
                        },
                        move |end| {
                            if !self.runs(ticket) {
                                return;
                            }
                            self.running.replace(None);
                            self.apply(host, end.map_or_else(|message| Event::Failed { id, message }, |()| Event::Done { id }));
                            self.save(host);
                        },
                    );
                }
                Effect::Append { text } => self.tail.update(|t| t.push_str(&text)),
                Effect::Stop { id } => {
                    if let Some((_, _, cancel)) = self.running.with(|r| r.take_if(|(_, r, _)| *r == id)) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                Effect::Render => self.render(),
            }
        }
    }
}

/// The provider choice, which `changed` follows; Ollama only where the client can reach it.
fn providers<H: Host>(host: H, changed: impl Fn() + Copy + 'static) -> View {
    let chosen = host.with_store(settings::provider);
    let gap = host.platform().ollama_gap;
    segmented(
        Provider::ALL
            .into_iter()
            .filter(|p| *p != Provider::Ollama || gap.is_none())
            .map(|p| {
                let pick = move || {
                    host.with_store(|s| settings::set_provider(s, p));
                    changed();
                };
                (p.name().to_owned(), Box::new(move || p == chosen) as Box<dyn Fn() -> bool>, Box::new(pick) as Box<dyn Fn()>)
            })
            .collect(),
    )
}

/// The Claude model choice, which `changed` follows.
fn claude_models<H: Host>(host: H, changed: impl Fn() + Copy + 'static) -> View {
    let chosen = host.with_store(settings::claude_model);
    segmented(
        settings::CLAUDE_MODELS
            .iter()
            .enumerate()
            .map(|(ix, (_, name))| {
                let pick = move || {
                    host.with_store(|s| settings::set_claude_model(s, ix));
                    changed();
                };
                ((*name).to_owned(), Box::new(move || ix == chosen) as Box<dyn Fn() -> bool>, Box::new(pick) as Box<dyn Fn()>)
            })
            .collect(),
    )
}

/// What answers now: the Claude model's name or Ollama's model.
fn answers_with<H: Host>(host: H) -> String {
    match host.with_store(settings::provider) {
        Provider::ClaudeApi => settings::CLAUDE_MODELS[host.with_store(settings::claude_model)].1.to_owned(),
        Provider::Ollama => format!("Ollama: {}", host.with_store(settings::ollama_model)),
    }
}

pub const PLACEHOLDERS: &str =
    "In a template, {page} stands for the page or note attached, {selection} for the text selected on the page and {question} for what you type.";

/// A custom command being added (`was` none) or edited.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandDraft {
    pub was: Option<Id>,
    pub name: String,
    pub template: String,
}

impl CommandDraft {
    /// Saves the draft for `profile` among its `custom` commands; the message to show when it cannot.
    pub fn save(&self, store: &Store, profile: Id, custom: &[store::AiCommand]) -> Result<(), String> {
        commands::check(&self.name, &self.template, custom, self.was)?;
        let name = self.name.trim();
        let saved = match self.was {
            None => store.create_ai_command(profile, name, &self.template).map(drop),
            Some(id) => store.update_ai_command(profile, id, name, &self.template),
        };
        saved.map_err(|_| "The command could not be saved.".to_owned())
    }
}

/// Settings › AI › Commands: the built-in commands, and the profile's own to add, edit and delete.
fn commands_section<H: Host>(host: H, profile: Id) -> AnyElement {
    let rev = signal(0u64);
    let draft: Signal<Option<CommandDraft>> = signal(None);
    let error: Signal<Option<String>> = signal(None);
    let editing = computed(move || draft.with(Option::is_some));
    let heading = computed(move || draft.with(|d| d.as_ref().map(|d| if d.was.is_none() { "New Command" } else { "Edit Command" })));
    let edit = move |next: Option<CommandDraft>| {
        error.set(None);
        draft.set(next);
    };
    let add = move || add_button("Add Command", move || edit(Some(CommandDraft::default())));
    dynamic(move || {
        rev.get();
        let custom = host.with_store(|s| s.ai_commands(profile).unwrap_or_default());
        let built_in = group().children(commands::BUILT_IN.iter().enumerate().map(|(ix, (name, template))| list_row(ix, *name, *template).into_element()));
        let own: Vec<AnyElement> = custom
            .iter()
            .enumerate()
            .map(|(ix, c)| {
                let (id, name) = (c.id, c.name.clone());
                let open = CommandDraft { was: Some(id), name: c.name.clone(), template: c.template.clone() };
                let gone = name.clone();
                list_row(ix, name.clone(), c.template.clone())
                    .child(row_action(icons::PENCIL, format!("Edit {name}"), move || edit(Some(open.clone()))))
                    .child(row_action(icons::TRASH, format!("Delete {name}"), move || {
                        confirm(format!("Delete {gone}?"), "The command is removed from this profile.", "Delete", move || {
                            host.with_store(|s| s.delete_ai_command(profile, id).ok());
                            host.changed();
                            rev.update(|r| *r += 1);
                        })
                    }))
                    .into_element()
            })
            .collect();
        let has_own = !own.is_empty();
        let (listed, total) = (commands::names(&custom).len(), commands::all(&custom).len());
        let editor = dynamic(move || {
            let title = heading.get()?;
            let save = move || {
                let Some(d) = draft.get_untracked() else { return };
                let custom = host.with_store(|s| s.ai_commands(profile).unwrap_or_default());
                match host.with_store(|s| d.save(s, profile, &custom)) {
                    Ok(()) => {
                        edit(None);
                        host.changed();
                        rev.update(|r| *r += 1);
                    }
                    Err(message) => error.set(Some(message)),
                }
            };
            let set = move |f: &dyn Fn(&mut CommandDraft)| draft.update(|d| d.iter_mut().for_each(f));
            Some(
                editor(title)
                    .child(field(
                        "Name",
                        move || draft.with(|d| d.as_ref().map(|d| d.name.clone()).unwrap_or_default()),
                        move |v| set(&|d| d.name = v.to_owned()),
                    ))
                    .child(
                        field(
                            "Template",
                            move || draft.with(|d| d.as_ref().map(|d| d.template.clone()).unwrap_or_default()),
                            move |v| set(&|d| d.template = v.to_owned()),
                        )
                        .multiline(true)
                        .min_h(px(88.)),
                    )
                    .child(hint(PLACEHOLDERS))
                    .child(dynamic(move || error.get().map(|e| hint(e).text_color(Theme::accent_text))))
                    .child(editor_actions(save, move || edit(None))),
            )
        });
        let shown = if total > listed {
            format!("A selection's menu lists the first {listed}; the chat's / list runs every one.")
        } else {
            "In a selection's menu, and in the chat when you type /.".to_owned()
        };
        Some(
            view()
                .gap_2()
                .child(section_with("Commands", shown))
                .child(built_in)
                .child(section_with("Your commands", ""))
                .child(if has_own { group().children(own) } else { group().child(empty(icons::COMMAND, "No commands of your own yet.", None)) })
                .child(dynamic(move || (!editing.get()).then(|| view().flex_row().child(add()))))
                .child(editor),
        )
    })
    .into_element()
}

/// Settings › AI › Chats: how many chats the profile keeps, and Delete All Chats.
fn chats_section<H: Host>(host: H, profile: Id) -> AnyElement {
    let rev = signal(0u64);
    let failed: Signal<Option<String>> = signal(None);
    dynamic(move || {
        rev.get();
        let count = host.with_store(|s| s.chat_titles(profile).map(|t| t.len()).unwrap_or_default());
        let delete = move || {
            confirm("Delete all chats?", "Every saved chat of this profile is removed. This cannot be undone.", "Delete All Chats", move || {
                match host.with_store(|s| s.delete_chats(profile)) {
                    Ok(()) => {
                        forget_chats_of(profile);
                        failed.set(None);
                    }
                    Err(e) => failed.set(Some(format!("Could not delete the chats: {e}."))),
                }
                rev.update(|r| *r += 1);
            })
        };
        let saved = match count {
            0 => "None yet".to_owned(),
            1 => "1 chat".to_owned(),
            n => format!("{n} chats"),
        };
        Some(
            view()
                .gap_2()
                .child(section_with("Chats", "A chat is saved in this profile once it has an answer."))
                .child(group().child(list_row(0, "Saved chats", saved).when(count > 0, |v| v.child(pill("Delete All Chats", delete)))))
                .child(dynamic(move || failed.get().map(|e| hint(e).text_color(Theme::accent_text)))),
        )
    })
    .into_element()
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
            return Some(view().gap_2().child(hint(ABOUT)).child(group().child(empty(icons::LOCK, NOT_HERE, None))));
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
            host.changed();
            rev.update(|r| *r += 1);
        };
        let bump = move || rev.update(|r| *r += 1);
        let claude = move || {
            let status = dynamic(move || {
                let (glyph, color, line) = match connected.get() {
                    None => (icons::KEY_ROUND, Theme::muted(), "Checking…".to_owned()),
                    Some(Ok(true)) => (icons::CIRCLE_CHECK, Theme::success(), format!("Connected · key kept in {}", platform.keyring)),
                    Some(Ok(false)) => (icons::KEY_ROUND, Theme::muted(), format!("Not connected · a key you enter is kept in {} only", platform.keyring)),
                    Some(Err(e)) => (icons::TRIANGLE_ALERT, Theme::warning(), format!("{} is not available: {e}", platform.keyring)),
                };
                Some(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(view().flex_none().text_color(color).child(icons::icon(glyph, 14.)))
                        .child(label(line).text_size(px(TEXT_SMALL)).flex_1().min_w(px(0.))),
                )
            });
            let connect = move || {
                let value = token.get_untracked().trim().to_owned();
                token.set(String::new());
                if !value.is_empty() {
                    keep(Box::new(move || ai::secrets::store_token(secrets, &ApiToken::new(value))));
                }
            };
            let disconnect = move || keep(Box::new(move || ai::secrets::delete_token(secrets)));
            view()
                .gap_2()
                .child(
                    group()
                        .p_3()
                        .gap_2()
                        .child(status)
                        .child(signal_field("API key", token).masked(true).auto_correct(false).on_submit(move |_: &str| connect()))
                        .child(view().flex_row().flex_wrap().justify_end().gap_2().child(pill("Disconnect", disconnect)).child(accent_pill("Connect", connect)))
                        .child(hint(CLAUDE).px_0()),
                )
                .child(section_with("Model", ""))
                .child(claude_models(host, bump))
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
                .child(signal_field("Model", model).auto_correct(false).on_submit(move |_: &str| save()))
                .child(view().flex_row().justify_end().child(pill("Save Model", save)))
                .child(hint(OLLAMA).px_0())
        };
        Some(
            view()
                .gap_2()
                .child(group().child(described(switch_row("Use the assistant in this profile", move || on, toggle), ABOUT)))
                .when(on, |v| {
                    v.child(section_with("Provider", "Who answers your questions."))
                        .child(providers(host, bump))
                        .when_some(platform.ollama_gap, |v, why| v.child(hint(format!("{}: unavailable. {why}", Provider::Ollama.name()))))
                        .child(match chosen {
                            Provider::Ollama if platform.ollama_gap.is_none() => ollama().into_element(),
                            _ => claude().into_element(),
                        })
                        .child(dynamic(move || error.get().map(|e| hint(e).text_color(Theme::accent_text))))
                        .child(section_with("What is sent", ""))
                        .child(group().p_3().child(hint(SENT).px_0()))
                        .child(commands_section(host, profile))
                })
                .child(chats_section(host, profile)),
        )
    })
    .into_element()
}

fn refuse(raw: &str) {
    drop(nsh_base::alert_dialog().title("This link cannot be opened").description(links::shown("", raw)).cancel("ok", "OK").show());
}

/// Opens `raw` after the confirmation `links::classify` asks for, showing the whole address, in a
/// new tab of the chat's own profile; refused once the window shows another profile or a space the
/// assistant may not see.
fn follow<H: Host>(host: H, chat: Chat, raw: &str) {
    let (title, url) = match links::classify(raw) {
        LinkAction::ConfirmOpen(url) if chat.here(host) => (format!("Open a page on {}?", url.host_str().unwrap_or_default()), url),
        LinkAction::ConfirmMailto(url) => ("Write an email?".to_owned(), url),
        _ => return refuse(raw),
    };
    let answer = nsh_base::alert_dialog().title(title).description(url.as_str().to_owned()).cancel("cancel", "Cancel").action("open", "Open").show();
    Owner::detached().run(|| {
        drop(spawn(async move {
            if answer.await.is_ok_and(|a| a.action.as_deref() == Some("open")) {
                if !chat.here(host) {
                    refuse(url.as_str());
                } else if url.scheme() == "mailto" {
                    drop(open_url(url.as_str()));
                } else {
                    host.open(url);
                }
            }
        }))
    });
}

fn spans<H: Host>(host: H, chat: Chat, spans: Vec<Span>) -> Text {
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
            Some(url) => span.underline().text_color(Theme::accent_text()).on_press(move |_: &PressEvent| follow(host, chat, &url)).into_element(),
            None => span.into_element(),
        }
    }))
}

fn block<H: Host>(host: H, chat: Chat, block: Block) -> AnyElement {
    match block {
        Block::Heading { level, spans: s } => {
            spans(host, chat, s).text_size(px(if level <= 2 { TEXT_TITLE } else { TEXT })).font_weight(FontWeight::SEMIBOLD).into_element()
        }
        Block::Paragraph(s) => spans(host, chat, s).into_element(),
        Block::ListItem { depth, number, spans: s } => view()
            .flex_row()
            .gap_1()
            .pl(px(f32::from(depth.saturating_sub(1)) * 14.))
            .child(label(number.map_or_else(|| "•".to_owned(), |n| format!("{n}."))).text_color(Theme::muted))
            .child(spans(host, chat, s).flex_1().min_w(px(0.)))
            .into_element(),
        Block::Quote(s) => view().pl_2().border_l_1().border_color(Theme::hairline()).child(spans(host, chat, s).text_color(Theme::muted())).into_element(),
        Block::Code { code, .. } => {
            let copied = code.clone();
            view()
                .flex_row()
                .gap_1()
                .p_2()
                .rounded(px(Theme::radius_small()))
                .bg(Theme::raised())
                .child(text(code).font_family("monospace").text_size(px(TEXT_SMALL)).text_color(Theme::text()).selectable(true).flex_1().min_w(px(0.)))
                .child(row_action(icons::COPY, "Copy Code", move || set_clipboard(copied.clone())))
                .into_element()
        }
        Block::Rule => view().h(px(1.)).bg(Theme::separator()).into_element(),
    }
}

fn answer_view<H: Host>(host: H, chat: Chat, blocks: Vec<Block>) -> View {
    view().gap_2().children(blocks.into_iter().map(move |b| block(host, chat, b)))
}

/// An answer's links again as buttons, so the keyboard reaches each one (Tab, then Enter) and
/// opens it through the same confirmation as a press on the link.
fn link_list<H: Host>(host: H, chat: Chat, blocks: &[Block]) -> Option<View> {
    let links: Vec<(String, String)> = blocks
        .iter()
        .cloned()
        .flat_map(|block| match block {
            Block::Heading { spans, .. } | Block::Paragraph(spans) | Block::ListItem { spans, .. } | Block::Quote(spans) => spans,
            Block::Code { .. } | Block::Rule => Vec::new(),
        })
        .filter_map(|s| s.link.map(|url| (links::shown(&s.text, &url), url)))
        .collect();
    (!links.is_empty()).then(|| {
        view().gap_1().child(hint("Links")).children(links.into_iter().map(move |(shown, url)| pill(shown, move || follow(host, chat, &url)).into_element()))
    })
}

/// The tab picker: the space's other tabs the assistant may read; a sleeping one is listed but
/// cannot be chosen, since adding it would load its page.
fn tab_picker<H: Host>(host: H, open: Signal<bool>) -> View {
    let tabs = host.tabs();
    if tabs.is_empty() {
        return group().child(empty(icons::APP_WINDOW, "No other tab here can be added.", None));
    }
    group().children(tabs.into_iter().enumerate().map(move |(ix, t)| {
        if !t.awake {
            return list_row(ix, t.title, "Asleep; open it to add it").opacity(0.6).into_element();
        }
        let key = t.key;
        open_row(ix, t.title.clone(), "", format!("Add {}", t.title), move || {
            open.set(false);
            host.read_tab(key);
        })
        .into_element()
    }))
}

/// The chat panel: the conversation, attached pills as chips, what a send will carry with its
/// sizes, the question field and Send or Stop.
pub fn panel<H: Host>(host: H, chat: Chat, close: impl Fn() + Copy + 'static) -> AnyElement {
    view().flex_col().p_3().h_full().min_h(px(0.)).bg(Theme::panel()).child(chat_body(host, chat, Some(close))).into_element()
}

/// The panel on a card that paints its own background, as the desktop draws it beside the page.
pub fn card<H: Host>(host: H, chat: Chat, close: impl Fn() + Copy + 'static) -> AnyElement {
    view().flex_col().p_3().h_full().min_h(px(0.)).child(chat_body(host, chat, Some(close))).into_element()
}

fn all_commands<H: Host>(host: H) -> Vec<(String, String)> {
    let custom = host.origin().map(|o| host.with_store(|s| s.ai_commands(o.profile).unwrap_or_default())).unwrap_or_default();
    commands::all(&custom)
}

/// The profile's saved chats: a search, and each to open, rename or delete.
fn chat_list<H: Host>(host: H, chat: Chat, open: Signal<bool>) -> AnyElement {
    let rev = signal(0u64);
    let query = signal(String::new());
    let renaming: Signal<Option<(Id, String)>> = signal(None);
    let renamed = computed(move || renaming.with(|r| r.as_ref().map(|(id, _)| *id)));
    let error: Signal<Option<String>> = signal(None);
    dynamic(move || {
        rev.get();
        let profile = host.origin()?.profile;
        let titles = host.with_store(|s| s.chat_titles(profile).unwrap_or_default());
        if titles.is_empty() {
            return Some(group().child(empty(icons::HISTORY, "No saved chats yet. A chat is saved once it has an answer.", None)));
        }
        let rows = dynamic(move || {
            let shown = browser_core::assistant::find_chats(&titles, &query.get());
            if shown.is_empty() {
                return Some(group().child(empty(icons::SEARCH, "No chats match.", None)));
            }
            let now = browser_core::now();
            Some(group().children(shown.into_iter().enumerate().map(|(ix, c)| {
                let (id, title) = (c.id, c.title.clone());
                let (named, gone) = (title.clone(), title.clone());
                let when = host.with_store(|s| browser_core::library::ago(s, c.updated, now));
                open_row(ix, title.clone(), when, format!("Open {title}"), move || {
                    if chat.restore(host, id) {
                        open.set(false);
                    }
                })
                .child(row_action(icons::PENCIL, format!("Rename {}", c.title), move || {
                    error.set(None);
                    renaming.set(Some((id, named.clone())));
                }))
                .child(row_action(icons::TRASH, format!("Delete {}", c.title), move || {
                    confirm(format!("Delete {gone}?"), "The chat is removed from this profile.", "Delete", move || {
                        host.with_store(|s| s.delete_chat(profile, id).ok());
                        if chat.saved() == Some(id) {
                            chat.close();
                        }
                        rev.update(|r| *r += 1);
                    })
                }))
                .into_element()
            })))
        });
        let editor = dynamic(move || {
            let id = renamed.get()?;
            let save = move || {
                let title = renaming.with_untracked(|r| r.as_ref().map(|(_, t)| t.clone())).unwrap_or_default();
                match host.with_store(|s| s.rename_chat(profile, id, &title)) {
                    Ok(()) => {
                        renaming.set(None);
                        rev.update(|r| *r += 1);
                        chat.render();
                    }
                    Err(e) => error.set(Some(browser_core::assistant::rename_problem(&e))),
                }
            };
            Some(
                editor("Rename Chat")
                    .child(
                        field(
                            "Title",
                            move || renaming.with(|r| r.as_ref().map(|(_, t)| t.clone()).unwrap_or_default()),
                            move |v| renaming.update(|r| r.iter_mut().for_each(|(_, t)| *t = v.to_owned())),
                        )
                        .on_submit(move |_: &str| save()),
                    )
                    .child(dynamic(move || error.get().map(|e| hint(e).text_color(Theme::accent_text))))
                    .child(editor_actions(save, move || renaming.set(None))),
            )
        });
        Some(view().gap_2().child(signal_field("Search chats", query)).child(rows).child(editor))
    })
    .into_element()
}

/// The provider and, for the Claude API, the model, as Settings › AI offers them.
fn model_picker<H: Host>(host: H, rev: Signal<u64>) -> View {
    rev.get();
    let bump = move || rev.update(|r| *r += 1);
    let claude = host.with_store(settings::provider) == Provider::ClaudeApi;
    view().gap_1().child(providers(host, bump)).when(claude, |v| v.child(claude_models(host, bump)))
}

fn bubble_actions<H: Host>(host: H, chat: Chat, copy: Option<String>, last: bool) -> View {
    view().flex_row().gap_1().children(copy.map(|text| row_action(icons::COPY, "Copy Answer", move || set_clipboard(text.clone())).into_element())).when(
        last,
        |v| {
            v.child(row_action(icons::RELOAD, "Retry", move || chat.retry(host)))
                .child(row_action(icons::PENCIL, "Edit Question", move || chat.edit_last(host)))
        },
    )
}

/// The chat's heading: its saved title, else its first question's first line, else "New Chat".
fn heading(saved: Option<&str>, first_question: Option<&str>) -> String {
    saved
        .or_else(|| first_question.and_then(|q| q.lines().map(str::trim).find(|l| !l.is_empty())))
        .filter(|t| !t.trim().is_empty())
        .map_or_else(|| "New Chat".to_owned(), str::to_owned)
}

/// What a send carries besides the question, on one line.
fn will_send_line(items: &[(String, usize)], total: usize) -> String {
    let each = items.iter().map(|(name, bytes)| format!("{name} · {}", size(*bytes))).collect::<Vec<_>>().join(", ");
    format!("Will send with your question ({}): {each}", size(total))
}

fn chip<H: Host>(host: H, chat: Chat, ix: usize, name: String) -> AnyElement {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .pl_3()
        .max_w(relative(1.))
        .rounded_full()
        .bg(Theme::control())
        .border_1()
        .border_color(Theme::hairline())
        .child(label(name.clone()).text_size(px(TEXT_SMALL)).number_of_lines(1_u32).flex_shrink(1.).min_w(px(0.)))
        .child(row_action(icons::CLOSE, format!("Remove {name}"), move || chat.apply(host, Event::RemovePill(ix))))
        .into_element()
}

/// The panel without its header, for a sheet that has its own.
pub fn body<H: Host>(host: H, chat: Chat) -> AnyElement {
    chat_body(host, chat, None::<fn()>)
}

fn chat_body<H: Host, C: Fn() + Copy + 'static>(host: H, chat: Chat, close: Option<C>) -> AnyElement {
    let notes_open = signal(false);
    let tabs_open = signal(false);
    let chats_open = signal(false);
    let model_open = signal(false);
    let model_rev = signal(0u64);
    let close_button = move || close.map(|close| row_action(icons::CLOSE, "Close Assistant", close));
    dynamic(move || {
        untracked(|| chat.open(host));
        let Some(chat_view) = chat.view.get() else {
            let here = host.origin().is_some_and(allowed);
            let top = view().flex_row().items_center().gap_2().child(label("Assistant").font_weight(FontWeight::SEMIBOLD).flex_1()).children(close_button());
            return Some(view().gap_2().child(top).child(empty(icons::SPARKLES, if here { OFF } else { NOT_HERE }, None)));
        };
        let (can_send, can_cancel) = (chat_view.can_send, chat_view.can_cancel);
        let last = chat_view.rows.len().saturating_sub(1);
        let asked = chat_view.rows.iter().any(|r| r.role == Role::You);
        let saved = chat.saved().and_then(|id| {
            let profile = host.origin()?.profile;
            host.with_store(|s| s.chat_titles(profile).ok()?.into_iter().find(|c| c.id == id).map(|c| c.title))
        });
        let title = heading(saved.as_deref(), chat_view.rows.iter().find(|r| r.role == Role::You).map(|r| r.text.as_str()));
        let model_chip = dynamic(move || {
            model_rev.get();
            let name = answers_with(host);
            Some(
                pressable(format!("Model: {name}"), move || model_open.update(|o| *o = !*o))
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .self_start()
                    .min_w(px(0.))
                    .flex_shrink(1.)
                    .h(px(26.))
                    .px_2()
                    .rounded_full()
                    .text_color(Theme::muted)
                    .child(text(name).text_size(px(TEXT_SMALL)).text_color(Theme::muted).number_of_lines(1_u32).flex_shrink(1.).min_w(px(0.)))
                    .child(icons::icon(if model_open.get() { icons::CHEVRON_DOWN } else { icons::CHEVRON_UP }, 10.)),
            )
        });
        let top = view()
            .flex_row()
            .items_center()
            .gap_1()
            .child(view().flex_1().min_w(px(0.)).child(label(title).font_weight(FontWeight::SEMIBOLD).number_of_lines(1_u32).px_1()))
            .child(row_action(icons::HISTORY, "Saved Chats", move || chats_open.update(|o| *o = !*o)).selected(move || chats_open.get()))
            .child(row_action(icons::SQUARE_PEN, "New Chat", move || {
                chats_open.set(false);
                chat.finish(host);
                chat.close();
            }))
            .children(close_button());
        let rows: Vec<AnyElement> = chat_view
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                if row.role == Role::You {
                    return view()
                        .self_end()
                        .max_w(relative(0.86))
                        .px_3()
                        .py_2()
                        .rounded(px(Theme::radius_group()))
                        .rounded_br(px(Theme::radius_small()))
                        .bg(Theme::selected())
                        .child(label(row.text.clone()).selectable(true))
                        .into_element();
                }
                let answer = view().gap_1().min_w(px(0.));
                let answer = if ix == last && row.streaming {
                    let base = row.text.clone();
                    answer
                        .child(dynamic(move || Some(answer_view(host, chat, ai::markdown::parse(&format!("{base}{}", chat.tail.get()))))))
                        .child(hint("Answering…").px_0())
                } else {
                    {
                        let blocks = ai::markdown::parse(&row.text);
                        let links = link_list(host, chat, &blocks);
                        answer.child(answer_view(host, chat, blocks)).children(links)
                    }
                };
                let copy = (!row.streaming && !row.text.is_empty() && row.text != browser_core::assistant::FORGOTTEN).then(|| row.text.clone());
                let latest = ix == last && asked && !can_cancel;
                answer
                    .children(row.by.clone().map(|by| hint(by).px_0()))
                    .when(row.interrupted, |b| b.opacity(0.6).child(hint("Interrupted; not sent again.").px_0()))
                    .when(copy.is_some() || latest, |b| b.child(bubble_actions(host, chat, copy, latest).ml(px(-8.))))
                    .into_element()
            })
            .collect();
        let conversation = if rows.is_empty() {
            view()
                .items_center()
                .gap_3()
                .px_2()
                .py_6()
                .child(view().text_color(Theme::faint).child(icons::icon(icons::SPARKLES, 24.)))
                .child(label("Ask about the page or anything. Type / for a command.").text_color(Theme::muted).text_center())
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .justify_center()
                        .gap_2()
                        .children(commands::BUILT_IN.iter().map(|(name, template)| pill(*name, move || chat.send(host, template)).into_element())),
                )
        } else {
            view().gap_4().children(rows)
        };
        let chips: Vec<AnyElement> = chat_view.chips.iter().enumerate().map(|(ix, name)| chip(host, chat, ix, name.clone())).collect();
        let has_chips = !chips.is_empty();
        let will_send = (!chat_view.will_send.is_empty()).then(|| hint(will_send_line(&chat_view.will_send, chat_view.will_send_bytes)));
        let submit = move |_: &str| chat.submit(host);
        let slash = dynamic(move || {
            let typed = chat.question.get();
            if !typed.trim_start().starts_with('/') || !can_send {
                return None;
            }
            let all = all_commands(host);
            let found = commands::slash_menu(&typed, &all);
            if found.is_empty() {
                return commands::slash(&typed, &all).is_none().then(|| group().child(empty(icons::COMMAND, "No command matches.", None)));
            }
            Some(group().children(found.into_iter().enumerate().map(|(row, ix)| {
                let (name, template) = all[ix].clone();
                open_row(row, format!("/{name}"), template, format!("Use {name}"), move || chat.question.set(format!("/{name} "))).into_element()
            })))
        });
        let ready = move || can_cancel || can_send && !chat.question.with(|q| q.trim().is_empty());
        let fill = move || if ready() { Theme::accent() } else { Theme::control() };
        let (glyph, name) = if can_cancel { (icons::SQUARE, "Stop") } else { (icons::ARROW_UP, "Send") };
        let send = pressable(name, move || {
            if can_cancel {
                chat.apply(host, Event::Cancel)
            } else if ready() {
                chat.submit(host)
            }
        })
        .w(px(28.))
        .h(px(28.))
        .flex_none()
        .rounded_full()
        .items_center()
        .justify_center()
        .bg(fill)
        .text_color(move || if ready() { Theme::on_fill(fill()) } else { Theme::faint() })
        .child(icons::icon(glyph, if can_cancel { 11. } else { 15. }));
        let composer = view()
            .gap_1()
            .p_1()
            .rounded(px(Theme::radius_tile()))
            .bg(Theme::raised())
            .border_1()
            .border_color(Theme::hairline())
            .when(has_chips, |v| v.child(view().flex_row().flex_wrap().gap_1().p_1().children(chips)))
            .child(
                signal_field("Ask anything, or type / for commands", chat.question)
                    .multiline(true)
                    .min_h(px(56.))
                    .on_submit(submit)
                    .bg(Theme::clear())
                    .border_color(Theme::clear()),
            )
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(row_action(icons::FILE_TEXT, "Attach Page", move || host.read_page()))
                    .child(row_action(icons::APP_WINDOW, "Attach Tab", move || tabs_open.update(|o| *o = !*o)).selected(move || tabs_open.get()))
                    .child(row_action(icons::PENCIL, "Attach Note", move || notes_open.update(|o| *o = !*o)).selected(move || notes_open.get()))
                    .child(view().flex_1())
                    .child(model_chip)
                    .child(send),
            );
        Some(
            view()
                .gap_2()
                .flex_1()
                .min_h(px(0.))
                .child(top)
                .child(dynamic(move || model_open.get().then(|| model_picker(host, model_rev))))
                .child(dynamic(move || chats_open.get().then(|| chat_list(host, chat, chats_open))))
                .child(scroll().flex_1().keyboard_taps(KeyboardTaps::Handled).child(conversation.py_2()))
                .children(chat_view.notice.clone().map(|n| hint(n).text_color(Theme::accent_text)))
                .child(dynamic(move || tabs_open.get().then(|| tab_picker(host, tabs_open))))
                .child(dynamic(move || notes_open.get().then(|| notes(host, chat, notes_open))))
                .child(slash)
                .children(will_send)
                .child(composer),
        )
    })
    .into_element()
}

fn notes<H: Host>(host: H, chat: Chat, open: Signal<bool>) -> View {
    let Some(origin) = host.origin() else { return view() };
    let saved = host.with_store(|s| s.notes(origin.profile).unwrap_or_default());
    if saved.is_empty() {
        return group().child(empty(icons::PENCIL, browser_core::notes::EMPTY, None));
    }
    group().children(saved.into_iter().enumerate().map(move |(ix, n)| {
        let name = browser_core::notes::title(&n);
        open_row(ix, name.clone(), "", format!("Add {name}"), move || {
            open.set(false);
            chat.attach_note(host, &n);
        })
        .into_element()
    }))
}

/// Clean Up with the assistant: sends `request`, checks the proposal against the tabs it named,
/// asks before anything moves and hands the checked groups to `apply`; `failed` gets the reason.
/// `here` (`sidebar::grouping_here`) tells whether the space it was asked in is still shown with
/// the assistant allowed, before the confirmation and again after it.
pub fn group_tabs<H: Host>(
    host: H,
    request: cleanup::ai::Request,
    here: impl Fn() -> bool + Copy + 'static,
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
        false,
        cancel,
        move |_, text| {
            let mut out = kept.borrow_mut();
            out.push_str(&text);
            if out.len() > ai::grouping::MAX_OUTPUT_BYTES {
                stop.store(true, Ordering::Relaxed);
            }
        },
        move |end| {
            let proposal = end.and_then(|()| proposal(&output.borrow(), &sent));
            match proposal {
                Ok(groups) => confirm_groups(sent, groups, here, apply),
                Err(why) => failed(why),
            }
        },
    );
}

fn proposal(output: &str, sent: &HashSet<Id>) -> Result<Vec<(String, Vec<Id>)>, String> {
    let groups = ai::grouping::parse(output, sent).map_err(|e| format!("The assistant's answer was not a usable grouping: {e}"))?;
    Ok(groups.into_iter().map(|g| (g.name, g.tab_ids)).collect())
}

fn confirm_groups(
    sent: HashSet<Id>,
    groups: Vec<(String, Vec<Id>)>,
    here: impl Fn() -> bool + Copy + 'static,
    apply: impl FnOnce(HashSet<Id>, Vec<(String, Vec<Id>)>) + 'static,
) {
    if !here() {
        return;
    }
    let groups: Vec<(String, Vec<Id>)> = groups.into_iter().map(|(name, ids)| (name.chars().filter(|c| !store::is_invisible(*c)).collect(), ids)).collect();
    let names = groups.iter().map(|(name, ids)| format!("{name} ({})", ids.len())).collect::<Vec<_>>().join(", ");
    let answer = nsh_base::alert_dialog()
        .title("Group tabs as the assistant suggests?")
        .description(format!("{names}. You can undo this."))
        .cancel("cancel", "Cancel")
        .action("group", "Group Tabs")
        .show();
    Owner::detached().run(|| {
        drop(spawn(async move {
            if answer.await.is_ok_and(|a| a.action.as_deref() == Some("group")) && here() {
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

    fn platform(secrets: &'static FakeSecrets, on_device: Option<(&'static dyn ai::LocalModel, &'static str)>) -> Platform {
        Platform { secrets, transport: || unreachable!(), keyring: "test", ollama_gap: None, on_device }
    }

    fn ask(backend: &Backend, secrets: &'static FakeSecrets) -> (Result<String, AiError>, Vec<(String, String)>) {
        let (result, _, asked) = ask_on(backend, &platform(secrets, None), false);
        (result, asked)
    }

    fn ask_on(backend: &Backend, platform: &Platform, on_device: bool) -> (Result<String, AiError>, String, Vec<(String, String)>) {
        let transport = Arc::new(FakeTransport::default());
        let req = ai::Request { system: "s".into(), page: None, question: "q".into() };
        let (mut out, mut name) = (String::new(), String::new());
        let result = answer(backend, platform, transport.clone(), on_device, &req, &AtomicBool::new(false), &mut |m| match m {
            Msg::Model(n) => name = n,
            Msg::Text(t) => out.push_str(&t),
            Msg::End(_) => {}
        });
        let asked = transport.0.lock().unwrap().clone();
        (result.map(|()| out), name, asked)
    }

    struct Device;

    impl ai::LocalModel for Device {
        fn availability(&self) -> ai::Availability {
            ai::Availability::Available
        }
        fn respond(&self, _: &ai::Request, _: &AtomicBool, on_text: &mut dyn FnMut(&str), _: ai::local::Checked) -> Result<(), AiError> {
            on_text("device");
            Ok(())
        }
    }

    #[test]
    fn commands_answer_on_the_device_and_name_it_while_questions_go_where_chosen() {
        let secrets: &'static FakeSecrets = Box::leak(Box::new(FakeSecrets(Mutex::new(Some("test-key".into())))));
        let with_device = platform(secrets, Some((&Device, "On-device")));
        let (out, name, asked) = ask_on(&Backend::Claude(settings::CLAUDE_MODELS[0].0.into()), &with_device, true);
        assert_eq!((out.unwrap().as_str(), name.as_str()), ("device", "On-device"));
        assert!(asked.is_empty(), "a command the device answers sends nothing");
        let (out, name, asked) = ask_on(&Backend::Claude(settings::CLAUDE_MODELS[0].0.into()), &with_device, false);
        assert_eq!((out.unwrap().as_str(), name.as_str(), asked.len()), ("Hi", "Claude API", 1));
        let (_, name, asked) = ask_on(&Backend::Ollama("llama3.2".into()), &with_device, true);
        assert_eq!(name, "Ollama on this computer (llama3.2)", "a local choice keeps its own model");
        assert!(asked.iter().all(|(url, _)| url.starts_with("http://127.0.0.1:11434/")));
    }

    #[test]
    fn claude_needs_a_stored_key_and_sends_it_only_to_anthropic() {
        let secrets: &'static FakeSecrets = Box::leak(Box::default());
        let claude = Backend::Claude("claude-haiku-5-5".into());
        let (result, asked) = ask(&claude, secrets);
        assert!(matches!(result, Err(AiError::Unavailable(ref m)) if m == NOT_CONNECTED));
        assert!(asked.is_empty(), "nothing is sent without a key");
        ai::secrets::store_token(secrets, &ApiToken::new("test-key".into())).unwrap();
        let (result, asked) = ask(&claude, secrets);
        assert_eq!(result.unwrap(), "Hi");
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].0, "https://api.anthropic.com/v1/messages");
        assert!(asked[0].1.starts_with("test-key ") && asked[0].1.contains("\"q\""));
        assert!(asked[0].1.contains("\"claude-haiku-5-5\""), "the chosen model answers");
    }

    #[test]
    fn ollama_answers_on_loopback_without_the_key() {
        let secrets: &'static FakeSecrets = Box::leak(Box::new(FakeSecrets(Mutex::new(Some("test-key".into())))));
        let (result, asked) = ask(&Backend::Ollama("llama3.2".into()), secrets);
        assert_eq!(result.unwrap(), "local");
        assert!(asked.iter().all(|(url, body)| url.starts_with("http://127.0.0.1:11434/") && !body.contains("test-key")));
        assert!(asked.iter().any(|(url, _)| url.ends_with("/api/chat")));
    }

    #[test]
    fn a_frame_takes_every_chunk_so_far_as_one() {
        let (tx, rx) = mpsc::channel();
        assert_eq!(drain(&rx), (None, String::new(), None));
        tx.send(Msg::Model("M".into())).unwrap();
        for t in ["a", "b", "c"] {
            tx.send(Msg::Text(t.into())).unwrap();
        }
        assert_eq!(drain(&rx), (Some("M".into()), "abc".into(), None));
        tx.send(Msg::Text("d".into())).unwrap();
        tx.send(Msg::End(Ok(()))).unwrap();
        assert_eq!(drain(&rx), (None, "d".into(), Some(Ok(()))));
        drop(tx);
        assert!(matches!(drain(&rx), (None, t, Some(Err(_))) if t.is_empty()), "a thread that died ends the request");
    }

    #[test]
    fn a_grouping_answer_is_checked_against_the_tabs_sent() {
        let sent: HashSet<Id> = [1, 2, 3].into();
        assert_eq!(proposal("{\"groups\":[{\"name\":\"News\",\"tab_ids\":[1,3]}]}", &sent).unwrap(), [("News".to_owned(), vec![1, 3])]);
        assert!(proposal("{\"groups\":[{\"name\":\"News\",\"tab_ids\":[1,9]}]}", &sent).is_err());
        assert!(proposal("not json", &sent).is_err());
    }

    thread_local! {
        static STORE: Store = Store::open_in_memory().unwrap();
        static SHOWN: std::cell::Cell<Option<Origin>> = const { std::cell::Cell::new(None) };
    }

    #[derive(Clone, Copy)]
    struct Window;

    impl Host for Window {
        fn platform(self) -> &'static Platform {
            unreachable!()
        }
        fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R {
            STORE.with(f)
        }
        fn origin(self) -> Option<Origin> {
            SHOWN.get()
        }
        fn level_of(self, _: &Url) -> ShieldLevel {
            ShieldLevel::Standard
        }
        fn level_in(self, _: Id, _: &Url) -> ShieldLevel {
            ShieldLevel::Standard
        }
        fn read_page(self) {}
        fn tabs(self) -> Vec<TabChoice> {
            Vec::new()
        }
        fn read_tab(self, _: TabKey) {}
        fn open(self, _: Url) {}
        fn changed(self) {}
    }

    #[test]
    fn the_chat_ends_once_the_window_shows_another_profile_or_a_space_it_may_not_see() {
        let profile = |name| STORE.with(|s| s.create_profile(name, name, false).and_then(|p| s.set_profile_ai(p.id, true).map(|()| p.id)).unwrap());
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let (work, home) = (profile("work"), profile("home"));
        let show = |profile, kind, level| SHOWN.set(Some(Origin { profile, kind, level }));
        let chat = Chat::new();
        let note = |chat: Chat| chat.attach(Window, SHOWN.get().unwrap(), Pill::Note(1, "n".into()));
        let attached = |chat: Chat| chat.view.get_untracked().is_some_and(|v| !v.chips.is_empty());

        show(work, ProfileKind::Normal, ShieldLevel::Standard);
        assert!(chat.open(Window));
        note(chat);
        chat.sync(Window);
        assert!(attached(chat), "the same profile keeps its conversation");

        show(work, ProfileKind::Private, ShieldLevel::Standard);
        chat.sync(Window);
        assert!(chat.view.get_untracked().is_none() && !chat.open(Window), "a private space ends the chat and opens none");

        show(work, ProfileKind::Normal, ShieldLevel::Standard);
        assert!(chat.open(Window) && !attached(chat), "back in the profile the chat starts empty");
        note(chat);
        show(home, ProfileKind::Normal, ShieldLevel::Standard);
        assert!(!chat.here(Window), "another profile's space ends the chat, so its links are refused");
        assert!(chat.open(Window) && !attached(chat), "the other profile gets a chat of its own");

        show(home, ProfileKind::Normal, ShieldLevel::Lockdown);
        assert!(!chat.here(Window), "a Lockdown page ends the chat");
        show(home, ProfileKind::Normal, ShieldLevel::Standard);
        assert!(chat.open(Window));
        STORE.with(|s| s.set_profile_ai(home, false).unwrap());
        forget_chats();
        assert!(!chat.here(Window), "AI turned off ends the chat");
        SHOWN.set(None);
        assert!(!chat.open(Window));
    }

    #[test]
    fn a_selection_command_sends_only_when_its_selection_was_attached() {
        let work = STORE.with(|s| s.create_profile("sel", "sel", false).and_then(|p| s.set_profile_ai(p.id, true).map(|()| p.id)).unwrap());
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let shown = Origin { profile: work, kind: ProfileKind::Normal, level: ShieldLevel::Standard };
        SHOWN.set(Some(shown));
        let chat = Chat::new();
        assert!(chat.open(Window));
        chat.attach(Window, shown, Pill::Note(1, "n".into()));
        let locked = Origin { level: ShieldLevel::Lockdown, ..shown };
        chat.ask_selection(Window, locked, Pill::Selection { url: "https://bank.test/".into(), text: "secret".into() }, Some("Explain"));
        let view = chat.view.get_untracked().unwrap();
        assert!(view.notice.is_some() && view.rows.is_empty() && !view.can_cancel, "a refused selection sends nothing, not even the note");
        SHOWN.set(None);
    }

    #[test]
    fn command_drafts_save_checked_and_unique() {
        let store = Store::open_in_memory().unwrap();
        let profile = store.create_profile("cmd", "cmd", false).unwrap().id;
        let draft = |was, name: &str, template: &str| CommandDraft { was, name: name.into(), template: template.into() };
        let custom = || store.ai_commands(profile).unwrap();
        assert!(draft(None, " ", "x").save(&store, profile, &[]).is_err());
        assert!(draft(None, "Tone", "  ").save(&store, profile, &[]).is_err());
        assert!(draft(None, "Tone", &"a".repeat(store::MAX_AI_TEMPLATE + 1)).save(&store, profile, &[]).is_err());
        assert!(draft(None, "summarize", "x").save(&store, profile, &[]).is_err(), "a built-in name is taken");
        assert_eq!(draft(None, " Tone ", "Rewrite {selection} kindly. {question}").save(&store, profile, &[]), Ok(()));
        assert_eq!(custom()[0].name, "Tone");
        assert!(draft(None, "TONE", "y").save(&store, profile, &custom()).is_err(), "a custom name is taken");
        let id = custom()[0].id;
        assert_eq!(draft(Some(id), "Tone", "Kinder: {selection}").save(&store, profile, &custom()), Ok(()), "an edit keeps its own name");
        assert_eq!(custom().iter().map(|c| (c.name.as_str(), c.template.as_str())).collect::<Vec<_>>(), [("Tone", "Kinder: {selection}")]);
    }

    #[test]
    fn a_chat_is_saved_when_its_answer_ends_and_restored_by_id() {
        let work = STORE.with(|s| s.create_profile("saved", "saved", false).and_then(|p| s.set_profile_ai(p.id, true).map(|()| p.id)).unwrap());
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let shown = Origin { profile: work, kind: ProfileKind::Normal, level: ShieldLevel::Standard };
        SHOWN.set(Some(shown));
        let chat = Chat::new();
        assert!(chat.open(Window));
        let effects = chat.session.with(|s| s.as_mut().unwrap().1.send("Hi?", ASK, 0, shown, true, |_| ShieldLevel::Standard));
        let Some(Effect::Start { id, .. }) = effects.into_iter().find(|e| matches!(e, Effect::Start { .. })) else { panic!("no request") };
        chat.apply(Window, Event::Chunk { id, text: "Hello".into() });
        chat.apply(Window, Event::Done { id });
        chat.save(Window);
        let saved = chat.saved().expect("saved once the answer ended");
        assert_eq!(STORE.with(|s| s.chat_titles(work).unwrap()).iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Hi?"]);
        assert!(chat.restore(Window, saved));
        assert_eq!(chat.view.get_untracked().unwrap().rows.len(), 2, "the saved turns come back");
        assert!(!chat.restore(Window, saved + 1000), "no such chat");
        SHOWN.set(Some(Origin { kind: ProfileKind::Private, ..shown }));
        assert!(!chat.restore(Window, saved), "never in a private space");
        SHOWN.set(Some(shown));
        chat.edit_last(Window);
        assert_eq!(chat.question.get_untracked(), "Hi?");
        assert!(chat.view.get_untracked().unwrap().rows.is_empty());
        assert!(!chat.restore(Window, saved), "a chat whose only question was taken back is deleted");
        SHOWN.set(None);
    }

    #[test]
    fn deleting_a_profiles_chats_forgets_only_its_chats() {
        let at = FORGOTTEN.get();
        forget_chats_of(7);
        assert!(forgotten(at, 7));
        assert!(!forgotten(at, 8), "another profile's chat stays");
        let later = FORGOTTEN.get();
        assert!(!forgotten(later, 7), "nothing since");
        forget_chats();
        forget_chats_of(9);
        assert!(forgotten(later, 8), "a forget of every chat came between");
    }

    #[test]
    fn the_heading_is_the_saved_title_then_the_first_question_then_new_chat() {
        assert_eq!(heading(Some("Trip"), Some("Where to?")), "Trip");
        assert_eq!(heading(None, Some("\n  Where to?\nmore")), "Where to?");
        assert_eq!(heading(Some(" "), None), "New Chat");
        assert_eq!(heading(None, Some("   ")), "New Chat");
    }

    #[test]
    fn what_a_send_carries_reads_on_one_line() {
        assert_eq!(
            will_send_line(&[("Page 1: News".into(), 2048), ("Note".into(), 12)], 2060),
            "Will send with your question (3 KB): Page 1: News · 2 KB, Note · 12 bytes"
        );
    }

    #[test]
    fn sizes_read_as_bytes_or_kilobytes() {
        assert_eq!(size(512), "512 bytes");
        assert_eq!(size(1025), "2 KB");
    }
}
