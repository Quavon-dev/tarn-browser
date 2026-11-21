use std::time::Instant;

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
pub use browser_core::hover::GRACE;
use browser_core::hover::{CLOSE_DELAY, Hover};

/// The card starts below the toolbar, so the window's own traffic lights stay above it.
const CARD_TOP: f32 = TOOLBAR_HEIGHT;
/// How long after a hand-back the main window's next activation still counts as that hand-back.
pub(crate) const HAND_BACK: std::time::Duration = std::time::Duration::from_secs(1);

type Origin = (Bounds<Pixels>, Option<DisplayId>);

#[derive(Default)]
pub struct Peek {
    hover: Hover,
    pub(crate) window: Option<WindowHandle<PeekWindow>>,
    pub(crate) main: Option<WindowHandle<Browser>>,
    origin: Option<Origin>,
    timer: Option<Task<()>>,
    pointer: Option<(engine::menu::EventMonitor, Task<()>)>,
    pub(crate) owns: [bool; 2],
    /// When the main window last took the keyboard back from this panel, which had it only for a click.
    pub(crate) handed_back: Option<Instant>,
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
        let [library, downloads] = self.peek.owns;
        !self.sidebar_open && (self.library.is_none() || library) && (!self.downloads_popover.is_open() || downloads)
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
        if !self.peek.hover.open && !(engine::app_active() && window.is_window_active()) {
            self.peek.hover.closed();
            self.peek.timer = None;
            return;
        }
        let reach = SIDEBAR_MARGIN + self.sidebar_width + GRACE;
        let height = f32::from(window.bounds().size.height);
        if self.peek.hover.moved(x, y, pressed || cx.has_active_drag(), reach, CARD_TOP..height, Instant::now()) {
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
        if cx.active_window().map(|w| w.window_id()) != self.peek.main.map(|m| m.window_id()) {
            return self.peek.hover.closed();
        }
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
            let (handle, _) = ui::controls::open_popup(cx, bounds, display, false, inset, ui::theme::Theme::radius_tile(), |window, cx| {
                PeekWindow::new(browser.clone(), window, cx)
            });
            browser.update(cx, |b, _| b.peek.window = handle).ok();
        });
    }

    pub(crate) fn main_bounds(&self, window: &Window) -> Bounds<Pixels> {
        self.peek.origin.filter(|_| self.peek.is_window(window)).map_or_else(|| window.bounds(), |(bounds, _)| bounds)
    }

    pub fn close_peek(&mut self, cx: &mut Context<Self>) {
        self.motion.forget_pointer();
        self.peek.hover.closed();
        self.peek.timer = None;
        if let Some(handle) = self.peek.window.take() {
            if self.peek.owns[0] {
                self.close_library(cx);
            }
            if self.peek.owns[1] {
                self.close_downloads(cx);
            }
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
        self.link_status.popup = None;
        let popups: [Option<AnyWindowHandle>; 11] = [
            self.space_popover.take().map(|(p, _)| p.into()),
            self.hover_card.window.take().map(Into::into),
            self.site_popover.window.take().map(Into::into),
            self.link_peek.window.take().map(Into::into),
            self.peek.window.take().map(Into::into),
            self.library.take().map(Into::into),
            self.downloads_popover.window.take().map(Into::into),
            self.settings_window.take().map(Into::into),
            self.qr.take().map(Into::into),
            self.toast.window.take().map(Into::into),
            self.link_status.window.take().map(Into::into),
        ];
        for handle in popups.into_iter().flatten() {
            handle.update(cx, |_, window, _| window.remove_window()).ok();
        }
    }
}

pub(crate) fn popovers_over_peek(browser: &WeakEntity<Browser>, cx: &mut App) {
    let Some((peek, popovers)) = browser.upgrade().and_then(|b| {
        let b = b.read(cx);
        let [library, downloads] = b.peek.owns;
        Some((
            b.peek.window?,
            [b.library.filter(|_| library).map(AnyWindowHandle::from), b.downloads_popover.window.filter(|_| downloads).map(AnyWindowHandle::from)],
        ))
    }) else {
        return;
    };
    let host = |handle: AnyWindowHandle, cx: &mut App| handle.update(cx, |_, window, _| engine::Host::from_window(window)).ok().flatten();
    let Some(peek) = host(peek.into(), cx) else { return };
    for popover in popovers.into_iter().flatten().filter_map(|p| host(p, cx)) {
        peek.follow(&popover);
    }
}

pub struct PeekWindow {
    browser: WeakEntity<Browser>,
    closing: bool,
    _subscriptions: Vec<Subscription>,
}

impl PeekWindow {
    fn new(browser: WeakEntity<Browser>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions: Vec<_> = browser.upgrade().map(|b| cx.observe(&b, |_, _, cx| cx.notify())).into_iter().collect();
        // A click makes this panel the key window; shortcuts and the menu bar belong to the main one unless a rename types here.
        subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                return;
            }
            let peek = window.window_handle().window_id();
            let browser = this.browser.clone();
            cx.defer(move |cx| {
                // A rename, menu or popup the same click opened (library, downloads, a space card) keeps the keyboard.
                if cx.active_window().map(|w| w.window_id()) != Some(peek) {
                    return;
                }
                let main = browser.update(cx, |b, _| {
                    let keyboard_here =
                        b.renaming.is_some() || b.menu_open || b.library.is_some() || b.downloads_popover.is_open() || b.space_popover.is_some();
                    let main = b.peek.main.filter(|_| !keyboard_here);
                    b.peek.handed_back = main.map(|_| Instant::now());
                    main
                });
                if let Ok(Some(main)) = main {
                    main.update(cx, |_, window, _| window.activate_window()).ok();
                }
            });
        }));
        PeekWindow { browser, closing: false, _subscriptions: subscriptions }
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
