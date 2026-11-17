use nsh_core::keymap::When;
use nsh_core::prelude::*;
use tarn_nsh_shared::controls::{icon_button, label};
use tarn_nsh_shared::icons as glyphs;
use tarn_nsh_shared::panes::{GROUPS, name};

use super::super::{Screen, Ui, panels};
use crate::theme::{ICON, TEXT_SMALL, Theme};

const WIDTH: f32 = 760.;
const NAV_WIDTH: f32 = 200.;
const DIM: u32 = 0x0000_0059;

fn nav() -> impl IntoElement {
    view().w(px(NAV_WIDTH)).p_2().gap_1().bg(Theme::panel_nav).children(GROUPS.iter().map(|(group, panes)| {
        view()
            .gap(px(2.))
            .pb_2()
            .child(text(*group).text_size(px(TEXT_SMALL)).text_color(Theme::muted).px_2())
            .children(panes.iter().map(|pane| label(name(*pane)).px_2().py_1()))
    }))
}

pub(super) fn window(ui: Ui) -> impl IntoElement {
    when(
        move || ui.screen.get() == Screen::Settings,
        move || {
            let always = KeyOptions { when: When::Always, ..KeyOptions::default() };
            on_key_with("escape", always, move |_| ui.show(Screen::Browse));
            view().absolute().inset_0().items_center().justify_center().bg(rgba(DIM)).child(
                view()
                    .w(px(WIDTH))
                    .max_w(relative(0.94))
                    .h(relative(0.85))
                    .flex_row()
                    .rounded(px(Theme::radius_card()))
                    .overflow_hidden()
                    .bg(Theme::panel)
                    .border_1()
                    .border_color(Theme::hairline)
                    .shadow(Theme::elevation(16., 40.))
                    .child(nav())
                    .child(
                        view()
                            .flex_1()
                            .child(view().flex_row().justify_end().p_1().child(icon_button(
                                view().text_color(Theme::muted).child(glyphs::icon(glyphs::CLOSE, ICON)),
                                "Close Settings",
                                move || ui.show(Screen::Browse),
                            )))
                            .child(scroll().flex_1().child(view().gap_2().px_4().pb_6().child(panels::settings(ui)))),
                    ),
            )
        },
    )
}
