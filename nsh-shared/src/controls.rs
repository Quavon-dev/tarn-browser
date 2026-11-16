use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use nsh_base::{ButtonElement, button};
use nsh_core::element::IntoProp;
use nsh_core::prelude::*;

use crate::theme::{Paint, RADIUS_ROW, SYSTEM_FONT, Secure, TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Density {
    Touch,
    Desktop,
}

struct Metrics {
    target: f32,
    text: f32,
    pill_height: f32,
    segment_height: f32,
    segment_radius: f32,
    switch_row: f32,
    track: (f32, f32),
    thumb: f32,
    icon: f32,
}

const TOUCH: Metrics = Metrics {
    target: 44.,
    text: TEXT_TITLE,
    pill_height: 44.,
    segment_height: 44.,
    segment_radius: RADIUS_ROW,
    switch_row: 44.,
    track: (51., 31.),
    thumb: 27.,
    icon: 20.,
};

const DESKTOP: Metrics =
    Metrics { target: 28., text: TEXT, pill_height: 26., segment_height: 22., segment_radius: 11., switch_row: 40., track: (34., 20.), thumb: 16., icon: 16. };

static DESKTOP_DENSITY: AtomicBool = AtomicBool::new(false);

pub fn set_density(density: Density) {
    DESKTOP_DENSITY.store(density == Density::Desktop, Ordering::Relaxed);
}

pub fn density() -> Density {
    if DESKTOP_DENSITY.load(Ordering::Relaxed) { Density::Desktop } else { Density::Touch }
}

fn metrics() -> &'static Metrics {
    match density() {
        Density::Touch => &TOUCH,
        Density::Desktop => &DESKTOP,
    }
}

pub fn icon_size() -> f32 {
    metrics().icon
}

pub fn label(content: impl IntoReactive<SharedString>) -> Text {
    text(content).text_size(px(metrics().text)).text_color(Theme::text)
}

pub fn pressable<M>(name: impl IntoProp<Option<SharedString>, M>, press: impl Fn() + 'static) -> ButtonElement {
    button().accessibility_label(name).on_press(press).hover(|s| s.bg(Theme::hover())).active(|s| s.bg(Theme::pressed()))
}

pub fn icon_button<M>(icon: impl IntoElement, name: impl IntoProp<Option<SharedString>, M>, press: impl Fn() + 'static) -> ButtonElement {
    let m = metrics();
    let button = pressable(name, press).w(px(m.target)).h(px(m.target)).items_center().justify_center();
    match density() {
        Density::Touch => button.rounded_full().text_color(Theme::text),
        Density::Desktop => button.rounded(px(Theme::radius_row())).text_color(Theme::muted),
    }
    .child(icon)
}

fn pill_button(name: String, press: impl Fn() + 'static, primary: bool) -> ButtonElement {
    let m = metrics();
    let button = if primary {
        button().accessibility_label(name.clone()).on_press(press).hover(|s| s.opacity(0.88)).active(|s| s.opacity(0.7))
    } else {
        pressable(name.clone(), press)
    }
    .flex_row()
    .items_center()
    .justify_center()
    .border_1()
    .border_color(if primary { Theme::surface_edge } else { Theme::hairline })
    .bg(if primary { Theme::accent } else { Theme::control });
    let text = label(name);
    match density() {
        Density::Touch => button.min_h(px(m.pill_height)).px_4().rounded_full(),
        Density::Desktop => button.h(px(m.pill_height)).px_3().rounded(px(m.pill_height / 2.)),
    }
    .child(if primary { text.font_weight(FontWeight::MEDIUM).text_color(Theme::on_accent) } else { text })
}

pub fn secure_button(name: impl Into<String>, press: impl Fn() + 'static, primary: bool) -> ButtonElement {
    let name: String = name.into();
    let m = metrics();
    let (fill, edge) = if primary { (Secure::accent(), Secure::surface_edge()) } else { (Secure::control(), Secure::hairline()) };
    let button = button()
        .accessibility_label(name.clone())
        .on_press(press)
        .flex_row()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(edge)
        .bg(fill)
        .hover(move |s| if primary { s.opacity(0.88) } else { s.bg(Secure::hover()) })
        .active(move |s| if primary { s.opacity(0.7) } else { s.bg(Secure::pressed()) });
    let text = text(name).text_size(px(m.text)).font_family(SYSTEM_FONT).text_color(if primary { Theme::on_accent() } else { Secure::text() });
    match density() {
        Density::Touch => button.min_h(px(m.pill_height)).px_4().rounded_full(),
        Density::Desktop => button.h(px(m.pill_height)).px_3().rounded(px(m.pill_height / 2.)),
    }
    .child(if primary { text.font_weight(FontWeight::MEDIUM) } else { text })
}

pub fn secure_surface(radius: f32) -> View {
    view()
        .rounded(px(radius))
        .bg(Secure::surface())
        .border_1()
        .border_color(Secure::surface_edge())
        .text_color(Secure::text())
        .font_family(SYSTEM_FONT)
        .overflow_hidden()
}

pub fn pill(name: impl Into<String>, press: impl Fn() + 'static) -> ButtonElement {
    pill_button(name.into(), press, false)
}

pub fn accent_pill(name: impl Into<String>, press: impl Fn() + 'static) -> ButtonElement {
    pill_button(name.into(), press, true)
}

pub type Segment = (String, Box<dyn Fn() -> bool>, Box<dyn Fn()>);

pub fn segmented(options: Vec<Segment>) -> View {
    let m = metrics();
    let touch = density() == Density::Touch;
    let segments: Vec<AnyElement> = options
        .into_iter()
        .map(|(name, chosen, press)| {
            let chosen = Rc::new(chosen);
            let selected = chosen.clone();
            let bold = chosen.clone();
            let shown = chosen.clone();
            let size = if touch { TEXT } else { TEXT_SMALL };
            pressable(name.clone(), press)
                .min_h(px(m.segment_height))
                .px_3()
                .items_center()
                .justify_center()
                .rounded(px(m.segment_radius))
                .bg(move || if chosen() { Theme::selected() } else { Theme::clear() })
                .selected(move || selected())
                .child(
                    text(name)
                        .text_size(px(size))
                        .style_when(move || bold(), |s| s.font_weight(FontWeight::MEDIUM))
                        .text_color(move || if shown() { Theme::text() } else { Theme::muted() }),
                )
                .into_element()
        })
        .collect();
    let group = view().flex_row().flex_wrap().gap_1().children(segments);
    if touch { group.p_1().rounded(px(Theme::radius_group())).bg(Theme::group) } else { group.p(px(2.)).gap(px(2.)).rounded_full().bg(Theme::control) }
}

pub fn switch_row(name: impl Into<String>, on: impl Fn() -> bool + Copy + 'static, toggle: impl Fn() + 'static) -> ButtonElement {
    let name: String = name.into();
    let m = metrics();
    let (width, height) = m.track;
    let spoken = name.clone();
    button()
        .flex_row()
        .items_center()
        .min_h(px(m.switch_row))
        .px_3()
        .accessibility_label(move || SharedString::from(format!("{spoken}: {}", if on() { "On" } else { "Off" })))
        .role(AccessibilityRole::Switch)
        .selected(on)
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .on_press(toggle)
        .child(label(name).flex_1())
        .child(
            view()
                .w(px(width))
                .h(px(height))
                .p(px((height - m.thumb) / 2.))
                .rounded_full()
                .flex_row()
                .bg(move || if on() { Theme::accent() } else { Theme::control() })
                .style_when(on, |s| s.justify_end())
                .child(view().w(px(m.thumb)).h(px(m.thumb)).rounded_full().bg(Theme::text).shadow(Theme::elevation(1., 2.))),
        )
}

pub fn section(name: impl IntoReactive<SharedString>) -> Text {
    let header = label(name).font_weight(FontWeight::SEMIBOLD).px_1();
    match density() {
        Density::Touch => header.pt_3(),
        Density::Desktop => header.pt_5().pb_2(),
    }
}

pub fn group() -> View {
    view().flex_col().rounded(px(Theme::radius_group())).bg(Theme::panel_row).border_1().border_color(Theme::surface_edge)
}

pub fn surface(radius: f32) -> View {
    view().rounded(px(radius)).surface()
}

pub fn card() -> View {
    surface(Theme::radius_panel()).flex_col()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_controls_reach_the_minimum_target_and_desktop_keeps_the_macos_sizes() {
        assert_eq!(density(), Density::Touch);
        assert_eq!((metrics().target, metrics().pill_height, metrics().switch_row), (44., 44., 44.));
        set_density(Density::Desktop);
        assert_eq!((metrics().target, metrics().pill_height, metrics().segment_height, metrics().text), (28., 26., 22., TEXT));
        set_density(Density::Touch);
    }
}
