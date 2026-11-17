use nsh_base::ButtonElement;
use nsh_core::element::IntoProp;
use nsh_core::prelude::*;

use crate::controls::{glyph_tile, icon_button, key_hint, pressable, site_tile, tip};
use crate::icons;
use crate::motion::{fading, popping};
use crate::theme::{Paint, SYSTEM_FONT, Secure, TEXT, TEXT_SMALL, Theme};

pub const FIELD: f32 = 30.;
pub const CHIP_INSET: f32 = 3.;
pub const CHIP: f32 = FIELD - 2. - 2. * CHIP_INSET;
pub const CHIP_GLYPH: f32 = 14.;
pub const ADDRESS_WIDTH: f32 = 560.;
pub const ADDRESS_MIN: f32 = 200.;
pub const TOOL_GAP: f32 = 2.;
pub const PALETTE_WIDTH: f32 = 640.;
pub const PALETTE_ROW: f32 = 40.;
pub const PALETTE_DIM: u32 = 0x0000_0059;
const INPUT_ROW: f32 = 52.;
const FOOTER: f32 = 36.;
const ROW_GLYPH: f32 = 28.;
const ROW_PAD: f32 = 6.;
const DETAIL_WIDTH: f32 = 260.;
const EXTENSION: f32 = 26.;
const EXTENSION_GLYPH: f32 = 16.;
const SEARCH_SLOT: f32 = 28.;
const TOOL_GLYPH: f32 = 16.;

pub fn chip_radius() -> f32 {
    Theme::radius_row() - 1. - CHIP_INSET
}

pub fn small(content: impl IntoReactive<SharedString>) -> Text {
    text(content).text_size(px(TEXT_SMALL)).text_color(Theme::muted)
}

/// A toolbar icon button whose tooltip goes on `side`, away from the page.
pub fn tool_button(glyph: &str, name: &str, side: Placement, press: impl Fn() + 'static) -> ButtonElement {
    let shown = view().size_full().items_center().justify_center().tooltip(tip(name, side)).child(icons::icon(glyph, TOOL_GLYPH));
    icon_button(shown, name.to_owned(), press)
}

pub fn tools() -> View {
    view().flex_row().items_center().gap(px(TOOL_GAP))
}

/// The address field at rest: a capsule up to `ADDRESS_WIDTH` wide, centred in the toolbar.
pub fn capsule(open: impl Fn() -> bool + Copy + 'static) -> View {
    view()
        .flex_row()
        .flex_1()
        .max_w(px(ADDRESS_WIDTH))
        .items_center()
        .px(px(CHIP_INSET))
        .h(px(FIELD))
        .rounded(px(Theme::radius_row()))
        .bg(move || if open() { Secure::pressed() } else { Secure::chip() })
        .border_1()
        .border_color(Secure::hairline)
        .font_family(SYSTEM_FONT)
        .text_color(Secure::faint)
}

/// The capsule's middle: the host, cut at its start so the registrable domain stays, after `Not Secure —` for
/// plain HTTP; or the placeholder.
pub fn address_text(name: &'static str, host: impl Fn() -> (String, bool) + Copy + 'static, press: impl Fn() + 'static) -> ButtonElement {
    pressable(name, press).flex_1().min_w(px(0.)).h_full().flex_row().items_center().justify_center().px_2().child(dynamic(move || {
        let (host, insecure) = host();
        if host.is_empty() {
            return text("Search or enter address").text_size(px(TEXT)).text_color(Secure::faint()).number_of_lines(1).into_element();
        }
        let line = |t: String| text(t).text_size(px(TEXT)).number_of_lines(1);
        view()
            .flex_row()
            .min_w(px(0.))
            .children(insecure.then(|| line("Not Secure — ".to_owned()).flex_none().text_color(Secure::warning())))
            .child(line(host).min_w(px(0.)).flex_shrink(1.).text_overflow(nsh_core::EllipsizeMode::Head).text_color(if insecure {
                Secure::warning()
            } else {
                Secure::text()
            }))
            .into_element()
    }))
}

/// The capsule's left slot without a site: a search glyph where the Shield chip goes.
pub fn search_slot() -> View {
    view().w(px(SEARCH_SLOT)).items_center().child(icons::icon(icons::SEARCH, CHIP_GLYPH))
}

/// The Shield chip: the level's glyph, its name once raised above Standard, and the count stopped on the site.
pub fn shield_chip(
    name: impl Fn() -> SharedString + 'static,
    raised: impl Fn() -> Option<String> + Copy + 'static,
    count: impl Fn() -> Option<String> + Copy + 'static,
    press: impl Fn() + 'static,
) -> ButtonElement {
    let color = move || if raised().is_some() { Secure::shield_raised() } else { Secure::muted() };
    nsh_base::button()
        .accessibility_label(name)
        .on_press(press)
        .flex_row()
        .items_center()
        .gap_1()
        .flex_none()
        .h(px(CHIP))
        .px(px(6.))
        .rounded(px(chip_radius()))
        .bg(Secure::control)
        .hover(|s| s.bg(Secure::hover()))
        .active(|s| s.bg(Secure::pressed()))
        .text_color(color)
        .child(dynamic(move || popping(view().child(icons::icon(if raised().is_some() { icons::SHIELD_CHECK } else { icons::SHIELD }, CHIP_GLYPH)))))
        .child(dynamic(move || raised().map(|level| small(level).text_color(color()).font_family(SYSTEM_FONT).font_weight(FontWeight::MEDIUM))))
        .child(dynamic(move || count().map(|n| small(n).text_color(color()).font_family(SYSTEM_FONT))))
}

/// A button inside the capsule, shorter than it so its hover fill keeps clear of the edge.
pub fn chip_button(glyph: &str, name: &str, on: impl Fn() -> bool + 'static, press: impl Fn() + 'static) -> ButtonElement {
    nsh_base::button()
        .accessibility_label(name.to_owned())
        .on_press(press)
        .h(px(CHIP))
        .px(px(6.))
        .items_center()
        .justify_center()
        .rounded(px(chip_radius()))
        .hover(|s| s.bg(Secure::hover()))
        .active(|s| s.bg(Secure::pressed()))
        .text_color(move || if on() { Theme::accent() } else { Secure::muted() })
        .child(view().h_full().justify_center().tooltip(tip(name, Placement::Bottom)).child(icons::icon(glyph, CHIP_GLYPH)))
}

pub fn badge_color(color: Option<[u8; 4]>) -> Rgba {
    color.map_or_else(Theme::accent, |[r, g, b, a]| Rgba { r: f32::from(r) / 255., g: f32::from(g) / 255., b: f32::from(b) / 255., a: f32::from(a) / 255. })
}

/// An extension's toolbar button: its icon or initial, with its badge in the corner.
pub fn extension_button(name: String, icon: Option<String>, badge: Option<(String, Rgba)>, press: impl Fn() + 'static) -> ButtonElement {
    let letter = name.chars().next().map(|c| c.to_lowercase().to_string()).unwrap_or_default();
    let badge = badge.map(|(text_, fill)| {
        view()
            .absolute()
            .bottom(px(-2.))
            .right(px(-4.))
            .px(px(3.))
            .rounded(px(4.))
            .bg(fill)
            .child(text(text_).text_size(px(9.)).text_color(Theme::on_fill(fill)))
    });
    pressable(name.clone(), press)
        .relative()
        .w(px(EXTENSION))
        .h(px(EXTENSION))
        .items_center()
        .justify_center()
        .rounded(px(Theme::radius_small()))
        .child(view().size_full().items_center().justify_center().tooltip(tip(&name, Placement::Bottom)).child(match icon {
            Some(path) => image(path).w(px(EXTENSION_GLYPH)).h(px(EXTENSION_GLYPH)).into_element(),
            None => site_tile(None, &letter, EXTENSION_GLYPH).into_element(),
        }))
        .children(badge)
}

pub fn picked_fill(picked: bool) -> Rgba {
    if picked { Theme::selected() } else { rgba(0) }
}

pub fn row<M>(name: impl IntoProp<Option<SharedString>, M>, press: impl Fn() + 'static) -> ButtonElement {
    pressable(name, press).flex_row().items_center().gap_3().mx(px(ROW_PAD)).h(px(PALETTE_ROW)).px(px(ROW_PAD)).rounded(px(Theme::radius_row()))
}

pub fn row_glyph(glyph: &str, picked: impl Fn() -> bool + Copy + 'static) -> View {
    glyph_tile(
        glyph,
        ROW_GLYPH,
        move || if picked() { Theme::accent() } else { Theme::control() },
        move || {
            if picked() { Theme::on_fill(Theme::accent()) } else { Theme::muted() }
        },
    )
}

pub fn row_site(icon: Option<String>, host: &str) -> View {
    view().w(px(ROW_GLYPH)).items_center().child(site_tile(icon, host, 20.))
}

pub fn row_section(name: &str) -> View {
    view().px(px(12.)).pt_2().pb_1().child(crate::controls::caps_label(name))
}

pub fn row_detail(content: impl IntoReactive<SharedString>) -> Text {
    small(content).max_w(px(DETAIL_WIDTH)).number_of_lines(1)
}

pub fn row_address(host: String, rest: String) -> View {
    view()
        .flex_row()
        .max_w(px(DETAIL_WIDTH))
        .min_w(px(0.))
        .child(small(host).min_w(px(0.)).flex_shrink(1.).number_of_lines(1).text_overflow(nsh_core::EllipsizeMode::Head))
        .child(small(rest).min_w(px(0.)).flex_shrink(100.).number_of_lines(1))
}

pub fn row_name(content: impl IntoReactive<SharedString>, color: Rgba) -> Text {
    text(content).text_size(px(TEXT)).text_color(color).flex_1().number_of_lines(1)
}

pub fn rows() -> View {
    view().flex_col().gap(px(TOOL_GAP)).py(px(ROW_PAD))
}

fn commands_badge() -> View {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .h(px(22.))
        .px_2()
        .rounded(px(Theme::radius_small()))
        .bg(Theme::accent_soft)
        .text_color(Theme::space_label)
        .child(icons::icon(icons::COMMAND, 11.))
        .child(small("Commands").text_color(Theme::space_label))
}

/// The command palette: a card over a dimmed page with the field, its rows and the key hints.
pub fn palette(field: impl IntoElement, listing: impl Fn() -> bool + 'static, list: impl IntoElement, close: impl Fn() + 'static) -> View {
    let head = view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_4()
        .h(px(INPUT_ROW))
        .border_b_1()
        .border_color(Theme::hairline)
        .text_color(Theme::muted)
        .child(icons::icon(icons::SEARCH, TOOL_GLYPH))
        .child(field)
        .child(when(listing, commands_badge));
    let footer = view()
        .flex_row()
        .items_center()
        .gap_4()
        .px_4()
        .h(px(FOOTER))
        .border_t_1()
        .border_color(Theme::hairline)
        .child(key_hint("↵", "Run"))
        .child(key_hint("↑+↓", "Move"))
        .child(view().flex_1())
        .child(key_hint("Esc", "Close"));
    let card = view()
        .flex_col()
        .w(px(PALETTE_WIDTH))
        .max_w(relative(1.))
        .rounded(px(Theme::radius_card()))
        .surface()
        .shadow(Theme::elevation(18., 48.))
        .overflow_hidden()
        .child(head)
        .child(scroll().flex_shrink(1.).child(list))
        .child(footer);
    let dim = nsh_base::button().accessibility_label("Close").on_press(close).absolute().inset_0().bg(rgba(PALETTE_DIM));
    view().absolute().inset_0().flex_col().items_center().px_4().child(fading(dim)).child(view().flex_grow(1.)).child(popping(card)).child(view().flex_grow(6.))
}
