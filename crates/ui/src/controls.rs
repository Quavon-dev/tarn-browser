use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyView, AnyWindowHandle, App, AppContext, Bounds, BoxShadow, Context, DisplayId, Div, ElementId, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Render, Rgba, SharedString, Stateful, StatefulInteractiveElement, Styled, Subscription, Svg, Transformation, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, div, percentage, point, prelude::FluentBuilder, px,
};

use crate::icon::{Icon, icon};
use crate::theme::{Elevation, ROW_HEIGHT, SYSTEM_FONT, Secure, TEXT, TEXT_SMALL, Theme};

pub fn button(id: impl Into<ElementId>, text: impl Into<SharedString>, primary: bool) -> Stateful<Div> {
    themed_button(id, None, text, primary, true)
}

/// A button that cannot be pressed: dimmed, with no hover or pressed state and no tab stop. The
/// caller adds no click handler.
pub fn disabled_button(id: impl Into<ElementId>, text: impl Into<SharedString>, primary: bool) -> Stateful<Div> {
    themed_button(id, None, text, primary, false)
}

pub fn icon_text_button(id: impl Into<ElementId>, glyph: Icon, text: impl Into<SharedString>) -> Stateful<Div> {
    themed_button(id, Some(glyph), text, false, true)
}

fn themed_button(id: impl Into<ElementId>, glyph: Option<Icon>, text: impl Into<SharedString>, primary: bool, enabled: bool) -> Stateful<Div> {
    let colors = if primary { (Theme::accent(), Theme::surface_edge()) } else { (Theme::control(), Theme::hairline()) };
    let label = if primary { Theme::on_fill(Theme::accent()) } else { Theme::text() };
    pill(id, glyph, text, primary, enabled, colors, label).when(enabled && !primary, |d| d.hover(|s| s.bg(Theme::hover())).active(|s| s.bg(Theme::pressed())))
}

/// A button on a security surface (prompts, banners, blocking pages), in colours no theme sets.
pub fn secure_button(id: impl Into<ElementId>, text: impl Into<SharedString>, primary: bool) -> Stateful<Div> {
    let colors = if primary { (Secure::accent(), Secure::surface_edge()) } else { (Secure::control(), Secure::hairline()) };
    pill(id, None, text, primary, true, colors, if primary { Theme::on_accent() } else { Secure::text() })
        .font_family(SYSTEM_FONT)
        .when(!primary, |d| d.hover(|s| s.bg(Secure::hover())).active(|s| s.bg(Secure::pressed())))
}

pub fn secure_ghost_button(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
    pill(id, None, text, false, true, (Secure::surface(), Secure::surface()), Secure::text())
        .font_family(SYSTEM_FONT)
        .hover(|s| s.bg(Secure::hover()))
        .active(|s| s.bg(Secure::pressed()))
}

fn pill(
    id: impl Into<ElementId>,
    glyph: Option<Icon>,
    text: impl Into<SharedString>,
    primary: bool,
    enabled: bool,
    (fill, edge): (Rgba, Rgba),
    color: Rgba,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(26.))
        .px_3()
        .flex()
        .items_center()
        .gap_1p5()
        .rounded_full()
        .text_size(px(TEXT))
        .border_1()
        .border_color(edge)
        .bg(fill)
        .when(primary, |d| d.font_weight(FontWeight::MEDIUM))
        .text_color(color)
        .map(|d| if enabled { tab_stop(d.cursor_pointer()) } else { d.opacity(0.4) })
        .when(enabled && primary, |d| d.hover(|s| s.opacity(0.88)).active(|s| s.opacity(0.7)))
        .children(glyph.map(|g| icon(g, 13., Theme::muted())))
        .child(text.into())
}

/// A key hint inside a button, such as "ESC" or "↩".
pub fn key_hint(text: impl Into<SharedString>) -> Div {
    div().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child(keys(&text.into()))
}

pub fn key_caps(glyphs: &str) -> Vec<String> {
    keys(glyphs)
        .split(' ')
        .flat_map(|chord| {
            if cfg!(windows) {
                return windows_caps(chord);
            }
            let mods = chord.chars().take_while(|c| "⌃⌥⇧⌘".contains(*c));
            let rest: String = chord.chars().skip_while(|c| "⌃⌥⇧⌘".contains(*c)).collect();
            mods.map(String::from).chain((!rest.is_empty()).then_some(rest)).collect()
        })
        .collect()
}

fn windows_caps(chord: &str) -> Vec<String> {
    let mut caps: Vec<String> = chord.split('+').filter(|k| !k.is_empty()).map(str::to_owned).collect();
    if chord.ends_with('+') {
        caps.push("+".to_owned());
    }
    caps
}

pub fn key_cap(key: impl Into<SharedString>) -> Div {
    div()
        .h(px(18.))
        .min_w(px(18.))
        .px_1()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(Theme::radius_small()))
        .border_1()
        .border_color(Theme::hairline())
        .bg(Theme::control())
        .text_size(px(TEXT_SMALL - 1.))
        .text_color(Theme::muted())
        .child(key.into())
}

/// A shortcut written with macOS glyphs (`⇧⌘J`) as the platform shows it. Windows writes
/// `Ctrl+Shift+J`; Ctrl with Cmd and Ctrl with a digit use Alt there (`app::bind_keys`).
pub fn keys(glyphs: &str) -> String {
    if !cfg!(windows) {
        return glyphs.to_owned();
    }
    glyphs.split(' ').map(|chord| if chord.is_empty() { String::new() } else { windows_chord(chord) }).collect::<Vec<_>>().join(" ")
}

fn windows_chord(glyphs: &str) -> String {
    let has = |c| glyphs.contains(c);
    let key: String = glyphs.chars().filter(|c| !"⌃⌥⇧⌘".contains(*c)).collect();
    let key = if key.len() > 1 { key.trim_start_matches('-').to_owned() } else { key };
    let key = match key.as_str() {
        "⇥" => "Tab".to_owned(),
        "F" if has('⌘') && has('⌃') => return "F11".to_owned(),
        _ => key,
    };
    let (ctrl, alt, shift) = match (has('⌘'), has('⌃')) {
        (true, true) => (false, true, true),
        (true, false) => (true, has('⌥'), has('⇧')),
        (false, true) if !key.is_empty() && key.chars().all(|c| c.is_ascii_digit()) => (false, true, has('⇧')),
        (false, true) => (true, has('⌥'), has('⇧')),
        (false, false) => (false, has('⌥'), has('⇧')),
    };
    let mods = [(ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift")];
    mods.iter().filter(|(on, _)| *on).map(|(_, m)| *m).chain((!key.is_empty()).then_some(key.as_str())).collect::<Vec<_>>().join("+")
}

fn ring(color: Rgba, spread: f32, blur: f32) -> BoxShadow {
    BoxShadow { color: color.into(), offset: point(px(0.), px(0.)), blur_radius: px(blur), spread_radius: px(spread) }
}

/// The flat opaque card behind every popover, banner, toast and centred card.
pub fn surface(radius: f32) -> Div {
    div().rounded(px(radius)).bg(Theme::surface()).border_1().border_color(Theme::surface_edge()).font_family(Theme::font()).overflow_hidden()
}

/// The transparent margin a pop-up window keeps around its card, room for the card's shadow.
pub const POPOVER_INSET: f32 = 16.;

/// A pop-up window's card: `surface` with the pop-up elevation, inset `top` from the window's
/// top edge and `POPOVER_INSET` from the others.
pub fn popover(radius: f32, top: f32) -> Div {
    surface(radius).absolute().top(px(top)).left(px(POPOVER_INSET)).right(px(POPOVER_INSET)).bottom(px(POPOVER_INSET)).shadow(Elevation::Popover.shadow())
}

/// Opens a pop-up window at `bounds` without the system frame and shadow, clipped to its card
/// `inset` (top, left, right, bottom) from its edges with `radius` corners. The new window renders
/// at once, so this runs outside an update, as from `cx.defer`.
pub fn open_popup<V: Render + 'static>(
    cx: &mut App,
    bounds: Bounds<Pixels>,
    display: Option<DisplayId>,
    focus: bool,
    inset: [f32; 4],
    radius: f32,
    build: impl FnOnce(&mut Window, &mut Context<V>) -> V,
) -> (Option<WindowHandle<V>>, Option<engine::Host>) {
    let mut popup = None;
    let handle = cx
        .open_window(popup_options(bounds, display, focus), |window, cx| {
            popup = engine::Host::from_window(window);
            if let Some(host) = &popup {
                host.strip_frame();
                let [top, left, right, bottom] = inset.map(f64::from);
                host.round_corners(top, left, right, bottom, f64::from(radius));
            }
            cx.new(|cx| build(window, cx))
        })
        .ok();
    (handle, popup)
}

/// `surface` in the fixed security palette and the system font, for prompts, banners and
/// blocking pages (threat model S29).
pub fn secure_surface(radius: f32) -> Div {
    div()
        .rounded(px(radius))
        .bg(Secure::surface())
        .border_1()
        .border_color(Secure::surface_edge())
        .text_color(Secure::text())
        .font_family(SYSTEM_FONT)
        .overflow_hidden()
}

/// A text field frame with a macOS focus ring.
pub fn field(focused: bool) -> Div {
    // A shadow would tint the translucent fill, so the ring is a border just outside the field.
    let ring = div()
        .absolute()
        .top(px(-3.))
        .left(px(-3.))
        .right(px(-3.))
        .bottom(px(-3.))
        .rounded(px(Theme::radius_row() + 3.))
        .border_3()
        .border_color(Theme::accent_soft());
    div()
        .relative()
        .px_2p5()
        .py_1()
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .border_1()
        .border_color(if focused { Theme::accent_text() } else { Theme::hairline() })
        .when(focused, |d| d.child(ring))
}

pub fn switch(on: bool) -> Div {
    div()
        .w(px(34.))
        .h(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .p(px(2.))
        .rounded_full()
        .bg(if on { Theme::accent() } else { Theme::faint() })
        .when(on, |d| d.justify_end())
        .child(knob())
}

thread_local! {
    static SWITCHES: RefCell<std::collections::HashMap<ElementId, (bool, u64)>> = RefCell::default();
}

const MAX_SWITCHES: usize = 256;

pub fn sliding_switch(id: impl Into<ElementId>, on: bool) -> gpui::AnyElement {
    let id = id.into();
    let flips = SWITCHES.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() >= MAX_SWITCHES && !s.contains_key(&id) {
            s.clear();
        }
        let entry = s.entry(id.clone()).or_insert((on, 0));
        if entry.0 != on {
            *entry = (on, entry.1 + 1);
        }
        entry.1
    });
    if flips == 0 || crate::motion::reduced() {
        return switch(on).into_any_element();
    }
    let track = div().relative().w(px(34.)).h(px(20.)).flex_none().rounded_full().bg(if on { Theme::accent() } else { Theme::faint() });
    let (from, to) = if on { (2., 16.) } else { (16., 2.) };
    let key = ElementId::NamedChild(Box::new(id), SharedString::from(format!("flip-{flips}")));
    crate::motion::animate(track, key, crate::motion::POP, false, move |track, t| track.child(knob().absolute().top(px(2.)).left(px(from + (to - from) * t))))
}

fn knob() -> Div {
    div().w(px(16.)).h(px(16.)).rounded_full().bg(Theme::on_accent()).shadow(vec![ring(rgba_black(), 0., 2.)])
}

fn rgba_black() -> Rgba {
    gpui::rgba(0x0000004d)
}

pub fn segmented() -> Div {
    div().flex().flex_none().p(px(2.)).gap(px(2.)).rounded_full().bg(Theme::control())
}

pub fn segment(id: impl Into<ElementId>, text: impl Into<SharedString>, selected: bool) -> Stateful<Div> {
    tab_stop(div().id(id))
        .h(px(22.))
        .px_2p5()
        .flex()
        .items_center()
        .rounded_full()
        .cursor_pointer()
        .text_size(px(TEXT_SMALL))
        .when(selected, |d| d.bg(Theme::selected()).text_color(Theme::text()).font_weight(FontWeight::MEDIUM))
        .when(!selected, |d| d.text_color(Theme::muted()).hover(|s| s.bg(Theme::hover()).text_color(Theme::text())))
        .active(|s| s.bg(Theme::pressed()))
        .child(text.into())
}

/// A segment that cannot be picked: dimmed, with no hover or pressed state.
pub fn disabled_segment(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
    div().id(id).h(px(22.)).px_2p5().flex().items_center().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).opacity(0.4).child(text.into())
}

/// A colour circle; the selected one gets a ring like the macOS accent picker.
pub fn swatch(id: impl Into<ElementId>, color: Rgba, selected: bool, size: f32) -> Stateful<Div> {
    tab_stop(div().id(id))
        .w(px(size))
        .h(px(size))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .bg(color)
        .border_1()
        .border_color(Theme::surface_edge())
        .when(selected, |d| d.shadow(vec![ring(Theme::text(), 2., 0.)]))
        .hover(|s| s.opacity(0.85))
        .active(|s| s.opacity(0.7))
}

/// What the platform calls showing a file in its folder.
pub const REVEAL: &str = if cfg!(windows) { "Show in File Explorer" } else { "Show in Finder" };
/// The user's computer, as the platform names it.
pub const THIS_DEVICE: &str = if cfg!(windows) { "this PC" } else { "this Mac" };

/// Minimize, maximize and close for a window without a system title bar; macOS draws its own.
pub fn window_controls() -> Option<Div> {
    use gpui::WindowControlArea;
    let control = |id: &'static str, area: WindowControlArea, glyph: gpui::AnyElement, close: bool| {
        div()
            .id(id)
            .w(px(40.))
            .h(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(Theme::radius_small()))
            .text_color(Theme::muted())
            .hover(move |s| if close { s.bg(gpui::rgb(0xc42b1c)).text_color(gpui::white()) } else { s.bg(Theme::hover()) })
            .occlude()
            .window_control_area(area)
            .child(glyph)
    };
    let line = || div().w(px(10.)).h(px(1.)).bg(Theme::muted());
    let square = || div().w(px(10.)).h(px(10.)).border_1().border_color(Theme::muted());
    cfg!(windows).then(|| {
        div()
            .flex()
            .flex_none()
            .items_center()
            .child(control("window-minimize", WindowControlArea::Min, line().into_any_element(), false))
            .child(control("window-maximize", WindowControlArea::Max, square().into_any_element(), false))
            .child(control("window-close", WindowControlArea::Close, icon(Icon::Close, 14., Theme::muted()).into_any_element(), true))
    })
}

pub fn icon_button(id: impl Into<ElementId>, glyph: Icon, enabled: bool) -> Stateful<Div> {
    icon_frame(id, enabled).child(icon(glyph, 16., if enabled { Theme::muted() } else { Theme::faint() }))
}

/// The least time between two spinner frames: just under 1/30 s, so that 60, 120 and 144 Hz
/// displays all draw the spinner about 30 times a second, every few display frames. Each step
/// redraws the whole window, so the display's full rate would cost several times the CPU.
pub const SPIN_STEP: Duration = Duration::from_millis(30);

const TURN: Duration = Duration::from_millis(900);

static SPIN_EPOCH: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);

/// How far through its turn the spinner is now. Not rounded to whole frames: a redraw that comes
/// late would then skip a step.
fn spin_turn(elapsed: Duration) -> f32 {
    (elapsed.as_secs_f32() / TURN.as_secs_f32()).fract()
}

/// The reload arrow where its turn is now, for a page that is loading; the caller redraws it
/// every `SPIN_STEP`.
pub fn spinner(size: f32, color: Rgba) -> Svg {
    icon(Icon::Reload, size, color).with_transformation(Transformation::rotate(percentage(spin_turn(SPIN_EPOCH.elapsed()))))
}

/// An icon button's frame, for a glyph the caller draws itself, such as a spinning one.
pub fn icon_frame(id: impl Into<ElementId>, enabled: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(28.))
        .h(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(Theme::radius_row()))
        .when(enabled, |d| tab_stop(d.hover(|s| s.bg(Theme::hover())).active(|s| s.bg(Theme::pressed())).cursor_pointer()))
}

#[derive(Clone)]
pub struct Tooltip {
    text: SharedString,
    keys: Option<SharedString>,
}

impl Tooltip {
    fn card(&self) -> Div {
        div()
            .px_2()
            .py(px(3.))
            .flex()
            .gap_1p5()
            .rounded(px(Theme::radius_small()))
            .bg(Theme::surface())
            .border_1()
            .border_color(Theme::surface_edge())
            .text_size(px(TEXT_SMALL))
            .text_color(Theme::text())
            .child(self.text.clone())
            .children(self.keys.clone().map(|keys| div().text_color(Theme::muted()).child(keys)))
    }
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().relative().child(self.card().absolute().top(px(POPOVER_INSET)).left(px(POPOVER_INSET)).shadow(Elevation::Popover.shadow()))
    }
}

/// Where a tooltip's pop-up window is in its life.
enum TooltipPopup {
    Closed,
    Opening,
    Open(AnyWindowHandle),
    /// The tooltip was hidden; a pop-up still opening closes as soon as it is open.
    Gone,
}

/// What GPUI shows as a tooltip: the card, laid out but not painted where GPUI places it, and a
/// pop-up window over that spot that paints it. GPUI draws inside its window, under the native
/// web view, so a card drawn there is cut off where it meets the page.
struct TooltipAnchor {
    tip: Tooltip,
    popup: Rc<RefCell<TooltipPopup>>,
    _release: Subscription,
}

impl TooltipAnchor {
    fn new(tip: Tooltip, cx: &mut Context<Self>) -> Self {
        let popup = Rc::new(RefCell::new(TooltipPopup::Closed));
        let release = cx.on_release({
            let popup = popup.clone();
            move |_, cx| {
                if let TooltipPopup::Open(handle) = popup.replace(TooltipPopup::Gone) {
                    cx.defer(move |cx| drop(handle.update(cx, |_, window, _| window.remove_window())));
                }
            }
        });
        TooltipAnchor { tip, popup, _release: release }
    }
}

impl Render for TooltipAnchor {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (tip, popup) = (self.tip.clone(), self.popup.clone());
        div().invisible().child(self.tip.card()).on_children_prepainted(move |cards, window, cx| {
            let Some(card) = cards.first().filter(|_| matches!(*popup.borrow(), TooltipPopup::Closed)) else { return };
            *popup.borrow_mut() = TooltipPopup::Opening;
            let margin = px(POPOVER_INSET);
            let bounds = Bounds::new(window.bounds().origin + card.origin - point(margin, margin), card.size + gpui::size(margin * 2., margin * 2.));
            let (display, shell) = (window.display(cx).map(|d| d.id()), engine::Host::from_window(window));
            let (tip, popup) = (tip.clone(), popup.clone());
            cx.defer(move |cx| {
                let (handle, host) = open_popup(cx, bounds, display, false, [POPOVER_INSET; 4], Theme::radius_small(), |_, _| tip);
                if let Some(host) = &host {
                    host.ignore_mouse();
                    if let Some(shell) = &shell {
                        shell.follow(host);
                    }
                }
                let Some(handle) = handle else { return };
                let mut state = popup.borrow_mut();
                if matches!(*state, TooltipPopup::Opening) {
                    *state = TooltipPopup::Open(handle.into());
                } else {
                    drop(handle.update(cx, |_, window, _| window.remove_window()));
                }
            });
        })
    }
}

/// A tooltip for `.tooltip(...)`: a label and, optionally, its shortcut in muted text.
pub fn tooltip(text: impl Into<SharedString>, keys: Option<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let tip = Tooltip { text: text.into(), keys: keys.filter(|k| !k.is_empty()).map(|k| SharedString::from(self::keys(&k))) };
    move |_, cx| cx.new(|cx| TooltipAnchor::new(tip.clone(), cx)).into()
}

/// A borderless pop-up window at `bounds` in screen points, for what must show over the page.
pub fn popup_options(bounds: Bounds<Pixels>, display_id: Option<DisplayId>, focus: bool) -> WindowOptions {
    #[cfg(windows)]
    engine::chromium::open_next_window_inactive(!focus);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    }
}

pub fn radio(on: bool) -> Div {
    div()
        .w(px(16.))
        .h(px(16.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(if on { Theme::accent() } else { Theme::faint() })
        .when(on, |d| {
            d.bg(Theme::accent()).shadow(vec![ring(Theme::accent_soft(), 2., 0.)]).child(div().w(px(6.)).h(px(6.)).rounded_full().bg(Theme::on_accent()))
        })
}

/// A site's initial on its stable tint, standing in for a favicon.
pub fn favicon(host: &str, size: f32) -> Div {
    let letter = host.trim_start_matches("www.").chars().next().map(|c| c.to_uppercase().to_string());
    let base = div().w(px(size)).h(px(size)).flex().flex_none().items_center().justify_center().rounded(px(size * 0.28));
    match letter {
        Some(letter) => base
            .bg(Theme::monogram(host))
            .text_size(px(size * 0.62))
            .font_weight(FontWeight::BOLD)
            .text_color(Theme::on_fill(Theme::monogram(host)))
            .child(letter),
        None => base.bg(Theme::control()),
    }
}

pub fn group() -> Div {
    div().flex().flex_col().rounded(px(Theme::radius_group())).bg(Theme::panel_row()).border_1().border_color(Theme::surface_edge()).overflow_hidden()
}

pub fn row(first: bool) -> Div {
    div()
        .relative()
        .min_h(px(40.))
        .px_3()
        .py_1p5()
        .flex()
        .items_center()
        .gap_3()
        .text_size(px(TEXT))
        .when(!first, |d| d.child(div().absolute().top_0().left(px(12.)).right_0().h(px(1.)).bg(Theme::separator())))
}

/// A row that toggles its switch: the whole row is the target, with hover and pressed fills, and
/// a tab stop. `body` is the row's text.
pub fn switch_row(id: impl Into<ElementId>, first: bool, body: impl IntoElement, on: bool) -> Stateful<Div> {
    let id = id.into();
    let knob = sliding_switch(ElementId::NamedChild(Box::new(id.clone()), "switch".into()), on);
    tab_stop(row(first).id(id).cursor_pointer().hover(|s| s.bg(Theme::hover())).active(|s| s.bg(Theme::pressed())).child(body).child(knob))
}

/// An interactive list row: one line high (`ROW_HEIGHT`), highlighted while `selected`,
/// otherwise filled on hover and press.
pub fn list_row(id: impl Into<ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(ROW_HEIGHT))
        .flex_none()
        .px_2p5()
        .flex()
        .items_center()
        .gap_2p5()
        .rounded(px(Theme::radius_row()))
        .cursor_pointer()
        .map(|d| if selected { d.bg(Theme::selected()) } else { d.hover(|s| s.bg(Theme::hover())).active(|s| s.bg(Theme::pressed())) })
}

/// The text column of a row: shrinks and wraps beside the row's control. `flex_1`'s relative
/// zero basis lets a block parent lay the text out at its unwrapped width, which GPUI then paints
/// over the control.
fn text_column() -> Div {
    div().flex_1().flex_basis(px(0.)).min_w_0()
}

/// A title over a muted detail; the detail wraps.
pub fn text_block(title: impl Into<SharedString>, detail: impl Into<SharedString>) -> Div {
    text_column().flex().flex_col().gap_0p5().child(title.into()).child(div().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child(detail.into()))
}

/// `text_block` kept to one line each, cut with an ellipsis, for list rows.
pub fn text_lines(title: impl Into<SharedString>, detail: impl Into<SharedString>) -> Div {
    let line = || div().overflow_hidden().whitespace_nowrap().text_ellipsis();
    text_column().flex().flex_col().child(line().child(title.into())).child(line().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child(detail.into()))
}

/// The heading of a group of rows in a pop-up or list, smaller and quieter than `section`.
pub fn caption(title: impl Into<SharedString>) -> Div {
    div().text_size(px(TEXT_SMALL)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::muted()).child(title.into())
}

/// What a list shows when it has nothing to list.
pub fn empty(text: impl Into<SharedString>) -> Div {
    div().h(px(ROW_HEIGHT + 10.)).flex_none().px_3().flex().items_center().justify_center().text_color(Theme::muted()).child(text.into())
}

/// Makes `control` reachable with Tab and rings it while it has the keyboard; Enter and Space
/// click it. A mouse press takes the focus off the field or control that had it without giving
/// it to `control`, so the ring shows only for the keyboard. Buttons, segments and swatches are
/// tab stops already; windows that move the focus on Tab are pop-ups and Settings.
pub fn tab_stop(control: Stateful<Div>) -> Stateful<Div> {
    // A click moves no ring onto the control and clears one a Tab left elsewhere.
    control.tab_index(0).focus(|s| s.shadow(focus_ring())).on_mouse_down(MouseButton::Left, |_, window, _| {
        window.blur();
        window.prevent_default();
    })
}

/// The keyboard focus ring: 2 points of the readable accent outside the control, at least 3:1 on
/// the theme's backgrounds.
pub fn focus_ring() -> Vec<BoxShadow> {
    vec![ring(Theme::accent_text(), 2., 0.)]
}

pub fn section(title: impl Into<SharedString>) -> Div {
    div().pt_5().pb_2().px_1().text_size(px(TEXT)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).child(title.into())
}

pub fn note(text: impl Into<SharedString>) -> Div {
    div().pt_1p5().px_1().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child(text.into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn windows_chords_keep_a_plus_key() {
        assert_eq!(super::windows_caps("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(super::windows_caps("Ctrl+Shift+S"), ["Ctrl", "Shift", "S"]);
        assert_eq!(super::windows_caps("+"), ["+"]);
    }

    #[test]
    fn shortcuts_split_into_one_cap_per_key() {
        if cfg!(windows) {
            assert_eq!(super::key_caps("⇧⌘S"), ["Ctrl", "Shift", "S"]);
        } else {
            assert_eq!(super::key_caps("⇧⌘S"), ["⇧", "⌘", "S"]);
            assert_eq!(super::key_caps("⌘F5"), ["⌘", "F5"]);
        }
        assert!(super::key_caps("").is_empty());
    }

    #[test]
    fn the_spinner_turns_evenly_with_time() {
        use std::time::Duration;
        assert_eq!(super::spin_turn(Duration::ZERO), 0.);
        let turn = |ms| super::spin_turn(Duration::from_millis(ms));
        assert!((turn(225) - 0.25).abs() < 1e-4 && (turn(450) - 0.5).abs() < 1e-4, "a late redraw lands where the turn is, not a step behind");
        assert!(turn(900) < 1e-4, "a full turn starts over");
    }

    #[test]
    fn shortcuts_read_as_the_platform_writes_them() {
        let cases = [
            ("⇧⌘J", "Ctrl+Shift+J"),
            ("⌘,", "Ctrl+,"),
            ("⌥⌘G", "Ctrl+Alt+G"),
            ("⌃1", "Alt+1"),
            ("⌃⌘N", "Alt+Shift+N"),
            ("⌃⌘F", "F11"),
            ("⌃⇥", "Ctrl+Tab"),
            ("Esc", "Esc"),
            ("⌘[  ⌘]", "Ctrl+[  Ctrl+]"),
            ("⇧-click", "Shift+click"),
            ("⌘-", "Ctrl+-"),
            ("Ctrl+T", "Ctrl+T"),
        ];
        for (glyphs, windows) in cases {
            assert_eq!(super::keys(glyphs), if cfg!(windows) { windows } else { glyphs });
        }
    }
}
