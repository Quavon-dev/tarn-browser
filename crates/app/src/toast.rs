use std::path::PathBuf;
use std::time::Duration;

use browser_core::toast::Tone;
use gpui::{
    Bounds, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled,
    Subscription, WeakEntity, Window, WindowHandle, actions, div, point, prelude::FluentBuilder, px, size,
};
use ui::controls::{POPOVER_INSET, open_popup, popover, secure_surface};
use ui::icon::{ICON_MEDIUM, Icon, icon};
use ui::motion::{EXIT, POPUP_TRAVEL, SETTLE, animate, animate_linear, arrive};
use ui::theme::{Elevation, SYSTEM_FONT, Secure, TEXT, Theme};

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
/// The most lines a message wraps to; a download warning needs three at `MAX_WIDTH`.
const MAX_LINES: usize = 3;
const WRAP_MARGIN: f32 = 1.15;
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
    /// Opens the release page of a later version in a new tab, for a portable Tarn.
    Download(url::Url),
    /// Puts back the download list Clear removed from the private or normal store's profile.
    RestoreDownloads(Box<(bool, store::Id, Vec<store::Download>)>),
    /// Takes a held warning away.
    Dismiss,
}

impl Action {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Action::Undo(_) => "Undo",
            Action::UndoSort(_) => "Undo Sort",
            Action::Reveal(_) => ui::controls::REVEAL,
            Action::RestartToUpdate => "Restart to Update",
            Action::Download(_) => "Download",
            Action::RestoreDownloads(_) => "Undo",
            Action::Dismiss => "Dismiss",
        }
    }
}

type TabKey = crate::browser::TabKey;
/// A profile's store: private or not, and its id.
type Owner = (bool, store::Id);

#[derive(Debug)]
pub struct Toast {
    pub text: SharedString,
    pub tone: Tone,
    pub action: Option<Action>,
    serial: u64,
    pub closing: bool,
    /// The tab it belongs to: it shows only while that tab is the active one.
    pub tab: Option<TabKey>,
    /// The profile whose download or capture it reports: security UI (S29), dropped with that
    /// profile's data, and without a tab shown while that profile's space is.
    pub owner: Option<Owner>,
}

/// The window's toasts and the PopUp window they show in at the bottom centre of the page, since
/// GPUI cannot draw over the page. A passing toast shows first; download warnings wait behind it
/// until dismissed, so nothing replaces them (S11).
#[derive(Default)]
pub struct Toasts {
    current: Option<Toast>,
    held: Vec<Toast>,
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
    /// Replaces any passing toast, or with `held` adds one that waits until dismissed; returns its serial.
    fn show(&mut self, tone: Tone, text: SharedString, action: Option<Action>, place: (Option<TabKey>, Option<Owner>), held: bool) -> u64 {
        self.serial += 1;
        let toast = Toast { text, tone, action, serial: self.serial, closing: false, tab: place.0, owner: place.1 };
        if held {
            self.held.push(toast)
        } else {
            self.current = Some(toast)
        }
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

    /// Takes the toast `serial` away, passing or held.
    fn take(&mut self, serial: u64) -> Option<Toast> {
        if self.current.as_ref().is_some_and(|t| t.serial == serial) {
            return self.current.take();
        }
        let ix = self.held.iter().position(|t| t.serial == serial)?;
        Some(self.held.remove(ix))
    }

    /// The toast to show; an Undo toast only while its closed-tab entry is still the newest, so
    /// Undo always reopens something; a tab's toast only while that tab is `active`, and an
    /// owner's toast without a tab only while the window shows that owner's `space`.
    pub fn live(&self, newest_closed: Option<u64>, active: Option<TabKey>, space: Option<Owner>) -> Option<&Toast> {
        let shown = |t: &&Toast| match t.tab {
            Some(tab) => active == Some(tab) && t.owner.is_none_or(|owner| space == Some(owner)),
            None => t.owner.is_none_or(|owner| space == Some(owner)),
        };
        let passing = self.current.as_ref().filter(|t| !matches!(t.action, Some(Action::Undo(serial)) if newest_closed != Some(serial)));
        passing.filter(shown).or_else(|| self.held.iter().find(shown))
    }

    /// The tabs held warnings belong to.
    fn held_tabs(&self) -> Vec<TabKey> {
        self.held.iter().filter_map(|t| t.tab).collect()
    }

    /// Held warnings on tabs that closed or left the window wait for their profile's space instead.
    fn release(&mut self, gone: &[TabKey]) {
        self.held.iter_mut().filter(|t| t.tab.is_some_and(|k| gone.contains(&k))).for_each(|t| t.tab = None);
    }

    /// Drops the toasts of a profile whose data was deleted, which may name its files.
    pub(crate) fn forget(&mut self, owner: Owner) {
        self.current.take_if(|t| t.owner == Some(owner));
        self.held.retain(|t| t.owner != Some(owner));
    }
}

pub fn listen(root: gpui::Div, cx: &mut Context<Browser>) -> gpui::Div {
    root.on_action(cx.listener(|b, _: &ToastAction, w, cx| b.take_toast_action(w, cx)))
}

impl Browser {
    pub(crate) fn show_toast(&mut self, text: impl Into<SharedString>, action: Option<Action>, cx: &mut Context<Self>) {
        self.show_toast_as(Tone::Info, text, action, cx);
    }

    /// After `command` ran from the command bar: a toast naming its `keys`, if the profile's
    /// hint throttle allows one and the command showed no toast of its own; none in a private
    /// space, a burner tab or while a text field has the keyboard.
    pub(crate) fn offer_hint(&mut self, command: &str, keys: &str, window: &Window, cx: &mut Context<Self>) {
        if self.live_toast().is_some() {
            return;
        }
        let field = window.context_stack().iter().any(|c| c.contains("TextInput"));
        let typing = self.command_mode.is_some() || self.renaming.is_some() || field;
        if let Some(text) = self.core.feature_hint(self.model.profile(), command, keys, browser_core::now(), typing) {
            self.show_toast_as(Tone::Info, text, None, cx);
        }
    }

    pub(crate) fn show_toast_as(&mut self, tone: Tone, text: impl Into<SharedString>, action: Option<Action>, cx: &mut Context<Self>) {
        let wait = if action.is_some() { ACTION_TIME } else { TOAST_TIME };
        self.show_toast_for(tone, text.into(), action, (None, None), wait, cx);
    }

    /// A toast about `owner`'s download or capture over the active tab's page, shown only while
    /// that tab is active, for `TOAST_TIME`.
    pub(crate) fn show_tab_toast(&mut self, tone: Tone, text: impl Into<SharedString>, action: Option<Action>, owner: Owner, cx: &mut Context<Self>) {
        self.show_toast_for(tone, text.into(), action, (self.model.active, Some(owner)), TOAST_TIME, cx);
    }

    /// A download warning that stays until dismissed: on `tab`, or without one while the window
    /// shows `owner`'s space.
    pub(crate) fn hold_toast(&mut self, text: impl Into<SharedString>, tab: Option<TabKey>, owner: Owner, cx: &mut Context<Self>) {
        self.toast.show(Tone::Warning, text.into(), Some(Action::Dismiss), (tab, Some(owner)), true);
        cx.notify();
    }

    /// Whether the tab still exists in one of this window's spaces, shown or not.
    fn tab_in_window(&self, key: TabKey) -> bool {
        let space = self.store(key.private).tab(key.id).map(|t| t.space_id);
        space.is_ok_and(|id| self.model.spaces.iter().any(|s| s.private == key.private && s.space.id == id))
    }

    pub(crate) fn live_toast(&self) -> Option<&Toast> {
        let space = self.model.current_space().map(|s| (s.private, s.space.profile_id));
        self.toast.live(self.model.closed_tabs.newest(), self.model.active, space)
    }

    fn show_toast_for(
        &mut self,
        tone: Tone,
        text: SharedString,
        action: Option<Action>,
        place: (Option<TabKey>, Option<Owner>),
        wait: Duration,
        cx: &mut Context<Self>,
    ) {
        let serial = self.toast.show(tone, text, action, place, false);
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
        let toast = self.live_toast()?;
        Some(format!("{} — {}", toast.action.as_ref()?.label(), toast.text))
    }

    pub fn take_toast_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toast.hovered = false;
        let Some(toast) = self.live_toast().map(|t| t.serial).and_then(|serial| self.toast.take(serial)) else { return };
        match toast.action {
            Some(Action::Undo(serial)) if self.model.closed_tabs.newest() == Some(serial) => {
                self.run_undo(browser_core::undo::Step::Closed(serial), false, window, cx)
            }
            Some(Action::UndoSort(record)) => {
                let (private, undo) = *record;
                self.run_undo(browser_core::undo::Step::CleanUp(undo), private, window, cx)
            }
            Some(Action::Reveal(path)) => engine::reveal_in_finder(&path),
            Some(Action::RestartToUpdate) => crate::shared::restart_to_update(cx),
            Some(Action::Download(page)) => self.new_tab(&page, window, cx),
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
        let gone: Vec<TabKey> = self.toast.held_tabs().into_iter().filter(|key| !self.tab_in_window(*key) || crate::shared::live_elsewhere(*key, cx)).collect();
        self.toast.release(&gone);
        let Some((width, lines)) = self.live_toast().map(|t| toast_size(t, window)) else {
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
                let live = b.live_toast().is_some();
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
        let font = if toast.owner.is_some() { SYSTEM_FONT.into() } else { Theme::font() };
        (style.font_family, style.font_weight) = (font, weight);
        let run = style.to_run(text.len());
        f32::from(window.text_system().shape_line(text, px(TEXT), &[run], None).width)
    };
    let action = toast.action.as_ref().map_or(PAD, |a| GAP + BUTTON_PAD + measure(a.label().into(), FontWeight::MEDIUM) + PAD);
    let (text, rest) = (measure(toast.text.clone(), FontWeight::NORMAL), PAD + ICON_MEDIUM + GAP + action + EDGES);
    let width = (text + rest).ceil();
    // Words wrap short of the edge, hence the margin on the text's width.
    let lines = (text * WRAP_MARGIN / (MAX_WIDTH - rest)).ceil().clamp(1., MAX_LINES as f32) as usize;
    (width.min(MAX_WIDTH), if width > MAX_WIDTH { lines.max(2) } else { 1 })
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
        let Some(toast) = b.live_toast() else { return div().into_any_element() };
        let main = self.main;
        let card = if toast.owner.is_some() { secure_card(toast, main) } else { card(toast, main) };
        let (reduce, id) = (b.pref(Pref::ReduceMotion), toast.serial as usize);
        // The card's insets need a parent of the window's size; as the root it would grow to its text.
        div()
            .size_full()
            .child(if toast.closing {
                animate(card, ("toast-out", id), EXIT, reduce, |el, t| shifted(el.opacity(1. - t), POPUP_TRAVEL * t)).into_any_element()
            } else {
                animate_linear(card, ("toast-in", id), SETTLE, reduce, |el, t| {
                    let (shown, ahead) = arrive(t);
                    shifted(el.opacity(shown), POPUP_TRAVEL * ahead)
                })
                .into_any_element()
            })
            .into_any_element()
    }
}

/// Only the passing toast's timer waits for the pointer; a held one has none.
fn hover(main: WindowHandle<Browser>, serial: u64, hovered: bool, cx: &mut gpui::App) {
    main.update(cx, |b, _, _| b.toast.hovered = hovered && b.toast.current.as_ref().is_some_and(|t| t.serial == serial)).ok();
}

fn card(toast: &Toast, main: WindowHandle<Browser>) -> gpui::Stateful<gpui::Div> {
    let serial = toast.serial;
    let label = toast.action.as_ref().map(Action::label);
    let (glyph, color) = tone_icon(toast.tone);
    popover(Theme::radius_panel(), POPOVER_INSET)
        .id("toast")
        .on_hover(move |hovered, _, cx| hover(main, serial, *hovered, cx))
        .pl(px(PAD))
        .pr(px(if label.is_some() { PAD - BUTTON_PAD } else { PAD }))
        .flex()
        .items_center()
        .gap(px(GAP))
        .text_size(px(TEXT))
        .text_color(Theme::text())
        .child(icon(glyph, ICON_MEDIUM, color))
        .child(div().flex_1().min_w_0().overflow_hidden().line_clamp(MAX_LINES).text_ellipsis().child(toast.text.clone()))
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
        })
}

fn shifted(card: gpui::Stateful<gpui::Div>, down: f32) -> gpui::Stateful<gpui::Div> {
    card.top(px(POPOVER_INSET + down)).bottom(px(POPOVER_INSET - down))
}

/// An owner's toast reports a download, which is security UI (S29): it draws only from `Secure`.
fn secure_card(toast: &Toast, main: WindowHandle<Browser>) -> gpui::Stateful<gpui::Div> {
    let serial = toast.serial;
    let label = toast.action.as_ref().map(Action::label);
    let color = match toast.tone {
        Tone::Info => Secure::muted(),
        Tone::Success => Secure::success(),
        Tone::Warning => Secure::warning(),
    };
    secure_surface(Theme::radius_panel())
        .absolute()
        .top(px(POPOVER_INSET))
        .left(px(POPOVER_INSET))
        .right(px(POPOVER_INSET))
        .bottom(px(POPOVER_INSET))
        .shadow(Elevation::Popover.shadow())
        .id("toast")
        .on_hover(move |hovered, _, cx| hover(main, serial, *hovered, cx))
        .pl(px(PAD))
        .pr(px(if label.is_some() { PAD - BUTTON_PAD } else { PAD }))
        .flex()
        .items_center()
        .gap(px(GAP))
        .text_size(px(TEXT))
        .child(icon(tone_icon(toast.tone).0, ICON_MEDIUM, color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .line_clamp(MAX_LINES)
                .text_ellipsis()
                .text_color(if toast.tone == Tone::Warning { color } else { Secure::text() })
                .child(toast.text.clone()),
        )
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
                    .text_color(Secure::text())
                    .hover(|s| s.bg(Secure::hover()))
                    .active(|s| s.bg(Secure::pressed()))
                    .child(label)
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        main.update(cx, |b, window, cx| b.take_toast_action(window, cx)).ok();
                    }),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_timer_leaves_a_newer_toast_alone() {
        let mut toasts = Toasts::default();
        let first = toasts.show(Tone::Info, "Link copied".into(), None, (None, None), false);
        let second = toasts.show(Tone::Info, "Screenshot saved".into(), Some(Action::Reveal(PathBuf::from("/tmp/a.png"))), (None, None), false);
        assert!(!toasts.fade(first) && !toasts.expire(first));
        assert_eq!(toasts.live(None, None, None).map(|t| (t.text.as_ref(), t.closing)), Some(("Screenshot saved", false)));
        assert!(toasts.fade(second));
        assert!(toasts.live(None, None, None).is_some_and(|t| t.closing));
        assert!(toasts.expire(second));
        assert!(toasts.live(None, None, None).is_none());
    }

    #[test]
    fn a_tab_toast_shows_only_on_its_tab() {
        let (a, b) = (TabKey { private: false, id: 1 }, TabKey { private: false, id: 2 });
        let mut toasts = Toasts::default();
        toasts.show(Tone::Success, "Downloaded “a.pdf”".into(), None, (Some(a), Some((false, 1))), false);
        assert!(toasts.live(None, Some(a), Some((false, 1))).is_some());
        assert!(toasts.live(None, Some(b), None).is_none() && toasts.live(None, None, None).is_none());
        toasts.forget((false, 2));
        assert!(toasts.live(None, Some(a), Some((false, 1))).is_some(), "another profile's wipe leaves it");
        toasts.forget((false, 1));
        assert!(toasts.live(None, Some(a), Some((false, 1))).is_none());
    }

    #[test]
    fn a_download_warning_waits_behind_passing_toasts_until_dismissed() {
        let a = TabKey { private: false, id: 1 };
        let mut toasts = Toasts::default();
        let warning = toasts.show(Tone::Warning, "Downloaded “a.app”".into(), Some(Action::Dismiss), (Some(a), Some((false, 1))), true);
        let copied = toasts.show(Tone::Success, "Link copied".into(), None, (None, None), false);
        assert_eq!(toasts.live(None, Some(a), Some((false, 1))).map(|t| t.serial), Some(copied));
        assert!(toasts.fade(copied) && toasts.expire(copied));
        assert!(!toasts.fade(warning) && !toasts.expire(warning), "no timer takes a warning away");
        assert_eq!(toasts.live(None, Some(a), Some((false, 1))).map(|t| t.serial), Some(warning));
        assert!(toasts.take(warning).is_some() && toasts.live(None, Some(a), Some((false, 1))).is_none());
    }

    #[test]
    fn a_tab_toast_shows_only_in_its_profiles_space() {
        let a = TabKey { private: false, id: 1 };
        let mut toasts = Toasts::default();
        toasts.show(Tone::Success, "Screenshot saved".into(), None, (Some(a), Some((true, 1))), false);
        assert!(toasts.live(None, Some(a), Some((false, 2))).is_none(), "a private profile's toast never shows over another profile");
        assert!(toasts.live(None, Some(a), Some((true, 1))).is_some());
    }

    #[test]
    fn a_warning_whose_tab_closed_waits_for_its_profiles_space() {
        let a = TabKey { private: false, id: 1 };
        let mut toasts = Toasts::default();
        toasts.show(Tone::Warning, "Download failed".into(), Some(Action::Dismiss), (Some(a), Some((false, 1))), true);
        toasts.release(&[TabKey { private: false, id: 2 }]);
        assert_eq!(toasts.held_tabs(), [a], "another tab's closing leaves it on its tab");
        toasts.release(&[a]);
        assert!(toasts.live(None, None, Some((false, 1))).is_some());
    }

    #[test]
    fn a_warning_without_a_tab_waits_for_its_profiles_space() {
        let mut toasts = Toasts::default();
        toasts.show(Tone::Warning, "Download failed".into(), Some(Action::Dismiss), (None, Some((true, 3))), true);
        assert!(toasts.live(None, None, Some((false, 3))).is_none());
        assert!(toasts.live(None, None, Some((true, 3))).is_some());
    }

    #[test]
    fn a_toast_under_the_pointer_stays() {
        let mut toasts = Toasts::default();
        let serial = toasts.show(Tone::Info, "Tab closed".into(), Some(Action::Undo(1)), (None, None), false);
        toasts.hovered = true;
        assert!(!toasts.fade(serial));
        toasts.hovered = false;
        assert!(toasts.fade(serial));
    }

    #[test]
    fn undo_shows_only_while_its_entry_is_the_newest() {
        let mut toasts = Toasts::default();
        toasts.show(Tone::Info, "Tab closed".into(), Some(Action::Undo(7)), (None, None), false);
        assert!(toasts.live(Some(7), None, None).is_some());
        assert!(toasts.live(Some(8), None, None).is_none());
        assert!(toasts.live(None, None, None).is_none());
        toasts.show(Tone::Warning, "Could not save to Reading List".into(), None, (None, None), false);
        assert_eq!(toasts.live(None, None, None).map(|t| t.tone), Some(Tone::Warning));
    }

    #[test]
    fn undo_sort_goes_with_its_toast() {
        let store = store::Store::open_in_memory().unwrap();
        let profile = store.create_profile("P", "uuid", false).unwrap();
        let space = store.create_space(profile.id, "S", "").unwrap();
        let record = browser_core::cleanup::apply(&store, space.id, &browser_core::cleanup::plan(&[], &[], |t| t.url.clone())).unwrap();
        let mut toasts = Toasts::default();
        let serial =
            toasts.show(Tone::Info, "Sorted 2 tabs into groups".into(), Some(Action::UndoSort(Box::new((false, record.clone())))), (None, None), false);
        assert!(matches!(toasts.live(None, None, None).and_then(|t| t.action.as_ref()), Some(Action::UndoSort(_))));
        assert!(toasts.expire(serial) && toasts.live(None, None, None).is_none());
        toasts.show(Tone::Info, "Sorted 2 tabs into groups".into(), Some(Action::UndoSort(Box::new((false, record)))), (None, None), false);
        toasts.show(Tone::Info, "Link copied".into(), None, (None, None), false);
        assert!(toasts.live(None, None, None).is_some_and(|t| t.action.is_none()), "a newer toast takes the undo with the old one");
    }
}
