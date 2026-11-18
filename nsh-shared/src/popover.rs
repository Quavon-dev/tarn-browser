use nsh_base::{ButtonElement, button};
use nsh_core::prelude::*;

use crate::controls::{GLYPH, card, glyph_tile, label};
use crate::icons;
use crate::theme::{Secure, TEXT, TEXT_SMALL, Theme};

pub const GAP: f32 = 4.;
pub const MARGIN: f32 = 8.;
pub const SITE_WIDTH: f32 = 340.;
pub const SHARE_WIDTH: f32 = 272.;
pub const DOWNLOADS_WIDTH: f32 = 320.;
pub const SPACE_WIDTH: f32 = 360.;
pub const QR_WIDTH: f32 = 260.;
pub const HEADER: f32 = 52.;
pub const ROW: f32 = 30.;
pub const PAD: f32 = 6.;
pub const DOWNLOAD_ROW: f32 = 48.;
pub const FILE_TILE: f32 = 28.;

#[derive(Clone, Copy)]
pub enum Anchor {
    Below(ElementRef, Align),
    Box(Bounds<Points>, Align),
    Above(ElementRef, Align),
}

pub fn popover(anchor: Anchor, width: f32, close: impl Fn() + Copy + 'static, content: impl IntoElement) -> View {
    fitted(anchor, width, close, content, None)
}

pub fn fit(placed: Option<OverlayPosition>, window: Size<Points>, keyboard: Points) -> (Option<f32>, f32) {
    let Some(p) = placed else { return (None, 0.) };
    let clear = window.height.0 - keyboard.0 - MARGIN;
    let room = p.available.height.0;
    match p.placement {
        Placement::Top if keyboard.0 > 0. => {
            let lift = (p.bounds.origin.y.0 + p.bounds.size.height.0 - clear).max(0.);
            (Some((room - lift).max(0.)), lift)
        }
        Placement::Bottom if keyboard.0 > 0. => (Some(room.min(clear - p.bounds.origin.y.0).max(0.)), 0.),
        _ => (Some(room), 0.),
    }
}

pub fn fitted(anchor: Anchor, width: f32, close: impl Fn() + Copy + 'static, body: impl IntoElement, footer: Option<AnyElement>) -> View {
    let placed = signal(None::<OverlayPosition>);
    let fitting = move || fit(placed.get(), window(), keyboard_height());
    let backdrop = overlay().inset_0().focusable(false).on_press(close);
    let popup = match anchor {
        Anchor::Below(at, align) => overlay().anchor(&at).align(align).offset(px(GAP)).placement(Placement::Bottom),
        Anchor::Box(at, align) => overlay().anchor_bounds(move || at).align(align).offset(px(GAP)).placement(Placement::Bottom),
        Anchor::Above(at, align) => overlay().anchor(&at).align(align).offset(px(GAP)).placement(Placement::Top),
    };
    let popup = popup.margin(px(MARGIN)).on_cancel(close).on_position(move |p: &OverlayPosition| placed.set(Some(*p))).child(
        card()
            .test_id("popover")
            .flex_col()
            .w(px(width))
            .max_h(move || fitting().0.map_or_else(|| relative(1.), |h| px(h).into()))
            .relative()
            .top(move || px(-fitting().1))
            .overflow_hidden()
            .text_size(px(TEXT))
            .text_color(Theme::text)
            .bind_motion(&crate::motion::fade_in())
            .child(scroll().flex_shrink(1.).min_h(px(0.)).child(body))
            .children(footer.map(|f| view().flex_none().child(f))),
    );
    view().child(backdrop).child(popup)
}

pub fn header(tile: impl IntoElement, title: impl Into<SharedString>, detail: impl Into<SharedString>) -> View {
    view().flex_row().items_center().gap(px(10.)).h(px(HEADER)).px_3().border_b_1().border_color(Theme::separator).child(tile).child(
        view()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .child(label(title.into()).font_weight(FontWeight::SEMIBOLD).number_of_lines(1))
            .child(text(detail.into()).text_size(px(TEXT_SMALL)).text_color(Theme::faint).number_of_lines(1)),
    )
}

pub fn row(height: f32, glyph: Option<&'static str>, name: &str, keys: Option<String>, press: impl Fn() + 'static) -> ButtonElement {
    button()
        .accessibility_label(name.to_owned())
        .on_press(press)
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(height))
        .mx(px(PAD))
        .px_2()
        .rounded(px(Theme::radius_row()))
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .children(glyph.map(|g| view().text_color(Theme::muted).child(icons::icon(g, GLYPH))))
        .child(label(name.to_owned()).flex_1().number_of_lines(1))
        .children(keys.map(|k| text(k).text_size(px(TEXT_SMALL)).text_color(Theme::faint)))
}

pub fn rows(items: Vec<AnyElement>) -> View {
    view().flex_col().py(px(PAD)).children(items)
}

pub fn separator() -> View {
    view().h(px(1.)).mx_3().bg(Theme::separator)
}

pub fn download_row(name: &str, note: String, warning: bool) -> View {
    let glyph = if warning { icons::TRIANGLE_ALERT } else { icons::glyph(browser_core::glyphs::file_glyph(name)) };
    let tint = move || if warning { Secure::warning() } else { Theme::muted() };
    view().flex_row().items_center().gap(px(10.)).h(px(DOWNLOAD_ROW)).px_2().child(glyph_tile(glyph, FILE_TILE, Theme::control, tint)).child(
        view()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .child(label(name.to_owned()).number_of_lines(1))
            .child(text(note).text_size(px(TEXT_SMALL)).text_color(tint).number_of_lines(1)),
    )
}

pub fn progress(done: u64, total: u64) -> View {
    let rest = total.saturating_sub(done) as f32;
    view()
        .flex_row()
        .h(px(3.))
        .rounded_full()
        .overflow_hidden()
        .bg(Theme::separator)
        .child(view().h_full().flex_grow(done as f32).bg(Theme::accent))
        .child(view().h_full().flex_grow(rest))
}
