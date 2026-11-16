use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AppContext, Bounds, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, StatefulInteractiveElement, Styled, Subscription,
    WeakEntity, Window, WindowHandle, canvas, div, point, px, size,
};
use ui::controls::surface;
use ui::motion::{EXIT, FADE, fade_in, progress};
use ui::theme::{TEXT, Theme};

use crate::browser::Browser;
use crate::content::{SHOWN_SUGGESTIONS, SUGGESTION_ROW, suggestion_row};
use crate::peek::popup_options;
use crate::prefs::Pref;

const PAD: f32 = 6.;
const GAP: f32 = 6.;

/// The address field opened in the toolbar, with its suggestions in a popup window below it,
/// since GPUI cannot draw over the page.
#[derive(Default)]
pub struct Address {
    opened: Option<Instant>,
    closing: Option<Instant>,
    /// The field's resting left and right edges, which it grows from and folds back to, from the
    /// bar's centre (classic) or right end (compact), so a sidebar toggle while open moves them too.
    from: (f32, f32),
    /// The toolbar, the field and the slot it grows into, as last painted.
    pub(crate) bar: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) field: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) slot: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) popup: Option<(WindowHandle<Suggestions>, engine::Host)>,
    opening: bool,
    placed: Option<(Bounds<Pixels>, usize)>,
    clicks: Option<engine::menu::EventMonitor>,
}

/// Records where an element was painted.
pub fn measure(cell: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {}).absolute().inset_0()
}

impl Browser {
    pub fn inline_editing(&self) -> bool {
        self.command_mode.is_some() && self.address.opened.is_some() && self.model.active.is_some()
    }

    /// How far the field has grown, 0 at rest and 1 at the slot's width; `None` when it is at rest.
    pub fn address_open(&self) -> Option<(f32, bool)> {
        if self.inline_editing() {
            let t = self.address.opened.filter(|_| !self.pref(Pref::ReduceMotion)).map_or(1., |at| progress(at, FADE));
            return Some((t, true));
        }
        let at = self.address.closing.filter(|at| at.elapsed() < EXIT)?;
        Some((1. - progress(at, EXIT), false))
    }

    fn address_anchor(&self) -> f32 {
        let bar = self.address.bar.get();
        f32::from(if self.pref(Pref::CompactToolbar) { bar.right() } else { bar.center().x })
    }

    /// The resting field's left and right edges in the window. The compact field rests where the slot starts.
    fn address_rest(&self) -> (f32, f32) {
        let (anchor, (left, right)) = (self.address_anchor(), self.address.from);
        let left = if self.pref(Pref::CompactToolbar) { f32::from(self.address.slot.get().origin.x) } else { anchor + left };
        (left, anchor + right)
    }

    pub fn address_width(&self, t: f32) -> f32 {
        let (origin, slot) = (f32::from(self.address.slot.get().origin.x), f32::from(self.address.slot.get().size.width));
        if slot <= 0. {
            return self.address.from.1 - self.address.from.0;
        }
        let (left, right) = self.address_rest();
        let from = (right - left).min(slot - (left - origin).max(0.)).max(0.);
        from + (slot - from) * t
    }

    /// How far the field's left edge sits inside the slot; `None` until the slot is measured.
    pub fn address_offset(&self, t: f32) -> Option<f32> {
        let slot = self.address.slot.get();
        (slot.size.width > px(0.)).then(|| ((self.address_rest().0 - f32::from(slot.origin.x)) * (1. - t)).max(0.))
    }

    pub(crate) fn address_moving(&self) -> bool {
        self.address.opened.is_some_and(|at| at.elapsed() < FADE) || self.address.closing.is_some_and(|at| at.elapsed() < EXIT)
    }

    /// Opens the field in place; the page stays visible. A click anywhere else in the window closes it.
    pub(crate) fn open_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.address.opened.is_some() {
            return;
        }
        let (field, anchor) = (self.address.field.get(), self.address_anchor());
        self.address.from = (f32::from(field.left()) - anchor, f32::from(field.right()) - anchor);
        self.address.opened = Some(Instant::now());
        self.address.closing = None;
        self.address.slot.set(Bounds::default());
        self.address.placed = None;
        let (tx, mut rx) = mpsc::unbounded();
        self.address.clicks = self.host.as_ref().and_then(|h| h.on_mouse_down(move |x, y| drop(tx.unbounded_send((x, y)))));
        cx.spawn_in(window, async move |this, cx| {
            while let Some((x, y)) = rx.next().await {
                let inside = |b: &Browser| b.address.field.get().contains(&point(px(x as f32), px(y as f32)));
                if this
                    .update_in(cx, |b, window, cx| {
                        if b.inline_editing() && !inside(b) {
                            b.close_command(window, cx)
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn close_inline(&mut self, cx: &mut Context<Self>) {
        if self.address.opened.take().is_some() && !self.pref(Pref::ReduceMotion) {
            self.address.closing = Some(Instant::now());
        }
        self.address.clicks = None;
        self.close_suggestions(cx);
    }

    fn close_suggestions(&mut self, cx: &mut Context<Self>) {
        self.address.placed = None;
        if let Some((popup, _)) = self.address.popup.take() {
            cx.defer(move |cx| {
                popup.update(cx, |_, window, _| window.remove_window()).ok();
            });
        }
    }

    /// Keeps the suggestions popup under the open field, sized to its rows.
    pub(crate) fn sync_suggestions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let slot = self.address.slot.get();
        if !self.inline_editing() || self.suggestions.is_empty() {
            return self.close_suggestions(cx);
        }
        if slot.size.width <= px(0.) || self.address.opening || self.address.placed == Some((slot, self.suggestions.len())) {
            return;
        }
        let height = PAD * 2. + SUGGESTION_ROW * self.suggestions.len().min(SHOWN_SUGGESTIONS) as f32;
        let (x, y, width) = (f32::from(slot.origin.x), f32::from(slot.bottom()) + GAP, f32::from(slot.size.width));
        if let (Some(host), Some(popup)) = (self.host.clone(), self.address.popup.as_ref().map(|p| p.1.clone())) {
            self.address.placed = Some((slot, self.suggestions.len()));
            // GPUI handles the resize this causes only outside an update, so the frame is set from a task.
            cx.spawn(async move |_, _| host.place(&popup, x.into(), y.into(), width.into(), height.into())).detach();
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>() else { return };
        let origin = window.bounds().origin;
        let bounds = Bounds::new(point(origin.x + px(x), origin.y + px(y)), size(px(width), px(height)));
        let display = window.display(cx).map(|d| d.id());
        let browser = cx.entity().downgrade();
        self.address.opening = true;
        cx.defer(move |cx| {
            let mut popup_host = None;
            let handle = cx.open_window(popup_options(bounds, display, false), |window, cx| {
                popup_host = engine::Host::from_window(window);
                if let Some(host) = &popup_host {
                    host.round_corners(0., 0., 0., 0., f64::from(ui::theme::Theme::radius_panel()));
                }
                cx.new(|cx| Suggestions::new(browser.clone(), main, cx))
            });
            browser
                .update(cx, |b, cx| {
                    b.address.opening = false;
                    match (handle, popup_host) {
                        (Ok(handle), Some(popup)) if b.inline_editing() => {
                            b.address.popup = Some((handle, popup));
                            b.address.placed = None;
                            cx.notify();
                        }
                        (Ok(handle), _) => cx.defer(move |cx| {
                            handle.update(cx, |_, window, _| window.remove_window()).ok();
                        }),
                        _ => {}
                    }
                })
                .ok();
        });
    }
}

/// The suggestion list below the open address field. It never takes keyboard focus; the field
/// keeps it, and a click picks a row on mouse-down.
pub struct Suggestions {
    browser: WeakEntity<Browser>,
    main: WindowHandle<Browser>,
    _observe: Option<Subscription>,
}

impl Suggestions {
    fn new(browser: WeakEntity<Browser>, main: WindowHandle<Browser>, cx: &mut Context<Self>) -> Self {
        let observe = browser.upgrade().map(|b| cx.observe(&b, |_, _, cx| cx.notify()));
        Suggestions { browser, main, _observe: observe }
    }
}

impl Render for Suggestions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let b = browser.read(cx);
        let main = self.main;
        let rows = b.suggestions.iter().enumerate().map(|(ix, s)| {
            suggestion_row(b, ix, s).on_mouse_down(MouseButton::Left, move |_, _, cx| {
                main.update(cx, |b, window, cx| b.submit_command(ix, window, cx)).ok();
            })
        });
        let list = div().id("suggestion-rows").track_scroll(&b.suggestions_scroll).size_full().overflow_y_scroll().children(rows);
        let card = surface(Theme::radius_panel()).size_full().py(px(PAD)).text_size(px(TEXT)).text_color(Theme::text()).child(list);
        fade_in(card, "suggestions", b.pref(Pref::ReduceMotion))
    }
}
