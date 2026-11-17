use std::path::PathBuf;
use std::rc::Rc;

use browser_core::Core;
use browser_core::prompts::Prompt;
use futures_util::StreamExt;
use nsh_core::JsonValue;
use nsh_core::prelude::*;
use tarn_nsh_shared::controls::{secure_button, secure_surface};

use super::{Ui, WINDOWS, act, read};
use crate::controls::{body, card, pill, small};
use crate::data::Data;
use crate::extensions::{self, Note, popup};
use crate::theme::{Secure, Theme};
use store::{Id, Store};

const TICK: std::time::Duration = std::time::Duration::from_secs(5);
const POPUP_WIDTH: f32 = 360.;
const POPUP_HEIGHT: f32 = 480.;

pub(super) fn route(core: Rc<Core<Data>>) {
    let mut notes = extensions::init(core);
    drop(set_interval(TICK, extensions::tick));
    drop(spawn(async move {
        while let Some(note) = notes.next().await {
            let windows = WINDOWS.with_borrow(Vec::clone);
            if let Note::PageClosed(page) = note {
                windows.iter().filter(|ui| ui.ext_popup.get_untracked().is_some_and(|(_, _, p)| p == page)).for_each(|ui| ui.ext_popup.set(None));
                continue;
            }
            let mut handled = false;
            for ui in windows {
                act(ui, |b| handled = handled || b.on_extension_note(&note));
                if handled && note != Note::Changed {
                    break;
                }
            }
        }
    }));
}

fn badge_color(color: Option<[u8; 4]>) -> Rgba {
    color.map_or_else(Theme::accent, |[r, g, b, a]| Rgba { r: f32::from(r) / 255., g: f32::from(g) / 255., b: f32::from(b) / 255., a: f32::from(a) / 255. })
}

pub(super) fn buttons(ui: Ui) -> AnyElement {
    dynamic(move || {
        ui.rev.get();
        let buttons = untracked(|| read(ui, |b| b.extension_buttons())).unwrap_or_default();
        let row = view().flex_row().items_center().gap(px(2.));
        Some(row.children(buttons.into_iter().map(|(profile, button)| {
            let install = button.install_id.clone();
            let letter = button.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            let label = if button.state.title.is_empty() { button.name.clone() } else { button.state.title.clone() };
            let press = move || {
                let mut open = false;
                act(ui, |b| open = b.click_extension(profile, &install));
                let shown = ui.ext_popup.get_untracked().is_some_and(|(_, i, _)| i == install);
                show_popup(ui, (open && !shown).then(|| (profile, install.clone())));
            };
            let badge = (!button.state.badge_text.is_empty()).then(|| {
                view()
                    .absolute()
                    .bottom(px(-2.))
                    .right(px(-4.))
                    .px(px(3.))
                    .rounded(px(4.))
                    .bg(badge_color(button.state.badge_color))
                    .child(text(button.state.badge_text.clone()).text_size(px(9.)).text_color(Theme::text()))
            });
            crate::controls::pressable(label, press)
                .relative()
                .w(px(26.))
                .h(px(26.))
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_small()))
                .hover(|s| s.bg(Theme::hover()))
                .child(match &button.icon {
                    Some(path) => image(path.clone()).w(px(16.)).h(px(16.)).into_element(),
                    None => tarn_nsh_shared::controls::site_tile(None, &letter.to_lowercase(), 16.).into_element(),
                })
                .children(badge)
                .into_element()
        })))
    })
}

/// Closes the popup on screen and opens `next`'s, if any.
pub(super) fn show_popup(ui: Ui, next: Option<(Id, String)>) {
    if let Some((profile, install, page)) = ui.ext_popup.get_untracked() {
        extensions::popup_closed(profile, &install, page);
    }
    ui.ext_popup.set(next.and_then(|(profile, install)| Some((profile, install.clone(), extensions::open_popup(profile, &install)?))));
}

pub(super) fn popup_panel(ui: Ui) -> AnyElement {
    dynamic(move || {
        let (_, _, page) = ui.ext_popup.get()?;
        let close = move || show_popup(ui, None);
        let page = native(popup::NAME).prop("page", JsonValue::from(page)).w(px(POPUP_WIDTH)).h(px(POPUP_HEIGHT));
        Some(view().flex_row().justify_end().mb_2().child(
            card().flex_col().gap_2().p_2().shadow(Theme::elevation(4., 14.)).child(page).child(view().flex_row().justify_end().child(pill("Close", close))),
        ))
    })
}

pub(super) fn site_prompt(ui: Ui, prompt: &Prompt) -> View {
    let id = prompt.id;
    let name = prompt.message.clone().unwrap_or_default();
    let badge = prompt.realm.clone().unwrap_or_default();
    let reply = move |accept: bool| {
        let remember = ui.remember.get_untracked();
        ui.remember.set(false);
        act(ui, |b| b.answer_site_prompt(id, accept, remember));
    };
    let remember = ui.remember.get();
    let toggle = crate::controls::switch(remember, "Remember for this site", move || ui.remember.update(|r| *r = !*r));
    secure_surface(Theme::radius_panel())
        .flex_col()
        .gap_2()
        .px_4()
        .py_3()
        .mb_2()
        .shadow(Theme::elevation(4., 14.))
        .role(AccessibilityRole::Alert)
        .child(
            body(format!("The extension \u{201C}{name}\u{201D} ({badge}) wants to read and change {}", prompt.origin))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(Secure::text()),
        )
        .child(small("Without Remember, Allow lasts an hour or until the tab closes.").text_color(Secure::muted()))
        .child(toggle)
        .child(view().flex_row().items_center().gap_2().child(secure_button("Allow", move || reply(true), true)).child(secure_button(
            "Deny",
            move || reply(false),
            false,
        )))
}

#[derive(Clone, Copy)]
struct Installs(Ui);

impl tarn_nsh_shared::extensions::Host for Installs {
    type Transport = crate::fetch::Soup;
    fn transport() -> crate::fetch::Soup {
        crate::fetch::Soup::new()
    }
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R {
        f(&super::core().disk)
    }
    fn profile(self) -> Option<(Id, String)> {
        read(self.0, |b| {
            let space = b.model.current_space().filter(|s| !s.private && !b.model.burner_shown())?;
            let id = space.space.profile_id;
            Some((id, b.core.profiles.borrow().get(&(false, id))?.own_store.clone()))
        })
        .flatten()
    }
    fn root(self) -> PathBuf {
        crate::data::root().join("extensions")
    }
    fn catalog_dir(self) -> PathBuf {
        crate::data::root().join("catalog-extensions")
    }
    fn new_id(self) -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }
    fn ui_locales(self) -> Vec<String> {
        extensions::ui_locales()
    }
    fn forget_answers(self, profile: Id, install_id: &str) -> Result<(), String> {
        extensions::forget_answers(profile, install_id)
    }
    fn changed(self) {
        if let Some((id, _)) = self.profile() {
            extensions::reload(id);
        }
        act(self.0, |_| {});
    }
    fn notice(self) -> Option<String> {
        None
    }
}

pub(super) fn pane(ui: Ui) -> AnyElement {
    tarn_nsh_shared::extensions::extensions(Installs(ui))
}
