use browser_core::suggest::step_selection;
use nsh_base::{button, input};
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use serde_json::Value;
use shield::ShieldLevel;

use super::controls::icon;
use super::{Screen, Ui, accent_pill, caption, detail_row, icon_button, icons, label, pill, small, surface};
use crate::shell::Page;
use tarn_nsh_shared::icons as glyphs;

use crate::theme::{BADGE_SIZE, ICON, SYSTEM_FONT, Secure, TEXT_SMALL, TEXT_TITLE, TOOLBAR_HEIGHT, TOUCH_TARGET, Theme, themed};

const PILL_HEIGHT: f32 = 46.;
const PILL_RADIUS: f32 = 15.;
const CHIP_INSET: f32 = 6.;
const CHIP_HEIGHT: f32 = PILL_HEIGHT - 2. - 2. * CHIP_INSET;
const CHIP_GLYPH: f32 = 15.;

fn level_name(level: Option<ShieldLevel>) -> &'static str {
    match level {
        Some(ShieldLevel::Standard) | None => "Standard",
        Some(ShieldLevel::Strict) => "Strict",
        Some(ShieldLevel::Lockdown) => "Lockdown",
    }
}

fn history_button(ui: Ui, asset: ImageAsset, name: &'static str, command: &'static str, can: fn(&Page) -> bool) -> impl IntoElement {
    icon_button(asset, name, move || ui.act_on_screen(|s| s.active_command(command, Value::Null)))
        .disabled(move || !ui.read(|s| s.active_page().is_some_and(can)))
        .disabled_style(|s| s.opacity(0.35))
}

fn back(ui: Ui) -> impl IntoElement {
    history_button(ui, icons::BACK, "Back", "back", |p| p.back)
}

fn forward(ui: Ui) -> impl IntoElement {
    history_button(ui, icons::FORWARD, "Forward", "forward", |p| p.forward)
}

fn tabs(ui: Ui) -> impl IntoElement {
    button()
        .accessibility_label(move || SharedString::from(format!("Tabs {}", ui.read(|s| s.tab_count()))))
        .w(px(TOUCH_TARGET))
        .h(px(TOUCH_TARGET))
        .items_center()
        .justify_center()
        .rounded_full()
        .active(|s| s.bg(Theme::pressed))
        .on_press(move || ui.toggle(Screen::Tabs))
        .child(
            view()
                .min_w(px(BADGE_SIZE))
                .h(px(BADGE_SIZE))
                .px_1()
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_small()))
                .border_2()
                .border_color(Theme::text)
                .child(text(move || ui.read(|s| s.tab_count()).to_string()).text_size(px(TEXT_SMALL)).font_weight(FontWeight::BOLD).text_color(Theme::text)),
        )
}

fn new_tab(ui: Ui) -> impl IntoElement {
    icon_button(icons::PLUS, "New Tab", move || {
        let before = ui.read(|s| s.model.active);
        let mut opened = None;
        ui.act(|s| {
            opened = s.new_tab(None);
            Vec::new()
        });
        ui.show(Screen::Browse);
        ui.fresh.set(opened.zip(before).filter(|(k, b)| k != b));
        ui.field.focus();
    })
}

pub(super) fn cancel(ui: Ui) {
    ui.editing.set(false);
    dismiss_keyboard();
    let Some((key, before)) = ui.fresh.get_untracked() else { return };
    ui.fresh.set(None);
    ui.act(|s| {
        s.discard_new_tab(key, before);
        Vec::new()
    });
}

fn shield_button(ui: Ui) -> impl IntoElement {
    let level = move || ui.read(|s| s.site_level());
    let private = move || ui.read(|s| s.model.private());
    let burner = move || ui.read(|s| s.in_burner());
    let named = move || level().is_some_and(|l| l != ShieldLevel::Standard);
    button()
        .accessibility_label(move || {
            SharedString::from(format!(
                "Shield: {}{}",
                if burner() {
                    "Burner, "
                } else if private() {
                    "Private, "
                } else {
                    ""
                },
                level_name(level())
            ))
        })
        .min_w(px(TOUCH_TARGET))
        .h(px(TOUCH_TARGET))
        .px_2()
        .flex_row()
        .gap_1()
        .items_center()
        .justify_center()
        .rounded(px(Theme::radius_row()))
        .active(|s| s.bg(Theme::pressed))
        .on_press(move || ui.show(Screen::Shield))
        .child(dynamic(move || {
            if burner() {
                icon(icons::FLAME, Secure::warning).into_element()
            } else {
                icon(icons::SHIELD_CHECK, move || if private() { Secure::warning() } else { Secure::muted() }).into_element()
            }
        }))
        .child(dynamic(move || named().then(|| small(level_name(level())).text_color(Secure::muted).font_family(SYSTEM_FONT))))
}

fn reload(ui: Ui) -> impl IntoElement {
    let reloaded = signal(0u64);
    let again = move || {
        ui.act(|s| s.active_command("reload", Value::Null));
        reloaded.update(|n| *n += 1);
        let serial = reloaded.get_untracked();
        set_timeout(tarn_nsh_shared::motion::RELOAD_SPIN, move || {
            if reloaded.get_untracked() == serial {
                reloaded.set(0);
            }
        });
    };
    let state = computed(move || (ui.read(|s| s.active_page().is_some_and(|p| p.loading)), reloaded.get() > 0));
    dynamic(move || {
        let (loading, again_spinning) = state.get();
        if !loading && !again_spinning {
            return icon_button(icons::RELOAD, "Reload", again).into_element();
        }
        let press = move || {
            if untracked(|| ui.read(|s| s.active_page().is_some_and(|p| p.loading))) {
                ui.act(|s| s.active_command("stop", Value::Null));
            } else {
                again();
            }
        };
        let name = if loading { "Stop Loading" } else { "Reload" };
        if reduce_motion() {
            return icon_button(icons::CLOSE, name, press).into_element();
        }
        tarn_nsh_shared::controls::icon_button(tarn_nsh_shared::motion::spinner(20., icon(icons::RELOAD, Theme::text)), name, press).into_element()
    })
}

fn address_input(ui: Ui) -> nsh_base::Input {
    let field = ui.field;
    let typed = signal(String::new());
    let completed = signal(None::<(String, String)>);
    let history = signal(std::rc::Rc::new(Vec::<store::HistoryEntry>::new()));
    let resting = move || ui.read(|s| browser_core::command::display_host(&s.active_url()));
    input()
        .element_ref(&field)
        .value(move || if ui.editing.get() { ui.address.get() } else { resting() })
        .on_focus(move || {
            let current = ui.read(|s| s.active_url());
            let end = current.encode_utf16().count() as u32;
            ui.address.set(current);
            typed.set(String::new());
            completed.set(None);
            ui.picked.set(None);
            ui.suggest.set(Vec::new());
            history.set(std::rc::Rc::new(ui.read(|s| s.suggestion_history())));
            ui.editing.set(true);
            field.set_selection(0, end);
        })
        .on_change(move |value: &str| {
            if !ui.editing.get_untracked() || value == ui.address.get_untracked() {
                return;
            }
            let (rows, completion) = ui.read(|s| s.suggest_from(&history.get_untracked(), &typed.get_untracked(), value));
            typed.set(value.to_owned());
            ui.picked.set(None);
            ui.suggest.set(rows);
            let full = format!("{value}{}", completion.as_ref().map_or("", |c| c.suffix.as_str()));
            completed.set(completion.map(|c| (full.clone(), c.url.to_string())));
            ui.address.set(full.clone());
            if full.len() > value.len() {
                field.set_selection(value.encode_utf16().count() as u32, full.encode_utf16().count() as u32);
            }
        })
        .on_submit(move |value: &str| {
            let picked = ui.picked.get_untracked().and_then(|ix| ui.suggest.with_untracked(|rows| rows.get(ix).map(|h| h.url.clone())));
            let typed = picked.or_else(|| completed.get_untracked().filter(|(full, _)| full == value).map(|(_, url)| url)).unwrap_or_else(|| value.to_owned());
            ui.editing.set(false);
            ui.fresh.set(None);
            ui.screen.set(Screen::Browse);
            dismiss_keyboard();
            ui.act(|s| s.navigate(&typed));
        })
        .keyboard_type(KeyboardType::Url)
        .return_key_type(ReturnKeyType::Go)
        .auto_capitalize(AutoCapitalize::None)
        .placeholder("Search or enter address")
        .placeholder_color(Theme::faint)
        .label("Address")
        .test_id("address")
        .flex_1()
        .px_2()
        .text_size(px(TEXT_TITLE))
        .style_when(move || !ui.editing.get() && !ui.wide(), |s| s.text_center())
        .text_color(move || match (ui.editing.get(), resting().starts_with("Not Secure")) {
            (true, _) => Theme::text(),
            (false, false) => Secure::text(),
            (false, true) => Secure::warning(),
        })
}

fn cancel_button(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        ui.editing.get().then(|| {
            button()
                .accessibility_label("Cancel")
                .h(px(TOUCH_TARGET))
                .px_2()
                .justify_center()
                .on_press(move || cancel(ui))
                .child(text("Cancel").text_size(px(TEXT_TITLE)).text_color(Theme::accent))
        })
    })
}

fn page_actions(ui: Ui) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .child(themed(move || {
            dynamic(move || {
                (!ui.editing.get() && ui.read(|s| s.active_page().is_some_and(|p| p.article)))
                    .then(|| icon_button(icons::BOOK_OPEN, "Reader", move || ui.act(|s| s.open_reader())))
            })
        }))
        .child(themed(move || {
            dynamic(move || {
                (!ui.editing.get() && !ui.read(|s| s.active_url().is_empty())).then(|| {
                    tarn_nsh_shared::controls::icon_button(view().text_color(Theme::text).child(glyphs::icon(glyphs::SHARE, ICON)), "Share", move || {
                        ui.show(Screen::Share)
                    })
                })
            })
        }))
}

pub fn top_bar(ui: Ui) -> impl IntoElement {
    when(
        move || ui.wide(),
        move || {
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .min_h(px(TOOLBAR_HEIGHT))
                .child(themed(move || view().flex_row().child(back(ui)).child(forward(ui)).child(reload(ui))))
                .child(
                    view()
                        .flex_1()
                        .flex_row()
                        .items_center()
                        .h(px(TOUCH_TARGET))
                        .pl_1()
                        .rounded(move || px(Theme::radius_group()))
                        .bg(move || if ui.editing.get() { Theme::raised() } else { Secure::chip() })
                        .border_1()
                        .border_color(move || if ui.editing.get() { Theme::accent() } else { Secure::hairline() })
                        .font_family(SYSTEM_FONT)
                        .child(themed(move || dynamic(move || (!ui.editing.get()).then(|| shield_button(ui)))))
                        .child(address_input(ui))
                        .child(page_actions(ui)),
                )
                .child(cancel_button(ui))
                .child(themed(move || {
                    dynamic(move || {
                        (!ui.editing.get()).then(|| {
                            view().flex_row().child(new_tab(ui)).child(tabs(ui)).child(icon_button(icons::MENU, "Menu", move || ui.toggle(Screen::Menu)))
                        })
                    })
                }))
        },
    )
}

pub fn page_error(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let error = ui.read(|s| s.active_page().and_then(|p| p.error.clone()));
        error.filter(|_| ui.screen.get() == Screen::Browse && !ui.editing.get()).map(|e| {
            surface(Theme::radius_panel())
                .flex_row()
                .items_center()
                .gap_2()
                .mx_3()
                .my_1()
                .pl_4()
                .pr_1()
                .py_1()
                .shadow(Theme::elevation(4., 14.))
                .child(label(format!("This page could not be loaded: {e}")).flex_1().test_id("page-error").text_color(Theme::danger))
                .child(accent_pill("Retry", move || ui.act(|s| s.active_command("reload", Value::Null))))
                .child(pill("Dismiss", move || ui.act(|s| s.dismiss_error())))
        })
    })
}

fn shield_chip(ui: Ui) -> impl IntoElement {
    let level = computed(move || ui.read(|s| s.site_level()));
    let count = move || ui.read(|s| s.site_shield_total());
    let private = move || ui.read(|s| s.model.private());
    let burner = move || ui.read(|s| s.in_burner());
    button()
        .accessibility_label(move || {
            let n = count();
            let kind = if burner() {
                "Burner, "
            } else if private() {
                "Private, "
            } else {
                ""
            };
            let stopped = if n > 0 { format!(", {n} stopped on this site") } else { String::new() };
            SharedString::from(format!("Shield: {kind}{}{stopped}", level_name(level.get())))
        })
        .h(px(CHIP_HEIGHT))
        .min_w(px(CHIP_HEIGHT))
        .px_2()
        .flex_row()
        .gap_1()
        .items_center()
        .justify_center()
        .rounded(px(PILL_RADIUS - 1. - CHIP_INSET))
        .bg(Secure::control)
        .active(|s| s.bg(Secure::pressed()))
        .on_press(move || ui.show(Screen::Shield))
        .child(dynamic(move || {
            let (glyph, tint): (&'static str, fn() -> Rgba) = match (burner(), level.get()) {
                (true, _) => (glyphs::FLAME, Secure::warning),
                (false, Some(ShieldLevel::Strict | ShieldLevel::Lockdown)) => (glyphs::SHIELD_CHECK, if private() { Secure::warning } else { Secure::accent }),
                (false, _) => (glyphs::SHIELD, if private() { Secure::warning } else { Secure::muted }),
            };
            view().text_color(tint).bind_motion(&tarn_nsh_shared::motion::fade_in()).child(glyphs::icon(glyph, CHIP_GLYPH))
        }))
        .child(dynamic(move || {
            let raised = matches!(level.get(), Some(ShieldLevel::Strict | ShieldLevel::Lockdown));
            raised.then(|| small(level_name(level.get())).text_color(Secure::accent).font_family(SYSTEM_FONT).font_weight(FontWeight::MEDIUM))
        }))
        .child(dynamic(move || {
            let n = count();
            (n > 0).then(|| small(browser_core::shield_card::thousands(n as u64)).text_color(Secure::muted).font_family(SYSTEM_FONT))
        }))
}

fn address_pill(ui: Ui) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .h(px(PILL_HEIGHT))
        .px(px(CHIP_INSET))
        .gap_1()
        .rounded(px(PILL_RADIUS))
        .bg(move || if ui.editing.get() { Theme::raised() } else { Secure::chip() })
        .border_1()
        .border_color(move || if ui.editing.get() { Theme::accent() } else { Secure::hairline() })
        .font_family(SYSTEM_FONT)
        .child(themed(move || dynamic(move || (!ui.editing.get()).then(|| shield_chip(ui)))))
        .child(address_input(ui))
        .child(page_actions(ui))
}

pub fn bottom_bar(ui: Ui) -> impl IntoElement {
    when(
        move || !ui.wide(),
        move || {
            view()
                .px_2()
                .pt_2()
                .gap_1()
                .border_t_1()
                .border_color(Theme::hairline)
                .style_with(move |s| if ui.editing.get() { s.pb_keyboard_plus(px(8.)) } else { s.pb_safe_area() })
                .child(view().flex_row().items_center().gap_1().child(view().flex_1().child(address_pill(ui))).child(cancel_button(ui)))
                .child(themed(move || {
                    view().flex_row().justify_around().items_center().child(back(ui)).child(forward(ui)).child(new_tab(ui)).child(tabs(ui)).child(
                        tarn_nsh_shared::controls::icon_button(view().text_color(Theme::text).child(glyphs::icon(glyphs::ELLIPSIS, ICON)), "Menu", move || {
                            ui.toggle(Screen::Menu)
                        }),
                    )
                }))
        },
    )
}

/// While the address field is edited, Up and Down step through the rows and wrap at the ends; Page
/// Up and Page Down jump to the first and last. Elsewhere the keys stay with the page.
fn suggestion_keys(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        ui.editing.get().then(|| {
            bind_suggestion_keys(ui);
            view()
        })
    })
}

fn bind_suggestion_keys(ui: Ui) {
    for key in ["up", "down", "pageup", "pagedown"] {
        on_key_with(key, KeyOptions { when: When::Always, repeat: true, ..KeyOptions::default() }, move |_| {
            let len = ui.suggest.with_untracked(Vec::len);
            if len == 0 {
                return;
            }
            let forward = matches!(key, "down" | "pagedown");
            let next = match (key, ui.picked.get_untracked()) {
                ("pageup", _) => 0,
                ("pagedown", _) => len - 1,
                (_, Some(at)) => step_selection(at, len, forward),
                (_, None) => step_selection(if forward { len - 1 } else { 0 }, len, forward),
            };
            ui.picked.set(Some(next));
        });
    }
}

pub fn suggestions(ui: Ui) -> impl IntoElement {
    let found = move || if ui.editing.get() { ui.suggest.get() } else { Vec::new() };
    let list = scroll().flex_shrink(1.).style_when(move || ui.wide(), |s| s.flex_1()).child(suggestion_keys(ui)).child(
        surface(Theme::radius_panel()).p_1().m_2().style_when(move || found().is_empty(), |s| s.hidden()).child(each(
            found,
            |h: &store::HistoryEntry| h.url.clone(),
            move |h, ix| {
                let entry = h.get_untracked();
                let url = entry.url.clone();
                let name = if entry.title.is_empty() { entry.url.clone() } else { entry.title.clone() };
                let detail = entry.url.trim_start_matches("https://").trim_end_matches('/').to_owned();
                detail_row(Some(icons::GLOBE), name, Some(caption(detail).number_of_lines(1)), move || {
                    ui.editing.set(false);
                    dismiss_keyboard();
                    ui.act(|s| s.navigate(&url));
                })
                .border_b(px(0.))
                .rounded(px(Theme::radius_row()))
                .bg(move || if ui.picked.get() == Some(ix.get()) { Theme::selected() } else { Theme::clear() })
            },
        )),
    );
    view().flex_1().style_when(move || !ui.editing.get(), |s| s.hidden()).style_when(move || !ui.wide(), |s| s.justify_end()).child(list)
}
