use std::time::{Duration, Instant};

use browser_core::hover::{CLOSE_DELAY, EDGE_WIDTH, GRACE, Hover, Rest};
use browser_core::tabs::TabKey;
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use super::{Panel, REFS, SIDEBAR_WIDTH, Ui, read, sidebar};
use crate::controls::{body, small};
use crate::theme::{TOOLBAR_HEIGHT, Theme, themed};

const INSET: f32 = 8.;
const CARD_TOP: f32 = INSET + TOOLBAR_HEIGHT;
const WIDTH: f32 = 300.;
const HEIGHT: f32 = 96.;
const STATE_LINE: f32 = 16.;
const GAP: f32 = 8.;
const LIFT: f32 = 20.;
const PEEK_WIDTH: f32 = 400.;
const SHOT_WIDTH: f32 = PEEK_WIDTH - 24.;
const SHOT_HEIGHT: f32 = SHOT_WIDTH * 10. / 16.;
const POLL: Duration = Duration::from_millis(150);
const REFRESH: Duration = Duration::from_secs(1);

#[derive(Clone, Default, PartialEq)]
pub(super) struct Floating {
    hover: Hover,
    serial: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct Shown {
    key: TabKey,
    live: bool,
    title: String,
    place: String,
    state: String,
    shot: Option<String>,
}

#[derive(Clone, Default, PartialEq)]
pub(super) struct HoverCard {
    rest: Rest,
    y: f32,
    height: f32,
    shown: Option<Shown>,
    asked: Option<(TabKey, Instant)>,
    serial: u64,
}

pub(super) fn later(ui: Ui, wait: Duration, f: impl FnOnce() + 'static) {
    ui.root.run(|| drop(set_timeout(wait, f)));
}

fn alt_held() -> bool {
    use webkit6::gdk;
    use webkit6::prelude::*;
    let keyboard = gdk::Display::default().and_then(|d| d.default_seat()).and_then(|s| s.keyboard());
    keyboard.is_some_and(|k| k.modifier_state().contains(gdk::ModifierType::ALT_MASK))
}

fn busy(ui: Ui) -> bool {
    ui.panel.get_untracked() != Panel::None || ui.dragging.get_untracked().is_some()
}

fn moved(ui: Ui, x: f32, y: f32, pressed: bool) {
    let reach = INSET + SIDEBAR_WIDTH + GRACE;
    let mut changed = false;
    ui.floating.update(|f| changed = f.hover.moved(x, y, pressed || ui.dragging.get_untracked().is_some(), reach, CARD_TOP..f32::MAX, Instant::now()));
    if changed {
        schedule(ui);
    }
}

fn card_hovered(ui: Ui, inside: bool) {
    ui.floating.update(|f| f.hover.card(inside, Instant::now()));
    schedule(ui);
}

fn schedule(ui: Ui) {
    let mut serial = 0;
    ui.floating.update(|f| {
        f.serial += 1;
        serial = f.serial;
    });
    let Some((open, wait)) = ui.floating.with_untracked(|f| f.hover.next(Instant::now())) else { return };
    let wait = match (wait.is_zero(), open) {
        (true, true) => return ui.floating.update(|f| f.hover.opened()),
        (true, false) if !busy(ui) => return close(ui),
        (true, false) => CLOSE_DELAY,
        (false, _) => wait,
    };
    later(ui, wait, move || {
        if ui.floating.with_untracked(|f| f.serial) == serial {
            schedule(ui);
        }
    });
}

pub(super) fn close(ui: Ui) {
    ui.floating.update(|f| {
        f.hover.closed();
        f.serial += 1;
    });
}

pub(super) fn edge(ui: Ui) -> AnyElement {
    dynamic(move || {
        let shown = !ui.sidebar.get() && !ui.snap.with(|s| s.tabs_top) && !ui.floating.with(|f| f.hover.open);
        shown.then(|| {
            view()
                .absolute()
                .left(px(0.))
                .top(px(CARD_TOP))
                .bottom(px(0.))
                .w(px(EDGE_WIDTH))
                .on_mouse_move(move |e: &MouseEvent| moved(ui, e.page_position.x.0, e.page_position.y.0, e.pressed_button.is_some()))
                .on_mouse_leave(move |e: &MouseEvent| moved(ui, EDGE_WIDTH, e.page_position.y.0, false))
        })
    })
}

pub(super) fn floating(ui: Ui) -> AnyElement {
    dynamic(move || {
        let open = !ui.sidebar.get() && ui.floating.with(|f| f.hover.open);
        open.then(|| {
            view()
                .absolute()
                .left(px(INSET))
                .top(px(CARD_TOP))
                .bottom(px(INSET))
                .pr(px(GRACE))
                .flex_col()
                .on_mouse_enter(move |_: &MouseEvent| card_hovered(ui, true))
                .on_mouse_leave(move |_: &MouseEvent| card_hovered(ui, false))
                .child(themed(move || sidebar(ui).flex_1()))
        })
    })
}

pub(super) fn hover_row(ui: Ui, key: TabKey, hovered: bool, y: f32) {
    if !hovered {
        let mut left = false;
        ui.hover_card.update(|c| left = c.rest.leave(key));
        if left {
            hide(ui);
        }
        return;
    }
    if busy(ui) {
        return;
    }
    ui.hover_card.update(|c| {
        c.y = y;
        if c.rest.enter(key, Instant::now()) {
            c.shown = None;
        }
    });
    poll(ui);
}

fn poll(ui: Ui) {
    let mut serial = 0;
    ui.hover_card.update(|c| {
        c.serial += 1;
        serial = c.serial;
    });
    let Some((key, wait)) = ui.hover_card.with_untracked(|c| c.rest.due(Instant::now())) else { return };
    let held = alt_held();
    let live = held && REFS.with_borrow(|refs| refs.contains_key(&(ui.window, key)));
    let next = if held || wait.is_zero() { POLL } else { wait.min(POLL) };
    later(ui, next, move || {
        if ui.hover_card.with_untracked(|c| c.serial) == serial {
            poll(ui);
        }
    });
    if !wait.is_zero() && !held {
        return;
    }
    if live {
        request_shot(ui, key);
    }
    if ui.hover_card.with_untracked(|c| c.shown.as_ref().is_some_and(|s| s.key == key && s.live == live)) {
        return;
    }
    let Some((title, place, state)) = read(ui, |b| lines(ui, b, key)).flatten() else { return };
    ui.hover_card.update(|c| c.shown = Some(Shown { key, live, title, place, state, shot: None }));
}

fn lines(ui: Ui, b: &crate::browser::Browser, key: TabKey) -> Option<(String, String, String)> {
    let tab = b.model.tab(key)?;
    let url = b.pages.get(&key).map(|p| p.url.clone()).filter(|u| !u.is_empty()).unwrap_or_else(|| tab.url.clone());
    let asleep =
        ui.snap.with_untracked(|s| s.rows.iter().chain(s.rows.iter().flat_map(|r| &r.panes)).any(|r| r.kind == crate::snapshot::RowKind::Tab(key) && r.asleep));
    let (place, state) = browser_core::hover::card_lines(&url, asleep, b.tab_muted(key));
    Some((crate::snapshot::tab_label(tab, b, key), place, state))
}

fn request_shot(ui: Ui, key: TabKey) {
    if ui.hover_card.with_untracked(|c| c.asked.is_some_and(|(k, at)| k == key && at.elapsed() < REFRESH)) {
        return;
    }
    let Some(web) = REFS.with_borrow(|refs| refs.get(&(ui.window, key)).map(|(_, r)| *r)) else { return };
    ui.hover_card.update(|c| c.asked = Some((key, Instant::now())));
    web.command("snapshot", JsonValue::Null);
}

pub(super) fn shot(ui: Ui, key: TabKey, png: &JsonValue) {
    let Some(png) = png.as_str().filter(|p| p.starts_with("data:image/png;base64,")) else { return };
    ui.hover_card.update(|c| {
        if let Some(shown) = c.shown.as_mut().filter(|s| s.key == key && s.live) {
            shown.shot = Some(png.to_owned());
        }
    });
}

pub(super) fn hide(ui: Ui) {
    ui.hover_card.update(|c| *c = HoverCard { height: c.height, ..HoverCard::default() });
}

pub(super) fn window_height(ui: Ui, height: f32) {
    if ui.hover_card.with_untracked(|c| c.height) != height {
        ui.hover_card.update(|c| c.height = height);
    }
}

pub(super) fn card(ui: Ui) -> AnyElement {
    let shown = computed(move || ui.hover_card.with(|c| c.shown.clone().map(|s| (s, c.y, c.height))));
    themed(move || {
        let (shown, y, height) = shown.get()?;
        let (width, mut tall) = if shown.live { (PEEK_WIDTH, HEIGHT + SHOT_HEIGHT + 8.) } else { (WIDTH, HEIGHT) };
        if shown.state.is_empty() {
            tall -= STATE_LINE;
        }
        let top = (y - LIFT).min(height - tall - GAP).max(0.);
        let frame = shown.live.then(|| {
            let frame = view().w(px(SHOT_WIDTH)).h(px(SHOT_HEIGHT)).mb_1().flex_none().rounded(px(Theme::radius_row())).overflow_hidden().bg(Theme::hover());
            match shown.shot.clone() {
                Some(png) => frame.child(image(png).w_full().h_full().object_fit(ResizeMode::Cover)),
                None => frame,
            }
        });
        Some(
            view()
                .absolute()
                .left(px(INSET + SIDEBAR_WIDTH + GAP))
                .top(px(top))
                .w(px(width))
                .pointer_events(PointerEvents::None)
                .px_3()
                .py_2()
                .flex_col()
                .gap_1()
                .rounded(px(Theme::radius_panel()))
                .bg(Theme::panel())
                .border_1()
                .border_color(Theme::surface_edge())
                .shadow(Theme::elevation(10., 30.))
                .children(frame)
                .child(body(shown.title).font_weight(FontWeight::MEDIUM).number_of_lines(2_u32))
                .child(small(shown.place).text_color(Theme::muted()).truncate())
                .children((!shown.state.is_empty()).then(|| small(shown.state).text_color(Theme::faint()))),
        )
    })
}
