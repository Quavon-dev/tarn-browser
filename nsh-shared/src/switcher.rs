use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::controls::{pressable, site_tile, surface};
use crate::theme::{RADIUS_PANEL, RADIUS_TILE, TEXT_SMALL, Theme};

const CARD_WIDTH: f32 = 168.;
const STILL_HEIGHT: f32 = 105.;
const ICON: f32 = 40.;
const PER_ROW: f32 = 5.;

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
                .border_2()
                .border_color(if on { Theme::accent() } else { Theme::surface_edge() })
                .items_center()
                .justify_center()
                .child(site_tile(card.icon, &card.host, ICON));
            let title = text(card.title.clone())
                .text_size(px(TEXT_SMALL))
                .text_color(if on { Theme::text() } else { Theme::muted() })
                .font_weight(if on { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                .number_of_lines(1_u32)
                .px_1();
            pressable(card.title, move || pick(ix)).w(px(CARD_WIDTH)).flex_col().gap_1().child(still).child(title).into_element()
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
        .child(surface(RADIUS_PANEL).p_4().flex_row().flex_wrap().justify_center().gap_4().max_w(px((CARD_WIDTH + 16.) * PER_ROW + 32.)).children(cards))
}
