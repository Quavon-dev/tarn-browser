use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyWindowHandle, App, Bounds, Context, DisplayId, InteractiveElement, IntoElement, MouseExitEvent, ParentElement, Pixels, Render,
    StatefulInteractiveElement, Styled, Subscription, Task, WeakEntity, Window, WindowHandle, canvas, div, point, px, size,
};
pub use ui::controls::popup_options;
use ui::motion::EXIT;
use ui::theme::TOOLBAR_HEIGHT;

use crate::browser::Browser;
use crate::prefs::Pref;
use crate::sidebar::{self, SIDEBAR_MARGIN};

/// The invisible strip at the window's left edge that opens the floating sidebar.
const EDGE_WIDTH: f32 = 12.;
const OPEN_DELAY: Duration = Duration::from_millis(100);
const CLOSE_DELAY: Duration = Duration::from_millis(350);
/// Faster than this, in points per millisecond, the pointer is passing through, not resting.
const REST_SPEED: f32 = 0.5;
/// Room around the card that still counts as on it; the popup window includes it.
pub const GRACE: f32 = 16.;
/// The card starts below the toolbar, so the window's own traffic lights stay above it.
const CARD_TOP: f32 = TOOLBAR_HEIGHT;

/// The floating sidebar's hover rules, fed with pointer moves over the main window and with the
/// pointer entering or leaving the card's window.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Hover {
    pub open: bool,
    /// Since when the pointer has rested in the edge strip.
    rest: Option<Instant>,
    /// Since when the pointer has been off the open card.
    left: Option<Instant>,
    last: Option<(f32, f32, Instant)>,
}

impl Hover {
    /// A pointer move at `(x, y)` in main-window points, which may lie outside the window of
    /// `height`; `reach` is the card's right edge plus its grace margin. Returns whether a pending
    /// open or close changed.
    pub fn moved(&mut self, x: f32, y: f32, pressed: bool, reach: f32, height: f32, now: Instant) -> bool {
        let before = (self.rest, self.left);
        let fast = self.last.is_some_and(|(lx, ly, at)| (x - lx).hypot(y - ly) > REST_SPEED * (now - at).as_secs_f32().max(0.001) * 1000.);
        self.last = Some((x, y, now));
        if self.open {
            self.card((0. ..reach).contains(&x) && (CARD_TOP - GRACE..height).contains(&y), now);
        } else {
            let edge = (0. ..EDGE_WIDTH).contains(&x) && (CARD_TOP..height).contains(&y) && !pressed;
            self.rest = if edge && !fast { self.rest.or(Some(now)) } else { edge.then_some(now) };
        }
        before != (self.rest, self.left)
    }

    /// The pointer entered or left the card's window.
    pub fn card(&mut self, inside: bool, now: Instant) {
        self.left = if inside { None } else { self.left.or(Some(now)) };
    }

    fn opened(&mut self) {
        *self = Hover { open: true, last: self.last, ..Hover::default() };
    }

    fn closed(&mut self) {
        *self = Hover { last: self.last, ..Hover::default() };
    }

    /// Whether to open (`true`) or close, and how much longer to wait first.
    pub fn next(&self, now: Instant) -> Option<(bool, Duration)> {
        let wait = |since: Instant, delay: Duration| delay.saturating_sub(now - since);
        match (self.open, self.rest, self.left) {
            (false, Some(since), _) => Some((true, wait(since, OPEN_DELAY))),
            (true, _, Some(since)) => Some((false, wait(since, CLOSE_DELAY))),
            _ => None,
        }
    }
}

type Origin = (Bounds<Pixels>, Option<DisplayId>);

#[derive(Default)]
pub struct Peek {
    hover: Hover,
    pub(crate) window: Option<WindowHandle<PeekWindow>>,
    pub(crate) main: Option<WindowHandle<Browser>>,
    origin: Option<Origin>,
    timer: Option<Task<()>>,
    pointer: Option<(engine::menu::EventMonitor, Task<()>)>,
}

impl Peek {
    pub fn is_window(&self, window: &Window) -> bool {
        self.window.is_some_and(|h| h.window_id() == window.window_handle().window_id())
    }
}

/// Removes a popup window once its exit animation has played.
pub fn remove_after_exit<V: 'static>(reduce: bool, window: &mut Window, cx: &mut Context<V>) {
    if reduce {
        return window.remove_window();
    }
    cx.spawn_in(window, async move |_, cx| {
        cx.background_executor().timer(EXIT).await;
        cx.update(|window, _| window.remove_window()).ok();
    })
    .detach();
}

impl Browser {
    /// Hands `f` to the main window when `window` is the floating sidebar's: what it shows, focuses
    /// or subscribes to lives there, and the sidebar's window is gone a moment later.
    pub(crate) fn in_main(&self, window: &Window, cx: &mut Context<Self>, f: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) -> bool {
        let Some(main) = self.peek.main.filter(|_| self.peek.is_window(window)) else { return false };
        cx.defer(move |cx| drop(main.update(cx, f)));
        true
    }

    fn peek_enabled(&self) -> bool {
        !self.sidebar_open
    }

    /// Pointer moves over the main window, including over the page, which GPUI never sees.
    pub(crate) fn watch_peek_pointer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, mut rx) = mpsc::unbounded();
        let Some(monitor) = self.host.as_ref().and_then(|h| h.on_mouse_moved(move |x, y, pressed| drop(tx.unbounded_send((x as f32, y as f32, pressed)))))
        else {
            return;
        };
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some((x, y, pressed)) = rx.next().await {
                if this.update_in(cx, |b, window, cx| b.peek_pointer(x, y, pressed, window, cx)).is_err() {
                    break;
                }
            }
        });
        self.peek.pointer = Some((monitor, task));
    }

    fn peek_pointer(&mut self, x: f32, y: f32, pressed: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.hover_card_pointer(x, cx);
        if !self.peek_enabled() {
            return;
        }
        if !self.peek.hover.open && !engine::app_active() {
            self.peek.hover.closed();
            self.peek.timer = None;
            return;
        }
        let reach = SIDEBAR_MARGIN + self.sidebar_width + GRACE;
        let height = f32::from(window.bounds().size.height);
        if self.peek.hover.moved(x, y, pressed || cx.has_active_drag(), reach, height, Instant::now()) {
            self.peek.origin = Some((window.bounds(), window.display(cx).map(|d| d.id())));
            self.schedule_peek(cx);
        }
    }

    pub fn peek_card_hover(&mut self, hovered: bool, cx: &mut Context<Self>) {
        self.peek.hover.card(hovered, Instant::now());
        self.schedule_peek(cx);
    }

    fn peek_busy(&self, cx: &App) -> bool {
        self.renaming.is_some() || self.library.is_some() || self.downloads_popover.is_open() || self.menu_open || cx.has_active_drag()
    }

    /// Opens or closes once the hover rules' wait is over; a busy sidebar stays open and is asked again later.
    fn schedule_peek(&mut self, cx: &mut Context<Self>) {
        self.peek.timer = None;
        let Some((open, wait)) = self.peek.hover.next(Instant::now()) else { return };
        let wait = match (wait.is_zero(), open) {
            (true, true) => return self.open_peek(cx),
            (true, false) if !self.peek_busy(cx) => return self.close_peek(cx),
            (true, false) => CLOSE_DELAY,
            (false, _) => wait,
        };
        self.peek.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |b, cx| b.schedule_peek(cx)).ok();
        }));
    }

    /// The card's window spans from the main window's left edge, so the card slides in from it.
    fn open_peek(&mut self, cx: &mut Context<Self>) {
        let Some((main, display)) = self.peek.origin.filter(|_| self.peek_enabled() && self.peek.window.is_none()) else { return };
        let bounds = Bounds::new(
            point(main.origin.x, main.origin.y + px(CARD_TOP)),
            size(px(SIDEBAR_MARGIN + self.sidebar_width + GRACE), main.size.height - px(CARD_TOP)),
        );
        self.peek.hover.opened();
        let browser = cx.entity().downgrade();
        // The new window renders from the browser at once, so it must open after this update ends.
        cx.defer(move |cx| {
            let inset = [0., crate::sidebar::SIDEBAR_MARGIN, GRACE, crate::sidebar::SIDEBAR_MARGIN];
            let (handle, _) =
                ui::controls::open_popup(cx, bounds, display, false, inset, ui::theme::Theme::radius_tile(), |_, cx| PeekWindow::new(browser.clone(), cx));
            browser.update(cx, |b, _| b.peek.window = handle).ok();
        });
    }

    pub fn close_peek(&mut self, cx: &mut Context<Self>) {
        self.motion.forget_pointer();
        self.peek.hover.closed();
        self.peek.timer = None;
        if let Some(handle) = self.peek.window.take() {
            let reduce = self.pref(Pref::ReduceMotion);
            cx.defer(move |cx| {
                handle
                    .update(cx, |peek, window, cx| {
                        peek.closing = true;
                        cx.notify();
                        remove_after_exit(reduce, window, cx);
                    })
                    .ok();
            });
        }
    }

    /// Popups belong to this browser; they go with it instead of outliving the main window.
    pub(crate) fn close_popups(&mut self, cx: &mut App) {
        self.toast.popup = None;
        let popups: [Option<AnyWindowHandle>; 9] = [
            self.hover_card.window.take().map(Into::into),
            self.site_popover.window.take().map(Into::into),
            self.link_peek.window.take().map(Into::into),
            self.peek.window.take().map(Into::into),
            self.library.take().map(Into::into),
            self.downloads_popover.window.take().map(Into::into),
            self.settings_window.take().map(Into::into),
            self.qr.take().map(Into::into),
            self.toast.window.take().map(Into::into),
        ];
        for handle in popups.into_iter().flatten() {
            handle.update(cx, |_, window, _| window.remove_window()).ok();
        }
    }
}

pub struct PeekWindow {
    browser: WeakEntity<Browser>,
    closing: bool,
    _observe: Option<Subscription>,
}

impl PeekWindow {
    fn new(browser: WeakEntity<Browser>, cx: &mut Context<Self>) -> Self {
        let observe = browser.upgrade().map(|b| cx.observe(&b, |_, _, cx| cx.notify()));
        PeekWindow { browser, closing: false, _observe: observe }
    }
}

impl Render for PeekWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let height = window.bounds().size.height;
        let closing = self.closing;
        let card = browser.update(cx, |b, cx| sidebar::floating(b, height, closing, cx));
        let weak = browser.downgrade();
        // GPUI updates hover only on mouse moves, so leaving the window needs its exit event.
        let exit = canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let browser = weak.clone();
                window.on_mouse_event(move |_: &MouseExitEvent, _, _, cx| {
                    browser.update(cx, |b, cx| b.peek_card_hover(false, cx)).ok();
                });
            },
        );
        div()
            .id("peek")
            .size_full()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.browser.update(cx, |b, cx| b.peek_card_hover(*hovered, cx)).ok();
            }))
            .child(exit.absolute().size_full())
            .child(card)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REACH: f32 = 300.;
    const Y: f32 = 400.;
    const H: f32 = 800.;

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn opens_after_resting_in_the_edge_strip() {
        let (mut hover, t) = (Hover::default(), Instant::now());
        assert!(!hover.moved(200., Y, false, REACH, H, t));
        assert_eq!(hover.next(t), None);
        assert!(hover.moved(EDGE_WIDTH - 1., Y, false, REACH, H, at(t, 400)));
        assert_eq!(hover.next(at(t, 430)), Some((true, OPEN_DELAY - Duration::from_millis(30))));
        assert!(!hover.moved(EDGE_WIDTH - 2., Y + 1., false, REACH, H, at(t, 450)), "a slow drift keeps resting");
        assert_eq!(hover.next(at(t, 500)), Some((true, Duration::ZERO)));
    }

    #[test]
    fn ignores_fast_moves_buttons_and_the_toolbar() {
        let (mut hover, t) = (Hover::default(), Instant::now());
        hover.moved(4., Y, false, REACH, H, t);
        hover.moved(4., Y + 60., false, REACH, H, at(t, 20));
        assert_eq!(hover.next(at(t, 100)), Some((true, Duration::from_millis(20))), "a fast move restarts the wait");
        hover.moved(40., Y + 60., false, REACH, H, at(t, 30));
        assert_eq!(hover.next(at(t, 500)), None, "passing through never opens");
        hover.moved(4., Y, true, REACH, H, at(t, 600));
        assert_eq!(hover.next(at(t, 800)), None, "not while a button is down");
        hover.moved(4., CARD_TOP - 1., false, REACH, H, at(t, 900));
        assert_eq!(hover.next(at(t, 1100)), None, "not beside the traffic lights");
        hover.moved(-4., Y, false, REACH, H, at(t, 1200));
        hover.moved(4., H + 10., false, REACH, H, at(t, 1300));
        assert_eq!(hover.next(at(t, 1500)), None, "not from outside the window");
    }

    #[test]
    fn stays_open_on_the_card_and_its_margin_then_closes_after_leaving() {
        let (mut hover, t) = (Hover::default(), Instant::now());
        hover.opened();
        assert_eq!(hover.next(t), None);
        hover.moved(REACH - 1., Y, false, REACH, H, at(t, 10));
        assert_eq!(hover.next(at(t, 1000)), None);
        hover.moved(REACH + 50., Y, false, REACH, H, at(t, 1000));
        assert_eq!(hover.next(at(t, 1100)), Some((false, CLOSE_DELAY - Duration::from_millis(100))));
        hover.moved(REACH - 1., H + 10., false, REACH, H, at(t, 1150));
        assert!(hover.next(at(t, 1150)).is_some(), "below the window is off the card");
        hover.card(true, at(t, 1200));
        assert_eq!(hover.next(at(t, 2000)), None, "coming back cancels the close");
        hover.card(false, at(t, 2000));
        assert_eq!(hover.next(at(t, 2000 + CLOSE_DELAY.as_millis() as u64)), Some((false, Duration::ZERO)));
        hover.closed();
        assert_eq!(hover.next(at(t, 3000)), None);
    }
}
