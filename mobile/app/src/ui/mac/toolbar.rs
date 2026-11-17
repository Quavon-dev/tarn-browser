use nsh_core::prelude::*;

use super::super::{Screen, Ui, bars, icon_button, icons};
use crate::theme::{TOOLBAR_HEIGHT, themed};

const ADDRESS_WIDTH: f32 = 560.;

pub(super) fn toolbar(ui: Ui) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(TOOLBAR_HEIGHT))
        .px_2()
        .child(themed(move || view().flex_row().items_center().child(bars::back(ui)).child(bars::forward(ui)).child(bars::reload(ui))))
        .child(view().flex_1().items_center().child(view().w_full().max_w(px(ADDRESS_WIDTH)).child(bars::address_pill(ui))))
        .child(bars::cancel_button(ui))
        .child(themed(move || icon_button(icons::MENU, "Menu", move || ui.toggle(Screen::Menu))))
}
