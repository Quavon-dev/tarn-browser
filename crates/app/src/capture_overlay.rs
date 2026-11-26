//! A still of the page that the shell draws in place of the web view, so the user can drag out a
//! rectangle on it without any script of Tarn's running on the page.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use browser_core::capture::{AREA_HINT, drag_area, drag_rect};
use browser_core::measure;
use engine::view_policy::SnapshotArea;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, AppContext, Bounds, Context, CursorStyle, Focusable, Image, ImageFormat, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, Pixels, Point, StatefulInteractiveElement, Styled, StyledImage, Window, canvas, div, img,
    prelude::FluentBuilder, px, relative,
};
use ui::controls::surface;
use ui::theme::{Elevation, TEXT_SMALL, TEXT_TINY, Theme};

use crate::browser::{Browser, TabKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Area,
    Measure,
}

pub struct Overlay {
    pub key: TabKey,
    pub purpose: Purpose,
    image: Arc<Image>,
    /// The page's size in points when the still was taken, and its zoom, which turns points into CSS pixels.
    page: (f32, f32),
    zoom: f64,
    drag: Option<((f32, f32), (f32, f32))>,
    /// Where the pointer rests over Measure with no button down, in page points.
    cursor: Option<(f32, f32)>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl Overlay {
    fn css(&self, (x, y): (f32, f32)) -> (f64, f64) {
        (f64::from(x) / self.zoom, f64::from(y) / self.zoom)
    }

    /// A window position in page points; the still fills the bounds it was last drawn in.
    fn local(&self, at: Point<Pixels>) -> (f32, f32) {
        let b = self.bounds.get();
        let scale = |v: Pixels, origin: Pixels, size: Pixels, page: f32| if size > px(0.) { (v - origin) / size * page } else { 0. };
        (scale(at.x, b.origin.x, b.size.width, self.page.0), scale(at.y, b.origin.y, b.size.height, self.page.1))
    }
}

impl Browser {
    pub(crate) fn open_overlay(&mut self, purpose: Purpose, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let (Some(view), Some((_, _, width, height))) = (self.views.get(&key).cloned(), self.link_rect(key)) else { return };
        let (weak, zoom, (tx, mut rx)) = (Rc::downgrade(&view), view.zoom(), mpsc::unbounded());
        self.overlay_request = (self.overlay_request.0 + 1, purpose == Purpose::Measure);
        let request = self.overlay_request.0;
        view.snapshot_png(None, move |png| drop(tx.unbounded_send(png)));
        cx.spawn_in(window, async move |this, cx| {
            let Some(png) = rx.next().await else { return };
            this.update_in(cx, |b, window, cx| {
                if b.overlay_request.0 != request {
                    return;
                }
                b.overlay_request.1 = false;
                if b.model.active != Some(key) || !b.same_view(key, &weak) {
                    return;
                }
                let Some(png) = png else {
                    return if purpose == Purpose::Area {
                        b.capture_tab(key, true, |_, done| done(None), cx)
                    };
                };
                b.capture_overlay = Some(Overlay {
                    key,
                    purpose,
                    image: Arc::new(Image::from_bytes(ImageFormat::Png, png)),
                    page: (width, height),
                    zoom,
                    drag: None,
                    cursor: None,
                    bounds: Default::default(),
                });
                if let Some(host) = &b.host {
                    host.focus();
                }
                window.focus(&b.focus_handle(cx));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Closes the overlay and gives the keyboard back to its page; false when none was open.
    pub(crate) fn close_capture_overlay(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(overlay) = self.capture_overlay.take() else { return false };
        if let Some(view) = self.views.get(&overlay.key).filter(|_| self.model.active == Some(overlay.key)) {
            view.focus();
        }
        cx.notify();
        true
    }

    /// Measure from the toolbar, the palette or its menu: a second use closes it.
    pub(crate) fn toggle_measure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.measuring() || self.overlay_request.1 {
            self.overlay_request = (self.overlay_request.0 + 1, false);
            self.close_capture_overlay(cx);
        } else if self.devtools_on() {
            self.open_overlay(Purpose::Measure, window, cx);
        }
    }

    pub(crate) fn measuring(&self) -> bool {
        self.capture_overlay.as_ref().is_some_and(|o| o.purpose == Purpose::Measure)
    }

    fn hover_capture(&mut self, at: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        let Some(overlay) = self.capture_overlay.as_mut().filter(|o| o.purpose == Purpose::Measure) else { return };
        let cursor = at.map(|at| overlay.local(at));
        if overlay.cursor != cursor {
            overlay.cursor = cursor;
            cx.notify();
        }
    }

    fn drag_capture(&mut self, at: Point<Pixels>, start: bool, cx: &mut Context<Self>) {
        let Some(overlay) = self.capture_overlay.as_mut() else { return };
        let point = overlay.local(at);
        match &mut overlay.drag {
            _ if start => {
                overlay.drag = Some((point, point));
                overlay.cursor = None;
            }
            Some((_, to)) => *to = point,
            None => return,
        }
        cx.notify();
    }

    /// A drag selects the area; a click keeps Capture Area open and closes Measure. A measured box stays until the next drag.
    fn end_capture_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(overlay) = self.capture_overlay.as_ref().filter(|o| o.purpose == Purpose::Measure) {
            if overlay.drag.is_some_and(|(from, to)| measure::is_click(overlay.css(from), overlay.css(to))) {
                self.close_capture_overlay(cx);
            }
            return;
        }
        let Some(overlay) = self.capture_overlay.as_mut().filter(|o| o.purpose == Purpose::Area) else { return };
        let Some((from, to)) = overlay.drag.take() else { return };
        let area = drag_area(overlay.css(from), overlay.css(to), overlay.css(overlay.page));
        let Some(SnapshotArea::Rect { x, y, width, height }) = area else { return cx.notify() };
        let (key, image, zoom) = (overlay.key, overlay.image.clone(), overlay.zoom);
        let scale = image_width(&image).map_or(1., |w| f64::from(w) / f64::from(overlay.page.0)) * zoom;
        self.close_capture_overlay(cx);
        // The hidden web view only gets its frame back on the next layout, so the area is cut from the still the user saw.
        let cropped = cx.background_spawn(async move { crop(image.bytes(), [x, y, width, height].map(|v| v * scale)) });
        cx.spawn(async move |this, cx| {
            let bytes = cropped.await;
            this.update(cx, |b, cx| b.capture_tab(key, true, move |_, done| done(bytes), cx)).ok();
        })
        .detach();
    }
}

fn image_width(image: &Image) -> Option<u32> {
    image::ImageReader::with_format(std::io::Cursor::new(image.bytes()), image::ImageFormat::Png).into_dimensions().ok().map(|(w, _)| w)
}

/// `rect` of the PNG `png`, in its pixels, as a PNG.
fn crop(png: &[u8], [x, y, width, height]: [f64; 4]) -> Option<Vec<u8>> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let part = image.crop_imm(x.round() as u32, y.round() as u32, width.round().max(1.) as u32, height.round().max(1.) as u32);
    let mut out = std::io::Cursor::new(Vec::new());
    part.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// The overlay over `key`'s page, when it is open there.
pub(crate) fn render(b: &Browser, key: TabKey, cx: &mut Context<Browser>) -> Option<AnyElement> {
    let overlay = b.capture_overlay.as_ref().filter(|o| o.key == key)?;
    let (w, h) = overlay.page;
    let dim = Theme::scrim(crate::address::DIM);
    let band = |x: f32, y: f32, bw: f32, bh: f32| div().absolute().left(relative(x / w)).top(relative(y / h)).w(relative(bw / w)).h(relative(bh / h));
    let bounds = overlay.bounds.clone();
    let f = |p: (f32, f32)| (f64::from(p.0), f64::from(p.1));
    let rect = overlay.drag.map(|(from, to)| drag_rect(f(from), f(to), f(overlay.page)).map(|v| v as f32));
    let shade = match (overlay.purpose, rect) {
        (Purpose::Measure, None) => Vec::new(),
        (Purpose::Measure, Some([x, y, rw, rh])) => {
            let [px_, py_] = pill_origin([x, y, rw, rh], h);
            let css = measure::css_box(f(from_to(overlay).0), f(from_to(overlay).1), f(overlay.page), overlay.zoom);
            vec![
                band(x, y, rw, rh).border_1().border_color(Theme::accent()).bg(Theme::accent_soft()),
                band(px_, py_, 0., 0.).child(
                    div()
                        .absolute()
                        .px_1p5()
                        .py_0p5()
                        .rounded(px(Theme::radius_small()))
                        .bg(Theme::accent())
                        .text_color(Theme::on_accent())
                        .text_size(px(TEXT_SMALL))
                        .whitespace_nowrap()
                        .child(measure::label(css)),
                ),
            ]
        }
        (Purpose::Area, None) => vec![band(0., 0., w, h).bg(dim)],
        (Purpose::Area, Some([x, y, rw, rh])) => vec![
            band(0., 0., w, y).bg(dim),
            band(0., y + rh, w, h - y - rh).bg(dim),
            band(0., y, x, rh).bg(dim),
            band(x + rw, y, w - x - rw, rh).bg(dim),
            band(x, y, rw, rh).border_2().border_color(Theme::accent()),
        ],
    };
    let hint = surface(Theme::radius_panel()).shadow(Elevation::Floating.shadow()).px_3().py_1p5().text_size(px(TEXT_SMALL)).text_color(Theme::text()).child(
        match overlay.purpose {
            Purpose::Area => AREA_HINT,
            Purpose::Measure => measure::HINT,
        },
    );
    let hint_top = 12. + if overlay.purpose == Purpose::Measure { measure::RULER as f32 } else { 0. };
    let end = |b: &mut Browser, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Browser>| b.end_capture_drag(cx);
    let layer = div()
        .id("capture-overlay")
        .absolute()
        .inset_0()
        .overflow_hidden()
        .rounded(px(Theme::radius_page() - 1.))
        .cursor(CursorStyle::Crosshair)
        .child(img(overlay.image.clone()).size_full().object_fit(ObjectFit::Fill))
        .child(canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {}).absolute().inset_0())
        .children(shade)
        .when(overlay.purpose == Purpose::Measure, |d| d.children(rulers(overlay)).children(crosshair(overlay).into_iter().flatten()))
        .when(rect.is_none(), |d| d.child(div().absolute().top(px(hint_top)).left_0().right_0().flex().justify_center().child(hint)))
        .on_mouse_down(MouseButton::Left, cx.listener(|b, e: &MouseDownEvent, _, cx| b.drag_capture(e.position, true, cx)))
        .on_mouse_move(cx.listener(|b, e: &MouseMoveEvent, _, cx| {
            if e.dragging() {
                b.drag_capture(e.position, false, cx);
            } else {
                b.hover_capture(Some(e.position), cx);
            }
        }))
        .on_hover(cx.listener(|b, hovered: &bool, _, cx| {
            if !hovered {
                b.hover_capture(None, cx);
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(end))
        .on_mouse_up_out(MouseButton::Left, cx.listener(end));
    Some(layer.into_any_element())
}

/// Guides through the resting pointer and its position in CSS pixels, while no button is down.
fn crosshair(overlay: &Overlay) -> Option<[gpui::Div; 3]> {
    let (x, y) = overlay.cursor?;
    let (w, h) = overlay.page;
    let (lx, ly) = (x.min(w - LABEL.0).max(0.), y.min(h - LABEL.1).max(0.));
    let line = || div().absolute().bg(Theme::accent());
    let label = div()
        .absolute()
        .left(relative(lx / w))
        .top(relative(ly / h))
        .ml(px(10.))
        .mt(px(10.))
        .px_1p5()
        .py_0p5()
        .rounded(px(Theme::radius_small()))
        .bg(Theme::accent())
        .text_color(Theme::on_accent())
        .text_size(px(TEXT_SMALL))
        .whitespace_nowrap()
        .child(measure::point_label(overlay.css((x, y))));
    Some([line().left(relative(x / w)).top_0().bottom_0().w(px(1.)), line().top(relative(y / h)).left_0().right_0().h(px(1.)), label])
}

fn from_to(overlay: &Overlay) -> ((f32, f32), (f32, f32)) {
    overlay.drag.unwrap_or_default()
}

/// Where the measured box's label goes: under the box, or over it when the page ends below.
fn pill_origin([x, y, _, h]: [f32; 4], page_height: f32) -> [f32; 2] {
    let below = y + h + 6.;
    [x, if below + PILL < page_height { below } else { (y - PILL - 6.).max(0.) }]
}

const PILL: f32 = 22.;
/// Room the pointer's position label needs, so it stays inside the page near its right and bottom edges.
const LABEL: (f32, f32) = (90., 32.);

/// The top and left rulers, numbered in the page's CSS pixels.
fn rulers(overlay: &Overlay) -> [gpui::Div; 2] {
    let ((w, h), zoom, thick) = (overlay.page, overlay.zoom, measure::RULER as f32);
    let ruler = |horizontal: bool| {
        let length = if horizontal { w } else { h };
        let marks = measure::ticks(f64::from(length), zoom).into_iter().flat_map(move |tick| {
            let at = relative(tick.at as f32 / length);
            let size = thick * if tick.label.is_some() { 0.5 } else { 0.25 };
            let mark = div().absolute().bg(Theme::faint());
            let mark = if horizontal { mark.left(at).bottom_0().w(px(1.)).h(px(size)) } else { mark.top(at).right_0().h(px(1.)).w(px(size)) };
            let text = tick.label.map(|label| {
                let text = div().absolute().text_size(px(TEXT_TINY)).line_height(px(TEXT_TINY)).text_color(Theme::muted());
                // Stacked, four-digit numbers stay clear of the left ruler's marks.
                if horizontal {
                    text.whitespace_nowrap().left(at).ml(px(3.)).top_0().child(label)
                } else {
                    text.top(at).left(px(3.)).mt(px(2.)).flex().flex_col().items_center().children(label.chars().map(|c| c.to_string()))
                }
            });
            std::iter::once(mark).chain(text)
        });
        let bar = div().absolute().overflow_hidden().bg(Theme::panel()).border_color(Theme::separator());
        let bar = if horizontal { bar.top_0().left_0().right_0().h(px(thick)).border_b_1() } else { bar.top_0().left_0().bottom_0().w(px(thick)).border_r_1() };
        bar.children(marks)
    };
    [ruler(true), ruler(false)]
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_measure_label_sits_under_the_box_unless_the_page_ends() {
        assert_eq!(super::pill_origin([10., 20., 100., 50.], 600.), [10., 76.]);
        assert_eq!(super::pill_origin([10., 500., 100., 90.], 600.), [10., 472.]);
        assert_eq!(super::pill_origin([0., 2., 100., 595.], 600.), [0., 0.]);
    }

    #[test]
    fn crops_the_still_to_the_dragged_area() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_fn(8, 6, |x, _| image::Rgba([x as u8, 0, 0, 255])).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let part = image::load_from_memory(&super::crop(png.get_ref(), [2., 1., 3., 4.]).unwrap()).unwrap().to_rgba8();
        assert_eq!(part.dimensions(), (3, 4));
        assert_eq!(part.get_pixel(0, 0)[0], 2);
        assert!(super::crop(b"not a png", [0., 0., 1., 1.]).is_none());
    }
}
