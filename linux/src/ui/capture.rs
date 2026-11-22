use browser_core::tabs::TabKey;
use nsh_core::JsonValue;
use nsh_core::prelude::*;
use serde_json::json;
use tarn_nsh_shared::page_overlay::{self, Outcome, Purpose};

use super::{Ui, act, read, split};

/// The active tab and the size of its visible page in CSS pixels.
fn active_page(ui: Ui) -> Option<(TabKey, (f64, f64))> {
    let (key, (_, _, w, h)) = ui.snap.with_untracked(|s| s.views.iter().find(|v| v.active).map(|v| (v.key, split::view_rect(ui, v.active, v.pane))))?;
    let zoom = read(ui, |b| b.pages.get(&key).map(|p| p.zoom)).flatten().filter(|z| *z > 0.).unwrap_or(1.);
    (w > 0. && h > 0.).then(|| (key, (f64::from(w) / zoom, f64::from(h) / zoom)))
}

pub(super) fn capture_element(ui: Ui) {
    if let Some((_, page)) = active_page(ui) {
        act(ui, |b| b.capture_element(page));
    }
}

/// Capture Area or Measure over a still of the active page.
pub(super) fn open(ui: Ui, purpose: Purpose) {
    let Some((_, page)) = active_page(ui) else { return };
    let mut overlay = None;
    act(ui, |b| overlay = b.page_still(purpose, page));
    if overlay.is_some() {
        ui.page_overlay.set(overlay);
    }
}

pub(super) fn shown(ui: Ui, key: TabKey, data: &JsonValue) {
    let mut failed = false;
    ui.page_overlay.update(|o| failed = o.as_mut().and_then(|o| o.shown(key, data["seq"].as_u64(), data["png"].as_str())) == Some(false));
    if failed {
        ui.page_overlay.set(None);
        act(ui, |b| b.show_toast("The page could not be captured", None));
    }
}

/// Esc: whether an overlay was open to cancel.
pub(super) fn cancel(ui: Ui) -> bool {
    let open = ui.page_overlay.with_untracked(Option::is_some);
    ui.page_overlay.set(None);
    open
}

/// The overlay over the view of its tab; it closes when that tab stops being the one shown.
pub(super) fn layer(ui: Ui) -> AnyElement {
    let rect = move || {
        let key = ui.page_overlay.with(|o| o.as_ref().map(|o| o.key))?;
        ui.snap.with(|s| s.views.iter().find(|v| v.key == key && v.active).map(|v| split::view_rect(ui, v.active, v.pane)))
    };
    effect(move || {
        if ui.page_overlay.with(Option::is_some) && rect().is_none() {
            untracked(|| ui.page_overlay.set(None));
        }
    });
    let done = move |outcome: Outcome| {
        ui.page_overlay.set(None);
        if let Outcome::Capture(key, [x, y, width, height]) = outcome {
            act(ui, |b| b.screenshot(key, json!({ "x": x, "y": y, "width": width, "height": height })));
        }
    };
    view()
        .absolute()
        .left(move || px(rect().map_or(0., |r| r.0)))
        .top(move || px(rect().map_or(0., |r| r.1)))
        .w(move || px(rect().map_or(0., |r| r.2)))
        .h(move || px(rect().map_or(0., |r| r.3)))
        .pointer_events(PointerEvents::BoxNone)
        .child(page_overlay::layer(ui.page_overlay, browser_core::capture::AREA_HINT, done))
        .into_element()
}
