use browser_core::assistant::settings::{self as ai_settings, Provider};
use gpui::{AnyElement, AppContext, Context, Entity, Focusable, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Subscription, Window, div};
use shield::ShieldLevel;
use ui::controls::{button, field, group, note, row, section, segment, segmented, switch_row, tab_stop, text_block, wide_row};
use ui::text::bounded_title;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::Theme;

use super::{SettingsWindow, Snapshot, label, value};

pub(super) const SENT: &str = "Nothing is sent until you press Send in the assistant. A send carries your question, the earlier turns of that chat and the pills shown above the question field, and nothing else: no other tabs, history, cookies or passwords. Addresses go without their query and fragment. Private spaces, burner tabs and Lockdown pages are never offered.";

pub(super) struct Pane {
    token: Entity<TextInput>,
    model: Entity<TextInput>,
    /// Whether the secret store holds a token; `None` until the pane first asks.
    connected: Option<bool>,
    message: Option<String>,
    _subscriptions: [Subscription; 2],
}

impl Pane {
    pub(super) fn new(cx: &mut Context<SettingsWindow>) -> Pane {
        let token = cx.new(|cx| TextInput::new("Claude API key", cx).masked());
        let model = cx.new(|cx| TextInput::new(ai_settings::DEFAULT_OLLAMA_MODEL, cx));
        let subscriptions = [
            cx.subscribe(&token, |this: &mut SettingsWindow, _, event, cx| {
                if matches!(event, TextInputEvent::Submit) {
                    this.connect(cx);
                }
            }),
            cx.subscribe(&model, |this: &mut SettingsWindow, _, event, cx| {
                if matches!(event, TextInputEvent::Submit) {
                    this.save_model(cx);
                }
            }),
        ];
        Pane { token, model, connected: None, message: None, _subscriptions: subscriptions }
    }
}

impl SettingsWindow {
    /// Stores the key in the platform's secret store only, never in settings, the database or a log.
    fn connect(&mut self, cx: &mut Context<Self>) {
        let key = self.ai.token.read(cx).text().trim().to_owned();
        if key.is_empty() {
            return;
        }
        self.ai.token.update(cx, |t, cx| t.set_text("", cx));
        let stored = ai::secrets::store_token(&ai::secrets::Keyring, &ai::ApiToken::new(key));
        self.ai.message = stored.as_ref().err().map(|e| format!("Could not store the key: {e}."));
        self.ai.connected = Some(stored.is_ok());
        cx.notify();
    }

    fn disconnect(&mut self, cx: &mut Context<Self>) {
        let deleted = ai::secrets::delete_token(&ai::secrets::Keyring);
        self.ai.message = deleted.as_ref().err().map(|e| format!("Could not delete the key: {e}."));
        self.ai.connected = Some(deleted.is_err());
        cx.notify();
    }

    fn save_model(&mut self, cx: &mut Context<Self>) {
        let model = self.ai.model.read(cx).text().trim().to_owned();
        let mut saved = false;
        self.with_browser(cx, |b, _, _| saved = ai_settings::set_ollama_model(&b.core.disk, &model));
        self.ai.message = (!saved).then(|| "Model names use letters, digits and . _ - : / only, at most 128 characters.".to_owned());
        cx.notify();
    }

    pub(super) fn ai(&mut self, s: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(profile) = s.profile.clone() else { return note("No profile is open.").into_any_element() };
        if s.private || s.burner || profile.shield_level == ShieldLevel::Lockdown {
            return div()
                .child(section("Assistant"))
                .child(note("The assistant is not available in private spaces, burner tabs or Lockdown profiles."))
                .into_any_element();
        }
        let provider = s.ai_provider;
        let on = profile.ai_enabled;
        let id = profile.id;
        let mut page = div()
            .child(section("Assistant"))
            .child(
                group().child(
                    switch_row(
                        "ai-enabled",
                        true,
                        text_block(format!("Assistant for {}", bounded_title(&profile.name)), "Each profile has its own switch; it starts off."),
                        on,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.set_profile_ai(id, !on, cx)))),
                ),
            )
            .child(note(SENT));
        if !on {
            return page.into_any_element();
        }
        page = page.child(section("Provider")).child(group().child(wide_row(
            true,
            text_block("Where questions go", "Only after you press Send."),
            segmented().children(Provider::ALL.into_iter().enumerate().map(|(ix, choice)| {
                tab_stop(segment(("ai-provider", ix), choice.name(), provider == choice)).on_click(cx.listener(move |this, _, _, cx| {
                    this.with_browser(cx, |b, _, _| {
                        ai_settings::set_provider(&b.core.disk, choice);
                    });
                    cx.notify();
                }))
            })),
        )));
        page = match provider {
            Provider::ClaudeApi => {
                let connected = *self.ai.connected.get_or_insert_with(|| ai::secrets::load_token(&ai::secrets::Keyring).ok().flatten().is_some());
                let account = if connected {
                    row(true)
                        .child(label("Claude API key"))
                        .child(value("Connected"))
                        .child(tab_stop(button("ai-disconnect", "Disconnect", false)).on_click(cx.listener(|this, _, _, cx| this.disconnect(cx))))
                } else {
                    let focused = self.ai.token.focus_handle(cx).is_focused(window);
                    row(true)
                        .child(div().flex_1().child(field(focused).child(self.ai.token.clone())))
                        .child(tab_stop(button("ai-connect", "Connect", true)).on_click(cx.listener(|this, _, _, cx| this.connect(cx))))
                };
                page.child(section("Claude API"))
                    .child(group().child(account))
                    .child(note(if cfg!(windows) {
                        "The key is kept in Windows Credential Manager, never in Tarn's settings or database. Questions go to api.anthropic.com over HTTPS. Disconnect deletes the key."
                    } else {
                        "The key is kept in the Keychain, never in Tarn's settings or database. Questions go to api.anthropic.com over HTTPS. Disconnect deletes the key."
                    }))
            }
            Provider::Ollama => {
                let focused = self.ai.model.focus_handle(cx).is_focused(window);
                let current = &s.ollama_model;
                page.child(section("Ollama"))
                    .child(
                        group().child(
                            row(true)
                                .child(label("Model"))
                                .child(div().w(gpui::px(220.)).child(field(focused).child(self.ai.model.clone())))
                                .child(tab_stop(button("ai-model", "Save", false)).on_click(cx.listener(|this, _, _, cx| this.save_model(cx)))),
                        ),
                    )
                    .child(note(format!("Using {current}. Tarn talks to Ollama at 127.0.0.1:11434 only; nothing leaves this computer.")))
            }
        };
        page.children(self.ai.message.clone().map(|m| note(m).text_color(Theme::warning()))).into_any_element()
    }
}
