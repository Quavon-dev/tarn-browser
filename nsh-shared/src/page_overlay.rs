use browser_core::tabs::TabKey;
use browser_core::{capture, measure};
use nsh_core::prelude::*;

use crate::controls::{label, pill};
use crate::theme::{TEXT_SMALL, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Area,
    Measure,
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

    pub fn measure(key: TabKey, seq: u64, page: (f64, f64)) -> Self {
        Overlay { purpose: Purpose::Measure, ..Overlay::area(key, seq, page) }
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

    /// What the press ending at `at` does: a drag over Capture Area captures its area and a click
    /// keeps the overlay open; Measure keeps a dragged box shown until the next drag and closes on a click.
    pub fn release(&mut self, at: (f64, f64)) -> Option<Outcome> {
        if self.purpose == Purpose::Measure {
            let (from, _) = self.drag?;
            self.drag_to(at);
            return measure::is_click(from, at).then_some(Outcome::Cancel);
        }
        let (from, _) = self.drag.take()?;
        capture::drag_area(from, at, self.page).map(|_| Outcome::Capture(self.key, capture::drag_rect(from, at, self.page)))
    }
}

/// The overlay of `state`, filling its parent, which lays it over the page; `done` hears a drag's
/// area or a cancel from its Cancel button (the shell maps Esc). Capture Area says `area_hint`;
/// Measure adds rulers and keeps its box with a label of its CSS size.
pub fn layer(state: Signal<Option<Overlay>>, area_hint: &'static str, done: impl Fn(Outcome) + Copy + 'static) -> AnyElement {
    let shown = computed(move || state.with(|o| o.as_ref().map(|o| (o.key, o.seq, o.purpose))));
    dynamic(move || {
        let (_, _, purpose) = shown.get()?;
        let measuring = purpose == Purpose::Measure;
        let hint = if measuring { measure::HINT } else { area_hint };
        let size = signal((0f32, 0f32));
        let cursor: Signal<Option<(f64, f64)>> = signal(None);
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
        let size_label = dynamic(move || {
            let rect @ [x, y, _, h] = state.with(|o| o.as_ref().filter(|o| o.purpose == Purpose::Measure)?.rect())?;
            let s = scale(true);
            let top = (((y + h) * s) as f32 + 4.).min(size.get().1 - 24.).max(0.);
            Some(
                view()
                    .absolute()
                    .left(px((x * s) as f32))
                    .top(px(top))
                    .px_2()
                    .rounded(px(Theme::radius_row()))
                    .bg(Theme::accent())
                    .pointer_events(PointerEvents::None)
                    .child(text(measure::label(rect)).text_size(px(TEXT_SMALL)).text_color(|| Theme::on_fill(Theme::accent())).number_of_lines(1_u32)),
            )
        });
        let crosshair = dynamic(move || {
            let (x, y) = cursor.get()?;
            let s = scale(true);
            let (left, top) = ((x * s) as f32, (y * s) as f32);
            let (width, height) = size.get();
            let (label_left, label_top) = ((left + 10.).min(width - 90.).max(0.), (top + 10.).min(height - 28.).max(0.));
            let line = || view().absolute().bg(Theme::accent()).pointer_events(PointerEvents::None);
            Some(
                view()
                    .absolute()
                    .inset_0()
                    .pointer_events(PointerEvents::None)
                    .child(line().left(px(left)).top(px(0.)).bottom(px(0.)).w(px(1.)))
                    .child(line().top(px(top)).left(px(0.)).right(px(0.)).h(px(1.)))
                    .child(view().absolute().left(px(label_left)).top(px(label_top)).px_2().rounded(px(Theme::radius_row())).bg(Theme::accent()).child(
                        text(measure::point_label((x, y))).text_size(px(TEXT_SMALL)).text_color(|| Theme::on_fill(Theme::accent())).number_of_lines(1_u32),
                    )),
            )
        });
        let surface = view()
            .absolute()
            .inset_0()
            .accessible(true)
            .label(hint)
            .on_layout(move |b: &Bounds<Points>| size.set((b.size.width.0, b.size.height.0)))
            .claim_touches(TouchClaim::on_start())
            .on_responder_grant(move |e: &ResponderEvent| {
                cursor.set(None);
                edit(&|o| o.press(css(e.location)));
            })
            .on_responder_move(move |e: &ResponderEvent| edit(&|o| o.drag_to(css(e.location))))
            .on_responder_release(move |e: &ResponderEvent| {
                let mut outcome = None;
                state.update(|o| outcome = o.as_mut().and_then(|o| o.release(css(e.location))));
                if let Some(outcome) = outcome {
                    done(outcome);
                }
            })
            .on_mouse_move(move |e: &MouseEvent| {
                if measuring && e.pressed_button.is_none() {
                    cursor.set(Some(css(e.position)));
                }
            })
            .on_mouse_leave(move |_: &MouseEvent| cursor.set(None))
            .on_responder_terminate(move |_: &ResponderEvent| edit(&|o| o.drag = None))
            .child(still)
            .children((!measuring).then(|| view().absolute().inset_0().bg(Rgba { a: 0.35, ..rgb(0) }).pointer_events(PointerEvents::None)))
            .child(outline)
            .child(size_label)
            .child(crosshair)
            .children(measuring.then(|| ruler(size, move || scale(true), true)))
            .children(measuring.then(|| ruler(size, move || scale(true), false)));
        let inset = if measuring { 12. + measure::RULER as f32 } else { 12. };
        let bar = view().absolute().top(px(inset)).left(px(0.)).right(px(0.)).items_center().pointer_events(PointerEvents::BoxNone).child(
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
                .child(pill(if measuring { "Close" } else { "Cancel" }, move || done(Outcome::Cancel))),
        );
        Some(view().absolute().inset_0().z_index(10).child(surface).child(bar))
    })
    .into_element()
}

/// A ruler along the top (`top`) or the left edge, numbered in the page's CSS pixels at `zoom`.
fn ruler(size: Signal<(f32, f32)>, zoom: impl Fn() -> f64 + Copy + 'static, top: bool) -> AnyElement {
    dynamic(move || {
        let (width, height) = size.get();
        let thick = measure::RULER as f32;
        let mut marks: Vec<AnyElement> = Vec::new();
        for tick in measure::ticks(f64::from(if top { width } else { height }), zoom()) {
            let at = tick.at as f32;
            let long = if tick.label.is_some() { thick * 0.6 } else { thick * 0.3 };
            let line = if top { view().left(px(at)).bottom(px(0.)).w(px(1.)).h(px(long)) } else { view().top(px(at)).right(px(0.)).h(px(1.)).w(px(long)) };
            marks.push(line.absolute().bg(Theme::muted()).into_element());
            if let Some(number) = tick.label {
                let place = if top { view().left(px(at + 2.)).top(px(0.)) } else { view().top(px(at + 2.)).left(px(1.)) };
                marks.push(place.absolute().child(text(number).text_size(px(TEXT_SMALL)).text_color(Theme::muted).number_of_lines(1_u32)).into_element());
            }
        }
        let bar = if top {
            view().top(px(0.)).left(px(0.)).right(px(0.)).h(px(thick)).border_b_1()
        } else {
            view().top(px(0.)).left(px(0.)).bottom(px(0.)).w(px(thick)).border_r_1()
        };
        Some(
            bar.absolute()
                .bg(Theme::panel())
                .border_color(Theme::surface_edge())
                .overflow_hidden()
                .pointer_events(PointerEvents::None)
                .accessibility_elements_hidden(true)
                .children(marks),
        )
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
        assert_eq!(overlay.release((110., 70.)), Some(Outcome::Capture(KEY, [10., 20., 100., 50.])));
        assert_eq!(overlay.drag, None);
        overlay.press((40., 40.));
        assert_eq!(overlay.release((41., 41.)), None, "a click");
        assert_eq!(overlay.release((200., 200.)), None, "no drag held");
        overlay.press((700., 500.));
        assert_eq!(overlay.release((900., 900.)), Some(Outcome::Capture(KEY, [700., 500., 100., 100.])), "cut to the page");
    }

    #[test]
    fn a_measure_keeps_its_box_until_the_next_drag() {
        let mut overlay = Overlay::measure(KEY, 7, (800., 600.));
        overlay.press((10., 20.));
        assert_eq!(overlay.release((110., 70.)), None, "nothing is captured");
        assert_eq!(overlay.rect(), Some([10., 20., 100., 50.]));
        overlay.press((300., 300.));
        overlay.drag_to((350., 320.));
        assert_eq!(overlay.rect(), Some([300., 300., 50., 20.]), "a new drag replaces it");
    }

    #[test]
    fn a_click_closes_measure_and_a_drag_keeps_its_box() {
        let mut overlay = Overlay::measure(KEY, 7, (800., 600.));
        overlay.press((10., 20.));
        overlay.drag_to((110., 70.));
        assert_eq!(overlay.release((110., 70.)), None);
        overlay.press((300., 300.));
        assert_eq!(overlay.release((301., 302.)), Some(Outcome::Cancel), "a click closes");
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
