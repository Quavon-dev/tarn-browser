use nsh_base::{ButtonElement, TooltipElement, button, tooltip};
use nsh_core::prelude::*;

use crate::controls::icon_button;
use crate::icons;
use crate::theme::{PROFILE_COLORS, Paint, TEXT, TEXT_SMALL, TOOLBAR_HEIGHT, Theme};

pub const WIDTH: f32 = 256.;
pub const MARGIN: f32 = 6.;
pub const HEADER_HEIGHT: f32 = TOOLBAR_HEIGHT - MARGIN;
pub const HEADER_INSET: f32 = TILE_GAP + 4.;
pub const TILE_GAP: f32 = 8.;
pub const ROW_GAP: f32 = 1.;
pub const TAB_ROW: f32 = 30.;
pub const ROW_INSET: f32 = 10.;
pub const INDENT: f32 = 16.;
pub const CLOSE_WIDTH: f32 = 28.;
pub const CLOSE_GAP: f32 = 10.;
pub const INNER_PAD: f32 = 3.;
pub const BUTTON: f32 = 28.;
pub const GLYPH: f32 = 16.;
pub const STATE_GLYPH: f32 = 13.;
pub const TILE_ICON: f32 = 22.;
pub const FOOTER_HEIGHT: f32 = 36.;
pub const MENU_WIDTH: f32 = 230.;
pub const MENU_ROW: f32 = 24.;

pub fn tile_grid(width: f32) -> (usize, f32) {
    browser_core::tabs::tile_grid(width - 2. - 2. * TILE_GAP, TILE_GAP)
}

pub fn group_color(color: &str, id: i64) -> Rgba {
    let fixed = PROFILE_COLORS[id.unsigned_abs() as usize % PROFILE_COLORS.len()];
    rgb(PROFILE_COLORS.iter().find(|(n, _)| *n == color).unwrap_or(&fixed).1)
}

pub fn tip(name: &str, side: Placement) -> TooltipElement {
    let bubble = view().px_2().py_1().rounded(px(Theme::radius_small())).surface();
    tooltip().placement(side).child(bubble.child(text(name.to_owned()).text_size(px(TEXT_SMALL)).text_color(Theme::text())))
}

pub fn glyph_button(glyph: &'static str, name: &str, side: Placement, press: impl Fn() + 'static) -> ButtonElement {
    let face = view().size_full().items_center().justify_center().tooltip(tip(name, side)).child(icons::icon(glyph, GLYPH));
    icon_button(face, name.to_owned(), press).flex_none()
}

pub fn new_tab_mark() -> View {
    view()
        .w(px(GLYPH))
        .h(px(GLYPH))
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(Theme::radius_small()))
        .bg(Theme::accent)
        .text_color(|| Theme::on_fill(Theme::accent()))
        .child(icons::icon(icons::PLUS, 11.))
}

pub fn ring_markup(share: f32) -> String {
    let share = share.clamp(0.02, 0.999);
    let angle = std::f32::consts::TAU * share - std::f32::consts::FRAC_PI_2;
    let (x, y, large) = (12. + 10. * angle.cos(), 12. + 10. * angle.sin(), u8::from(share > 0.5));
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25"><path d="M12 2A10 10 0 {large} 1 {x:.2} {y:.2}"/></svg>"#
    )
}

pub fn progress_ring(share: f32) -> Svg {
    icons::picture(&ring_markup(share), GLYPH, GLYPH)
}

pub fn divider() -> View {
    view().h(px(1.)).mx_2().my(px(6.)).bg(Theme::hairline)
}

pub fn menu_card() -> View {
    view().w(px(MENU_WIDTH)).p_1().rounded(px(Theme::radius_row())).surface()
}

pub fn menu_item(name: impl Into<String>, enabled: bool, press: impl Fn() + 'static) -> ButtonElement {
    let name: String = name.into();
    let item = button()
        .accessibility_label(name.clone())
        .disabled(!enabled)
        .flex_row()
        .items_center()
        .h(px(MENU_ROW))
        .px_2()
        .rounded(px(Theme::radius_small()))
        .text_color(if enabled { Theme::text } else { Theme::faint })
        .child(text(name).text_size(px(TEXT)).flex_1().truncate());
    if enabled { item.on_press(press).hover(|s| s.bg(Theme::accent()).text_color(Theme::on_fill(Theme::accent()))) } else { item }
}

pub fn menu_heading(name: impl Into<String>) -> View {
    view().h(px(MENU_ROW)).px_2().justify_center().child(text(name.into()).text_size(px(TEXT_SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::faint))
}

pub fn menu_separator() -> View {
    view().h(px(1.)).mx_2().my_1().bg(Theme::hairline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidebar_keeps_the_mac_metrics() {
        let mac = include_str!("../../crates/app/src/sidebar.rs");
        for line in [
            "pub const SIDEBAR_WIDTH: f32 = 256.;",
            "pub const SIDEBAR_MARGIN: f32 = 6.;",
            "pub(crate) const TILE_GAP: f32 = 8.;",
            "const ROW_GAP: f32 = 1.;",
            "const TAB_ROW: f32 = 30.;",
            "const CLOSE_WIDTH: f32 = 28.;",
            "const CLOSE_GAP: f32 = 10.;",
            "const INNER_PAD: f32 = 3.;",
        ] {
            assert!(mac.contains(line), "{line}");
        }
        assert_eq!((WIDTH, MARGIN, HEADER_HEIGHT, TILE_GAP, TAB_ROW, CLOSE_WIDTH, CLOSE_GAP), (256., 6., 40., 8., 30., 28., 10.));
    }

    #[test]
    fn tiles_are_four_squares_a_row_at_the_default_width() {
        assert_eq!(tile_grid(WIDTH), (4, 53.));
        assert_eq!(tile_grid(420.).0, 7);
    }

    #[test]
    fn the_ring_runs_clockwise_from_the_top_over_the_icon_circle() {
        assert!(ring_markup(0.25).contains(r#"d="M12 2A10 10 0 0 1 22.00 12.00""#));
        assert!(ring_markup(0.75).contains(r#"d="M12 2A10 10 0 1 1 2.00 12.00""#));
        assert!(ring_markup(0.).contains("0 0 1 "), "an empty ring still shows a sliver");
    }

    #[test]
    fn a_group_takes_its_pick_or_a_colour_fixed_by_its_id() {
        let (name, hex) = PROFILE_COLORS[2];
        assert_eq!(group_color(name, 0), rgb(hex));
        assert_eq!(group_color("", 1), rgb(PROFILE_COLORS[1].1));
        assert_eq!(group_color("nope", -9), rgb(PROFILE_COLORS[9 % PROFILE_COLORS.len()].1));
    }
}
