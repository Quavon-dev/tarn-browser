use browser_core::split::{self, Divider, Rect, Share};
use browser_core::tabs::TabKey;
use nsh_core::prelude::*;

use super::{Ui, act, read, site_tile, tab_menu};
use crate::browser::Browser;
use crate::controls::{body, icon_button, small};
use crate::icons;
use crate::snapshot::Pane;
use crate::theme::Theme;

const GAP: f32 = 8.;
const HEADER: f32 = 30.;
/// The narrowest pane whose header names the host beside the title.
const HOST_ROOM: f32 = 380.;

/// A pane's card in a page area `size` big: its share, with half the gap on every inner edge.
fn card_rect((x, y, w, h): Share, (width, height): (f32, f32)) -> Rect {
    let inner = |at: f32| at > 1e-4 && at < 1. - 1e-4;
    let half = |at: f32| if inner(at) { GAP / 2. } else { 0. };
    let (left, right, top, bottom) = (half(x), half(x + w), half(y), half(y + h));
    (x * width + left, y * height + top, (w * width - left - right).max(0.), (h * height - top - bottom).max(0.))
}

/// Where a pane's page goes: its card below the header, inside the card's border.
fn page_rect((x, y, w, h): Rect) -> Rect {
    (x + 1., y + HEADER, (w - 2.).max(0.), (h - HEADER - 1.).max(0.))
}

/// Where the web view at `pane` of the split on screen goes; the active tab's alone fills the area.
pub(super) fn view_rect(ui: Ui, active: bool, pane: Option<usize>) -> Rect {
    let area = ui.area.get();
    match pane {
        Some(ix) => {
            ui.snap.with(|s| s.split.as_ref().and_then(|v| v.layout.get(ix).copied())).map(|share| page_rect(card_rect(share, area))).unwrap_or_default()
        }
        None if active => match ui.snap.with(|s| s.responsive) {
            Some(preset) => {
                let fit = devkit::responsive::fit(&preset, area.0, area.1);
                (fit.x, fit.y, fit.width, fit.height)
            }
            None => (0., 0., area.0, area.1),
        },
        None => Rect::default(),
    }
}

/// Drops the dragged tab on `key`'s row or pane header, where the drop was named.
pub(super) fn drop_on(ui: Ui, key: TabKey) {
    let dragged = ui.dragging.get_untracked().filter(|_| ui.split_drop.with_untracked(|d| d.is_some_and(|(k, _)| k == key)));
    if let Some(dragged) = dragged {
        release(ui);
        act(ui, |b| b.drop_on_pane(key, dragged));
    }
}

/// Drops the dragged tab on `key`'s sidebar row: in the row's middle it splits as named, near its
/// edges it moves the tab above the row, or below it when `below`.
pub(super) fn drop_on_row(ui: Ui, key: TabKey, below: bool) {
    match ui.dragging.get_untracked().filter(|d| *d != key && ui.split_drop.with_untracked(|s| s.is_none_or(|(k, _)| k != key))) {
        Some(dragged) => {
            release(ui);
            act(ui, |b| b.move_tab_to(dragged, key, below));
        }
        None => drop_on(ui, key),
    }
}

/// The button came up over the page area: a dragged tab drops on the pane named.
pub(super) fn released(ui: Ui) {
    if let Some((key, _)) = ui.split_drop.get_untracked() {
        drop_on(ui, key);
    }
}

/// While a tab is dragged near the window's edge, names what leaving the window there opens;
/// `e` is any move with its root position.
pub(super) fn edge_hint(ui: Ui, e: &MouseEvent) {
    let (x, y) = (e.page_position.x.0, e.page_position.y.0);
    let (width, height) = ui.size.get_untracked();
    let hint = ui
        .dragging
        .get_untracked()
        .filter(|_| browser_core::tabs::near_edge(x, y, width, height))
        .and_then(|key| read(ui, |b| b.model.tear_off_label(key, b.pref(browser_core::prefs::TEAR_OFF_TEMPORARY))).flatten());
    if ui.tear_hint.get_untracked() != hint {
        ui.tear_hint.set(hint);
    }
}

/// The accent edge and label of `edge_hint`, which let every pointer event through.
pub(super) fn edge_layer(ui: Ui) -> AnyElement {
    dynamic(move || {
        ui.tear_hint.get().map(|label| {
            view()
                .absolute()
                .inset_0()
                .pointer_events(PointerEvents::None)
                .border_1()
                .border_color(Theme::accent())
                .rounded(px(Theme::radius_panel()))
                .items_center()
                .pt_3()
                .child(tarn_nsh_shared::motion::popping(
                    view()
                        .px_3()
                        .py_1()
                        .rounded_full()
                        .bg(Theme::panel())
                        .border_1()
                        .border_color(Theme::accent())
                        .child(small(label).text_color(Theme::accent_text())),
                ))
        })
    })
    .into_element()
}

/// The pointer left the window holding a tab: the tab tears off into a window of its own. A
/// release outside the window reaches no view here, so leaving it is the drop.
pub(super) fn dragged_out(ui: Ui) {
    let Some(key) = ui.dragging.get_untracked() else { return };
    release(ui);
    super::tear_off(ui, key);
}

/// Ends a drag or a divider's resize: the button came up, or moves arrive without it after it
/// came up over a page, which keeps the release.
pub(super) fn release(ui: Ui) {
    if ui.dragging.with_untracked(Option::is_some) {
        ui.dragging.set(None);
    }
    if ui.split_drop.with_untracked(Option::is_some) {
        ui.split_drop.set(None);
    }
    if ui.tear_hint.with_untracked(Option::is_some) {
        ui.tear_hint.set(None);
    }
    if ui.resizing.with_untracked(Option::is_some) {
        ui.resizing.set(None);
    }
}

/// Names what dropping the dragged tab on `key` does while the pointer is over it; `middle` says
/// whether a row's drop point is in its middle, where it splits instead of moving the tab.
pub(super) fn drop_over(ui: Ui, key: TabKey, middle: bool) {
    let does = ui
        .dragging
        .get_untracked()
        .and_then(|dragged| read(ui, |b| b.splits.drop_label(key, dragged).filter(|_| !b.elsewhere.contains(&dragged))).flatten())
        .filter(|_| middle);
    let target = does.map(|does| (key, does));
    if ui.split_drop.get_untracked() != target {
        ui.split_drop.set(target);
    }
}

pub(super) fn drop_leave(ui: Ui, key: TabKey) {
    if ui.split_drop.with_untracked(|d| d.is_some_and(|(k, _)| k == key)) {
        ui.split_drop.set(None);
    }
}

/// The cards, headers and dividers of the split on screen, around its web views.
pub(super) fn layer(ui: Ui) -> AnyElement {
    let shown = computed(move || ui.snap.with(|s| s.split.clone()));
    dynamic(move || {
        let split = shown.get()?;
        let area = ui.area.get();
        let cards: Vec<View> = split.panes.into_iter().zip(&split.layout).map(|(pane, share)| card(ui, pane, card_rect(*share, area))).collect();
        let grips: Vec<View> = split.dividers.iter().map(|(divider, line)| grip(ui, *divider, *line, area)).collect();
        Some(view().absolute().inset_0().children(cards).children(grips))
    })
    .into_element()
}

/// The pointer moved over the page area, `e` in its points: a dragged tab names what dropping it
/// on the pane below does, and a held divider follows. Over a page the move reaches the page's
/// element and the area around it, not the layer.
pub(super) fn pointer_moved(ui: Ui, e: &MouseEvent) {
    edge_hint(ui, e);
    let (x, y) = (e.position.x.0, e.position.y.0);
    let (width, height) = ui.area.get_untracked();
    if ui.dragging.with_untracked(Option::is_some) {
        let under = ui.snap.with_untracked(|s| {
            let split = s.split.as_ref()?;
            let cards: Vec<Rect> = split.layout.iter().map(|share| card_rect(*share, (width, height))).collect();
            split::pane_at(&cards, x, y).map(|ix| split.panes[ix].key)
        });
        match under {
            Some(key) => drop_over(ui, key, true),
            None if ui.split_drop.with_untracked(Option::is_some) => ui.split_drop.set(None),
            None => {}
        }
        return;
    }
    let Some(divider) = ui.resizing.get_untracked().filter(|_| e.pressed_button.is_some()) else { return };
    let share = match divider {
        Divider::Column => split::ratio_at(x, width),
        Divider::Row => split::ratio_at(y, height),
    };
    super::act_quietly(ui, |b| {
        b.resize_split(divider, share);
    });
}

fn card(ui: Ui, pane: Pane, (x, y, w, h): Rect) -> View {
    let Pane { key, title, host, focused } = pane;
    let dropping = move || ui.split_drop.with(|d| d.is_some_and(|(k, _)| k == key));
    let roomy = w >= HOST_ROOM && !host.is_empty();
    let header = view()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(HEADER))
        .flex_none()
        .pl(px(10.))
        .pr(px(2.))
        .border_b_1()
        .border_color(Theme::hairline())
        .bg(move || if dropping() || focused { Theme::accent_soft() } else { rgba(0) })
        .accessible(true)
        .label(title.clone())
        .on_press_down(move || ui.dragging.set(Some(key)))
        .on_press(move || {
            act(ui, |b| {
                if b.in_split(key) {
                    b.activate(key)
                }
            })
        })
        .on_mouse_up(move |_: &MouseEvent| drop_on(ui, key))
        .on_context_menu(move |_: &ContextMenuEvent| tab_menu(ui, key))
        .on_mouse_move(move |_: &MouseEvent| drop_over(ui, key, true))
        .on_mouse_leave(move |_: &MouseEvent| drop_leave(ui, key))
        .child(site_tile(ui, &host, 14.))
        .child(body(title).text_color(if focused { Theme::text() } else { Theme::muted() }).flex_1().truncate())
        .children(roomy.then(|| small(host.clone()).text_color(Theme::faint()).max_w(px(w * 0.4)).truncate()))
        .child(dynamic(move || ui.split_drop.with(|d| d.filter(|(k, _)| *k == key).map(|(_, does)| small(does).text_color(Theme::accent_text())))))
        .child(icon_button(icons::ARROW_UP_RIGHT, "Remove from Split View", move || act(ui, |b| b.leave_split(key))))
        .child(icon_button(icons::CLOSE, "Close Tab", move || act(ui, |b| b.close_tab(key))));
    view()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .flex_col()
        .rounded(px(Theme::radius_page()))
        .overflow_hidden()
        .bg(Theme::page())
        .border_1()
        .border_color(move || if focused || dropping() { Theme::accent_text() } else { Theme::hairline() })
        .child(header)
}

/// The handle on a divider: dragging resizes, a double click evens the panes out again.
fn grip(ui: Ui, divider: Divider, (x, y, w, h): Share, (width, height): (f32, f32)) -> View {
    let column = divider == Divider::Column;
    let held = move || ui.resizing.get() == Some(divider);
    let pill = view().rounded_full().bg(move || if held() { Theme::accent() } else { Theme::faint() });
    let handle = view().absolute().items_center().justify_center().left(px(x * width)).top(px(y * height));
    let handle = if column {
        handle.w(px(GAP)).h(px(h * height)).ml(px(-GAP / 2.)).cursor(Cursor::ColResize).child(pill.w(px(3.)).h(px(32.)))
    } else {
        handle.w(px(w * width)).h(px(GAP)).mt(px(-GAP / 2.)).cursor(Cursor::RowResize).child(pill.w(px(32.)).h(px(3.)))
    };
    let equalise = move |e: &PressEvent| {
        if e.clicks == 2 {
            act(ui, Browser::equalise_split);
        }
    };
    let name = if column { "Column divider" } else { "Row divider" };
    handle.accessible(true).label(name).on_press_down(move || ui.resizing.set(Some(divider))).on_press(equalise)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_keep_the_gap_between_them_and_reach_the_outer_edges() {
        let area = (1000., 600.);
        assert_eq!(card_rect((0., 0., 0.5, 1.), area), (0., 0., 496., 600.));
        assert_eq!(card_rect((0.5, 0., 0.5, 1.), area), (504., 0., 496., 600.));
        assert_eq!(card_rect((0.5, 0.5, 0.5, 0.5), area), (504., 304., 496., 296.));
        assert_eq!(page_rect((504., 304., 496., 296.)), (505., 334., 494., 265.));
        assert_eq!(page_rect((0., 0., 10., 10.)), (1., 30., 8., 0.), "a pane lower than its header shows no page");
    }
}
