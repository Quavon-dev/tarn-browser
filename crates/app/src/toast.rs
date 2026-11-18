use std::path::PathBuf;
use std::time::Duration;

use browser_core::toast::Tone;
use gpui::{
    Bounds, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled,
    Subscription, WeakEntity, Window, WindowHandle, actions, div, point, prelude::FluentBuilder, px, size,
};
use ui::controls::{POPOVER_INSET, open_popup, popover};
use ui::icon::{ICON_MEDIUM, Icon, icon};
use ui::motion::{EXIT, animate, fade_in};
use ui::theme::{TEXT, Theme};

use crate::browser::Browser;
use crate::prefs::Pref;
use crate::sidebar::SIDEBAR_MARGIN;

actions!(tarn, [ToastAction]);

const TOAST_TIME: Duration = Duration::from_secs(5);
/// How often a toast under the pointer checks whether the pointer left.
const HOVER_CHECK: Duration = Duration::from_secs(1);
/// A toast with a button stays longer, so the action is still there after a glance away.
const ACTION_TIME: Duration = Duration::from_secs(10);
/// The widest a toast grows before its message wraps to a second line.
const MAX_WIDTH: f32 = 440.;
const HEIGHT: f32 = 44.;
const LINE: f32 = 18.;
const PAD: f32 = 16.;
const GAP: f32 = 10.;
/// The action's padding beside its label; the card's inset after it keeps the label `PAD` from the edge.
const BUTTON_PAD: f32 = 10.;
const BUTTON_HEIGHT: f32 = 28.;
/// The card's left and right border, and a pixel so rounding never cuts the message.
const EDGES: f32 = 3.;
/// Between the toast and the window's bottom edge.
const MARGIN: f32 = 24.;

#[derive(Clone, PartialEq, Debug)]
pub enum Action {
    /// Reopens the closed-tab entry with this serial.
    Undo(u64),
    /// Puts back what Clean Up sorted, in the private or normal space it sorted.
    UndoSort(Box<(bool, browser_core::cleanup::Undo)>),
    Reveal(PathBuf),
    RestartToUpdate,
    /// Puts back the download list Clear removed from the private or normal store's profile.
    RestoreDownloads(Box<(bool, store::Id, Vec<store::Download>)>),
}

impl Action {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Action::Undo(_) => "Undo",
            Action::UndoSort(_) => "Undo Sort",
            Action::Reveal(_) => ui::controls::REVEAL,
            Action::RestartToUpdate => "Restart to Update",
            Action::RestoreDownloads(_) => "Undo",
        }
    }
}

#[derive(Debug)]
pub struct Toast {
    pub text: SharedString,
    pub tone: Tone,
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
    placed: (f32, f32, f32, f32),
    opening: bool,
    /// The pointer is over the toast, which then stays.
    pub(crate) hovered: bool,
}

impl Toasts {
    /// Replaces any toast; returns the new toast's serial.
    fn show(&mut self, tone: Tone, text: SharedString, action: Option<Action>) -> u64 {
        self.serial += 1;
        self.current = Some(Toast { text, tone, action, serial: self.serial, closing: false });
        self.serial
    }

    /// Starts fading the toast `serial` out, unless a newer one replaced it or the pointer holds it.
    fn fade(&mut self, serial: u64) -> bool {
        let Some(toast) = self.current.as_mut().filter(|t| t.serial == serial && !self.hovered) else { return false };
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

pub fn listen(root: gpui::Div, cx: &mut Context<Browser>) -> gpui::Div {
    root.on_action(cx.listener(|b, _: &ToastAction, w, cx| b.take_toast_action(w, cx)))
}

impl Browser {
    pub(crate) fn show_toast(&mut self, text: impl Into<SharedString>, action: Option<Action>, cx: &mut Context<Self>) {
        self.show_toast_as(Tone::Info, text, action, cx);
    }

    pub(crate) fn show_toast_as(&mut self, tone: Tone, text: impl Into<SharedString>, action: Option<Action>, cx: &mut Context<Self>) {
        let wait = if action.is_some() { ACTION_TIME } else { TOAST_TIME };
        let serial = self.toast.show(tone, text.into(), action);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            while this.update(cx, |b, _| b.toast.hovered && b.toast.current.as_ref().is_some_and(|t| t.serial == serial)).unwrap_or(false) {
                cx.background_executor().timer(HOVER_CHECK).await;
            }
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

    /// The live toast's button as a command row: its label and the toast's message.
    pub(crate) fn toast_command(&self) -> Option<String> {
        let toast = self.toast.live(self.model.closed_tabs.newest())?;
        Some(format!("{} — {}", toast.action.as_ref()?.label(), toast.text))
    }

    pub fn take_toast_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toast.hovered = false;
        let Some(toast) = self.toast.current.take() else { return };
        match toast.action {
            Some(Action::Undo(serial)) if self.model.closed_tabs.newest() == Some(serial) => self.reopen_closed_tab(window, cx),
            Some(Action::UndoSort(record)) => self.undo_sort(*record, cx),
            Some(Action::Reveal(path)) => engine::reveal_in_finder(&path),
            Some(Action::RestartToUpdate) => crate::shared::restart_to_update(cx),
            Some(Action::RestoreDownloads(cleared)) => {
                let (private, profile_id, list) = *cleared;
                self.core.restore_downloads(private, profile_id, &list);
                self.downloads_popover.changed += 1;
            }
            _ => {}
        }
        cx.notify();
    }

    /// Opens the toast's popup while a toast is live and closes it once none is.
    pub(crate) fn sync_toast(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((width, lines)) = self.toast.live(self.model.closed_tabs.newest()).map(|t| toast_size(t, window)) else {
            self.toast.hovered = false;
            return self.close_toast_window(cx);
        };
        if self.toast.opening {
            return;
        }
        let frame = window.bounds();
        let left = (self.sidebar_width + SIDEBAR_MARGIN) * self.sidebar_shown();
        let x = (left + (f32::from(frame.size.width) - left - width) / 2.).clamp(0., (f32::from(frame.size.width) - width).max(0.)) - POPOVER_INSET;
        let card = HEIGHT + LINE * (lines - 1) as f32;
        let y = f32::from(frame.size.height) - card - MARGIN - POPOVER_INSET;
        let (width, height) = (width + 2. * POPOVER_INSET, card + 2. * POPOVER_INSET);
        if self.toast.window.is_some() {
            if let (Some(host), Some(popup)) = (self.host.clone(), self.toast.popup.clone())
                && self.toast.placed != (x, y, width, height)
            {
                self.toast.placed = (x, y, width, height);
                // GPUI handles the resize this causes only outside an update, so the frame is set from a task.
                cx.spawn(async move |_, _| host.place(&popup, x.into(), y.into(), width.into(), height.into())).detach();
            }
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>() else { return };
        self.toast.placed = (x, y, width, height);
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

fn tone_icon(tone: Tone) -> (Icon, Rgba) {
    match tone {
        Tone::Info => (Icon::Info, Theme::muted()),
        Tone::Success => (Icon::CircleCheck, Theme::success()),
        Tone::Warning => (Icon::TriangleAlert, Theme::warning()),
    }
}

/// The pill's width for its message, icon and button, so the pop-up hugs the pill and covers no
/// more of the page, and its lines: a message too wide for `MAX_WIDTH` wraps once.
fn toast_size(toast: &Toast, window: &Window) -> (f32, usize) {
    let measure = |text: SharedString, weight: FontWeight| {
        let mut style = window.text_style();
        (style.font_family, style.font_weight) = (Theme::font(), weight);
        let run = style.to_run(text.len());
        f32::from(window.text_system().shape_line(text, px(TEXT), &[run], None).width)
    };
    let action = toast.action.as_ref().map_or(PAD, |a| GAP + BUTTON_PAD + measure(a.label().into(), FontWeight::MEDIUM) + PAD);
    let width = (PAD + ICON_MEDIUM + GAP + measure(toast.text.clone(), FontWeight::NORMAL) + action + EDGES).ceil();
    (width.min(MAX_WIDTH), if width > MAX_WIDTH { 2 } else { 1 })
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
        let (glyph, color) = tone_icon(toast.tone);
        let card = popover(Theme::radius_panel(), POPOVER_INSET)
            .id("toast")
            .on_hover(move |hovered, _, cx| {
                let hovered = *hovered;
                main.update(cx, |b, _, _| b.toast.hovered = hovered).ok();
            })
            .pl(px(PAD))
            .pr(px(if label.is_some() { PAD - BUTTON_PAD } else { PAD }))
            .flex()
            .items_center()
            .gap(px(GAP))
            .text_size(px(TEXT))
            .text_color(Theme::text())
            .child(icon(glyph, ICON_MEDIUM, color))
            .child(div().flex_1().min_w_0().overflow_hidden().line_clamp(2).text_ellipsis().child(toast.text.clone()))
            .when_some(label, |d, label| {
                d.child(
                    div()
                        .id("toast-action")
                        .flex_none()
                        .h(px(BUTTON_HEIGHT))
                        .px(px(BUTTON_PAD))
                        .flex()
                        .items_center()
                        .rounded(px(Theme::radius_row()))
                        .cursor_pointer()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(Theme::accent_text())
                        .hover(|s| s.bg(Theme::hover()))
                        .active(|s| s.bg(Theme::pressed()))
                        .child(label)
                        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                            main.update(cx, |b, window, cx| b.take_toast_action(window, cx)).ok();
                        }),
                )
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
        let first = toasts.show(Tone::Info, "Link copied".into(), None);
        let second = toasts.show(Tone::Info, "Screenshot saved".into(), Some(Action::Reveal(PathBuf::from("/tmp/a.png"))));
        assert!(!toasts.fade(first) && !toasts.expire(first));
        assert_eq!(toasts.live(None).map(|t| (t.text.as_ref(), t.closing)), Some(("Screenshot saved", false)));
        assert!(toasts.fade(second));
        assert!(toasts.live(None).is_some_and(|t| t.closing));
        assert!(toasts.expire(second));
        assert!(toasts.live(None).is_none());
    }

    #[test]
    fn a_toast_under_the_pointer_stays() {
        let mut toasts = Toasts::default();
        let serial = toasts.show(Tone::Info, "Tab closed".into(), Some(Action::Undo(1)));
        toasts.hovered = true;
        assert!(!toasts.fade(serial));
        toasts.hovered = false;
        assert!(toasts.fade(serial));
    }

    #[test]
    fn undo_shows_only_while_its_entry_is_the_newest() {
        let mut toasts = Toasts::default();
        toasts.show(Tone::Info, "Tab closed".into(), Some(Action::Undo(7)));
        assert!(toasts.live(Some(7)).is_some());
        assert!(toasts.live(Some(8)).is_none());
        assert!(toasts.live(None).is_none());
        toasts.show(Tone::Warning, "Could not save to Reading List".into(), None);
        assert_eq!(toasts.live(None).map(|t| t.tone), Some(Tone::Warning));
    }

    #[test]
    fn undo_sort_goes_with_its_toast() {
        let store = store::Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "uuid", false).unwrap();
        let space = store.create_space(profile.id, "S", "").unwrap();
        let record = browser_core::cleanup::apply(&store, space.id, &browser_core::cleanup::plan(&[], &[], |t| t.url.clone())).unwrap();
        let mut toasts = Toasts::default();
        let serial = toasts.show(Tone::Info, "Sorted 2 tabs into groups".into(), Some(Action::UndoSort(Box::new((false, record.clone())))));
        assert!(matches!(toasts.live(None).and_then(|t| t.action.as_ref()), Some(Action::UndoSort(_))));
        assert!(toasts.expire(serial) && toasts.live(None).is_none());
        toasts.show(Tone::Info, "Sorted 2 tabs into groups".into(), Some(Action::UndoSort(Box::new((false, record)))));
        toasts.show(Tone::Info, "Link copied".into(), None);
        assert!(toasts.live(None).is_some_and(|t| t.action.is_none()), "a newer toast takes the undo with the old one");
    }
}
