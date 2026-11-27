use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::controls::{pressable, site_tile, surface};
use crate::theme::{RADIUS_PANEL, RADIUS_TILE, TEXT_SMALL, TEXT_TITLE, Theme};

const CARD_WIDTH: f32 = 168.;
const ICON: f32 = 40.;
/// Cards on the strip's one row.
pub const PER_ROW: usize = 5;
const RING: f32 = 4.;
const GAP: f32 = 12.;

pub struct Card {
    pub icon: Option<String>,
    pub host: String,
    pub title: String,
}

/// The recent-tab switcher's strip on a panel, with the selected tab's title and host spelled out
/// under it and `hint` below: Return switches to the `selected` card, a click to the card clicked
/// (`pick`), Esc cancels.
pub fn strip(
    cards: Vec<Card>,
    selected: usize,
    hint: &str,
    pick: impl Fn(usize) + Clone + 'static,
    commit: impl Fn() + 'static,
    cancel: impl Fn() + 'static,
) -> View {
    // NSH matches modifiers exactly, and Ctrl is usually still held here.
    let always = KeyOptions { when: When::Always, ..KeyOptions::default() };
    let (commit, cancel) = (std::rc::Rc::new(commit), std::rc::Rc::new(cancel));
    for held in ["", "ctrl+", "ctrl+shift+"] {
        let (commit, cancel) = (commit.clone(), cancel.clone());
        on_key_with(&format!("{held}enter"), always, move |_| commit());
        on_key_with(&format!("{held}escape"), always, move |_| cancel());
    }
    let detail = cards.get(selected).map(|card| {
        view()
            .items_center()
            .gap(px(2.))
            .child(text(card.title.clone()).text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).number_of_lines(1_u32))
            .child(text(card.host.clone()).text_size(px(TEXT_SMALL)).text_color(Theme::muted()).number_of_lines(1_u32))
    });
    let cards: Vec<AnyElement> = cards
        .into_iter()
        .enumerate()
        .map(|(ix, card)| {
            let on = ix == selected;
            let pick = pick.clone();
            let still = view()
                .w_full()
                .aspect_ratio(16. / 10.)
                .rounded(px(RADIUS_TILE))
                .bg(Theme::panel())
                .border_1()
                .border_color(Theme::surface_edge())
                .items_center()
                .justify_center()
                .opacity(if on { 1. } else { 0.72 })
                .child(site_tile(card.icon, &card.host, ICON));
            let still = view()
                .p(px(RING))
                .rounded(px(RADIUS_TILE + RING))
                .border_2()
                .border_color(Rgba { a: if on { 1. } else { 0. }, ..Theme::accent() })
                .bg(Rgba { a: if on { 0.14 } else { 0. }, ..Theme::accent() })
                .child(still);
            let title = text(card.title.clone())
                .text_size(px(TEXT_SMALL))
                .text_color(if on { Theme::text() } else { Theme::muted() })
                .font_weight(if on { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                .number_of_lines(1_u32)
                .px(px(RING + 2.));
            pressable(card.title, move || pick(ix))
                .flex_1()
                .min_w(px(0.))
                .max_w(px(CARD_WIDTH + 2. * RING))
                .flex_col()
                .gap_1()
                .child(still)
                .child(title)
                .into_element()
        })
        .collect();
    view().absolute().inset_0().z_index(10).items_center().justify_center().p_6().test_id("tab-switcher").bg(Rgba { r: 0., g: 0., b: 0., a: 0.35 }).child(
        surface(RADIUS_PANEL)
            .w_full()
            .max_w(px((CARD_WIDTH + 2. * RING + GAP) * PER_ROW as f32 + 40.))
            .p(px(20.))
            .gap(px(14.))
            .items_center()
            .child(view().w_full().flex_row().justify_center().gap(px(GAP)).children(cards))
            .children(detail)
            .child(text(hint.to_owned()).text_size(px(TEXT_SMALL)).text_color(Theme::muted())),
    )
}
