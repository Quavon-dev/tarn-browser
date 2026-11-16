use nsh_base::input;
use nsh_core::prelude::*;

use nsh_base::ButtonElement;
use tarn_nsh_shared::controls::{secure_button, secure_surface};
use tarn_nsh_shared::theme::Secure;

use super::Ui;

use crate::events::Answer;
use crate::shell::PromptRequest;
use crate::theme::{TEXT_HEADING, TEXT_TITLE, TOUCH_TARGET, Theme};

fn label(content: impl IntoReactive<SharedString>) -> Text {
    super::label(content).text_color(Secure::text)
}

fn caption(content: impl IntoReactive<SharedString>) -> Text {
    super::caption(content).text_color(Secure::muted)
}

fn heading(content: &'static str) -> Text {
    text(content).text_size(px(TEXT_HEADING)).font_weight(FontWeight::BOLD).text_color(Secure::text)
}

fn accent_pill(name: &str, press: impl Fn() + 'static) -> ButtonElement {
    secure_button(name, press, true)
}

fn pill(name: &str, press: impl Fn() + 'static) -> ButtonElement {
    secure_button(name, press, false)
}

fn answer(ui: Ui, accept: bool, remember: bool, text: Option<String>, user: Option<String>, password: Option<String>) {
    ui.act(|s| s.answer(Answer { accept, text, user, password, remember }));
}

fn field(value: Signal<String>, label: &'static str, masked: bool) -> impl IntoElement {
    input()
        .value(move || value.get())
        .on_change(move |v: &str| value.set(v.to_owned()))
        .masked(masked)
        .auto_capitalize(AutoCapitalize::None)
        .placeholder(label)
        .label(label)
        .test_id(label)
        .h(px(TOUCH_TARGET))
        .px_3()
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised)
        .border_1()
        .border_color(Theme::hairline)
        .placeholder_color(Theme::faint)
        .text_size(px(TEXT_TITLE))
        .text_color(Theme::text)
}

fn title(prompt: &PromptRequest) -> String {
    match prompt.kind.as_str() {
        "alert" | "confirm" | "text" => format!("{} says", prompt.origin),
        "credentials" => format!("{} asks you to sign in", prompt.origin),
        kind => format!("{} wants to use your {kind}", prompt.origin),
    }
}

fn body(ui: Ui, prompt: PromptRequest) -> AnyElement {
    let buttons = view().flex_row().flex_wrap().gap_2();
    match prompt.kind.as_str() {
        "alert" => buttons.child(accent_pill("OK", move || answer(ui, true, false, None, None, None))).into_element(),
        "confirm" => buttons
            .child(accent_pill("OK", move || answer(ui, true, false, None, None, None)))
            .child(pill("Cancel", move || answer(ui, false, false, None, None, None)))
            .into_element(),
        "text" => {
            let value = signal(prompt.default.clone().unwrap_or_default());
            view()
                .gap_2()
                .child(field(value, "Answer", false))
                .child(
                    buttons
                        .child(accent_pill("OK", move || answer(ui, true, false, Some(value.get_untracked()), None, None)))
                        .child(pill("Cancel", move || answer(ui, false, false, None, None, None))),
                )
                .into_element()
        }
        "credentials" => {
            let (user, password) = (signal(String::new()), signal(String::new()));
            view()
                .gap_2()
                .child(prompt.realm.clone().map(|r| caption(format!("Realm: {r}"))))
                .child(prompt.insecure.then(|| caption("This connection is not secure: the password would be sent unencrypted.").text_color(Secure::warning)))
                .child(field(user, "User name", false))
                .child(field(password, "Password", true))
                .child(
                    buttons
                        .child(accent_pill("Sign In", move || answer(ui, true, false, None, Some(user.get_untracked()), Some(password.get_untracked()))))
                        .child(pill("Cancel", move || answer(ui, false, false, None, None, None))),
                )
                .into_element()
        }
        _ => buttons
            .child(accent_pill("Allow Once", move || answer(ui, true, false, None, None, None)))
            .child(pill("Deny", move || answer(ui, false, false, None, None, None)))
            .child(prompt.rememberable().then(|| pill("Always Allow", move || answer(ui, true, true, None, None, None))))
            .into_element(),
    }
}

fn card() -> View {
    secure_surface(Theme::radius_panel()).flex_col().gap_3().p_4().m_3().shadow(Theme::elevation(4., 14.))
}

pub fn prompt_panel(ui: Ui) -> impl IntoElement {
    let external = signal(None);
    let first = signal(None);
    effect(move || external.set(ui.read(|s| s.external.clone())));
    effect(move || first.set(ui.read(|s| s.active_prompt().cloned())));
    dynamic(move || {
        if let Some((_, url)) = external.get() {
            let from = ui.read(|s| s.external_origin.clone());
            let asked = if from.is_empty() { "This address opens another app:".to_owned() } else { format!("{from} wants to open:") };
            return Some(
                card()
                    .child(heading("Open in another app?"))
                    .child(caption(asked))
                    .child(label(url))
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(accent_pill("Open", move || ui.act(|s| s.confirm_external(true))))
                            .child(pill("Cancel", move || ui.act(|s| s.confirm_external(false)))),
                    )
                    .into_element(),
            );
        }
        let Some(prompt) = first.get() else { return super::extensions::ask(ui).map(IntoElement::into_element) };
        let prompt: PromptRequest = prompt;
        Some(
            card()
                .child(label(title(&prompt)).font_weight(FontWeight::SEMIBOLD).text_color(Secure::text))
                .child(prompt.message.clone().map(|m| label(m).text_color(Secure::muted)))
                .child(body(ui, prompt))
                .into_element(),
        )
    })
}
