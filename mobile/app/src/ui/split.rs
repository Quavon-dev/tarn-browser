use browser_core::split::{self, Divider, Share};
use browser_core::tabs::TabKey;
use nsh_core::prelude::*;

use super::Ui;
use crate::theme::Theme;

const GRIP: f32 = 24.;
/// How far a finger moves before a press on a header becomes a drag, or stops counting as a tap on a divider.
const SLOP: f32 = 10.;
const DOUBLE_TAP_MS: f64 = 350.;

/// `e`'s finger in the page area's points.
fn in_area(ui: Ui, e: &ResponderEvent) -> (f32, f32, Size<Points>) {
    let area = ui.area.get_untracked();
    (e.page.x.0 - area.origin.x.0, e.page.y.0 - area.origin.y.0, area.size)
}

/// The pane of the split on screen under `e`'s finger.
fn pane_under(ui: Ui, e: &ResponderEvent) -> Option<TabKey> {
    let (x, y, size) = in_area(ui, e);
    let (width, height) = (size.width.0, size.height.0);
    ui.read(|s| {
        let split = s.shown_split()?;
        let rects: Vec<split::Rect> = split.layout().into_iter().map(|(px, py, w, h)| (px * width, py * height, w * width, h * height)).collect();
        split::pane_at(&rects, x, y).map(|ix| split.panes()[ix])
    })
}

/// A pane's header that, dragged onto another pane, swaps the two; a tap still reaches its buttons.
pub(super) fn draggable(ui: Ui, key: TabKey, header: View) -> View {
    let other = move |e: &ResponderEvent| pane_under(ui, e).filter(|k| *k != key);
    header
        .claim_touches(TouchClaim::on_move().ask(|r: &ClaimRequest| {
            let d = r.event.gesture.delta;
            if d.x.0.abs().max(d.y.0.abs()) > SLOP {
                r.claim();
            }
        }))
        .on_responder_grant(move |_: &ResponderEvent| ui.dragging.set(Some((key, None))))
        .on_responder_move(move |e: &ResponderEvent| {
            let over = Some((key, other(e)));
            if ui.dragging.get_untracked() != over {
                ui.dragging.set(over);
            }
        })
        .on_responder_release(move |e: &ResponderEvent| {
            ui.dragging.set(None);
            if let Some(target) = other(e) {
                ui.act_on_screen(|s| {
                    s.splits.swap(key, target);
                    Vec::new()
                });
            }
        })
        .on_responder_terminate(move |_: &ResponderEvent| ui.dragging.set(None))
}

/// The handles on the dividers of the split on screen.
pub(super) fn dividers(ui: Ui) -> impl IntoElement {
    let lines = move || if ui.wide() && ui.browsing() { ui.read(|s| s.shown_split().map(|s| s.dividers()).unwrap_or_default()) } else { Vec::new() };
    each(lines, |(divider, _): &(Divider, Share)| *divider == Divider::Column, move |line, _| grip(ui, line))
}

/// Dragging resizes; a double tap evens the panes out again.
fn grip(ui: Ui, line: ReadSignal<(Divider, Share)>) -> impl IntoElement {
    let divider = line.get_untracked().0;
    let column = divider == Divider::Column;
    let held = signal(false);
    let tapped = signal(f64::NEG_INFINITY);
    let resize = move |share: f32| {
        ui.act_on_screen(|s| {
            if let Some(active) = s.model.active {
                s.splits.resize(active, divider, share);
            }
            Vec::new()
        })
    };
    let pill = view().rounded_full().bg(move || if held.get() { Theme::accent() } else { Theme::faint() });
    view()
        .absolute()
        .z_index(1)
        .items_center()
        .justify_center()
        .style_with(move |s| {
            let (_, (x, y, w, h)) = line.get();
            let s = s.left(relative(x)).top(relative(y));
            if column { s.w(px(GRIP)).h(relative(h)).ml(px(-GRIP / 2.)) } else { s.w(relative(w)).h(px(GRIP)).mt(px(-GRIP / 2.)) }
        })
        .accessible(true)
        .label(if column { "Column divider" } else { "Row divider" })
        .test_id(if column { "column-divider" } else { "row-divider" })
        .claim_touches(TouchClaim::on_start())
        .on_responder_grant(move |_: &ResponderEvent| held.set(true))
        .on_responder_move(move |e: &ResponderEvent| {
            let (x, y, size) = in_area(ui, e);
            resize(if column { split::ratio_at(x, size.width.0) } else { split::ratio_at(y, size.height.0) });
        })
        .on_responder_release(move |e: &ResponderEvent| {
            held.set(false);
            let d = e.gesture.delta;
            if d.x.0.abs().max(d.y.0.abs()) > SLOP {
                return;
            }
            if e.timestamp - tapped.get_untracked() > DOUBLE_TAP_MS {
                return tapped.set(e.timestamp);
            }
            tapped.set(f64::NEG_INFINITY);
            ui.act_on_screen(|s| {
                if let Some(active) = s.model.active {
                    s.splits.equalise(active);
                }
                Vec::new()
            });
        })
        .on_responder_terminate(move |_: &ResponderEvent| held.set(false))
        .child(if column { pill.w(px(4.)).h(px(36.)) } else { pill.w(px(36.)).h(px(4.)) })
}
