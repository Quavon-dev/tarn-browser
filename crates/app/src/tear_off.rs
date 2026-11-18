use engine::Host;
use gpui::{AppContext, Bounds, Context, IntoElement, ParentElement, Pixels, Point, Render, SharedString, Styled, Window, WindowHandle, div, point, px, size};
use ui::motion::{FADE, animate};
use ui::text::OneLine;
use ui::theme::{Elevation, TEXT, TEXT_SMALL, Theme};

use crate::browser::{Browser, TabKey};
use crate::peek::popup_options;
use crate::prefs::Pref;

const WIDTH: f32 = 240.;
const HEIGHT: f32 = 92.;

/// The small window that follows a tab dragged out of its window and says what letting go opens.
pub struct Ghost {
    title: SharedString,
    label: &'static str,
    reduce: bool,
}

impl Render for Ghost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let light = |color| div().size(px(8.)).rounded_full().bg(color);
        let card = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_1p5()
            .p_3()
            .rounded(px(Theme::radius_card()))
            .bg(Theme::surface())
            .border_1()
            .border_color(Theme::accent())
            .shadow(Elevation::Popover.shadow())
            .child(div().flex().gap_1().children([light(Theme::faint()), light(Theme::faint()), light(Theme::faint())]))
            .child(div().text_size(px(TEXT)).text_color(Theme::text()).one_line(self.title.clone()))
            .child(div().text_size(px(TEXT_SMALL)).text_color(Theme::accent_text()).child(self.label));
        animate(card, "tear-off-ghost", FADE, self.reduce, |el, t| el.opacity(t).relative().top(px((1. - t) * 8.)))
    }
}

#[derive(Default)]
pub(crate) struct Ghosting {
    pub(crate) popup: Option<(WindowHandle<Ghost>, Host)>,
    opening: bool,
}

impl Browser {
    /// Shows the ghost of `key` under the pointer at `at` while it is outside the window, and
    /// takes it away when the pointer comes back.
    pub(crate) fn follow_tab_drag(&mut self, key: TabKey, title: SharedString, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if Bounds::new(Point::default(), window.viewport_size()).contains(&at) {
            return self.drop_ghost(cx);
        }
        let (x, y) = (f32::from(at.x) - WIDTH / 2., f32::from(at.y) + 14.);
        if let (Some(host), Some((_, popup))) = (self.host.clone(), &self.ghost.popup) {
            let popup = popup.clone();
            cx.spawn(async move |_, _| host.place(&popup, x.into(), y.into(), WIDTH.into(), HEIGHT.into())).detach();
            return;
        }
        if self.ghost.opening {
            return;
        }
        let Some(label) = self.model.tear_off_label(key, self.pref(Pref::TearOffTemporary)) else { return };
        let origin = window.bounds().origin;
        let bounds = Bounds::new(point(origin.x + px(x), origin.y + px(y)), size(px(WIDTH), px(HEIGHT)));
        let (display, reduce, browser) = (window.display(cx).map(|d| d.id()), self.pref(Pref::ReduceMotion), cx.entity().downgrade());
        self.ghost.opening = true;
        cx.defer(move |cx| {
            let mut host = None;
            let handle = cx
                .open_window(popup_options(bounds, display, false), |window, cx| {
                    host = Host::from_window(window);
                    if let Some(host) = &host {
                        host.round_corners(0., 0., 0., 0., f64::from(Theme::radius_card()));
                    }
                    cx.new(|_| Ghost { title, label, reduce })
                })
                .ok();
            let stray = browser
                .update(cx, |b, _| {
                    b.ghost.opening = false;
                    match (handle, host) {
                        (Some(handle), Some(host)) if b.tab_drag.is_some() => {
                            b.ghost.popup = Some((handle, host));
                            None
                        }
                        (handle, _) => handle,
                    }
                })
                .unwrap_or(handle);
            if let Some(handle) = stray {
                handle.update(cx, |_, window, _| window.remove_window()).ok();
            }
        });
    }

    /// Takes the ghost away, when the pointer is back in the window or the drag ended.
    pub(crate) fn drop_ghost(&mut self, cx: &mut Context<Self>) {
        let Some((handle, _)) = self.ghost.popup.take() else { return };
        cx.defer(move |cx| {
            handle.update(cx, |_, window, _| window.remove_window()).ok();
        });
    }
}
