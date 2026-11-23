use browser_core::tabs::TabKey;
use nsh_core::prelude::*;
use serde_json::{Value, json};
use tarn_nsh_shared::page_overlay::{self, Outcome, Overlay};

use super::{PANE_GAP, PANE_HEADER, Screen, Ui};
use crate::shell::Effect;

/// Where `key`'s page sits in the page area, in points: all of it, or its split pane under the header.
fn page_rect(ui: Ui, key: TabKey) -> (f32, f32, f32, f32) {
    let size = ui.area.get().size;
    let (width, height) = (size.width.0, size.height.0);
    match ui.pane(key) {
        Some((x, y, w, h)) => {
            let inset = PANE_GAP + 1.;
            (x * width + inset, y * height + inset + PANE_HEADER, w * width - 2. * inset, h * height - 2. * inset - PANE_HEADER)
        }
        None => (0., 0., width, height),
    }
}

fn shown_key(ui: Ui) -> Option<TabKey> {
    let key = ui.page_overlay.with(|o| o.as_ref().map(|o| o.key))?;
    (ui.screen.get() == Screen::Browse && ui.read(|s| s.model.active) == Some(key)).then_some(key)
}

/// The active tab and the size of its visible page in CSS pixels.
fn active_page(ui: Ui) -> Option<(TabKey, (f64, f64))> {
    let key = untracked(|| ui.read(|s| s.model.active))?;
    let (_, _, w, h) = untracked(|| page_rect(ui, key));
    let zoom = untracked(|| ui.read(|s| s.pages.get(&key).map(|p| p.zoom))).filter(|z| *z > 0.).unwrap_or(1.);
    (w > 0. && h > 0.).then(|| (key, (f64::from(w) / zoom, f64::from(h) / zoom)))
}

pub fn capture_element(ui: Ui) {
    ui.show(Screen::Browse);
    if let Some((_, page)) = active_page(ui) {
        ui.act_on_screen(|s| s.capture_element(page));
    }
}

pub fn capture_area(ui: Ui) {
    ui.show(Screen::Browse);
    let Some((key, page)) = active_page(ui) else { return };
    let mut seq = 0;
    ui.act_on_screen(|s| {
        seq = s.next_seq();
        vec![Effect::Command(key, "snapshotRect", json!({ "seq": seq, "x": 0, "y": 0, "width": page.0, "height": page.1 }))]
    });
    ui.page_overlay.set(Some(Overlay::area(key, seq, page)));
}

pub fn shown(ui: Ui, key: TabKey, data: &Value) {
    let mut failed = false;
    ui.page_overlay.update(|o| failed = o.as_mut().and_then(|o| o.shown(key, data["seq"].as_u64(), data["png"].as_str())) == Some(false));
    if failed {
        ui.page_overlay.set(None);
        ui.act_on_screen(|s| {
            s.notice = Some("The page could not be captured.".into());
            Vec::new()
        });
    }
}

/// The overlay over the page of its tab; it closes when that tab or the browsing screen goes.
pub fn layer(ui: Ui) -> AnyElement {
    effect(move || {
        if ui.page_overlay.with(Option::is_some) && shown_key(ui).is_none() {
            untracked(|| ui.page_overlay.set(None));
        }
    });
    let rect = move || shown_key(ui).map_or((0., 0., 0., 0.), |key| page_rect(ui, key));
    let done = move |outcome: Outcome| {
        ui.page_overlay.set(None);
        if let Outcome::Capture(key, [x, y, width, height]) = outcome {
            ui.act(|s| s.screenshot(key, json!({ "x": x, "y": y, "width": width, "height": height })));
        }
    };
    view()
        .absolute()
        .z_index(2)
        .left(move || px(rect().0))
        .top(move || px(rect().1))
        .w(move || px(rect().2))
        .h(move || px(rect().3))
        .pointer_events(PointerEvents::BoxNone)
        .child(page_overlay::layer(ui.page_overlay, browser_core::capture::AREA_HINT_TOUCH, done))
        .into_element()
}
