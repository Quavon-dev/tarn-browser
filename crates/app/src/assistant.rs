use std::collections::HashSet;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ai::local::Model;
use ai::markdown::{Block, Span};
use browser_core::assistant::links::{self, LinkAction};
use browser_core::assistant::settings::Provider as Choice;
use browser_core::assistant::{self as core, ChatView, Effect, Event, Origin, Pill, ProfileKind, Role, Session, commands};
use browser_core::tabs::TabKey;
use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, FontStyle, FontWeight, HighlightStyle, InteractiveElement, InteractiveText, IntoElement, ParentElement,
    PromptLevel, StatefulInteractiveElement, StrikethroughStyle, Styled, StyledText, Subscription, Task, UnderlineStyle, Window, actions, div,
    prelude::FluentBuilder, px,
};
use shield::ShieldLevel;
use ui::controls::{button, field, icon_button, note, tab_stop, tooltip};
use ui::icon::{Icon, icon};
use ui::motion::{FADE, SLIDE, progress};
use ui::text::OneLine;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{TEXT_SMALL, TEXT_TITLE, Theme};
use url::Url;

use crate::browser::{Browser, Msg};
use crate::settings::SettingsPane;

actions!(tarn, [ToggleAssistant, Ask, AskPage]);

pub(crate) const WIDTH: f32 = 340.;
/// Streamed text reaches the chat at most this often, about once a frame.
const FRAME: Duration = Duration::from_millis(16);

/// One answer from the provider chosen in Settings › AI, or the on-device model for a command
/// (`ai::local::answer`), naming the model that answers first; blocks, so it runs off the UI thread.
pub(crate) type Answer = Box<dyn FnOnce(&ai::Request, bool, &AtomicBool, &mut dyn FnMut(&str), &mut dyn FnMut(&str)) -> Result<(), ai::AiError> + Send>;

#[derive(Default)]
struct Stream {
    model: Option<String>,
    text: String,
    end: Option<Result<(), String>>,
}

struct Run {
    id: u64,
    cancel: Arc<AtomicBool>,
    stream: Arc<Mutex<Stream>>,
    _pump: Task<()>,
}

pub(crate) struct Assistant {
    open: bool,
    toggled: Option<Instant>,
    from: f32,
    session: Option<Session>,
    input: Entity<TextInput>,
    run: Option<Run>,
    /// A send found no connected provider; the column offers Settings › AI.
    connect: bool,
    /// The tab picker is open.
    picking: bool,
    _submit: Subscription,
}

impl Assistant {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Browser>) -> Self {
        let input = cx.new(|cx| TextInput::new("Ask about the pills above, or anything", cx));
        let submit = cx.subscribe_in(&input, window, |b: &mut Browser, _, event, window, cx| match event {
            TextInputEvent::Submit => b.send_question(cx),
            TextInputEvent::Cancel => b.toggle_assistant(false, window, cx),
            _ => {}
        });
        Assistant { open: false, toggled: None, from: 0., session: None, input, run: None, connect: false, picking: false, _submit: submit }
    }
}

pub(crate) fn listen(root: gpui::Div, cx: &mut Context<Browser>) -> gpui::Div {
    root.on_action(cx.listener(|b, _: &ToggleAssistant, w, cx| b.toggle_assistant(!b.assistant.open, w, cx)))
        .on_action(cx.listener(|b, _: &Ask, w, cx| b.toggle_assistant(true, w, cx)))
        .on_action(cx.listener(|b, _: &AskPage, w, cx| b.ask_page(w, cx)))
}

impl Browser {
    /// The profile's AI switch, for a normal profile only; off ends its chat in every window.
    pub(crate) fn set_profile_ai(&mut self, id: store::Id, on: bool, cx: &mut Context<Self>) {
        if self.store(false).set_profile_ai(id, on).is_err() {
            return;
        }
        if let Some(rt) = self.core.profiles.borrow_mut().get_mut(&(false, id)) {
            rt.profile.ai_enabled = on;
        }
        self.sync_session();
        self.sync_assistant_menus();
        crate::shared::others(cx, |b, _, cx| {
            b.sync_session();
            b.sync_assistant_menus();
            cx.notify();
        });
        cx.notify();
    }

    /// The current space's profile as the assistant sees it, at the profile's Shield level, and
    /// whether its AI switch is on.
    pub(crate) fn assistant_origin(&self) -> Option<(Origin, bool)> {
        let space = self.model.current_space()?;
        let rt = self.current_loaded()?;
        let kind = if self.model.burner_shown() {
            ProfileKind::Burner
        } else if space.private {
            ProfileKind::Private
        } else {
            ProfileKind::Normal
        };
        Some((Origin { profile: rt.profile.id, kind, level: rt.profile.shield_level }, rt.profile.ai_enabled))
    }

    /// Where a send made now comes from: the active page's effective Shield level, its per-site
    /// setting included.
    fn page_origin(&self) -> Option<(Origin, bool)> {
        let (mut origin, on) = self.assistant_origin()?;
        if let Some((key, url)) = self.model.active.and_then(|k| self.tab_url(k).map(|u| (k, u))) {
            origin.level = self.level_for_tab(key, &url);
        }
        Some((origin, on))
    }

    pub(crate) fn assistant_allowed(&self) -> bool {
        self.page_origin().is_some_and(|(o, on)| on && core::allowed(o))
    }

    /// Ends the chat of `profile` once the profile or its data is deleted.
    pub(crate) fn forget_assistant(&mut self, profile: store::Id) {
        if self.assistant.session.as_ref().is_some_and(|s| s.profile() == profile) {
            self.end_run();
            self.assistant.session = None;
        }
    }

    fn end_run(&mut self) {
        if let Some(run) = self.assistant.run.take() {
            run.cancel.store(true, Ordering::Relaxed);
        }
        self.assistant.connect = false;
    }

    /// A chat for the current profile; switching to another profile, a private space or a burner
    /// ends the one before.
    fn sync_session(&mut self) {
        let origin = self.assistant_origin();
        let keep = match (&self.assistant.session, origin) {
            (Some(s), Some((o, on))) => on && core::allowed(o) && s.profile() == o.profile,
            _ => false,
        };
        if keep {
            return;
        }
        self.end_run();
        self.assistant.session = origin.and_then(|(o, on)| Session::new(o, on));
    }

    pub(crate) fn toggle_assistant(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.assistant.open != open {
            self.assistant.from = self.assistant_shown();
            self.assistant.toggled = Some(Instant::now());
            self.layout_moving(if self.pref(crate::prefs::Pref::ReduceMotion) { Duration::ZERO } else { SLIDE });
        }
        self.assistant.open = open;
        if open {
            self.sync_session();
            if let Some(host) = &self.host {
                host.focus();
            }
            window.focus(&self.assistant.input.focus_handle(cx));
        } else {
            self.refocus = true;
        }
        cx.notify();
    }

    /// How far the column is shown, from 0 to 1, as it slides.
    pub(crate) fn assistant_shown(&self) -> f32 {
        let a = &self.assistant;
        let duration = if self.pref(crate::prefs::Pref::ReduceMotion) { FADE } else { SLIDE };
        let t = a.toggled.map_or(1., |at| progress(at, duration));
        browser_core::layout::slide_at(a.from, if a.open { 1. } else { 0. }, t)
    }

    /// Opens the chat with the active page as a pill. A page the assistant may not see is never
    /// read; the session says why it was not added.
    pub(crate) fn ask_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_assistant(true, window, cx);
        let Some(key) = self.model.active else { return };
        let (Some(profile), Some(url)) = (self.profile_of(key), self.tab_url(key).and_then(|u| Url::parse(&u).ok())) else { return };
        if !matches!(url.scheme(), "http" | "https") {
            return;
        }
        let kind = match self.assistant_origin() {
            Some((o, _)) => o.kind,
            None => return,
        };
        let origin = Origin { profile, kind, level: self.level_for_tab(key, url.as_str()) };
        let title = self.model.tab(key).map(|t| t.title.clone()).unwrap_or_default();
        let view = self.views.get(&key).cloned();
        match view {
            Some(view) if core::allowed(origin) => {
                let events = self.events.clone();
                let page = url.to_string();
                view.page_text(&url, core::MAX_PILL_BYTES, move |text| {
                    events.unbounded_send(Msg::AssistantPill(origin, Pill::Page { url: page.clone(), title: title.clone(), text })).ok();
                });
            }
            _ => self.add_pill(origin, Pill::Page { url: url.to_string(), title, text: String::new() }, cx),
        }
    }

    pub(crate) fn add_pill(&mut self, origin: Origin, pill: Pill, cx: &mut Context<Self>) {
        if let Some(session) = self.assistant.session.as_mut() {
            let effects = session.apply(Event::AddPill(origin, pill));
            self.run_effects(effects, None, cx);
        }
    }

    /// What `key`'s selection menu offers the assistant for its page now (`Model::assistant_menu`).
    pub(crate) fn sync_assistant_menu(&self, key: TabKey) {
        if let Some(view) = self.views.get(&key) {
            view.set_assistant_menu(self.tab_url(key).and_then(|url| self.model.assistant_menu(&self.core, &url)));
        }
    }

    fn sync_assistant_menus(&self) {
        self.views.keys().for_each(|key| self.sync_assistant_menu(*key));
    }

    /// Ask Assistant about Selection, or one of its commands, from `key`'s page menu: the selection
    /// is read in the page's isolated world only where the assistant may see the page now.
    pub(crate) fn ask_selection(&mut self, key: TabKey, command: Option<String>, cx: &mut Context<Self>) {
        let (Some(view), Some(url)) = (self.views.get(&key).cloned(), self.tab_url(key)) else { return };
        let origin = self.model.assistant_origin(&self.core, Some(&url)).filter(|o| self.profile_of(key) == Some(o.profile));
        let Some(origin) = origin.filter(|_| self.model.assistant_offered(&self.core, Some(&url))) else {
            return self.show_toast_as(browser_core::toast::Tone::Warning, "This page cannot be shared with the assistant.", None, cx);
        };
        let events = self.events.clone();
        view.selected_text(move |raw| {
            events.unbounded_send(Msg::AssistantSelection(origin, url.clone(), raw, command.clone())).ok();
        });
    }

    /// Opens the chat with the selection as a pill and, for a command, sends it with that
    /// command's template: one click, one send.
    pub(crate) fn selection_for_assistant(
        &mut self,
        origin: Origin,
        url: &str,
        raw: &str,
        command: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pill) = core::selection(url, raw) else {
            return self.show_toast_as(browser_core::toast::Tone::Info, "No text that can be shared is selected.", None, cx);
        };
        self.toggle_assistant(true, window, cx);
        self.add_pill(origin, pill, cx);
        let custom = self.core.disk.ai_commands(origin.profile).unwrap_or_default();
        if let Some(template) = command.and_then(|name| commands::for_selection(&custom, &name)) {
            self.send(String::new(), &template, cx);
        }
    }

    /// Adds another tab of the chat's space as a pill from its loaded page; a sleeping tab is
    /// never woken for it.
    fn add_tab(&mut self, key: TabKey, cx: &mut Context<Self>) {
        self.assistant.picking = false;
        let (Some(view), Some(url)) = (self.views.get(&key).cloned(), self.tab_url(key).and_then(|u| Url::parse(&u).ok())) else {
            return self.show_toast_as(browser_core::toast::Tone::Info, "That tab is asleep. Open it to add it.", None, cx);
        };
        let Some(origin) = self.model.assistant_origin(&self.core, Some(url.as_str())).filter(|_| self.model.assistant_offered(&self.core, Some(url.as_str())))
        else {
            return self.show_toast_as(browser_core::toast::Tone::Warning, "This tab cannot be shared with the assistant.", None, cx);
        };
        let title = self.model.tab(key).map(|t| t.title.clone()).unwrap_or_default();
        let events = self.events.clone();
        let page = url.to_string();
        view.page_text(&url, core::MAX_PILL_BYTES, move |text| {
            events.unbounded_send(Msg::AssistantPill(origin, Pill::Tab { url: page.clone(), title: title.clone(), text })).ok();
        });
        cx.notify();
    }

    fn send_question(&mut self, cx: &mut Context<Self>) {
        let question = self.assistant.input.read(cx).text().to_owned();
        if !question.trim().is_empty() {
            self.send(question, core::ASK, cx);
        }
    }

    fn send(&mut self, question: String, template: &str, cx: &mut Context<Self>) {
        self.sync_session();
        if self.assistant.session.as_ref().is_none_or(|s| !core::view(s).can_send) {
            return;
        }
        let Some((origin, on)) = self.page_origin() else { return };
        let Some(answer) = provider(&self.core.disk) else {
            self.assistant.connect = true;
            return cx.notify();
        };
        self.assistant.connect = false;
        let Some(session) = self.assistant.session.as_mut() else { return };
        let (shell, profile) = (&self.core, session.profile());
        let level_of = |url: &Url| shell.level(false, profile, Some(url)).unwrap_or(ShieldLevel::Lockdown);
        let effects = session.send(&question, template, browser_core::now(), origin, on, level_of);
        if template == core::ASK && effects.iter().any(|e| matches!(e, Effect::Start { .. })) {
            self.assistant.input.update(cx, |i, cx| i.set_text("", cx));
        }
        self.run_effects(effects, Some((answer, core::on_device_task(template))), cx);
    }

    fn cancel_answer(&mut self, cx: &mut Context<Self>) {
        if let Some(session) = self.assistant.session.as_mut() {
            let effects = session.apply(Event::Cancel);
            self.run_effects(effects, None, cx);
        }
    }

    fn remove_pill(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(session) = self.assistant.session.as_mut() {
            let effects = session.apply(Event::RemovePill(ix));
            self.run_effects(effects, None, cx);
        }
    }

    fn run_effects(&mut self, effects: Vec<Effect>, mut answer: Option<(Answer, bool)>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::Start { id, request } => {
                    let Some((answer, on_device)) = answer.take() else { continue };
                    let cancel = Arc::new(AtomicBool::new(false));
                    let stream = Arc::new(Mutex::new(Stream::default()));
                    let request = ai::Request { system: request.system(), page: None, question: request.user_message() };
                    let (thread_cancel, thread_stream) = (cancel.clone(), stream.clone());
                    std::thread::spawn(move || {
                        let named_stream = thread_stream.clone();
                        let mut named = |model: &str| {
                            if let Ok(mut s) = named_stream.lock() {
                                s.model = Some(model.to_owned());
                            }
                        };
                        let mut push = |text: &str| {
                            if let Ok(mut s) = thread_stream.lock() {
                                s.text.push_str(text);
                            }
                        };
                        let result = answer(&request, on_device, &thread_cancel, &mut named, &mut push).map_err(|e| e.to_string());
                        if let Ok(mut s) = thread_stream.lock() {
                            s.end = Some(result);
                        }
                    });
                    let pump = cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor().timer(FRAME).await;
                            if !matches!(this.update(cx, |b, cx| b.pump(id, cx)), Ok(true)) {
                                break;
                            }
                        }
                    });
                    self.assistant.run = Some(Run { id, cancel, stream, _pump: pump });
                }
                Effect::Stop { id } => {
                    if let Some(run) = self.assistant.run.take_if(|r| r.id == id) {
                        run.cancel.store(true, Ordering::Relaxed);
                    }
                }
                Effect::Append { .. } | Effect::Render => cx.notify(),
            }
        }
    }

    /// Hands what arrived since the last frame to the session; false once the answer has ended.
    fn pump(&mut self, id: u64, cx: &mut Context<Self>) -> bool {
        let Some(run) = self.assistant.run.as_ref().filter(|r| r.id == id) else { return false };
        let (model, text, end) = match run.stream.lock() {
            Ok(mut s) => (s.model.take(), std::mem::take(&mut s.text), s.end.take()),
            Err(_) => (None, String::new(), Some(Err("the answer stopped".to_owned()))),
        };
        let Some(session) = self.assistant.session.as_mut() else { return false };
        let mut effects = Vec::new();
        if let Some(model) = model {
            effects.extend(session.apply(Event::AnsweredBy { id, model }));
        }
        if !text.is_empty() {
            effects.extend(session.apply(Event::Chunk { id, text }));
        }
        let going = end.is_none();
        match end {
            Some(Ok(())) => effects.extend(session.apply(Event::Done { id })),
            Some(Err(message)) => effects.extend(session.apply(Event::Failed { id, message })),
            None => {}
        }
        if !going {
            self.assistant.run = None;
        }
        self.run_effects(effects, None, cx);
        going
    }

    fn chat_profile_shown(&self) -> bool {
        self.assistant.session.as_ref().zip(self.assistant_origin()).is_some_and(|(s, (o, _))| s.profile() == o.profile && core::allowed(o))
    }

    /// A link in an answer opens only as `assistant::links` allows, after a confirmation naming its
    /// host and whole address, as a new tab in the chat's own profile; a mail address asks like
    /// any link to another app.
    fn open_answer_link(&mut self, raw: &str, window: &mut Window, cx: &mut Context<Self>) {
        match links::classify(raw) {
            LinkAction::ConfirmOpen(url) if self.chat_profile_shown() => {
                let detail = format!("{}\n\n{url}", url.host_str().unwrap_or_default());
                let answer = window.prompt(PromptLevel::Info, "Open this link from the answer?", Some(&detail), &["Open", "Cancel"], cx);
                cx.spawn_in(window, async move |this, cx| {
                    if answer.await == Ok(0) {
                        this.update_in(cx, |b, window, cx| b.open_confirmed_link(&url, window, cx)).ok();
                    }
                })
                .detach();
            }
            LinkAction::ConfirmMailto(url) => {
                self.pending_external = Some(crate::browser::PendingExternal { tab: None, origin: None, url });
                self.layout_changed();
                cx.notify();
            }
            _ => self.show_toast_as(browser_core::toast::Tone::Warning, "Tarn does not open this link.", None, cx),
        }
    }

    /// Clean Up with the assistant: sends the id, host and title of the space's loose web tabs
    /// (`cleanup::ai::request`), checks the proposal against what was sent and sorts only after
    /// the person agrees to the groups it names.
    pub(crate) fn group_tabs_with_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((origin, true)) = self.assistant_origin() else { return };
        let Some((private, space_id)) = self.model.current_space().map(|s| (s.private, s.space.id)) else { return };
        let Ok(tabs) = self.store(private).tabs(space_id) else {
            return self.show_toast_as(browser_core::toast::Tone::Warning, "Could not read the tabs", None, cx);
        };
        let page = |t: &store::Tab| {
            let key = crate::browser::TabKey { private, id: t.id };
            let url = self.tab_url(key).unwrap_or_else(|| t.url.clone());
            let level = self.level_for_tab(key, &url);
            (url, level)
        };
        let Some(request) = browser_core::cleanup::ai::request(origin, &tabs, page) else {
            return self.show_toast_as(browser_core::toast::Tone::Info, "The assistant needs two or more loose web tabs to group", None, cx);
        };
        let Some(answer) = provider(&self.core.disk) else {
            self.toggle_assistant(true, window, cx);
            self.assistant.connect = true;
            return cx.notify();
        };
        self.show_toast_as(browser_core::toast::Tone::Info, "Asking the assistant to group the tabs…", None, cx);
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let req = ai::Request { system: request.system.to_owned(), page: None, question: request.prompt };
            let mut output = String::new();
            let result = answer(&req, false, &AtomicBool::new(false), &mut |_| {}, &mut |text| {
                if output.len() <= ai::grouping::MAX_OUTPUT_BYTES {
                    output.push_str(text);
                }
            })
            .map_err(|e| e.to_string())
            .and_then(|()| ai::grouping::parse(&output, &request.sent));
            tx.send(result.map(|groups| (request.sent, groups))).ok();
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(result) = rx.await {
                this.update_in(cx, |b, window, cx| b.confirm_grouping(private, space_id, origin.profile, result, window, cx)).ok();
            }
        })
        .detach();
    }

    fn confirm_grouping(
        &mut self,
        private: bool,
        space_id: store::Id,
        profile: store::Id,
        result: Result<(HashSet<store::Id>, Vec<ai::grouping::Group>), String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (sent, groups) = match result {
            Ok(proposal) => proposal,
            Err(e) => return self.show_toast_as(browser_core::toast::Tone::Warning, format!("The assistant could not group the tabs ({e})"), None, cx),
        };
        let here = self.model.current_space().map(|s| (s.private, s.space.id)) == Some((private, space_id));
        if !here || self.assistant_origin().is_none_or(|(o, on)| !on || o.profile != profile) {
            return;
        }
        let proposal: Vec<(String, Vec<store::Id>)> =
            groups.into_iter().map(|g| (g.name.chars().filter(|c| !store::is_invisible(*c)).collect(), g.tab_ids)).collect();
        let detail =
            proposal.iter().map(|(name, ids)| format!("{name}: {} {}", ids.len(), if ids.len() == 1 { "tab" } else { "tabs" })).collect::<Vec<_>>().join("\n");
        let answer = window.prompt(PromptLevel::Info, "Sort the tabs into these groups?", Some(&detail), &["Sort", "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |b, cx| {
                if b.model.current_space().map(|s| (s.private, s.space.id)) != Some((private, space_id)) {
                    return;
                }
                let store = b.store(private);
                if let (Ok(tabs), Ok(groups)) = (store.tabs(space_id), store.groups(space_id)) {
                    let sort = browser_core::cleanup::ai::sort_from(&tabs, &groups, &sent, &proposal);
                    b.apply_sort(private, space_id, &sort, 0, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_confirmed_link(&mut self, url: &Url, window: &mut Window, cx: &mut Context<Self>) {
        if self.chat_profile_shown() && engine::nav_policy::decide(url, engine::nav_policy::Source::Link) == engine::nav_policy::Decision::Allow {
            self.new_tab(url, window, cx);
        } else {
            self.show_toast_as(browser_core::toast::Tone::Warning, "Tarn does not open this link.", None, cx);
        }
    }
}

/// The provider chosen in Settings › AI, or `None` while the Claude API has no token. The token is
/// read from the platform's secret store only, at the moment of a send. Ollama answers only through
/// `ai::local::answer`, so a local choice never reaches the cloud; with the Claude API, a command
/// stays on this Mac where its on-device model can answer.
pub(crate) fn provider(store: &store::Store) -> Option<Answer> {
    let transport: Arc<dyn ai::Transport> = Arc::new(ai::UreqTransport);
    match browser_core::assistant::settings::provider(store) {
        Choice::ClaudeApi => {
            let api = ai::ClaudeApi::new(ai::secrets::load_token(&ai::secrets::Keyring).ok().flatten()?, None, transport);
            Some(Box::new(move |req, on_device, cancel, named, on_text| {
                ai::local::answer(Model::Cloud(&api, Choice::ClaudeApi.name()), on_device_model(), on_device, req, cancel, named, on_text)
            }))
        }
        Choice::Ollama => {
            let name = browser_core::assistant::settings::ollama_model(store);
            let label = format!("{} ({name})", Choice::Ollama.name());
            let model = ai::Ollama::new(name, transport);
            Some(Box::new(move |req, on_device, cancel, named, on_text| {
                ai::local::answer(Model::Local(&model, &label), None, on_device, req, cancel, named, on_text)
            }))
        }
    }
}

#[cfg(all(target_os = "macos", feature = "foundation-models"))]
fn on_device_model() -> Option<(&'static dyn ai::LocalModel, &'static str)> {
    Some((&engine::foundation_models::FoundationModel, browser_core::assistant::settings::ON_DEVICE))
}

#[cfg(not(all(target_os = "macos", feature = "foundation-models")))]
fn on_device_model() -> Option<(&'static dyn ai::LocalModel, &'static str)> {
    None
}

pub(crate) fn render(b: &mut Browser, window: &mut Window, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let shown = b.assistant_shown();
    if shown <= 0. {
        b.assistant.toggled = None;
        return None;
    }
    if shown < 1. {
        window.request_animation_frame();
    }
    let reduce = b.pref(crate::prefs::Pref::ReduceMotion);
    b.sync_session();
    let body = match b.assistant.session.as_ref() {
        Some(session) => chat(&core::view(session), b, window, cx),
        None => unavailable(b, cx),
    };
    let column = div()
        .id("assistant")
        .flex_none()
        .h_full()
        .w(px(WIDTH))
        .flex()
        .flex_col()
        .bg(Theme::panel())
        .border_l_1()
        .border_color(Theme::separator())
        // The window moves the focus on Tab only outside text fields; the column moves it from its
        // question field too, so the keyboard reaches its links and buttons.
        .on_key_down(|e: &gpui::KeyDownEvent, w, cx| {
            let m = e.keystroke.modifiers;
            if e.keystroke.key == "tab" && !(m.control || m.platform || m.alt || m.function) {
                if m.shift {
                    w.focus_prev()
                } else {
                    w.focus_next()
                }
                cx.stop_propagation();
            }
        })
        .child(
            div()
                .h(px(ui::theme::TOOLBAR_HEIGHT))
                .flex_none()
                .px_3()
                .flex()
                .items_center()
                .gap_2()
                .child(icon(Icon::Sparkles, ui::icon::ICON, Theme::muted()))
                .child(div().flex_1().text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).one_line("Assistant"))
                .child(
                    tab_stop(icon_button("assistant-close", Icon::Close, true))
                        .tooltip(tooltip("Close Assistant", Some("⇧Esc".into())))
                        .on_click(cx.listener(|b, _, w, cx| b.toggle_assistant(false, w, cx))),
                ),
        )
        .child(body);
    let outer = div().flex_none().h_full().overflow_hidden();
    Some(if reduce { outer.w(px(WIDTH)).opacity(shown).child(column).into_any_element() } else { outer.w(px(WIDTH * shown)).child(column).into_any_element() })
}

fn unavailable(b: &Browser, cx: &mut Context<Browser>) -> AnyElement {
    let (text, settings) = match b.assistant_origin() {
        Some((o, _)) if o.kind == ProfileKind::Private => ("The assistant is not available in private spaces.", false),
        Some((o, _)) if o.kind == ProfileKind::Burner => ("The assistant is not available in a burner tab.", false),
        Some((o, _)) if o.level == ShieldLevel::Lockdown => ("The assistant is not available in Lockdown profiles.", false),
        Some(_) => ("The assistant is off for this profile. Turn it on in Settings › AI; nothing is sent until you press Send.", true),
        None => ("No profile is open.", false),
    };
    div()
        .p_3()
        .flex()
        .flex_col()
        .gap_2()
        .child(note(text).pt_0())
        .when(settings, |d| d.child(open_settings_button("assistant-settings", cx)))
        .into_any_element()
}

fn open_settings_button(id: &'static str, cx: &mut Context<Browser>) -> gpui::Stateful<gpui::Div> {
    tab_stop(button(id, "Open Settings › AI", false)).on_click(cx.listener(|b, _, w, cx| b.open_settings_pane(SettingsPane::Ai, w, cx)))
}

fn chat(view: &ChatView, b: &Browser, window: &mut Window, cx: &mut Context<Browser>) -> AnyElement {
    let me = cx.entity().downgrade();
    let rows = view
        .rows
        .iter()
        .enumerate()
        .map(|(ix, row)| match row.role {
            Role::You => div()
                .ml_auto()
                .max_w(px(WIDTH - 48.))
                .px_3()
                .py_2()
                .rounded(px(Theme::radius_row()))
                .bg(Theme::accent_soft())
                .child(row.text.clone())
                .into_any_element(),
            Role::Assistant => {
                let mut answer = div().flex().flex_col().gap_2().children(markdown(&row.text, ix, &me));
                if row.streaming {
                    answer = answer.child(ui::controls::spinner(ui::icon::ICON_SMALL, Theme::muted()));
                } else {
                    answer = answer.children(link_list(&row.text, ix, cx));
                }
                answer.children(row.by.clone().map(|by| note(by).pt_0())).into_any_element()
            }
        })
        .collect::<Vec<_>>();
    let picker = b.assistant.picking.then(|| tab_picker(b, cx));
    let chips = view
        .chips
        .iter()
        .enumerate()
        .map(|(ix, chip)| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .pl_2()
                .pr_1()
                .h(px(24.))
                .max_w(px(WIDTH - 32.))
                .rounded(px(Theme::radius_small()))
                .bg(Theme::control())
                .text_size(px(TEXT_SMALL))
                .child(div().text_color(Theme::muted()).child(chip.kind.name()))
                .child(div().min_w_0().one_line(chip.label.clone()))
                .child(
                    tab_stop(icon_button(("assistant-remove", ix), Icon::Close, true))
                        .tooltip(tooltip(format!("Remove {}", chip.label), None))
                        .on_click(cx.listener(move |b, _, _, cx| b.remove_pill(ix, cx))),
                )
        })
        .collect::<Vec<_>>();
    let will_send = if view.will_send.is_empty() {
        "Send carries your question only.".to_owned()
    } else {
        let parts: Vec<String> = view.will_send.iter().map(|(label, n)| format!("{label} ({})", crate::downloads::bytes(*n as u64))).collect();
        format!("Send carries your question and {}: {} in all.", parts.join(", "), crate::downloads::bytes(view.will_send_bytes as u64))
    };
    let focused = b.assistant.input.focus_handle(cx).is_focused(window);
    let send = if view.can_cancel {
        tab_stop(button("assistant-cancel", "Cancel", false)).on_click(cx.listener(|b, _, _, cx| b.cancel_answer(cx)))
    } else {
        tab_stop(button("assistant-send", "Send", true)).on_click(cx.listener(|b, _, _, cx| b.send_question(cx)))
    };
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .id("assistant-rows")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_3()
                .py_2()
                .flex()
                .flex_col()
                .gap_3()
                .when(view.rows.is_empty(), |d| {
                    d.child(note("Ask a question, or add this page with the button below. Nothing leaves this computer until you press Send.").pt_0())
                })
                .children(rows),
        )
        .child(
            div()
                .flex_none()
                .p_3()
                .flex()
                .flex_col()
                .gap_2()
                .border_t_1()
                .border_color(Theme::separator())
                .children(view.notice.clone().map(|n| note(n).pt_0().text_color(Theme::warning())))
                .when(b.assistant.connect, |d| {
                    d.child(note("Connect the Claude API, or choose Ollama on this computer, in Settings › AI to send.").pt_0())
                        .child(open_settings_button("assistant-connect", cx))
                })
                .child(div().flex().flex_wrap().gap_1().children(chips))
                .child(note(will_send).pt_0())
                .children(picker)
                .child(field(focused).child(b.assistant.input.clone()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(tab_stop(button("assistant-add-page", "Add This Page", false)).on_click(cx.listener(|b, _, w, cx| b.ask_page(w, cx))))
                        .child(tab_stop(button("assistant-add-tab", "Add Tab…", b.assistant.picking)).on_click(cx.listener(|b, _, _, cx| {
                            b.assistant.picking = !b.assistant.picking;
                            cx.notify();
                        })))
                        .child(div().flex_1())
                        .child(send),
                ),
        )
        .into_any_element()
}

/// The current space's other tabs the assistant may read; a sleeping one is shown but cannot be
/// chosen, since adding it would load its page.
fn tab_picker(b: &Browser, cx: &Context<Browser>) -> AnyElement {
    let choices = b.model.assistant_tabs(&b.core, |key, t| b.tab_url(key).unwrap_or_else(|| t.url.clone()));
    let list = div().id("assistant-tabs").max_h(px(180.)).overflow_y_scroll().flex().flex_col().gap_1();
    if choices.is_empty() {
        return list.child(note("No other tab here can be added.").pt_0()).into_any_element();
    }
    list.children(choices.into_iter().enumerate().map(|(ix, choice)| {
        let row = div()
            .id(("assistant-tab", ix))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .h(px(26.))
            .rounded(px(Theme::radius_small()))
            .text_size(px(TEXT_SMALL))
            .child(div().flex_1().min_w_0().one_line(choice.title.clone()));
        if !choice.awake {
            return row
                .text_color(Theme::muted())
                .child(div().flex_none().child("Asleep"))
                .tooltip(tooltip("Open this tab to add it", None))
                .into_any_element();
        }
        let key = choice.key;
        tab_stop(row.hover(|s| s.bg(Theme::control())).cursor_pointer())
            .tooltip(tooltip(choice.url, None))
            .on_click(cx.listener(move |b, _, _, cx| b.add_tab(key, cx)))
            .into_any_element()
    }))
    .into_any_element()
}

/// The links of an answer again as a list of tab stops, so the keyboard reaches each one: Tab and
/// Shift-Tab move between them, Enter opens one through the same confirmation as a click.
fn link_list(text: &str, row: usize, cx: &Context<Browser>) -> Option<AnyElement> {
    let links: Vec<(String, String)> = ai::markdown::parse(text)
        .into_iter()
        .flat_map(|block| match block {
            Block::Heading { spans, .. } | Block::Paragraph(spans) | Block::ListItem { spans, .. } | Block::Quote(spans) => spans,
            Block::Code { .. } | Block::Rule => Vec::new(),
        })
        .filter_map(|span| span.link.map(|url| (links::shown(&span.text, &url), url)))
        .collect();
    (!links.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(note("Links").pt_0())
            .children(links.into_iter().enumerate().map(|(ix, (shown, url))| {
                tab_stop(
                    div().id(("assistant-link", row * 1_000 + ix)).text_size(px(TEXT_SMALL)).text_color(Theme::accent_text()).cursor_pointer().child(shown),
                )
                .on_click(cx.listener(move |b, _, w, cx| b.open_answer_link(&url, w, cx)))
            }))
            .into_any_element()
    })
}

/// An answer drawn natively: no remote images (they arrive as alt text), every link with its whole
/// address shown, opened only by a click through `open_answer_link`.
fn markdown(text: &str, row: usize, me: &gpui::WeakEntity<Browser>) -> Vec<AnyElement> {
    ai::markdown::parse(text)
        .into_iter()
        .enumerate()
        .map(|(ix, block)| {
            let id = ("assistant-text", row * 10_000 + ix);
            match block {
                Block::Heading { level, spans } => div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(level <= 2, |d| d.text_size(px(TEXT_TITLE)))
                    .child(spans_text(id, &spans, me))
                    .into_any_element(),
                Block::Paragraph(spans) => div().child(spans_text(id, &spans, me)).into_any_element(),
                Block::ListItem { depth, number, spans } => div()
                    .pl(px(12. * depth.saturating_sub(1) as f32))
                    .flex()
                    .gap_2()
                    .child(div().flex_none().text_color(Theme::muted()).child(number.map_or("•".to_owned(), |n| format!("{n}."))))
                    .child(div().flex_1().min_w_0().child(spans_text(id, &spans, me)))
                    .into_any_element(),
                Block::Quote(spans) => {
                    div().pl_2().border_l_2().border_color(Theme::separator()).text_color(Theme::muted()).child(spans_text(id, &spans, me)).into_any_element()
                }
                Block::Code { code, .. } => div()
                    .p_2()
                    .rounded(px(Theme::radius_small()))
                    .bg(Theme::control())
                    .font_family("Menlo")
                    .text_size(px(TEXT_SMALL))
                    .child(code.trim_end().to_owned())
                    .into_any_element(),
                Block::Rule => div().h(px(1.)).bg(Theme::separator()).into_any_element(),
            }
        })
        .collect()
}

fn spans_text(id: (&'static str, usize), spans: &[Span], me: &gpui::WeakEntity<Browser>) -> InteractiveText {
    let mut text = String::new();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    let mut targets: Vec<(Range<usize>, String)> = Vec::new();
    for span in spans {
        let start = text.len();
        let mut style = HighlightStyle::default();
        match &span.link {
            Some(url) => {
                text.push_str(&links::shown(&span.text, url));
                style.color = Some(Theme::accent_text().into());
                style.underline = Some(UnderlineStyle { thickness: px(1.), ..Default::default() });
                targets.push((start..text.len(), url.clone()));
            }
            None => text.push_str(&span.text),
        }
        if span.style.bold {
            style.font_weight = Some(FontWeight::BOLD);
        }
        if span.style.italic {
            style.font_style = Some(FontStyle::Italic);
        }
        if span.style.strike {
            style.strikethrough = Some(StrikethroughStyle { thickness: px(1.), ..Default::default() });
        }
        if span.style.code {
            style.background_color = Some(Theme::control().into());
        }
        if style != HighlightStyle::default() && start < text.len() {
            highlights.push((start..text.len(), style));
        }
    }
    let (ranges, urls): (Vec<_>, Vec<_>) = targets.into_iter().unzip();
    let me = me.clone();
    InteractiveText::new(id, StyledText::new(text).with_highlights(highlights)).on_click(ranges, move |ix, window, cx| {
        if let Some(url) = urls.get(ix) {
            me.update(cx, |b, cx| b.open_answer_link(url, window, cx)).ok();
        }
    })
}
