use browser_core::shortcuts;
use nsh_base::button;
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use tarn_nsh_shared::controls::{group, icon_button, label, pressable, section};
use tarn_nsh_shared::icons as glyphs;
use tarn_nsh_shared::panes::{self, GROUPS, Pane};

use super::super::{Screen, Ui, panels, sheets};
use crate::theme::{TEXT, TEXT_SMALL, TEXT_TITLE, Theme};

const WIDTH: f32 = 1000.;
const HEIGHT: f32 = 700.;
const NAV_WIDTH: f32 = 196.;
const HEADER_HEIGHT: f32 = 52.;
const CONTENT_WIDTH: f32 = 760.;
const WINDOW_SHARE: f32 = 0.94;
const PANE_INSET: f32 = 64.;
const NAV_ROW: f32 = 30.;
const NAV_GLYPH: f32 = 15.;
const CAPTION: f32 = 10.;
const LIGHT: f32 = 12.;
const LIGHT_INSET: f32 = 18.;
const LIGHT_TOP: f32 = 10.;
const LIGHT_GAP: f32 = 8.;
const CLOSE_RED: u32 = 0xff5f_57ff;
const CLOSE_EDGE: u32 = 0xe044_3eff;
const CLOSE_MARK: u32 = 0x4d00_00ff;
const DIM: u32 = 0x0000_0059;
const EXTENSIONS: &str = "Extensions";

pub(super) fn owns(screen: Screen) -> bool {
    matches!(screen, Screen::Settings | Screen::Extensions | Screen::Licences | Screen::Licence(_))
}

fn private(ui: Ui) -> bool {
    ui.read(|s| s.model.private())
}

fn nav_row(name: &'static str, glyph: &'static str, selected: impl Fn() -> bool + Copy + 'static, press: impl Fn() + 'static) -> impl IntoElement {
    let color = move || if selected() { Theme::text() } else { Theme::muted() };
    pressable(name, press)
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(NAV_ROW))
        .px_2()
        .rounded(px(Theme::radius_small()))
        .bg(move || if selected() { Theme::selected() } else { Theme::clear() })
        .selected(selected)
        .text_color(color)
        .child(glyphs::icon(glyph, NAV_GLYPH))
        .child(text(name).text_size(px(TEXT)).text_color(color).number_of_lines(1))
}

fn nav(ui: Ui, pane: Signal<Pane>) -> impl IntoElement {
    let shown = move |p: Pane| move || ui.screen.get() != Screen::Extensions && pane.get() == p;
    let groups = GROUPS.iter().enumerate().map(move |(ix, (group, members))| {
        view()
            .gap(px(2.))
            .child(
                text(group.to_uppercase())
                    .px_2()
                    .pt(px(if ix == 0 { 4. } else { 12. }))
                    .pb_1()
                    .text_size(px(CAPTION))
                    .font_weight(FontWeight::BOLD)
                    .text_color(Theme::faint),
            )
            .children(
                members
                    .iter()
                    .filter(move |p| {
                        !matches!(p, Pane::Tabs | Pane::Developer)
                            && (**p != Pane::Ai || tarn_nsh_shared::assistant::offered(super::super::assistant::Host(ui)))
                    })
                    .map(move |p| {
                        let p = *p;
                        nav_row(panes::name(p), panes::icon(p), shown(p), move || {
                            pane.set(p);
                            if ui.screen.get_untracked() != Screen::Settings {
                                ui.show(Screen::Settings);
                            }
                        })
                        .into_element()
                    }),
            )
            .child(when(
                move || last_group(ix) && !private(ui),
                move || nav_row(EXTENSIONS, glyphs::SPARKLES, move || ui.screen.get() == Screen::Extensions, move || ui.show(Screen::Extensions)),
            ))
    });
    view()
        .w(px(NAV_WIDTH))
        .flex_none()
        .h_full()
        .pt(px(HEADER_HEIGHT))
        .px_2()
        .bg(Theme::panel_nav)
        .border_r_1()
        .border_color(Theme::separator)
        .child(scroll().flex_1().child(view().pb_4().children(groups.collect::<Vec<_>>())))
}

fn last_group(ix: usize) -> bool {
    ix + 1 == GROUPS.len()
}

fn traffic_lights(ui: Ui) -> impl IntoElement {
    let hovered = signal(false);
    let off = move || view().w(px(LIGHT)).h(px(LIGHT)).rounded_full().bg(Theme::hairline);
    view()
        .absolute()
        .top(px(LIGHT_TOP))
        .left(px(LIGHT_INSET))
        .flex_row()
        .items_center()
        .gap(px(LIGHT_GAP))
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .child(
            button()
                .accessibility_label("Close Settings")
                .on_press(move || ui.show(Screen::Browse))
                .w(px(LIGHT))
                .h(px(LIGHT))
                .rounded_full()
                .items_center()
                .justify_center()
                .bg(rgba(CLOSE_RED))
                .border_1()
                .border_color(rgba(CLOSE_EDGE))
                .text_color(rgba(CLOSE_MARK))
                .child(when(move || hovered.get(), || glyphs::icon(glyphs::CLOSE, LIGHT - 4.))),
        )
        .child(off())
        .child(off())
}

fn title(ui: Ui, pane: Signal<Pane>) -> String {
    match ui.screen.get() {
        Screen::Extensions => EXTENSIONS.to_owned(),
        screen => sheets::heading_for(screen).map_or_else(|| panes::name(pane.get()).to_owned(), str::to_owned),
    }
}

fn kept_chip(ui: Ui) -> impl IntoElement {
    view()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded_full()
        .border_1()
        .border_color(Theme::hairline)
        .text_color(Theme::muted)
        .child(glyphs::icon(glyphs::LOCK, 11.))
        .child(text(move || if private(ui) { "Kept in memory only" } else { "Counted on this device only" }).text_size(px(TEXT_SMALL)).text_color(Theme::muted))
}

pub(super) fn up(screen: Screen) -> Option<Screen> {
    match screen {
        Screen::Licences => Some(Screen::Settings),
        Screen::Licence(_) => Some(Screen::Licences),
        _ => None,
    }
}

fn header(ui: Ui, pane: Signal<Pane>) -> impl IntoElement {
    let up = move || up(ui.screen.get());
    view()
        .flex_row()
        .flex_none()
        .items_center()
        .gap_2()
        .h(px(HEADER_HEIGHT))
        .px_8()
        .border_b_1()
        .border_color(Theme::separator)
        .child(dynamic(move || {
            up().map(|to| {
                icon_button(glyphs::icon(glyphs::CHEVRON_LEFT, NAV_GLYPH), "Back", move || {
                    ui.show(to);
                })
                .ml(px(-8.))
            })
        }))
        .child(text(move || title(ui, pane)).flex_1().text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text).number_of_lines(1))
        .child(when(move || ui.screen.get() == Screen::Settings && pane.get() == Pane::Profile, move || kept_chip(ui)))
}

fn shortcuts() -> View {
    let note =
        if cfg!(target_os = "android") { "Shortcuts work with a hardware keyboard; Ctrl stands for ⌘." } else { "Shortcuts work with a hardware keyboard." };
    view().gap_2().child(text(note).pt_3().px_1().text_size(px(TEXT_SMALL)).text_color(Theme::muted)).children(
        shortcuts::table(super::input::keyboard(), super::input::offered).into_iter().map(|(name, rows)| {
            view().child(section(name)).child(group().children(rows.into_iter().enumerate().map(|(ix, (does, keys))| {
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .min_h(px(40.))
                    .px_3()
                    .when(ix > 0, |v| v.border_t_1().border_color(Theme::separator))
                    .child(label(does).flex_1().number_of_lines(1))
                    .child(text(keys).text_size(px(TEXT)).text_color(Theme::muted))
                    .into_element()
            })))
        }),
    )
}

fn profiles(ui: Ui) -> View {
    let (list, current) = ui.read_data(|s| (s.profiles(), s.model.profile()));
    let rows = list.into_iter().enumerate().map(move |(ix, p)| {
        let this = current == Some((p.is_private, p.id));
        let mark = if p.is_private {
            view().text_color(Theme::warning).child(glyphs::icon(glyphs::PERSON, 14.))
        } else {
            view().w(px(12.)).h(px(12.)).rounded_full().bg(Theme::profile_color(&p.color).unwrap_or_else(Theme::muted))
        };
        let value = |t: &str| text(t.to_owned()).text_size(px(TEXT)).text_color(Theme::muted);
        view()
            .flex_row()
            .items_center()
            .gap_3()
            .min_h(px(40.))
            .px_3()
            .when(ix > 0, |v| v.border_t_1().border_color(Theme::separator))
            .child(view().w(px(14.)).items_center().child(mark))
            .child(label(p.name.clone()).flex_1().number_of_lines(1))
            .child(this.then(|| value("This window")))
            .child(p.is_private.then(|| value("Private, not saved")))
            .child(value(browser_core::shield_card::motto(p.shield_level).0))
            .into_element()
    });
    view().child(section("Profiles")).child(group().children(rows))
}

fn body(ui: Ui, pane: Signal<Pane>) -> AnyElement {
    match ui.screen.get() {
        Screen::Extensions => super::super::extensions::pane(ui),
        Screen::Licences => sheets::licences(ui).into_element(),
        Screen::Licence(ix) => sheets::licence(ix).into_element(),
        _ => match pane.get() {
            Pane::Shortcuts => shortcuts().into_element(),
            Pane::Profile => view().pt_4().children(panels::pane_body(ui, Pane::Profile)).into_element(),
            Pane::Privacy => view().children(panels::pane_body(ui, Pane::Privacy)).child(panels::your_data(ui)).into_element(),
            Pane::General => view().child(profiles(ui)).children(panels::pane_body(ui, Pane::General)).into_element(),
            p => view().children(panels::pane_body(ui, p)).into_element(),
        },
    }
}

pub(super) fn pane_width() -> f32 {
    ((nsh_core::prelude::window().width.0 * WINDOW_SHARE).min(WIDTH) - NAV_WIDTH).min(CONTENT_WIDTH) - PANE_INSET
}

fn content(ui: Ui, pane: Signal<Pane>) -> impl IntoElement {
    let key = move || format!("{:?}{:?}", ui.screen.get(), pane.get());
    view().flex_1().min_w(px(0.)).h_full().child(header(ui, pane)).child(dynamic(move || {
        let shown = key();
        Some(scroll().key(shown).flex_1().child(view().w_full().max_w(px(CONTENT_WIDTH)).self_center().px_8().pt_1().pb_8().child(body(ui, pane))))
    }))
}

pub(super) fn window(ui: Ui) -> impl IntoElement {
    let pane = signal(if ui.screen.get_untracked() == Screen::Settings { Pane::Appearance } else { Pane::Profile });
    when(
        move || owns(ui.screen.get()),
        move || {
            let always = KeyOptions { when: When::Always, ..KeyOptions::default() };
            on_key_with("escape", always, move |_| ui.show(Screen::Browse));
            view().absolute().inset_0().items_center().justify_center().bg(rgba(DIM)).child(
                view()
                    .relative()
                    .w(px(WIDTH))
                    .max_w(relative(WINDOW_SHARE))
                    .h(px(HEIGHT))
                    .max_h(relative(0.9))
                    .flex_row()
                    .rounded(px(Theme::radius_card()))
                    .overflow_hidden()
                    .bg(Theme::panel)
                    .border_1()
                    .border_color(Theme::hairline)
                    .shadow(Theme::elevation(16., 40.))
                    .child(nav(ui, pane))
                    .child(content(ui, pane))
                    .child(traffic_lights(ui)),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_takes_every_settings_screen() {
        assert!([Screen::Settings, Screen::Extensions, Screen::Licences, Screen::Licence(0)].into_iter().all(owns));
        assert!(!owns(Screen::Browse) && !owns(Screen::Shield));
    }

    #[test]
    fn back_steps_up_from_a_licence_to_the_pane() {
        assert_eq!(up(Screen::Licence(3)), Some(Screen::Licences));
        assert_eq!(up(Screen::Licences), Some(Screen::Settings));
        assert_eq!(up(Screen::Settings), None);
        assert_eq!(up(Screen::Extensions), None);
    }
}
