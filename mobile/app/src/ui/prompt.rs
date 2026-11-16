use nsh_base::Input;
use nsh_core::keymap::When;
use nsh_core::prelude::*;

use browser_core::prompts::meaning;
use nsh_base::ButtonElement;
use nsh_base::button;
use tarn_nsh_shared::controls::{glyph_tile, secure_button, secure_surface};
use tarn_nsh_shared::theme::Secure;

use super::{Screen, Ui};

use crate::events::Answer;
use crate::shell::PromptRequest;
use crate::theme::{SYSTEM_FONT, TEXT_TITLE, TOUCH_TARGET, Theme};

const TILE: f32 = 30.;
const RADIUS: f32 = 12.;

fn label(content: impl IntoReactive<SharedString>) -> Text {
    super::label(content).text_color(Secure::text)
}

fn caption(content: impl IntoReactive<SharedString>) -> Text {
    super::caption(content).text_color(Secure::muted)
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

/// A prompt's answer field, in the `Secure` colours no theme reaches.
fn field(value: Signal<String>, label: &'static str, masked: bool) -> Input {
    super::text_field(label, label)
        .bg(Secure::control)
        .border_color(Secure::hairline)
        .placeholder_color(Secure::faint)
        .text_color(Secure::text)
        .value(move || value.get())
        .on_change(move |v: &str| value.set(v.to_owned()))
        .masked(masked)
        .auto_capitalize(AutoCapitalize::None)
        .test_id(label)
}

fn ask(prompt: &PromptRequest) -> String {
    match prompt.kind.as_str() {
        "alert" | "confirm" | "text" => "says".to_owned(),
        "credentials" => "asks you to sign in".to_owned(),
        kind => format!("wants to use your {kind}"),
    }
}

fn ghost(name: &str, press: impl Fn() + 'static) -> ButtonElement {
    let name = name.to_owned();
    button()
        .accessibility_label(name.clone())
        .on_press(press)
        .min_h(px(TOUCH_TARGET))
        .px_3()
        .items_center()
        .justify_center()
        .rounded_full()
        .active(|s| s.bg(Secure::pressed()))
        .child(text(name).text_size(px(TEXT_TITLE)).font_family(SYSTEM_FONT).text_color(Secure::text))
}

fn tile(kind: &str) -> View {
    let name = browser_core::glyphs::glyph(kind);
    let source = tarn_nsh_shared::icons::named(name).unwrap_or(tarn_nsh_shared::icons::INFO);
    let warn = name == "triangle-alert";
    glyph_tile(source, TILE, Secure::control, move || if warn { Secure::warning() } else { Secure::accent() })
}

fn head(origin: &str, asked: String, kind: &str, meaning: Option<String>) -> View {
    view().flex_row().gap_3().child(tile(kind)).child(
        view()
            .flex_1()
            .gap_1()
            .child(view().flex_row().flex_wrap().gap_1().child(label(origin.to_owned()).font_weight(FontWeight::BOLD)).child(label(asked)))
            .child(meaning.map(|m| caption(m).number_of_lines(0))),
    )
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
                .child(field(value, "Answer", false).on_submit(move |v: &str| answer(ui, true, false, Some(v.to_owned()), None, None)))
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
                .child(
                    field(password, "Password", true).on_submit(move |v: &str| answer(ui, true, false, None, Some(user.get_untracked()), Some(v.to_owned()))),
                )
                .child(
                    buttons
                        .child(accent_pill("Sign In", move || answer(ui, true, false, None, Some(user.get_untracked()), Some(password.get_untracked()))))
                        .child(pill("Cancel", move || answer(ui, false, false, None, None, None))),
                )
                .into_element()
        }
        _ => {
            let remember = prompt.rememberable();
            buttons
                .justify_end()
                .child(ghost("Deny", move || answer(ui, false, false, None, None, None)))
                .child(secure_button("Allow Once", move || answer(ui, true, false, None, None, None), !remember))
                .child(remember.then(|| accent_pill("Always Allow", move || answer(ui, true, true, None, None, None))))
                .into_element()
        }
    }
}

/// Esc declines without remembering the answer; Enter, not held, accepts an alert or a confirm
/// only while neither a field nor the page has the keyboard, so typing cannot answer it.
fn keys(decline: impl Fn() + 'static, accept: Option<impl Fn() + 'static>) {
    on_key_with("escape", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| decline());
    if let Some(accept) = accept {
        on_key_with("enter", KeyOptions { when: When::NotEditing, ..KeyOptions::default() }, move |_| accept());
    }
}

fn card() -> View {
    secure_surface(RADIUS).flex_col().gap_3().p_3().m_3().shadow(Theme::elevation(4., 14.)).bind_motion(&tarn_nsh_shared::motion::pop_in())
}

pub fn prompt_panel(ui: Ui) -> impl IntoElement {
    let external = signal(None);
    let first = signal(None);
    effect(move || external.set(ui.read(|s| s.external.clone())));
    effect(move || first.set(ui.read(|s| s.active_prompt().cloned())));
    dynamic(move || {
        if let Some((_, url)) = external.get() {
            let from = ui.read(|s| s.external_origin.clone());
            if !ui.editing.get() && ui.screen.get() == Screen::Browse {
                keys(move || ui.act(|s| s.confirm_external(false)), None::<fn()>);
            }
            let asked = if from.is_empty() { "This address opens another app:".to_owned() } else { format!("{from} wants to open:") };
            return Some(
                card()
                    .child(head("Open in another app?", String::new(), "external", Some(asked)))
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
        let prompt: PromptRequest = first.get()?;
        let simple = matches!(prompt.kind.as_str(), "alert" | "confirm");
        // The address field and the sheets keep their own Esc and Enter.
        if !ui.editing.get() && ui.screen.get() == Screen::Browse {
            keys(move || answer(ui, false, false, None, None, None), simple.then_some(move || answer(ui, true, false, None, None, None)));
        }
        Some(
            card()
                .child(head(&prompt.origin, ask(&prompt), &prompt.kind, prompt.message.clone().or_else(|| meaning(&prompt.kind).map(str::to_owned))))
                .child(body(ui, prompt))
                .into_element(),
        )
    })
}
