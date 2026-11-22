use browser_core::capture;
use browser_core::tabs::TabKey;
use nsh_core::prelude::*;

use crate::controls::{label, pill};
use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Area,
}

/// A still of a tab's visible page drawn over it, which the user drags across; `page` and the
/// drag are in the page's CSS pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Overlay {
    pub purpose: Purpose,
    pub key: TabKey,
    pub seq: u64,
    pub page: (f64, f64),
    pub image: Option<String>,
    pub drag: Option<((f64, f64), (f64, f64))>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Capture(TabKey, [f64; 4]),
    Cancel,
}

impl Overlay {
    pub fn area(key: TabKey, seq: u64, page: (f64, f64)) -> Self {
        Overlay { purpose: Purpose::Area, key, seq, page, image: None, drag: None }
    }

    /// Takes the `snapshotRect` answer `seq` from `key`: `None` when it answers something else,
    /// `Some(false)` when the page gave no PNG.
    pub fn shown(&mut self, key: TabKey, seq: Option<u64>, png: Option<&str>) -> Option<bool> {
        if key != self.key || seq != Some(self.seq) {
            return None;
        }
        self.image = png.filter(|p| p.starts_with("data:image/png;base64,")).map(str::to_owned);
        Some(self.image.is_some())
    }

    pub fn press(&mut self, at: (f64, f64)) {
        self.drag = Some((at, at));
    }

    pub fn drag_to(&mut self, at: (f64, f64)) {
        if let Some((_, to)) = self.drag.as_mut() {
            *to = at;
        }
    }

    pub fn rect(&self) -> Option<[f64; 4]> {
        self.drag.map(|(from, to)| capture::drag_rect(from, to, self.page))
    }

    /// The area the drag ending at `at` selects; `None` for a click, which keeps the overlay open.
    pub fn release(&mut self, at: (f64, f64)) -> Option<[f64; 4]> {
        let (from, _) = self.drag.take()?;
        capture::drag_area(from, at, self.page).map(|_| capture::drag_rect(from, at, self.page))
    }
}

/// The overlay of `state` saying `hint`, filling its parent, which lays it over the page; `done` hears a drag's
/// area or a cancel from its Cancel button (the shell maps Esc).
pub fn layer(state: Signal<Option<Overlay>>, hint: &'static str, done: impl Fn(Outcome) + Copy + 'static) -> AnyElement {
    let shown = computed(move || state.with(|o| o.as_ref().map(|o| (o.key, o.seq))));
    dynamic(move || {
        let (key, _) = shown.get()?;
        let size = signal((0f32, 0f32));
        let scale = move |tracked: bool| {
            let width = if tracked { size.get().0 } else { size.get_untracked().0 };
            let page = state.with_untracked(|o| o.as_ref().map_or(0., |o| o.page.0));
            if page > 0. && width > 0. { f64::from(width) / page } else { 1. }
        };
        let css = move |p: Point<Points>| (f64::from(p.x.0) / scale(false), f64::from(p.y.0) / scale(false));
        let edit = move |f: &dyn Fn(&mut Overlay)| state.update(|o| o.iter_mut().for_each(f));
        let still = dynamic(move || state.with(|o| o.as_ref()?.image.clone()).map(|png| image(png).w_full().h_full().pointer_events(PointerEvents::None)));
        let outline = dynamic(move || {
            let [x, y, w, h] = state.with(|o| o.as_ref()?.rect())?;
            let s = scale(true);
            Some(
                view()
                    .absolute()
                    .left(px((x * s) as f32))
                    .top(px((y * s) as f32))
                    .w(px((w * s) as f32))
                    .h(px((h * s) as f32))
                    .border_2()
                    .border_color(Theme::accent())
                    .pointer_events(PointerEvents::None),
            )
        });
        let surface = view()
            .absolute()
            .inset_0()
            .accessible(true)
            .label(hint)
            .on_layout(move |b: &Bounds<Points>| size.set((b.size.width.0, b.size.height.0)))
            .claim_touches(TouchClaim::on_start())
            .on_responder_grant(move |e: &ResponderEvent| edit(&|o| o.press(css(e.location))))
            .on_responder_move(move |e: &ResponderEvent| edit(&|o| o.drag_to(css(e.location))))
            .on_responder_release(move |e: &ResponderEvent| {
                let mut area = None;
                state.update(|o| area = o.as_mut().and_then(|o| o.release(css(e.location))));
                if let Some(area) = area {
                    done(Outcome::Capture(key, area));
                }
            })
            .on_responder_terminate(move |_: &ResponderEvent| edit(&|o| o.drag = None))
            .child(still)
            .child(view().absolute().inset_0().bg(Rgba { a: 0.35, ..rgb(0) }).pointer_events(PointerEvents::None))
            .child(outline);
        let hint = view().absolute().top(px(12.)).left(px(0.)).right(px(0.)).items_center().pointer_events(PointerEvents::BoxNone).child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .pl_4()
                .pr_1()
                .py_1()
                .rounded(px(Theme::radius_panel()))
                .bg(Theme::panel())
                .border_1()
                .border_color(Theme::surface_edge())
                .shadow(Theme::elevation(4., 14.))
                .child(label(hint).text_color(Theme::text))
                .child(pill("Cancel", move || done(Outcome::Cancel))),
        );
        Some(view().absolute().inset_0().z_index(10).child(surface).child(hint))
    })
    .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: TabKey = TabKey { private: false, id: 1 };

    #[test]
    fn a_drag_selects_its_area_and_a_click_keeps_the_overlay() {
        let mut overlay = Overlay::area(KEY, 7, (800., 600.));
        overlay.press((10., 20.));
        overlay.drag_to((5., 300.));
        assert_eq!(overlay.rect(), Some([5., 20., 5., 280.]));
        assert_eq!(overlay.release((110., 70.)), Some([10., 20., 100., 50.]));
        assert_eq!(overlay.drag, None);
        overlay.press((40., 40.));
        assert_eq!(overlay.release((41., 41.)), None, "a click");
        assert_eq!(overlay.release((200., 200.)), None, "no drag held");
        overlay.press((700., 500.));
        assert_eq!(overlay.release((900., 900.)), Some([700., 500., 100., 100.]), "cut to the page");
    }

    #[test]
    fn only_its_own_png_answer_is_shown() {
        let mut overlay = Overlay::area(KEY, 7, (800., 600.));
        let png = "data:image/png;base64,AAAA";
        assert_eq!(overlay.shown(TabKey { private: true, id: 1 }, Some(7), Some(png)), None);
        assert_eq!(overlay.shown(KEY, Some(8), Some(png)), None);
        assert_eq!(overlay.shown(KEY, Some(7), Some("data:text/html,x")), Some(false));
        assert_eq!(overlay.shown(KEY, Some(7), None), Some(false));
        assert_eq!(overlay.shown(KEY, Some(7), Some(png)), Some(true));
        assert_eq!(overlay.image.as_deref(), Some(png));
    }
}
