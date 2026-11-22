//! A still of the page that the shell draws in place of the web view, so the user can drag out a
//! rectangle on it without any script of Tarn's running on the page.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use browser_core::capture::{AREA_HINT, drag_area, drag_rect};
use engine::view_policy::SnapshotArea;
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyElement, AppContext, Bounds, Context, CursorStyle, Focusable, Image, ImageFormat, InteractiveElement, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ObjectFit, ParentElement, Pixels, Point, Rgba, Styled, StyledImage, Window, canvas, div, img, prelude::FluentBuilder, px,
    relative,
};
use ui::controls::surface;
use ui::theme::{Elevation, TEXT_SMALL, Theme};

use crate::browser::{Browser, TabKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Area,
}

pub struct Overlay {
    pub key: TabKey,
    pub purpose: Purpose,
    image: Arc<Image>,
    /// The page's size in points when the still was taken, and its zoom, which turns points into CSS pixels.
    page: (f32, f32),
    zoom: f64,
    drag: Option<((f32, f32), (f32, f32))>,
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
    pub(crate) fn capture_area(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let (Some(view), Some((_, _, width, height))) = (self.views.get(&key).cloned(), self.link_rect(key)) else { return };
        let (weak, zoom, (tx, mut rx)) = (Rc::downgrade(&view), view.zoom(), mpsc::unbounded());
        view.snapshot_png(None, move |png| drop(tx.unbounded_send(png)));
        cx.spawn_in(window, async move |this, cx| {
            let Some(png) = rx.next().await else { return };
            this.update_in(cx, |b, window, cx| {
                if b.model.active != Some(key) || !b.same_view(key, &weak) {
                    return;
                }
                let Some(png) = png else { return b.capture_tab(key, true, |_, done| done(None), cx) };
                b.capture_overlay = Some(Overlay {
                    key,
                    purpose: Purpose::Area,
                    image: Arc::new(Image::from_bytes(ImageFormat::Png, png)),
                    page: (width, height),
                    zoom,
                    drag: None,
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

    fn drag_capture(&mut self, at: Point<Pixels>, start: bool, cx: &mut Context<Self>) {
        let Some(overlay) = self.capture_overlay.as_mut() else { return };
        let point = overlay.local(at);
        match &mut overlay.drag {
            _ if start => overlay.drag = Some((point, point)),
            Some((_, to)) => *to = point,
            None => return,
        }
        cx.notify();
    }

    /// A drag selects the area; a click keeps the overlay open.
    fn end_capture_drag(&mut self, cx: &mut Context<Self>) {
        let Some(overlay) = self.capture_overlay.as_mut() else { return };
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
    let dim = Rgba { r: 0., g: 0., b: 0., a: crate::address::DIM };
    let band = |x: f32, y: f32, bw: f32, bh: f32| div().absolute().left(relative(x / w)).top(relative(y / h)).w(relative(bw / w)).h(relative(bh / h));
    let bounds = overlay.bounds.clone();
    let rect = overlay.drag.map(|(from, to)| {
        let f = |p: (f32, f32)| (f64::from(p.0), f64::from(p.1));
        drag_rect(f(from), f(to), f(overlay.page)).map(|v| v as f32)
    });
    let shade = match rect {
        None => vec![band(0., 0., w, h).bg(dim)],
        Some([x, y, rw, rh]) => vec![
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
        },
    );
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
        .when(rect.is_none(), |d| d.child(div().absolute().top_3().left_0().right_0().flex().justify_center().child(hint)))
        .on_mouse_down(MouseButton::Left, cx.listener(|b, e: &MouseDownEvent, _, cx| b.drag_capture(e.position, true, cx)))
        .on_mouse_move(cx.listener(|b, e: &MouseMoveEvent, _, cx| {
            if e.dragging() {
                b.drag_capture(e.position, false, cx);
            }
        }))
        .on_mouse_up(MouseButton::Left, cx.listener(end))
        .on_mouse_up_out(MouseButton::Left, cx.listener(end));
    Some(layer.into_any_element())
}

#[cfg(test)]
mod tests {
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
