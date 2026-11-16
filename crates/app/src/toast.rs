use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    Bounds, Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString, Styled, Subscription, WeakEntity, Window, WindowHandle,
    div, point, prelude::FluentBuilder, px, size,
};
use ui::controls::{POPOVER_INSET, button, open_popup, popover};
use ui::motion::{EXIT, animate, fade_in};
use ui::theme::{TEXT, Theme};

use crate::browser::Browser;
use crate::prefs::Pref;
use crate::sidebar::SIDEBAR_MARGIN;

const TOAST_TIME: Duration = Duration::from_secs(5);
/// A toast with a button stays longer, so the action is still there after a glance away.
const ACTION_TIME: Duration = Duration::from_secs(10);
const WIDTH: f32 = 340.;
/// Room for the message beside the widest button, "Restart to Update".
const ACTION_WIDTH: f32 = 440.;
const HEIGHT: f32 = 40.;
/// Between the toast and the window's bottom edge.
const MARGIN: f32 = 24.;

#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    /// Reopens the closed-tab entry with this serial.
    Undo(u64),
    Reveal(PathBuf),
    RestartToUpdate,
}

impl Action {
    fn label(&self) -> &'static str {
        match self {
            Action::Undo(_) => "Undo",
            Action::Reveal(_) => ui::controls::REVEAL,
            Action::RestartToUpdate => "Restart to Update",
        }
    }
}

#[derive(Debug)]
pub struct Toast {
    pub text: SharedString,
    pub action: Option<Action>,
    serial: u64,
    pub closing: bool,
}

/// The window's toast and the PopUp window it shows in at the bottom centre of the page, since
/// GPUI cannot draw over the page.
#[derive(Default)]
pub struct Toasts {
    current: Option<Toast>,
    serial: u64,
    pub(crate) window: Option<WindowHandle<ToastWindow>>,
    pub(crate) popup: Option<engine::Host>,
    /// Where the popup sits in the window, so it follows the sidebar and window size.
    placed: (f32, f32, f32),
    opening: bool,
}

impl Toasts {
    /// Replaces any toast; returns the new toast's serial.
    fn show(&mut self, text: SharedString, action: Option<Action>) -> u64 {
        self.serial += 1;
        self.current = Some(Toast { text, action, serial: self.serial, closing: false });
        self.serial
    }

    /// Starts fading the toast `serial` out, unless a newer one replaced it.
    fn fade(&mut self, serial: u64) -> bool {
        let Some(toast) = self.current.as_mut().filter(|t| t.serial == serial) else { return false };
        toast.closing = true;
        true
    }

    /// Drops the toast `serial`, unless a newer one replaced it.
    fn expire(&mut self, serial: u64) -> bool {
        let current = self.current.as_ref().is_some_and(|t| t.serial == serial);
        if current {
            self.current = None;
        }
        current
    }

    /// The toast to show; an Undo toast only while its closed-tab entry is still the newest, so
    /// Undo always reopens something.
    pub fn live(&self, newest_closed: Option<u64>) -> Option<&Toast> {
        self.current.as_ref().filter(|t| !matches!(t.action, Some(Action::Undo(serial)) if newest_closed != Some(serial)))
    }
}

impl Browser {
    pub(crate) fn show_toast(&mut self, text: impl Into<SharedString>, action: Option<Action>, cx: &mut Context<Self>) {
        let wait = if action.is_some() { ACTION_TIME } else { TOAST_TIME };
        let serial = self.toast.show(text.into(), action);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let fade = this.update(cx, |b, cx| {
                cx.notify();
                (b.toast.fade(serial) && !b.pref(Pref::ReduceMotion)).then_some(EXIT)
            });
            if let Ok(Some(fade)) = fade {
                cx.background_executor().timer(fade).await;
            }
            this.update(cx, |b, cx| {
                // Closed here too, since a minimised window does not render to close it.
                if b.toast.expire(serial) {
                    b.close_toast_window(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn take_toast_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(toast) = self.toast.current.take() else { return };
        match toast.action {
            Some(Action::Undo(serial)) if self.model.closed_tabs.newest() == Some(serial) => self.reopen_closed_tab(window, cx),
            Some(Action::Reveal(path)) => engine::reveal_in_finder(&path),
            Some(Action::RestartToUpdate) => crate::shared::restart_to_update(cx),
            _ => {}
        }
        cx.notify();
    }

    /// Opens the toast's popup while a toast is live and closes it once none is.
    pub(crate) fn sync_toast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(width) = self.toast.live(self.model.closed_tabs.newest()).map(|t| if t.action.is_some() { ACTION_WIDTH } else { WIDTH }) else {
            return self.close_toast_window(cx);
        };
        if self.toast.opening {
            return;
        }
        let frame = window.bounds();
        let left = (self.sidebar_width + SIDEBAR_MARGIN) * self.sidebar_shown();
        let x = (left + (f32::from(frame.size.width) - left - width) / 2.).clamp(0., (f32::from(frame.size.width) - width).max(0.)) - POPOVER_INSET;
        let y = f32::from(frame.size.height) - HEIGHT - MARGIN - POPOVER_INSET;
        let (width, height) = (width + 2. * POPOVER_INSET, HEIGHT + 2. * POPOVER_INSET);
        if self.toast.window.is_some() {
            if let (Some(host), Some(popup)) = (self.host.clone(), self.toast.popup.clone())
                && self.toast.placed != (x, y, width)
            {
                self.toast.placed = (x, y, width);
                // GPUI handles the resize this causes only outside an update, so the frame is set from a task.
                cx.spawn(async move |_, _| host.place(&popup, x.into(), y.into(), width.into(), height.into())).detach();
            }
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>() else { return };
        self.toast.placed = (x, y, width);
        let bounds = Bounds::new(point(frame.origin.x + px(x), frame.origin.y + px(y)), size(px(width), px(height)));
        let display = window.display(cx).map(|d| d.id());
        let browser = cx.entity().downgrade();
        self.toast.opening = true;
        cx.defer(move |cx| {
            let (handle, popup) =
                open_popup(cx, bounds, display, false, [POPOVER_INSET; 4], Theme::radius_panel(), |_, cx| ToastWindow::new(browser.clone(), main, cx));
            let kept = browser.update(cx, |b, cx| {
                b.toast.opening = false;
                let live = b.toast.live(b.model.closed_tabs.newest()).is_some();
                if let (Some(handle), true) = (handle, live) {
                    if let (Some(host), Some(popup)) = (&b.host, &popup) {
                        host.follow(popup);
                    }
                    (b.toast.window, b.toast.popup) = (Some(handle), popup);
                    cx.notify();
                }
                live
            });
            if let (Ok(false) | Err(_), Some(handle)) = (kept, handle) {
                handle.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
    }

    fn close_toast_window(&mut self, cx: &mut Context<Self>) {
        self.toast.popup = None;
        if let Some(popup) = self.toast.window.take() {
            cx.defer(move |cx| {
                popup.update(cx, |_, window, _| window.remove_window()).ok();
            });
        }
    }
}

/// The toast's card. It never takes keyboard focus; its button acts on mouse-down.
pub struct ToastWindow {
    browser: WeakEntity<Browser>,
    main: WindowHandle<Browser>,
    _observe: Option<Subscription>,
}

impl ToastWindow {
    fn new(browser: WeakEntity<Browser>, main: WindowHandle<Browser>, cx: &mut Context<Self>) -> Self {
        let observe = browser.upgrade().map(|b| cx.observe(&b, |_, _, cx| cx.notify()));
        ToastWindow { browser, main, _observe: observe }
    }
}

impl Render for ToastWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let b = browser.read(cx);
        let Some(toast) = b.toast.live(b.model.closed_tabs.newest()) else { return div().into_any_element() };
        let main = self.main;
        let label = toast.action.as_ref().map(Action::label);
        let card = popover(Theme::radius_panel(), POPOVER_INSET)
            .pl_3()
            .pr(px(if label.is_some() { 4. } else { 12. }))
            .flex()
            .items_center()
            .gap_2()
            .text_size(px(TEXT))
            .text_color(Theme::text())
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(toast.text.clone()))
            .when_some(label, |d, label| {
                d.child(button("toast-action", label, true).on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    main.update(cx, |b, window, cx| b.take_toast_action(window, cx)).ok();
                }))
            });
        let (reduce, id) = (b.pref(Pref::ReduceMotion), toast.serial as usize);
        if toast.closing {
            animate(card, ("toast-out", id), EXIT, reduce, |el, t| el.opacity(1. - t)).into_any_element()
        } else {
            fade_in(card, ("toast-in", id), reduce).into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_timer_leaves_a_newer_toast_alone() {
        let mut toasts = Toasts::default();
        let first = toasts.show("Link copied".into(), None);
        let second = toasts.show("Screenshot saved".into(), Some(Action::Reveal(PathBuf::from("/tmp/a.png"))));
        assert!(!toasts.fade(first) && !toasts.expire(first));
        assert_eq!(toasts.live(None).map(|t| (t.text.as_ref(), t.closing)), Some(("Screenshot saved", false)));
        assert!(toasts.fade(second));
        assert!(toasts.live(None).is_some_and(|t| t.closing));
        assert!(toasts.expire(second));
        assert!(toasts.live(None).is_none());
    }

    #[test]
    fn undo_shows_only_while_its_entry_is_the_newest() {
        let mut toasts = Toasts::default();
        toasts.show("Tab closed".into(), Some(Action::Undo(7)));
        assert!(toasts.live(Some(7)).is_some());
        assert!(toasts.live(Some(8)).is_none());
        assert!(toasts.live(None).is_none());
        toasts.show("Site muted".into(), None);
        assert!(toasts.live(None).is_some());
    }
}
