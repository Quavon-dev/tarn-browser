mod input;
mod popovers;
mod settings;
mod sidebar;
mod toolbar;

use nsh_core::prelude::*;

use super::{Screen, Ui};

pub(super) fn owns(screen: Screen) -> bool {
    screen == Screen::Settings || popovers::owns(screen)
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
            view().flex_1().flex_row().child(when(mac, move || sidebar::sidebar(ui))).child(
                view()
                    .flex_1()
                    .child(when(move || !mac(), touch_top))
                    .child(when(mac, move || toolbar::toolbar(ui)))
                    .child(content)
                    .child(when(move || !mac(), touch_bottom)),
            ),
        )
        .child(when(mac, move || popovers::popovers(ui)))
        .child(when(mac, move || settings::window(ui)))
        .child(when(mac, move || input::bindings(ui)))
}
