use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, Rgba,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, WindowHandle, canvas, div, point, prelude::FluentBuilder, px, size,
};
use ui::controls::{key_cap, surface};
use ui::icon::{ICON_MEDIUM, Icon, icon};
use ui::motion::{EXIT, FADE, POP, animate};
use ui::theme::{TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

use crate::browser::Browser;
use crate::content::{SECTION_HEADER, SHOWN_SUGGESTIONS, SUGGESTION_ROW, suggestion_rows};
use crate::peek::{popup_options, remove_after_exit};
use crate::prefs::Pref;

const WIDTH: f32 = 640.;
const INPUT_ROW: f32 = 52.;
const FOOTER: f32 = 36.;
const PAD: f32 = 6.;
const MARGIN: f32 = 16.;
const DIM: f32 = 0.35;

#[derive(Default)]
pub struct Address {
    opened: Option<Instant>,
    pub(crate) bar: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) field: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) slot: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) content: Rc<Cell<Bounds<Pixels>>>,
    popup: Option<Popups>,
    opening: bool,
    placed: Option<(Bounds<Pixels>, Bounds<Pixels>)>,
    clicks: Option<engine::menu::EventMonitor>,
}

struct Popups {
    palette: (WindowHandle<Palette>, engine::Host),
    dim: (WindowHandle<Dim>, engine::Host),
}

impl Address {
    fn close(&mut self, reduce: bool, cx: &mut App) {
        self.placed = None;
        let Some(popup) = self.popup.take() else { return };
        let (palette, dim) = (popup.palette.0, popup.dim.0);
        cx.defer(move |cx| {
            palette.update(cx, |p, window, cx| p.leave(reduce, window, cx)).ok();
            dim.update(cx, |d, window, cx| d.leave(reduce, window, cx)).ok();
        });
    }

    pub(crate) fn remove_windows(&mut self, cx: &mut App) {
        if let Some(popup) = self.popup.take() {
            popup.palette.0.update(cx, |_, window, _| window.remove_window()).ok();
            popup.dim.0.update(cx, |_, window, _| window.remove_window()).ok();
        }
    }
}

/// Records where an element was painted.
pub fn measure(cell: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {}).absolute().inset_0()
}

fn palette_frame(content: Bounds<Pixels>, list: f32) -> Bounds<Pixels> {
    let (x, y, w, h) = (f32::from(content.origin.x), f32::from(content.origin.y), f32::from(content.size.width), f32::from(content.size.height));
    let width = WIDTH.min(w - 2. * MARGIN).max(0.);
    let top = y + h / 7.;
    let height = (INPUT_ROW + list + FOOTER).min(y + h - MARGIN - top).max(INPUT_ROW + FOOTER);
    Bounds::new(point(px(x + (w - width) / 2.), px(top)), size(px(width), px(height)))
}

impl Browser {
    pub fn inline_editing(&self) -> bool {
        self.command_mode.is_some() && self.address.opened.is_some() && self.model.active.is_some()
    }

    pub(crate) fn open_inline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.address.opened.is_some() {
            return;
        }
        self.address.opened = Some(Instant::now());
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
        self.address.opened = None;
        self.address.clicks = None;
        let reduce = self.pref(Pref::ReduceMotion);
        self.address.close(reduce, cx);
    }

    pub(crate) fn palette_field(&self) -> Option<Bounds<Pixels>> {
        let content = self.address.content.get();
        (self.inline_editing() && content.size.width > px(0.)).then(|| {
            let frame = palette_frame(content, self.palette_list());
            let inset = 16. + ui::icon::ICON_MEDIUM + 12.;
            let width = (f32::from(frame.size.width) - inset - 16.).max(0.);
            Bounds::new(point(frame.origin.x + px(inset), frame.origin.y + px((INPUT_ROW - 22.) / 2.)), size(px(width), px(22.)))
        })
    }

    fn palette_list(&self) -> f32 {
        let shown = &self.suggestions[..self.suggestions.len().min(SHOWN_SUGGESTIONS)];
        let headers: f32 = crate::content::sections(shown).iter().flatten().map(|_| SECTION_HEADER).sum();
        if shown.is_empty() { 0. } else { PAD * 2. + SUGGESTION_ROW * shown.len() as f32 + headers }
    }

    pub(crate) fn sync_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let content = self.address.content.get();
        if !self.inline_editing() {
            let reduce = self.pref(Pref::ReduceMotion);
            return self.address.close(reduce, cx);
        }
        let frame = palette_frame(content, self.palette_list());
        if content.size.width <= px(0.) || self.address.opening || self.address.placed == Some((frame, content)) {
            return;
        }
        if let (Some(host), Some(popup)) = (self.host.clone(), self.address.popup.as_ref()) {
            self.address.placed = Some((frame, content));
            let (palette, dim) = (popup.palette.1.clone(), popup.dim.1.clone());
            let rect = |b: Bounds<Pixels>| {
                (f32::from(b.origin.x).into(), f32::from(b.origin.y).into(), f32::from(b.size.width).into(), f32::from(b.size.height).into())
            };
            let (p, d) = (rect(frame), rect(content));
            cx.spawn(async move |_, _| {
                host.place(&dim, d.0, d.1, d.2, d.3);
                host.place(&palette, p.0, p.1, p.2, p.3);
            })
            .detach();
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>() else { return };
        let origin = window.bounds().origin;
        let on_screen = |b: Bounds<Pixels>| Bounds::new(point(origin.x + b.origin.x, origin.y + b.origin.y), b.size);
        let (frame, content) = (on_screen(frame), on_screen(content));
        let display = window.display(cx).map(|d| d.id());
        let browser = cx.entity().downgrade();
        let reduce = self.pref(Pref::ReduceMotion);
        self.address.opening = true;
        cx.defer(move |cx| {
            let (dim, dim_host) = {
                let mut host = None;
                let handle = cx.open_window(popup_options(content, display, false), |window, cx| {
                    host = engine::Host::from_window(window);
                    if let Some(host) = &host {
                        host.round_corners(0., 0., 0., 0., f64::from(Theme::radius_page()));
                    }
                    cx.new(|_| Dim { main, reduce, leaving: false })
                });
                (handle.ok(), host)
            };
            let (palette, palette_host) = {
                let mut host = None;
                let handle = cx.open_window(popup_options(frame, display, false), |window, cx| {
                    host = engine::Host::from_window(window);
                    if let Some(host) = &host {
                        host.round_corners(0., 0., 0., 0., f64::from(Theme::radius_card()));
                    }
                    cx.new(|cx| Palette::new(browser.clone(), main, cx))
                });
                (handle.ok(), host)
            };
            browser
                .update(cx, |b, cx| {
                    b.address.opening = false;
                    match (palette, palette_host, dim, dim_host) {
                        (Some(palette), Some(ph), Some(dim), Some(dh)) if b.inline_editing() => {
                            b.address.popup = Some(Popups { palette: (palette, ph), dim: (dim, dh) });
                            b.address.placed = None;
                            cx.notify();
                        }
                        (palette, _, dim, _) => cx.defer(move |cx| {
                            palette.map(|p| p.update(cx, |_, window, _| window.remove_window()));
                            dim.map(|d| d.update(cx, |_, window, _| window.remove_window()));
                        }),
                    }
                })
                .is_err()
                .then(|| {
                    palette.map(|p| p.update(cx, |_, window, _| window.remove_window()));
                    dim.map(|d| d.update(cx, |_, window, _| window.remove_window()));
                });
        });
    }
}

pub struct Dim {
    main: WindowHandle<Browser>,
    reduce: bool,
    leaving: bool,
}

impl Dim {
    fn leave(&mut self, reduce: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.leaving = true;
        cx.notify();
        remove_after_exit(reduce, window, cx);
    }
}

impl Render for Dim {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let main = self.main;
        let shade = |a: f32| Rgba { r: 0., g: 0., b: 0., a };
        let dim = div().id("dim").size_full().rounded(px(Theme::radius_page())).bg(shade(DIM)).on_mouse_down(MouseButton::Left, move |_, _, cx| {
            main.update(cx, |b, window, cx| b.close_command(window, cx)).ok();
        });
        if self.leaving {
            animate(dim, "dim-out", EXIT, self.reduce, move |el, t| el.bg(shade(DIM * (1. - t))))
        } else {
            animate(dim, "dim-in", FADE, self.reduce, move |el, t| el.bg(shade(DIM * t)))
        }
    }
}

pub struct Palette {
    browser: WeakEntity<Browser>,
    main: WindowHandle<Browser>,
    leaving: bool,
    _observe: Option<[Subscription; 2]>,
}

impl Palette {
    fn new(browser: WeakEntity<Browser>, main: WindowHandle<Browser>, cx: &mut Context<Self>) -> Self {
        let observe = browser.upgrade().map(|b| {
            let field = b.read(cx).command.clone();
            [cx.observe(&b, |_, _, cx| cx.notify()), cx.observe(&field, |_, _, cx| cx.notify())]
        });
        Palette { browser, main, leaving: false, _observe: observe }
    }

    fn leave(&mut self, reduce: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.leaving = true;
        self._observe = None;
        cx.notify();
        remove_after_exit(reduce, window, cx);
    }
}

fn hint(keys: &[&'static str], label: &'static str) -> impl IntoElement {
    div().flex().items_center().gap_1().children(keys.iter().map(|k| key_cap(*k))).child(div().pl_0p5().child(label))
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let b = browser.read(cx);
        let main = self.main;
        let commands = b.command.read(cx).text().starts_with('>');
        let input = div()
            .h(px(INPUT_ROW))
            .flex_none()
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .border_b_1()
            .border_color(Theme::hairline())
            .text_size(px(TEXT_TITLE))
            .child(icon(Icon::Search, ICON_MEDIUM, Theme::muted()))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(ui::text_input::mirror(b.command.clone())))
            .when(commands, |d| {
                d.child(
                    div()
                        .h(px(22.))
                        .px_2()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .rounded(px(Theme::radius_small()))
                        .border_1()
                        .border_color(Theme::hairline())
                        .bg(Theme::accent_soft())
                        .text_size(px(TEXT_SMALL))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(Theme::accent())
                        .child(icon(Icon::Command, 12., Theme::accent()))
                        .child("Commands"),
                )
            });
        let rows = suggestion_rows(b).into_iter().enumerate().map(|(ix, row)| {
            row.on_mouse_down(MouseButton::Left, move |_, _, cx| {
                main.update(cx, |b, window, cx| b.submit_command(ix, window, cx)).ok();
            })
        });
        let list = div().id("suggestion-rows").track_scroll(&b.suggestions_scroll).flex_1().min_h_0().overflow_y_scroll().py(px(PAD)).children(rows);
        let footer = div()
            .h(px(FOOTER))
            .flex_none()
            .px_3()
            .flex()
            .items_center()
            .gap_4()
            .border_t_1()
            .border_color(Theme::hairline())
            .text_size(px(TEXT_SMALL))
            .text_color(Theme::faint())
            .child(hint(&["↵"], "Run"))
            .child(hint(&["↑", "↓"], "Move"))
            .child(div().flex_1())
            .child(hint(&["esc"], "Close"));
        let card = surface(Theme::radius_card())
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .text_size(px(TEXT))
            .text_color(Theme::text())
            .child(input)
            .when(!b.suggestions.is_empty(), |d| d.child(list))
            .when(b.suggestions.is_empty(), |d| d.child(div().flex_1()))
            .child(footer);
        let reduce = b.pref(Pref::ReduceMotion);
        if self.leaving {
            return animate(card, "palette-out", EXIT, reduce, ui::motion::pop_out);
        }
        animate(card, "palette-in", POP, reduce, |el, t| el.opacity(t).relative().top(px((1. - t) * 5.)))
    }
}

pub(crate) fn section_header(title: &'static str) -> AnyElement {
    div()
        .h(px(SECTION_HEADER))
        .px_4()
        .pt_1p5()
        .flex()
        .items_center()
        .text_size(px(10.5))
        .font_weight(FontWeight::BOLD)
        .text_color(Theme::faint())
        .block_mouse_except_scroll()
        .child(title)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{FOOTER, INPUT_ROW, MARGIN, WIDTH, palette_frame};
    use gpui::{Bounds, point, px, size};

    #[test]
    fn the_palette_sits_centred_a_seventh_down_the_content() {
        let content = Bounds::new(point(px(300.), px(50.)), size(px(1100.), px(700.)));
        let frame = palette_frame(content, 200.);
        assert_eq!(f32::from(frame.size.width), WIDTH);
        assert_eq!(f32::from(frame.center().x), 850.);
        assert_eq!(f32::from(frame.origin.y), 150.);
        assert_eq!(f32::from(frame.size.height), INPUT_ROW + 200. + FOOTER);
    }

    #[test]
    fn a_small_content_area_narrows_and_shortens_the_palette() {
        let content = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(350.)));
        let frame = palette_frame(content, 2000.);
        assert_eq!(f32::from(frame.size.width), 400. - 2. * MARGIN);
        assert_eq!(f32::from(frame.bottom()), 350. - MARGIN);
    }
}
