use std::cell::Cell;
use std::rc::Rc;

use browser_core::split::Rect;
use gpui::{Bounds, Context, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, WeakEntity, Window, WindowHandle, div, point, px, size};
use ui::controls::{open_popup, surface};
use ui::text::OneLine;
use ui::theme::{TEXT_SMALL, Theme};

use crate::browser::{Browser, TabKey};

const HEIGHT: f32 = 24.;
const PAD: f32 = 8.;
/// Between the bubble and the page's bottom-left corner.
const GAP: f32 = 4.;

/// The link under the pointer, at the bottom-left of its page in a pop-up window the pointer
/// passes through, since GPUI cannot draw over the page.
#[derive(Default)]
pub struct LinkStatus {
    link: browser_core::link_status::Hovered,
    /// The page's frame in the window outside split view, written as it is laid out.
    pub(crate) page: Rc<Cell<Rect>>,
    pub(crate) window: Option<WindowHandle<LinkStatusWindow>>,
    pub(crate) popup: Option<engine::Host>,
    placed: Rect,
    opening: bool,
}

impl Browser {
    pub(crate) fn link_hovered(&mut self, key: TabKey, raw: Option<&str>, cx: &mut Context<Self>) {
        if self.link_status.link.set(key, raw) {
            cx.notify();
        }
    }

    fn link_rect(&self, key: TabKey) -> Option<Rect> {
        let rect = match self.split_shown.pane_rect(key) {
            Some(rect) => rect,
            None if self.model.active == Some(key) => self.link_status.page.get(),
            None => return None,
        };
        // A blocked page or JSON view hides the web view, which then reports no pointer leaving.
        if self.pages.get(&key).is_some_and(|p| p.blocked.is_some()) || self.json_pages.contains_key(&key) {
            return None;
        }
        (rect.2 > 0. && rect.3 > 0.).then_some(rect)
    }

    /// Opens, moves or closes the bubble for the link under the pointer.
    pub(crate) fn sync_link_status(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let shown = self
            .link_status
            .link
            .get()
            .filter(|_| !self.overlay_open())
            .and_then(|(key, text)| Some((self.link_rect(key)?, SharedString::from(text.to_owned()))));
        let Some(((left, top, page_width, page_height), text)) = shown else {
            // A tab switched away from, or a page covered, shows its link again only once the pointer moves.
            self.link_status.link.clear();
            return self.close_link_status(cx);
        };
        if self.link_status.opening {
            return;
        }
        let mut style = window.text_style();
        style.font_family = Theme::font();
        let run = style.to_run(text.len());
        let measured = f32::from(window.text_system().shape_line(text, px(TEXT_SMALL), &[run], None).width);
        let width = (measured + 2. * PAD + 2.).ceil().min(page_width - 2. * GAP);
        let (x, y) = (left + GAP, top + page_height - HEIGHT - GAP);
        let placed = (x, y, width, HEIGHT);
        if self.link_status.window.is_some() {
            if let (Some(host), Some(popup)) = (self.host.clone(), self.link_status.popup.clone())
                && self.link_status.placed != placed
            {
                self.link_status.placed = placed;
                // GPUI handles the resize this causes only outside an update, so the frame is set from a task.
                cx.spawn(async move |_, _| host.place(&popup, x.into(), y.into(), width.into(), HEIGHT.into())).detach();
            }
            return;
        }
        if window.window_handle().downcast::<Browser>().is_none() {
            return;
        }
        self.link_status.placed = placed;
        let frame = window.bounds();
        let bounds = Bounds::new(point(frame.origin.x + px(x), frame.origin.y + px(y)), size(px(width), px(HEIGHT)));
        let display = window.display(cx).map(|d| d.id());
        let browser = cx.entity().downgrade();
        self.link_status.opening = true;
        cx.defer(move |cx| {
            let (handle, popup) = open_popup(cx, bounds, display, false, [0.; 4], Theme::radius_row(), |_, cx| LinkStatusWindow::new(browser.clone(), cx));
            if let Some(popup) = &popup {
                popup.ignore_mouse();
            }
            let kept = browser.update(cx, |b, cx| {
                b.link_status.opening = false;
                let live = b.link_status.link.get().is_some();
                if let (Some(handle), true) = (handle, live) {
                    if let (Some(host), Some(popup)) = (&b.host, &popup) {
                        host.follow(popup);
                    }
                    (b.link_status.window, b.link_status.popup) = (Some(handle), popup);
                    cx.notify();
                }
                live
            });
            if let (Ok(false) | Err(_), Some(handle)) = (kept, handle) {
                handle.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
    }

    fn close_link_status(&mut self, cx: &mut Context<Self>) {
        self.link_status.popup = None;
        if let Some(popup) = self.link_status.window.take() {
            cx.defer(move |cx| drop(popup.update(cx, |_, window, _| window.remove_window())));
        }
    }
}

pub struct LinkStatusWindow {
    browser: WeakEntity<Browser>,
    _observe: Option<Subscription>,
}

impl LinkStatusWindow {
    fn new(browser: WeakEntity<Browser>, cx: &mut Context<Self>) -> Self {
        let observe = browser.upgrade().map(|b| cx.observe(&b, |_, _, cx| cx.notify()));
        LinkStatusWindow { browser, _observe: observe }
    }
}

impl Render for LinkStatusWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = self.browser.upgrade().and_then(|b| b.read(cx).link_status.link.get().map(|(_, text)| text.to_owned())).unwrap_or_default();
        let (origin, rest) = browser_core::link_status::split(&text);
        surface(Theme::radius_row())
            .size_full()
            .px(px(PAD))
            .flex()
            .items_center()
            .text_size(px(TEXT_SMALL))
            .text_color(Theme::muted())
            .child(div().flex_none().child(SharedString::from(origin.to_owned())))
            .child(div().one_line(rest.to_owned()))
    }
}
