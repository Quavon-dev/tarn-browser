use nsh_core::keymap::When;
use nsh_core::prelude::*;

use super::super::{Screen, Ui};

pub(super) fn bindings(ui: Ui) -> impl IntoElement {
    let always = KeyOptions { when: When::Always, ..KeyOptions::default() };
    on_key_with("mod+,", always, move |_| ui.toggle(Screen::Settings));
    on_key_with("mod+l", always, move |_| {
        ui.show(Screen::Browse);
        ui.field.focus();
    });
    view()
}
