use browser_core::assistant::commands::BUILT_IN;
use browser_core::assistant::settings::{self as ai_settings, CLAUDE_MODELS, Provider};
use gpui::{AnyElement, AppContext, Context, Entity, Focusable, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Subscription, Window, div, px};
use shield::ShieldLevel;
use store::{AiCommand, Id};
use ui::controls::{
    button, empty_state, field, group, icon_text_button, leading_icon, note, row, row_action, section, segment, segmented, switch_row, tab_stop, text_block,
    text_lines, wide_row,
};
use ui::icon::Icon;
use ui::text::bounded_title;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{TEXT_SMALL, Theme};

use crate::prefs::Pref;

use super::{SettingsWindow, Snapshot, editor_actions, editor_row, label, list_heading, value, warning};

pub(super) struct Pane {
    token: Entity<TextInput>,
    model: Entity<TextInput>,
    /// Whether the secret store holds a token; `None` until a background read answers.
    connected: Option<bool>,
    message: Option<String>,
    command_name: Entity<TextInput>,
    command_template: Entity<TextInput>,
    editing: Option<Id>,
    /// The profile the command being edited belongs to.
    editing_in: Option<Id>,
    /// The editor for a new command is open.
    adding: bool,
    command_error: Option<String>,
    chats_error: Option<String>,
    _subscriptions: [Subscription; 4],
}

fn placeholders() -> String {
    format!(
        "In a command's text, {{page}} stands for the page, {{selection}} for the selected text and {{question}} for what you type after it. A selection's menu lists the first {} commands.",
        engine::view_policy::MAX_MENU_COMMANDS
    )
}

impl Pane {
    pub(super) fn new(cx: &mut Context<SettingsWindow>) -> Pane {
        let token = cx.new(|cx| TextInput::new("Claude API key", cx).masked());
        let model = cx.new(|cx| TextInput::new(ai_settings::DEFAULT_OLLAMA_MODEL, cx));
        let command_name = cx.new(|cx| TextInput::new("Name", cx));
        let command_template = cx.new(|cx| TextInput::new("Rewrite {selection} as a haiku. {question}", cx));
        let submit = |this: &mut SettingsWindow, _: Entity<TextInput>, event: &TextInputEvent, cx: &mut Context<SettingsWindow>| match event {
            TextInputEvent::Submit => this.save_command(cx),
            TextInputEvent::Cancel => {
                this.clear_command_editor(cx);
                cx.notify();
            }
            _ => {}
        };
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
            cx.subscribe(&command_name, submit),
            cx.subscribe(&command_template, submit),
        ];
        cx.spawn(async move |this, cx| {
            let found = cx.background_spawn(async { ai::secrets::load_token(&ai::secrets::Keyring).ok().flatten().is_some() }).await;
            this.update(cx, |this, cx| {
                this.ai.connected.get_or_insert(found);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Pane {
            token,
            model,
            connected: None,
            message: None,
            command_name,
            command_template,
            editing: None,
            editing_in: None,
            adding: false,
            command_error: None,
            chats_error: None,
            _subscriptions: subscriptions,
        }
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
        let token = ai::ApiToken::new(key);
        cx.spawn(async move |this, cx| {
            let stored = cx.background_spawn(async move { ai::secrets::store_token(&ai::secrets::Keyring, &token).map_err(|e| e.to_string()) }).await;
            this.update(cx, |this, cx| {
                this.ai.message = stored.as_ref().err().map(|e| format!("Could not store the key: {e}."));
                this.ai.connected = Some(stored.is_ok());
                cx.notify();
            })
            .ok();
        })
        .detach();
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

    fn save_command(&mut self, cx: &mut Context<Self>) {
        let Some(profile) = self.saved_profile(cx) else { return };
        if self.ai.editing_in.is_some_and(|p| p != profile) {
            self.ai.command_error = Some("This command belongs to another profile. Open Settings from one of its spaces to save it.".to_owned());
            return cx.notify();
        }
        let pane = &self.ai;
        let (name, template) = (pane.command_name.read(cx).text().trim().to_owned(), pane.command_template.read(cx).text().to_owned());
        let core = crate::shared::core(cx);
        let customs = core.disk.ai_commands(profile).unwrap_or_default();
        if let Err(problem) = browser_core::assistant::commands::check(&name, &template, &customs, pane.editing) {
            self.ai.command_error = Some(problem);
            return cx.notify();
        }
        let saved = match pane.editing {
            Some(id) => core.disk.update_ai_command(profile, id, &name, &template),
            None => core.disk.create_ai_command(profile, &name, &template).map(drop),
        };
        match saved {
            Ok(()) => {
                self.clear_command_editor(cx);
                commands_changed(cx);
            }
            Err(e) => self.ai.command_error = Some(e.to_string()),
        }
        cx.notify();
    }

    fn clear_command_editor(&mut self, cx: &mut Context<Self>) {
        self.ai.editing = None;
        self.ai.editing_in = None;
        self.ai.adding = false;
        self.ai.command_error = None;
        self.ai.command_name.update(cx, |i, cx| i.set_text("", cx));
        self.ai.command_template.update(cx, |i, cx| i.set_text("", cx));
    }

    fn edit_command(&mut self, command: AiCommand, window: &mut Window, cx: &mut Context<Self>) {
        self.ai.command_name.update(cx, |i, cx| i.set_text(command.name.clone(), cx));
        self.ai.command_template.update(cx, |i, cx| i.set_text(command.template.clone(), cx));
        window.focus(&self.ai.command_template.focus_handle(cx));
        self.ai.editing = Some(command.id);
        self.ai.editing_in = Some(command.profile_id);
        self.ai.command_error = None;
        cx.notify();
    }

    fn delete_command(&mut self, profile: Id, id: Id, cx: &mut Context<Self>) {
        if crate::shared::core(cx).disk.delete_ai_command(profile, id).is_ok() {
            if self.ai.editing == Some(id) {
                self.clear_command_editor(cx);
            }
            commands_changed(cx);
        }
        cx.notify();
    }

    fn delete_chats(&mut self, profile: Id, window: &mut Window, cx: &mut Context<Self>) {
        let detail = "Every chat saved for this profile is deleted from this computer. This cannot be undone.";
        self.ask("Delete all chats?", Some(detail), &["Delete All Chats"], window, cx, move |this, _, cx| {
            match crate::shared::core(cx).disk.delete_chats(profile) {
                Ok(()) => {
                    this.ai.chats_error = None;
                    crate::shared::each(cx, |b, _, cx| b.forget_chats(profile, cx));
                }
                Err(e) => this.ai.chats_error = Some(format!("Could not delete the chats: {e}.")),
            }
            cx.notify();
        });
    }

    fn add_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_command_editor(cx);
        self.ai.adding = true;
        window.focus(&self.ai.command_name.focus_handle(cx));
        cx.notify();
    }

    fn commands(&self, s: &Snapshot, profile: Id, window: &mut Window, cx: &mut Context<Self>) -> gpui::Div {
        let tag = || div().flex_none().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child("Built in");
        let built_in = BUILT_IN.iter().enumerate().map(|(ix, (name, template))| row(ix == 0).child(text_lines(*name, *template)).child(tag()));
        let custom = s.ai_commands.iter().enumerate().map(|(ix, command)| {
            let (id, edit) = (command.id, command.clone());
            row(false)
                .child(text_lines(command.name.clone(), command.template.clone()))
                .child(
                    row_action(("command-edit", ix), Icon::Pencil, "Edit")
                        .on_click(cx.listener(move |this, _, window, cx| this.edit_command(edit.clone(), window, cx))),
                )
                .child(
                    row_action(("command-delete", ix), Icon::Trash, "Delete").on_click(cx.listener(move |this, _, _, cx| this.delete_command(profile, id, cx))),
                )
        });
        let pane = &self.ai;
        let editing = pane.editing.is_some();
        let open = editing || pane.adding;
        let add = (!open)
            .then(|| icon_text_button("command-add", Icon::Plus, "Add Command").on_click(cx.listener(|this, _, window, cx| this.add_command(window, cx))));
        let page = div().child(list_heading("Commands", add)).child(group().children(built_in).children(custom));
        if !open {
            return page;
        }
        let focused = |input: &Entity<TextInput>| input.focus_handle(cx).is_focused(window);
        let actions = editor_actions(
            "command-editor",
            editing,
            Box::new(cx.listener(|this, _, _, cx| {
                this.clear_command_editor(cx);
                cx.notify();
            })),
            Box::new(cx.listener(|this, _, _, cx| this.save_command(cx))),
        );
        page.child(section(if editing { "Edit command" } else { "New command" }))
            .child(group().child(editor_row(true, "Name", &pane.command_name, focused(&pane.command_name))).child(editor_row(
                false,
                "Text",
                &pane.command_template,
                focused(&pane.command_template),
            )))
            .child(note(placeholders()))
            .children(pane.command_error.clone().map(warning))
            .child(actions)
    }

    pub(super) fn ai(&mut self, s: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let agent = self.agent_control(s, cx);
        let Some(profile) = s.profile.clone() else {
            return div().pt_6().child(empty_state(Icon::Person, "No profile is open.")).child(agent).into_any_element();
        };
        if s.private || s.burner {
            return div().pt_6().child(empty_state(Icon::Lock, browser_core::assistant::NOT_HERE)).child(agent).into_any_element();
        }
        if profile.shield_level == ShieldLevel::Lockdown {
            return div()
                .pt_6()
                .child(empty_state(Icon::Lock, browser_core::assistant::NOT_HERE))
                .child(self.chats(s, profile.id, cx))
                .child(agent)
                .into_any_element();
        }
        let provider = s.ai_provider;
        let on = profile.ai_enabled;
        let id = profile.id;
        let mut page =
            div()
                .child(section("Assistant"))
                .child(
                    group().child(
                        switch_row(
                            "ai-enabled",
                            true,
                            div().flex_1().min_w_0().flex().items_center().gap_3().child(leading_icon(Icon::Sparkles)).child(text_block(
                                format!("Assistant for {}", bounded_title(&profile.name)),
                                "Each profile has its own switch; it starts off.",
                            )),
                            on,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.set_profile_ai(id, !on, cx)))),
                    ),
                )
                .child(note(browser_core::assistant::SENT));
        if !on {
            return page.child(self.chats(s, id, cx)).into_any_element();
        }
        let providers = wide_row(
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
        );
        page = page.child(section("Provider"));
        page = match provider {
            Provider::ClaudeApi => {
                let connected = self.ai.connected.unwrap_or(false);
                let account = if connected {
                    row(false)
                        .child(leading_icon(Icon::KeyRound))
                        .child(label("Claude API key"))
                        .child(value("Connected"))
                        .child(tab_stop(button("ai-disconnect", "Disconnect", false)).on_click(cx.listener(|this, _, _, cx| this.disconnect(cx))))
                } else {
                    let focused = self.ai.token.focus_handle(cx).is_focused(window);
                    row(false)
                        .child(leading_icon(Icon::KeyRound))
                        .child(div().flex_1().child(field(focused).py_0p5().child(self.ai.token.clone())))
                        .child(tab_stop(button("ai-connect", "Connect", true)).on_click(cx.listener(|this, _, _, cx| this.connect(cx))))
                };
                let models = wide_row(
                    false,
                    text_block("Model", "Used for questions sent to the Claude API."),
                    segmented().children(CLAUDE_MODELS.iter().enumerate().map(|(ix, (_, name))| {
                        tab_stop(segment(("ai-claude-model", ix), *name, ix == s.claude_model)).on_click(cx.listener(move |this, _, _, cx| {
                            this.with_browser(cx, |b, _, cx| {
                                ai_settings::set_claude_model(&b.core.disk, ix);
                                cx.notify();
                            });
                            cx.notify();
                        }))
                    })),
                );
                page.child(group().child(providers).child(account).child(models)).child(note(if cfg!(windows) {
                    "The key is kept in Windows Credential Manager, never in Tarn's settings or database. Questions go to api.anthropic.com over HTTPS. Disconnect deletes the key."
                } else {
                    "The key is kept in the Keychain, never in Tarn's settings or database. Questions go to api.anthropic.com over HTTPS. Disconnect deletes the key."
                }))
            }
            Provider::Ollama => {
                let focused = self.ai.model.focus_handle(cx).is_focused(window);
                let current = &s.ollama_model;
                page.child(
                    group().child(providers).child(
                        row(false)
                            .child(label("Model"))
                            .child(div().w(gpui::px(220.)).child(field(focused).py_0p5().child(self.ai.model.clone())))
                            .child(tab_stop(button("ai-model", "Save", false)).on_click(cx.listener(|this, _, _, cx| this.save_model(cx)))),
                    ),
                )
                .child(note(format!("Using {current}. Tarn talks to Ollama at 127.0.0.1:11434 only; nothing leaves this computer.")))
            }
        };
        page = page.children(self.ai.message.clone().map(warning));
        page.child(self.commands(s, id, window, cx)).child(self.chats(s, id, cx)).child(agent).into_any_element()
    }

    /// Agent control (S31), for every profile: the switch, how to connect an agent and what it did.
    fn agent_control(&self, s: &Snapshot, cx: &mut Context<Self>) -> gpui::Div {
        let on = s.prefs.get(Pref::AgentControl);
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "tarn".into());
        let mut out = div().child(section("Agent control")).child(group().child(self.pref_row(
            true,
            Pref::AgentControl,
            "Let AI agents control Tarn",
            browser_core::agent::DETAIL,
            on,
            cx,
        )));
        if on {
            out = out.child(note(browser_core::agent::setup(&exe)));
            let entries = crate::agent::log(cx);
            if !entries.is_empty() {
                let rows = entries.into_iter().take(20).enumerate().map(|(ix, e)| {
                    let site = e.site.map(|s| format!(" · {}", ui::text::bounded_origin(&s))).unwrap_or_default();
                    row(ix == 0).child(label(format!("{}{site}", e.tool))).child(value(e.outcome))
                });
                out = out.child(section("This session")).child(group().children(rows));
            }
        }
        out
    }

    /// The profile's saved chats, which can be deleted whether or not the assistant is on.
    fn chats(&self, s: &Snapshot, id: Id, cx: &mut Context<Self>) -> gpui::Div {
        let chats = match s.chats {
            0 => "No saved chats".to_owned(),
            1 => "1 saved chat".to_owned(),
            n => format!("{n} saved chats"),
        };
        let delete = (s.chats > 0).then(|| {
            tab_stop(button("ai-delete-chats", "Delete All Chats", false)).on_click(cx.listener(move |this, _, window, cx| this.delete_chats(id, window, cx)))
        });
        div()
            .child(section("Chats"))
            .child(group().child(row(true).child(leading_icon(Icon::History)).child(label(chats)).children(delete)))
            .child(note("Chats are kept on this computer for this profile, and reopened from the chat list in the assistant."))
            .children(self.ai.chats_error.clone().map(warning))
    }
}

/// Every window's selection menus list the commands again.
fn commands_changed(cx: &mut gpui::App) {
    crate::shared::each(cx, |b, _, cx| {
        b.sync_assistant_menus();
        cx.notify();
    });
}
