use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use browser_core::toast::Tone;
use nsh_base::{ButtonElement, Input, button, input};
use nsh_core::element::IntoProp;
use nsh_core::prelude::*;

use crate::icons;
use crate::motion::{POP, transition_ms};
use crate::theme::{Paint, RADIUS_GROUP, SYSTEM_FONT, Secure, TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

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
    segment_radius: RADIUS_GROUP - SEGMENT_INSET,
    switch_row: 44.,
    track: (51., 31.),
    thumb: 27.,
    icon: 20.,
};

/// The group's padding around its segments and the gap between them, so their corners stay concentric.
const SEGMENT_INSET: f32 = 4.;

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

pub fn target_size() -> f32 {
    metrics().target
}

pub fn icon_size() -> f32 {
    metrics().icon
}

pub fn label(content: impl IntoReactive<SharedString>) -> Text {
    text(content).text_size(px(metrics().text)).text_color(Theme::text)
}

pub fn pressable<M>(name: impl IntoProp<Option<SharedString>, M>, press: impl Fn() + 'static) -> ButtonElement {
    tab_stop(button().accessibility_label(name).on_press(press).hover(|s| s.bg(Theme::hover())).active(|s| s.bg(Theme::pressed())))
}

/// A button reached with Tab: ringed while the keyboard focus shows on it, dimmed while disabled.
pub fn tab_stop(button: ButtonElement) -> ButtonElement {
    button.focus_visible(|s| s.ring(px(2.)).ring_color(Theme::accent_text())).disabled_style(|s| s.opacity(0.45))
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

fn pill_button(name: String, glyph: Option<&str>, press: impl Fn() + 'static, primary: bool) -> ButtonElement {
    let m = metrics();
    let button = if primary {
        tab_stop(button().accessibility_label(name.clone()).on_press(press).hover(|s| s.opacity(0.88)).active(|s| s.opacity(0.7)))
    } else {
        pressable(name.clone(), press)
    }
    .flex_row()
    .items_center()
    .justify_center()
    .gap_1p5()
    .border_1()
    .border_color(if primary { Theme::surface_edge } else { Theme::hairline })
    .bg(if primary { Theme::accent } else { Theme::control });
    let text = label(name);
    match density() {
        Density::Touch => button.min_h(px(m.pill_height)).px_4().rounded_full(),
        Density::Desktop => button.h(px(m.pill_height)).px_3().rounded(px(m.pill_height / 2.)),
    }
    .children(glyph.map(|g| view().text_color(Theme::muted).child(icons::icon(g, m.text))))
    .child(if primary { text.font_weight(FontWeight::MEDIUM).text_color(|| Theme::on_fill(Theme::accent())) } else { text })
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
    let button = tab_stop(button);
    let text = text(name).text_size(px(m.text)).font_family(SYSTEM_FONT).text_color(if primary { Theme::on_fill(Secure::accent()) } else { Secure::text() });
    match density() {
        Density::Touch => button.min_h(px(m.pill_height)).px_4().rounded_full(),
        Density::Desktop => button.h(px(m.pill_height)).px_3().rounded(px(m.pill_height / 2.)),
    }
    .child(if primary { text.font_weight(FontWeight::MEDIUM) } else { text })
}

pub fn secure_ghost(name: impl Into<String>, press: impl Fn() + 'static) -> ButtonElement {
    let name: String = name.into();
    let m = metrics();
    tab_stop(
        button()
            .accessibility_label(name.clone())
            .on_press(press)
            .items_center()
            .justify_center()
            .h(px(m.pill_height))
            .px_3()
            .rounded(px(m.pill_height / 2.))
            .hover(|s| s.bg(Secure::hover()))
            .active(|s| s.bg(Secure::pressed())),
    )
    .child(text(name).text_size(px(m.text)).font_family(SYSTEM_FONT).font_weight(FontWeight::MEDIUM).text_color(Secure::text()))
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
    pill_button(name.into(), None, press, false)
}

pub fn icon_pill(glyph: &str, name: impl Into<String>, press: impl Fn() + 'static) -> ButtonElement {
    pill_button(name.into(), Some(glyph), press, false)
}

pub fn accent_pill(name: impl Into<String>, press: impl Fn() + 'static) -> ButtonElement {
    pill_button(name.into(), None, press, true)
}

pub fn pill_grid(pills: Vec<ButtonElement>) -> View {
    let mut pills = pills.into_iter().map(|p| p.flex_1().min_w_0().into_element());
    let mut grid = view().flex_col().gap_2();
    while let Some(first) = pills.next() {
        let second = pills.next().unwrap_or_else(|| view().flex_1().into_element());
        grid = grid.child(view().flex_row().gap_2().child(first).child(second));
    }
    grid
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
                .flex_grow(1.)
                .min_h(px(m.segment_height))
                .px(px(if touch { 8. } else { 12. }))
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
    let inset = if touch { SEGMENT_INSET } else { 2. };
    let group = view().flex_row().flex_wrap().p(px(inset)).gap(px(inset)).children(segments);
    if touch { group.rounded(px(Theme::radius_group())).bg(Theme::group) } else { group.rounded_full().bg(Theme::control) }
}

pub fn switch_row(name: impl Into<String>, on: impl Fn() -> bool + Copy + 'static, toggle: impl Fn() + 'static) -> ButtonElement {
    switch_with(None, name.into(), on, toggle)
}

pub fn glyph_switch(glyph: &'static str, name: impl Into<String>, on: impl Fn() -> bool + Copy + 'static, toggle: impl Fn() + 'static) -> ButtonElement {
    switch_with(Some(glyph), name.into(), on, toggle)
}

fn switch_with(glyph: Option<&'static str>, name: String, on: impl Fn() -> bool + Copy + 'static, toggle: impl Fn() + 'static) -> ButtonElement {
    let m = metrics();
    let (width, height) = m.track;
    let spoken = name.clone();
    tab_stop(button())
        .flex_row()
        .items_center()
        .min_h(px(m.switch_row))
        .px_3()
        .accessibility_label(move || SharedString::from(format!("{spoken}: {}", if on() { "On" } else { "Off" })))
        .role(AccessibilityRole::Switch)
        .selected(on)
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .on_press(move || {
            crate::motion::animate_next(crate::motion::POP);
            toggle();
        })
        .gap(px(10.))
        .children(glyph.map(|g| view().text_color(Theme::muted).child(icons::icon(g, GLYPH))))
        .child(label(name).flex_1())
        .child(
            view()
                .w(px(width))
                .h(px(height))
                .p(px((height - m.thumb) / 2.))
                .rounded_full()
                .flex_row()
                .bg(move || match (on(), Theme::is_light()) {
                    (true, _) => Theme::accent(),
                    (false, true) => Theme::faint(),
                    (false, false) => Theme::control(),
                })
                .transition_colors()
                .duration(move || transition_ms(POP))
                .child(
                    view()
                        .w(px(m.thumb))
                        .h(px(m.thumb))
                        .rounded_full()
                        .bg(move || if on() { Theme::on_fill(Theme::accent()) } else { Theme::on_accent() })
                        .shadow(Theme::elevation(1., 2.))
                        .translate_x(move || px(if on() { width - height } else { 0. }))
                        .transition()
                        .duration(move || transition_ms(POP)),
                ),
        )
}

pub const GLYPH: f32 = 15.;
pub const CAPS: f32 = 10.5;

pub fn glyph_row(glyph: &'static str, name: impl Into<String>) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .px_3()
        .min_h(px(metrics().switch_row))
        .child(view().text_color(Theme::muted).child(icons::icon(glyph, GLYPH)))
        .child(label(name.into()).flex_1())
}

pub fn caps_label(name: &str) -> Text {
    text(name.to_uppercase()).text_size(px(CAPS)).font_weight(FontWeight::BOLD).text_color(Theme::faint).px_1()
}

pub fn keys_of(shortcut: &str) -> Vec<String> {
    let (body, plus) = match shortcut.strip_suffix("++") {
        Some(rest) => (rest, true),
        None => (shortcut, false),
    };
    body.split('+').filter(|k| !k.is_empty()).map(str::to_owned).chain(plus.then(|| "+".to_owned())).collect()
}

pub fn key_caps(shortcut: &str) -> View {
    let caps: Vec<AnyElement> = keys_of(shortcut)
        .into_iter()
        .map(|key| {
            view()
                .min_w(px(18.))
                .h(px(18.))
                .px(px(4.))
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(Theme::control)
                .border_1()
                .border_color(Theme::hairline)
                .child(text(key).text_size(px(CAPS)).font_weight(FontWeight::MEDIUM).text_color(Theme::muted))
                .into_element()
        })
        .collect();
    view().flex_row().items_center().gap(px(3.)).accessibility_elements_hidden(true).children(caps)
}

pub fn hint_label(key: &str, does: &str) -> String {
    format!("{}: {does}", keys_of(key).join(" "))
}

pub fn key_hint(key: &str, does: &str) -> View {
    view()
        .accessible(true)
        .label(hint_label(key, does))
        .flex_row()
        .items_center()
        .gap(px(5.))
        .child(key_caps(key))
        .child(text(does.to_owned()).text_size(px(TEXT_SMALL)).text_color(Theme::faint))
}

pub fn tip(name: &str, side: Placement) -> nsh_base::TooltipElement {
    let bubble = view().px_2().py_1().rounded(px(Theme::radius_small())).surface();
    nsh_base::tooltip().placement(side).child(bubble.child(text(name.to_owned()).text_size(px(TEXT_SMALL)).text_color(Theme::text)))
}

pub fn site_tile(icon: Option<String>, host: &str, size: f32) -> View {
    let tile = view().w(px(size)).h(px(size)).flex_none().rounded(px(size * 0.28)).overflow_hidden().items_center().justify_center();
    if let Some(icon) = icon {
        return tile.child(image(icon).w(px(size)).h(px(size)));
    }
    let letter = host.trim_start_matches("www.").chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    if letter.is_empty() {
        return tile.bg(Theme::control());
    }
    let fill = Theme::monogram(host);
    tile.bg(fill).child(text(letter).text_size(px((size * 0.62).round())).font_weight(FontWeight::BOLD).text_color(Theme::on_fill(fill)))
}

pub fn section(name: impl IntoReactive<SharedString>) -> Text {
    let header = label(name).font_weight(FontWeight::SEMIBOLD).px_1();
    match density() {
        Density::Touch => header.pt_3(),
        Density::Desktop => header.pt_5().pb_2(),
    }
}

/// A settings section's heading with one short line on what it holds.
pub fn section_with(name: impl Into<String>, description: impl Into<String>) -> View {
    let description: String = description.into();
    view().gap(px(2.)).child(section(name.into())).when(!description.is_empty(), |v| v.child(hint(description)))
}

/// `control` inside a group with its short explanation right under it.
pub fn described(control: impl IntoElement, note: impl IntoReactive<SharedString>) -> View {
    view().child(control).child(hint(note).px_3().pb_2())
}

/// A compact icon button for a row's action; `name` is what assistive technology and the
/// tooltip say.
pub fn row_action(glyph: &str, name: impl Into<String>, press: impl Fn() + 'static) -> ButtonElement {
    let name: String = name.into();
    let mut icon = view().size_full().items_center().justify_center().child(icons::icon(glyph, icon_size()));
    if density() == Density::Desktop {
        icon = icon.tooltip(tip(&name, Placement::Top));
    }
    icon_button(icon, name, press).flex_none()
}

/// The button that opens a list's editor for a new entry.
pub fn add_button(name: impl Into<String>, press: impl Fn() + 'static) -> ButtonElement {
    icon_pill(icons::PLUS, name, press)
}

/// The card an "Add …" or edit button reveals: its title, then the fields given to it.
pub fn editor(title: impl Into<String>) -> View {
    group().p_3().gap_2().child(label(title.into()).font_weight(FontWeight::SEMIBOLD))
}

/// Save and Cancel under an editor.
pub fn editor_actions(save: impl Fn() + 'static, cancel: impl Fn() + 'static) -> View {
    view().flex_row().justify_end().gap_2().child(pill("Cancel", cancel)).child(accent_pill("Save", save))
}

/// What a list shows while it has no rows: an icon, one line and the action that fills it.
pub fn empty(glyph: &str, line: impl IntoReactive<SharedString>, action: Option<ButtonElement>) -> View {
    view()
        .items_center()
        .gap_2()
        .px_4()
        .py_5()
        .child(view().text_color(Theme::faint).child(icons::icon(glyph, 22.)))
        .child(label(line).text_color(Theme::muted).text_center())
        .children(action)
}

/// Asks before a destructive action in the platform's alert; `then` runs only on `action`.
pub fn confirm(title: impl Into<SharedString>, description: impl Into<SharedString>, action: impl Into<SharedString>, then: impl FnOnce() + 'static) {
    let answer =
        nsh_base::alert_dialog().title(title).description(description).cancel("cancel", "Cancel").destructive("confirm", action).close_on_escape(true).show();
    Owner::detached().run(|| {
        drop(spawn(async move {
            if answer.await.is_ok_and(|a| a.action.as_deref() == Some("confirm")) {
                then();
            }
        }))
    });
}

pub fn empty_state(content: impl IntoReactive<SharedString>) -> Text {
    label(content).text_color(Theme::muted).text_center().px_4().py_6()
}

/// Small muted text under a control: what it does or why it cannot.
pub fn hint(content: impl IntoReactive<SharedString>) -> Text {
    text(content).text_size(px(TEXT_SMALL)).text_color(Theme::muted).px_1()
}

pub fn field(name: &'static str, value: impl Fn() -> String + 'static, set: impl Fn(&str) + 'static) -> Input {
    input()
        .value(value)
        .on_change(move |v: &str| set(v))
        .label(name)
        .placeholder(name)
        .w_full()
        .px(px(8.))
        .py(px(6.))
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .border_1()
        .border_color(Theme::hairline())
        .text_color(Theme::text())
}

pub fn signal_field(name: &'static str, value: Signal<String>) -> Input {
    field(name, move || value.get(), move |v| value.set(v.to_owned()))
}

/// A row of a pane's list: its title and a line of detail; the actions it is given wrap under
/// the title where the width is too narrow for both.
pub fn list_row(ix: usize, title: impl Into<String>, detail: impl Into<String>) -> View {
    row_frame(ix).child(row_text(title.into(), detail.into()).flex_1().min_w(px(120.)))
}

/// A `list_row` whose title and detail open it when pressed.
pub fn open_row(ix: usize, title: impl Into<String>, detail: impl Into<String>, name: impl Into<String>, open: impl Fn() + 'static) -> View {
    row_frame(ix).pl_1().child(
        pressable(name.into(), open).flex_1().min_w(px(120.)).px_2().py_1().rounded(px(Theme::radius_row())).child(row_text(title.into(), detail.into())),
    )
}

fn row_frame(ix: usize) -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_1()
        .px_3()
        .py_1()
        .min_h(px(metrics().switch_row))
        .when(ix > 0, |v| v.border_t_1().border_color(Theme::separator))
}

fn row_text(title: String, detail: String) -> View {
    view()
        .py_1()
        .child(label(title).font_weight(FontWeight::MEDIUM).number_of_lines(1_u32))
        .when(!detail.is_empty(), |v| v.child(hint(detail).px_0().number_of_lines(1_u32)))
}

pub fn group() -> View {
    view().flex_col().rounded(px(Theme::radius_group())).bg(Theme::panel_row).border_1().border_color(Theme::surface_edge)
}

pub fn glyph_tile(source: &str, size: f32, fill: impl Fn() -> Rgba + 'static, color: impl Fn() -> Rgba + 'static) -> View {
    view()
        .w(px(size))
        .h(px(size))
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px((size * 0.27).round()))
        .bg(fill)
        .text_color(color)
        .child(crate::icons::icon(source, (size * 0.55).round()))
}

pub fn surface(radius: f32) -> View {
    view().rounded(px(radius)).surface()
}

pub fn card() -> View {
    surface(Theme::radius_panel()).flex_col()
}

/// A toast's card: its tone's icon, ready for the message and any buttons.
pub fn toast(tone: Tone) -> View {
    let (glyph, color) = match tone {
        Tone::Info => (crate::icons::INFO, Theme::muted()),
        Tone::Success => (crate::icons::CIRCLE_CHECK, Theme::success()),
        Tone::Warning => (crate::icons::TRIANGLE_ALERT, Theme::warning()),
    };
    view()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .min_h(px(44.))
        .pl(px(16.))
        .pr(px(6.))
        .py_1()
        .rounded(px(Theme::radius_panel()))
        .surface()
        .child(view().flex_none().text_color(color).child(crate::icons::icon(glyph, 16.)))
}

/// A toast's action: quiet accent text, so the message stays the focus.
pub fn toast_action(name: impl Into<String>, press: impl Fn() + 'static) -> ButtonElement {
    let name: String = name.into();
    pressable(name.clone(), press)
        .flex_none()
        .items_center()
        .justify_center()
        .min_h(px(metrics().target))
        .px(px(10.))
        .rounded(px(Theme::radius_row()))
        .child(label(name).font_weight(FontWeight::MEDIUM).text_color(Theme::accent_text))
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

    #[test]
    fn shortcuts_split_into_key_caps() {
        assert_eq!(keys_of("Ctrl+Shift+Y"), ["Ctrl", "Shift", "Y"]);
        assert_eq!(keys_of("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(keys_of("F5"), ["F5"]);
        assert!(keys_of("").is_empty());
    }

    #[test]
    fn a_key_hint_speaks_the_key_and_what_it_does() {
        assert_eq!(hint_label("Enter", "Open"), "Enter: Open");
        assert_eq!(hint_label("Ctrl+Shift+Y", "Downloads"), "Ctrl Shift Y: Downloads");
    }
}
