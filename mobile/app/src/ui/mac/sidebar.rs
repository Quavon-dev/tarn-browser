use browser_core::tabs::TabKey;
use nsh_core::prelude::*;
use tarn_nsh_shared::controls::{icon_button, label, pressable};
use tarn_nsh_shared::icons as glyphs;

use super::super::{Screen, Ui, bars};
use crate::theme::{Paint, ROW_HEIGHT, TEXT_SMALL, Theme, themed};

const WIDTH: f32 = 256.;
const GLYPH: f32 = 14.;

fn tab_keys(ui: Ui) -> Vec<TabKey> {
    ui.read(|s| s.model.tabs.iter().map(|t| s.model.key(t)).collect())
}

fn tab_title(ui: Ui, key: TabKey) -> String {
    ui.read(|s| Some(s.tab_title(key)).filter(|t| !t.is_empty()).unwrap_or_else(|| "New Tab".to_owned()))
}

fn tab_row(ui: Ui, key: TabKey) -> impl IntoElement {
    let active = move || ui.read(|s| s.model.active == Some(key));
    view()
        .flex_row()
        .items_center()
        .h(px(ROW_HEIGHT))
        .rounded(px(Theme::radius_row()))
        .bg(move || if active() { Theme::selected() } else { Theme::clear() })
        .child(
            super::popovers::tab_context(ui, key, view())
                .test_id("tab")
                .accessible(true)
                .label(move || SharedString::from(tab_title(ui, key)))
                .role(AccessibilityRole::Button)
                .hover(|s| s.bg(Theme::hover()))
                .active(|s| s.bg(Theme::pressed()))
                .on_press(move |_: &PressEvent| {
                    ui.act(|s| {
                        s.activate(key);
                        Vec::new()
                    });
                    ui.show(Screen::Browse);
                })
                .flex_1()
                .h_full()
                .flex_row()
                .items_center()
                .gap_2()
                .pl_2()
                .rounded(px(Theme::radius_row()))
                .child(view().text_color(Theme::muted).child(glyphs::icon(glyphs::GLOBE, GLYPH)))
                .child(label(move || SharedString::from(tab_title(ui, key))).number_of_lines(1).flex_1()),
        )
        .child(icon_button(view().text_color(Theme::muted).child(glyphs::icon(glyphs::CLOSE, GLYPH)), "Close Tab", move || {
            ui.act(|s| {
                s.close_tab(key);
                Vec::new()
            })
        }))
}

fn new_tab_row(ui: Ui) -> impl IntoElement {
    pressable("New Tab", move || bars::open_new_tab(ui))
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(ROW_HEIGHT))
        .pl_2()
        .rounded(px(Theme::radius_row()))
        .child(view().text_color(Theme::space_label).child(glyphs::icon(glyphs::PLUS, GLYPH)))
        .child(label("New Tab").text_color(Theme::muted))
}

fn header(ui: Ui) -> impl IntoElement {
    view().flex_row().items_center().h(px(ROW_HEIGHT)).px_2().child(
        text(move || ui.read(|s| s.model.current_space().map(|c| c.space.name.clone()).unwrap_or_default()))
            .text_size(px(TEXT_SMALL))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(Theme::space_label),
    )
}

fn footer(ui: Ui) -> impl IntoElement {
    view().flex_row().items_center().justify_end().px_1().child(icon_button(
        view().text_color(Theme::muted).child(glyphs::icon(glyphs::GEAR, GLYPH)),
        "Settings",
        move || ui.toggle(Screen::Settings),
    ))
}

pub(super) fn sidebar(ui: Ui) -> impl IntoElement {
    view()
        .flex_col()
        .w(px(WIDTH))
        .m_1()
        .p_1()
        .rounded(px(Theme::radius_tile()))
        .border_1()
        .border_color(Theme::sidebar_edge)
        .child(themed(|| view().absolute().inset_0().rounded(px(Theme::radius_tile())).paint(Theme::sidebar())))
        .child(header(ui))
        .child(scroll().flex_1().child(view().flex_col().gap(px(2.)).child(new_tab_row(ui)).child(each(
            move || tab_keys(ui),
            |k: &TabKey| *k,
            move |key, _| tab_row(ui, key.get_untracked()),
        ))))
        .child(footer(ui))
}
