use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::controls::{pressable, site_tile};
use crate::theme::{RADIUS_TILE, TEXT_SMALL, Theme};

const CARD_WIDTH: f32 = 184.;
const STILL_HEIGHT: f32 = CARD_WIDTH * 10. / 16.;
const ICON: f32 = 40.;
/// Cards on the strip's one row.
pub const PER_ROW: usize = 5;
const RING: f32 = 5.;
const GAP: f32 = 20.;

pub struct Card {
    pub icon: Option<String>,
    pub host: String,
    pub title: String,
}

/// The recent-tab switcher's strip: Return switches to the `selected` card, a click to the card
/// clicked (`pick`), Esc cancels.
pub fn strip(cards: Vec<Card>, selected: usize, pick: impl Fn(usize) + Clone + 'static, commit: impl Fn() + 'static, cancel: impl Fn() + 'static) -> View {
    let always = KeyOptions { when: When::Always, ..KeyOptions::default() };
    on_key_with("enter", always, move |_| commit());
    on_key_with("escape", always, move |_| cancel());
    let cards: Vec<AnyElement> = cards
        .into_iter()
        .enumerate()
        .map(|(ix, card)| {
            let on = ix == selected;
            let pick = pick.clone();
            let still = view()
                .w_full()
                .h(px(STILL_HEIGHT))
                .rounded(px(RADIUS_TILE))
                .bg(Theme::panel())
                .border_1()
                .border_color(Theme::surface_edge())
                .items_center()
                .justify_center()
                .child(site_tile(card.icon, &card.host, ICON));
            let edge = Rgba { a: if on { 0.3 } else { 0. }, ..Theme::text() };
            let still = view()
                .p(px(RING))
                .rounded(px(RADIUS_TILE + RING))
                .border_1()
                .border_color(edge)
                .bg(Rgba { a: if on { 0.06 } else { 0. }, ..Theme::text() })
                .child(still);
            let title = text(card.title.clone())
                .text_size(px(TEXT_SMALL))
                .text_color(if on { Theme::text() } else { Theme::muted() })
                .number_of_lines(1_u32)
                .px(px(RING + 4.));
            pressable(card.title, move || pick(ix)).w(px(CARD_WIDTH + 2. * RING)).flex_col().gap_1().child(still).child(title).into_element()
        })
        .collect();
    view()
        .absolute()
        .inset_0()
        .z_index(10)
        .items_center()
        .justify_center()
        .p_6()
        .test_id("tab-switcher")
        .child(view().flex_row().justify_center().gap(px(GAP)).children(cards))
}
