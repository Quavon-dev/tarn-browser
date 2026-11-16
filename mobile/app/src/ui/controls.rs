use nsh_base::{Input, button, input};
use nsh_core::prelude::*;
pub use tarn_nsh_shared::controls::{accent_pill, label, pill, section, segmented, surface, switch_row};

use crate::theme::{ICON, TEXT, TEXT_HEADING, TEXT_SMALL, TEXT_TITLE, TOUCH_TARGET, Theme};

pub mod icons {
    use nsh_core::prelude::*;

    pub const BACK: ImageAsset = asset!("icons/back.svg");
    pub const FORWARD: ImageAsset = asset!("icons/forward.svg");
    pub const RELOAD: ImageAsset = asset!("icons/reload.svg");
    pub const SHARE: ImageAsset = asset!("icons/arrow-up-right.svg");
    pub const TABS: ImageAsset = asset!("icons/app-window.svg");
    pub const MENU: ImageAsset = asset!("icons/sidebar.svg");
    pub const SHIELD: ImageAsset = asset!("icons/shield.svg");
    pub const SHIELD_CHECK: ImageAsset = asset!("icons/shield-check.svg");
    pub const PLUS: ImageAsset = asset!("icons/plus.svg");
    pub const CLOSE: ImageAsset = asset!("icons/close.svg");
    pub const HISTORY: ImageAsset = asset!("icons/history.svg");
    pub const DOWNLOADS: ImageAsset = asset!("icons/circle-arrow-down.svg");
    pub const GEAR: ImageAsset = asset!("icons/gear.svg");
    pub const SEARCH: ImageAsset = asset!("icons/search.svg");
    pub const UNDO: ImageAsset = asset!("icons/undo-2.svg");
    pub const PERSON: ImageAsset = asset!("icons/person.svg");
    pub const HAND: ImageAsset = asset!("icons/hand.svg");
    pub const GLOBE: ImageAsset = asset!("icons/globe.svg");
    pub const CHEVRON_RIGHT: ImageAsset = asset!("icons/chevron-right.svg");
    pub const BOOK_OPEN: ImageAsset = asset!("icons/book-open.svg");
    pub const BOOKMARK: ImageAsset = asset!("icons/bookmark.svg");
    pub const FLAME: ImageAsset = asset!("icons/flame.svg");
    pub const PAUSE: ImageAsset = asset!("icons/pause.svg");
    pub const PLAY: ImageAsset = asset!("icons/play.svg");
    pub const TRASH: ImageAsset = asset!("icons/trash.svg");
    pub const PENCIL: ImageAsset = asset!("icons/pencil.svg");
    pub const CHECK: ImageAsset = asset!("icons/check.svg");
    pub const APP_WINDOW: ImageAsset = asset!("icons/app-window.svg");
    pub const SMARTPHONE: ImageAsset = asset!("icons/smartphone.svg");
    pub const MARK: ImageAsset = asset!("brand/mark.svg");
}

pub fn icon(asset: ImageAsset, color: impl Fn() -> Rgba + 'static) -> impl IntoElement {
    image(asset).w(px(ICON)).h(px(ICON)).tint_color(color)
}

pub fn icon_button(asset: ImageAsset, label: &'static str, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    tarn_nsh_shared::controls::icon_button(icon(asset, Theme::text), label, press)
}

pub fn caption(text_: impl IntoReactive<SharedString>) -> nsh_core::element::Text {
    text(text_).text_size(px(TEXT)).text_color(Theme::muted)
}

pub fn small(text_: impl IntoReactive<SharedString>) -> nsh_core::element::Text {
    text(text_).text_size(px(TEXT_SMALL)).text_color(Theme::faint)
}

pub fn heading(text_: &'static str) -> impl IntoElement {
    text(text_).text_size(px(TEXT_HEADING)).font_weight(FontWeight::BOLD).text_color(Theme::text)
}

pub fn row(asset: ImageAsset, text_: impl Into<String>, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    let text_ = text_.into();
    button()
        .accessibility_label(SharedString::from(text_.clone()))
        .flex_row()
        .items_center()
        .gap_3()
        .min_h(px(TOUCH_TARGET))
        .px_3()
        .border_b_1()
        .border_color(Theme::separator)
        .active(|s| s.bg(Theme::pressed))
        .on_press(press)
        .child(icon(asset, Theme::muted))
        .child(label(text_).flex_1())
}

pub fn detail_row(asset: Option<ImageAsset>, name: String, detail: Option<nsh_core::element::Text>, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    button()
        .accessibility_label(SharedString::from(name.clone()))
        .flex_row()
        .items_center()
        .gap_3()
        .min_h(px(TOUCH_TARGET))
        .px_3()
        .py_1()
        .border_b_1()
        .border_color(Theme::separator)
        .active(|s| s.bg(Theme::pressed))
        .on_press(press)
        .child(asset.map(|a| icon(a, Theme::muted)))
        .child(view().flex_1().child(label(name).number_of_lines(1)).child(detail))
}

pub fn framed(rows: View) -> View {
    tarn_nsh_shared::controls::group().overflow_hidden().child(rows)
}

pub fn text_field(placeholder: &'static str, name: &'static str) -> Input {
    input()
        .placeholder(placeholder)
        .placeholder_color(Theme::faint)
        .label(name)
        .h(px(TOUCH_TARGET))
        .px_3()
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised)
        .border_1()
        .border_color(Theme::hairline)
        .text_size(px(TEXT_TITLE))
        .text_color(Theme::text)
}
