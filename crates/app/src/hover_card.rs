use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use futures::future::{Either, select};
use gpui::prelude::FluentBuilder;
use gpui::{
    AppContext, Bounds, Context, FontWeight, IntoElement, ObjectFit, ParentElement, Render, RenderImage, SharedString, Styled, StyledImage, Task, Window,
    WindowHandle, div, img, point, px, size,
};
use ui::controls::{POPOVER_INSET, open_popup, popover};
use ui::motion::rise_in;
use ui::text::OneLine;
use ui::theme::{TEXT, TEXT_SMALL, Theme};

use browser_core::hover::Rest;

use crate::browser::{Browser, TabKey};
use crate::prefs::Pref;
use crate::sidebar::SIDEBAR_MARGIN;

const WIDTH: f32 = 300.;
/// Segoe UI, the Windows font, sets lines taller than macOS's.
const HEIGHT: f32 = if cfg!(windows) { 108. } else { 96. };
const GAP: f32 = 8.;
/// The state line ("Sleeping · Muted") and its gap; a card without one is that much shorter.
const STATE_LINE: f32 = 16.;
/// The larger card shown while ⌥ is held, with a snapshot of the page above the lines.
const PEEK_WIDTH: f32 = 400.;
const SHOT_WIDTH: f32 = PEEK_WIDTH - 24.;
const SHOT_HEIGHT: f32 = SHOT_WIDTH * 10. / 16.;
/// How often a resting pointer checks ⌥, and how old a snapshot gets before it is taken again.
const POLL: Duration = Duration::from_millis(150);
const REFRESH: Duration = Duration::from_secs(1);
/// Above the pointer, so the card's first line sits level with the hovered row.
const LIFT: f32 = 20.;

#[derive(Default)]
pub struct HoverCard {
    rest: Rest,
    /// Where the card opens, its display, and the bottom edge of the window it must stay above.
    anchor: Option<(Bounds<gpui::Pixels>, Option<gpui::DisplayId>, gpui::Pixels)>,
    pub(crate) window: Option<WindowHandle<CardWindow>>,
    timer: Option<Task<()>>,
    /// Whether the open card is the ⌥ peek.
    live: bool,
    /// The last snapshot, of one tab only, and when the last one was asked for.
    shot: Option<(TabKey, Arc<RenderImage>)>,
    asked: Option<(TabKey, Instant)>,
    waiting: bool,
}

/// A PNG snapshot as the BGRA image GPUI draws, kept out of GPUI's image cache so it can be dropped.
fn decode(png: &[u8]) -> Option<Arc<RenderImage>> {
    let mut rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.into_rgba8();
    for pixel in rgba.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new([image::Frame::new(rgba)])))
}

/// The card's lines: title, host and path with the host shortened from the left (S5), and state.
fn lines(b: &Browser, key: TabKey) -> Option<(String, String, String)> {
    let tab = b.model.tab(key)?;
    let url = b.tab_url(key).unwrap_or_else(|| tab.url.clone());
    let (place, state) = browser_core::hover::card_lines(&url, b.sleeping(key), b.tab_muted(key));
    Some((crate::tabs::display_title(tab), place, state))
}

impl Browser {
    /// Starts or ends the wait for a sidebar row's card; the pointer's place is read now, so the card opens beside the row.
    pub fn hover_tab(&mut self, key: TabKey, hovered: bool, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let motion = &mut self.motion;
        if hovered && motion.hovered.is_none_or(|(k, ..)| k != key) {
            let from = if motion.near_rest(window.mouse_position()) { 1. } else { motion.grown(key) };
            motion.rested = None;
            if let Some((k, ..)) = motion.hovered {
                motion.leave(k, now, motion.grown(k));
            }
            motion.hovered = Some((key, now, from));
            cx.notify();
        } else if !hovered && motion.hovered.is_some_and(|(k, ..)| k == key) {
            motion.leave(key, now, motion.grown(key));
            cx.notify();
        }
        if !hovered {
            if self.hover_card.rest.leave(key) {
                self.hide_hover_card(cx);
            }
            return;
        }
        if self.menu_open || self.renaming.is_some() || cx.has_active_drag() {
            return;
        }
        let origin = window.bounds().origin;
        let x = origin.x + px(SIDEBAR_MARGIN + self.sidebar_width + GAP);
        let y = origin.y + window.mouse_position().y - px(LIFT);
        let bottom = window.bounds().bottom();
        self.hover_card.anchor = Some((Bounds::new(point(x, y), size(px(WIDTH), px(HEIGHT))), window.display(cx).map(|d| d.id()), bottom));
        if self.hover_card.rest.enter(key, Instant::now())
            && let Some(handle) = self.hover_card.window.take()
        {
            self.hover_card.shot = None;
            cx.defer(move |cx| {
                handle.update(cx, |_, window, _| window.remove_window()).ok();
            });
        }
        self.schedule_hover_card(window, cx);
    }

    /// Runs while the pointer rests on a row: opens the card once it is due, at once and as the
    /// larger peek while ⌥ is held, and swaps between the two as ⌥ goes down or up. ⌥ is read
    /// globally, so the card closes when another app is in front.
    fn schedule_hover_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shell_active(window) {
            self.hide_hover_card(cx);
            return;
        }
        let Some((key, wait)) = self.hover_card.rest.due(Instant::now()) else { return };
        let held = engine::option_held();
        let live = held && self.views.get(&key).is_some_and(|v| !v.is_chromium());
        let next = if held || wait.is_zero() { POLL } else { wait.min(POLL) };
        self.hover_card.timer = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(next).await;
            this.update_in(cx, |b, window, cx| b.schedule_hover_card(window, cx)).ok();
        }));
        if let Some(handle) = self.hover_card.window.take_if(|_| self.hover_card.live != live) {
            cx.defer(move |cx| {
                handle.update(cx, |_, window, _| window.remove_window()).ok();
            });
        }
        if !wait.is_zero() && !held || self.menu_open {
            return;
        }
        if live {
            self.request_shot(key, cx);
        }
        if self.hover_card.window.is_some() {
            return;
        }
        let (Some((mut bounds, display, bottom)), Some(lines)) = (self.hover_card.anchor, lines(self, key)) else { return };
        if live {
            bounds.size = size(px(PEEK_WIDTH), px(HEIGHT + SHOT_HEIGHT + 8.));
        }
        if lines.2.is_empty() {
            bounds.size.height -= px(STATE_LINE);
        }
        bounds.origin.y = bounds.origin.y.min(bottom - bounds.size.height - px(GAP));
        let bounds = bounds.dilate(px(POPOVER_INSET));
        let shot = self.hover_card.shot.as_ref().filter(|(k, _)| *k == key).map(|(_, image)| image.clone());
        self.hover_card.live = live;
        let reduce = self.pref(Pref::ReduceMotion);
        let browser = cx.entity().downgrade();
        cx.defer(move |cx| {
            let (handle, host) =
                open_popup(cx, bounds, display, false, [POPOVER_INSET; 4], Theme::radius_panel(), |_, _| CardWindow { lines, reduce, live, shot });
            if let Some(host) = host {
                host.ignore_mouse();
            }
            let Some(handle) = handle else { return };
            let kept = browser.update(cx, |b, _| {
                let current =
                    b.hover_card.rest.due(Instant::now()).is_some_and(|(k, _)| k == key) && b.hover_card.window.is_none() && b.hover_card.live == live;
                if current {
                    b.hover_card.window = Some(handle);
                }
                current
            });
            if !kept.unwrap_or(false) {
                handle.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
    }

    /// A fresh, scaled-down snapshot of an awake tab, at most one per `REFRESH` and one at a time;
    /// it is kept only while the pointer rests on the tab.
    fn request_shot(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if self.hover_card.waiting || self.hover_card.asked.is_some_and(|(k, at)| k == key && at.elapsed() < REFRESH) {
            return;
        }
        let Some(view) = self.views.get(&key) else { return };
        self.hover_card.asked = Some((key, Instant::now()));
        self.hover_card.waiting = true;
        let (weak, (tx, mut rx)) = (std::rc::Rc::downgrade(view), mpsc::unbounded());
        view.snapshot_png(Some(f64::from(SHOT_WIDTH)), move |png| drop(tx.unbounded_send(png)));
        cx.spawn(async move |this, cx| {
            // A page that never answers, such as one with a hung process, must not hold `waiting` for good.
            let timeout = std::pin::pin!(cx.background_executor().timer(crate::tabs::PROBE_TIMEOUT));
            let png = match select(rx.next(), timeout).await {
                Either::Left((png, _)) => png.flatten(),
                Either::Right(_) => None,
            };
            let image = match png {
                Some(png) => cx.background_spawn(async move { decode(&png) }).await,
                None => None,
            };
            this.update(cx, |b, cx| {
                b.hover_card.waiting = false;
                let Some(image) = image.filter(|_| b.same_view(key, &weak) && b.hover_card.rest.due(Instant::now()).is_some_and(|(k, _)| k == key)) else {
                    return;
                };
                b.hover_card.shot = Some((key, image.clone()));
                if let Some(handle) = b.hover_card.window.filter(|_| b.hover_card.live) {
                    cx.defer(move |cx| {
                        handle.update(cx, |card, window, cx| card.show(image, window, cx)).ok();
                    });
                }
            })
            .ok();
        })
        .detach();
    }

    /// Also drops the snapshot, so none outlives the hover or its (possibly private) tab.
    pub fn hide_hover_card(&mut self, cx: &mut Context<Self>) {
        let waiting = self.hover_card.waiting;
        let card = std::mem::take(&mut self.hover_card);
        self.hover_card.waiting = waiting;
        if let Some(handle) = card.window {
            cx.defer(move |cx| {
                handle.update(cx, |_, window, _| window.remove_window()).ok();
            });
        }
    }

    /// A row that disappears under the pointer never reports leaving, so a pointer past the sidebar closes the card too.
    pub(crate) fn hover_card_pointer(&mut self, x: f32, cx: &mut Context<Self>) {
        if self.hover_card.rest.due(Instant::now()).is_some() && x > SIDEBAR_MARGIN + self.sidebar_width {
            self.hide_hover_card(cx);
        }
    }
}

pub struct CardWindow {
    lines: (String, String, String),
    reduce: bool,
    live: bool,
    shot: Option<Arc<RenderImage>>,
}

impl CardWindow {
    /// Replaces the snapshot and frees the old one's texture, so a long peek holds one image.
    fn show(&mut self, image: Arc<RenderImage>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(old) = self.shot.replace(image) {
            cx.drop_image(old, Some(window));
        }
        cx.notify();
    }
}

impl Render for CardWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (title, place, state) = self.lines.clone();
        let card = popover(Theme::radius_panel(), POPOVER_INSET)
            .px_3()
            .py_2p5()
            .flex()
            .flex_col()
            .gap_1()
            .when(self.live, |d| {
                let frame =
                    div().w(px(SHOT_WIDTH)).h(px(SHOT_HEIGHT)).mb_1p5().flex_none().rounded(px(Theme::radius_row())).overflow_hidden().bg(Theme::hover());
                d.child(match self.shot.clone() {
                    Some(shot) => frame.child(img(shot).size_full().object_fit(ObjectFit::Cover)),
                    None => frame,
                })
            })
            .text_size(px(TEXT))
            .text_color(Theme::text())
            .child(div().flex_none().font_weight(FontWeight::MEDIUM).line_clamp(2).child(SharedString::from(title)))
            .child(div().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).one_line(SharedString::from(place)))
            .when(!state.is_empty(), |d| d.child(div().text_size(px(TEXT_SMALL)).text_color(Theme::faint()).child(SharedString::from(state))));
        rise_in(div().size_full().child(card), "hover-card", 4., self.reduce)
    }
}
