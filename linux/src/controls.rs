use nsh_core::element::IntoProp;
use nsh_core::prelude::*;
pub use tarn_nsh_shared::controls::pressable;
use tarn_nsh_shared::controls::{Segment, icon_size, switch_row};
pub use tarn_nsh_shared::controls::{accent_pill, card, group, icon_pill, label, pill, pill_grid, tip};

use crate::icons;
use crate::theme::{ROW_HEIGHT, TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

pub fn body(content: impl IntoReactive<SharedString>) -> Text {
    text(content).text_size(px(TEXT))
}

pub fn small(content: impl IntoReactive<SharedString>) -> Text {
    text(content).text_size(px(TEXT_SMALL)).text_color(Theme::muted())
}

pub fn title(content: impl IntoReactive<SharedString>) -> Text {
    text(content).text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text())
}

pub fn icon_button(icon: &str, name: &str, on: impl Fn() + 'static) -> nsh_base::ButtonElement {
    icon_button_at(icon, name, Placement::Top, on)
}

/// An icon button whose tooltip goes on `side`: one that would land on the page is hidden behind the web view.
pub fn icon_button_at(icon: &str, name: &str, side: Placement, on: impl Fn() + 'static) -> nsh_base::ButtonElement {
    let glyph = view().size_full().items_center().justify_center().tooltip(tip(name, side)).child(icons::icon(icon, icon_size()));
    tarn_nsh_shared::controls::icon_button(glyph, name.to_owned(), on)
}

pub fn row<M>(name: impl IntoProp<Option<SharedString>, M>, on: impl Fn() + 'static) -> nsh_base::ButtonElement {
    pressable(name, on).flex_row().items_center().gap_2().px_2().h(px(ROW_HEIGHT)).rounded(px(Theme::radius_row()))
}

pub fn segmented<T: Copy + PartialEq + 'static>(options: Vec<(T, String)>, selected: T, on: impl Fn(T) + Clone + 'static) -> View {
    let segments: Vec<Segment> = options
        .into_iter()
        .map(|(value, name)| {
            let on = on.clone();
            (name, Box::new(move || value == selected) as Box<dyn Fn() -> bool>, Box::new(move || on(value)) as Box<dyn Fn()>)
        })
        .collect();
    tarn_nsh_shared::controls::segmented(segments)
}

pub fn switch(on: bool, name: &str, toggle: impl Fn() + 'static) -> nsh_base::ButtonElement {
    switch_row(name, move || on, toggle)
}

pub fn group_row() -> View {
    view().flex_row().items_center().gap_3().px_3().min_h(px(40.)).border_b_1().border_color(Theme::separator())
}
