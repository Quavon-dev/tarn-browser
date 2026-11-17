mod input;
pub(super) mod popovers;
mod settings;
mod sidebar;
mod toolbar;

use nsh_core::prelude::*;

use super::{Screen, Ui};

pub(super) fn owns(screen: Screen) -> bool {
    settings::owns(screen) || popovers::owns(screen)
}

pub(super) fn open_palette(ui: Ui) {
    toolbar::open_palette(ui);
}

pub(super) fn back_from(screen: Screen) -> Screen {
    settings::up(screen).unwrap_or(Screen::Browse)
}

pub(super) fn frame<T: IntoElements, B: IntoElements>(
    ui: Ui,
    touch_top: impl Fn() -> T + 'static,
    content: impl IntoElement,
    touch_bottom: impl Fn() -> B + 'static,
) -> View {
    let mac = move || ui.mac.get();
    view()
        .flex_1()
        .relative()
        .child(
            view().flex_1().flex_row().child(when(move || mac() && ui.read_data(|s| s.sidebar_open), move || sidebar::sidebar(ui))).child(
                view()
                    .flex_1()
                    .relative()
                    .child(when(move || !mac(), touch_top))
                    .child(when(mac, move || toolbar::toolbar(ui)))
                    .child(content)
                    .child(when(move || !mac(), touch_bottom))
                    .child(when(mac, move || toolbar::palette(ui))),
            ),
        )
        .child(when(mac, move || popovers::popovers(ui)))
        .child(when(mac, move || settings::window(ui)))
        .child(when(mac, move || input::bindings(ui)))
}
