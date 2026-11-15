use nsh_base::{button, input};
use nsh_core::prelude::*;
use serde_json::Value;
use shield::ShieldLevel;

use super::controls::icon;
use super::{Screen, Ui, accent_pill, caption, icon_button, icons, label, pill, small, surface};
use crate::theme::{BADGE_SIZE, SYSTEM_FONT, Secure, TEXT_SMALL, TEXT_TITLE, TOOLBAR_HEIGHT, TOUCH_TARGET, Theme, themed};

fn level_name(level: Option<ShieldLevel>) -> &'static str {
    match level {
        Some(ShieldLevel::Standard) | None => "Standard",
        Some(ShieldLevel::Strict) => "Strict",
        Some(ShieldLevel::Lockdown) => "Lockdown",
    }
}

fn back(ui: Ui) -> impl IntoElement {
    icon_button(icons::BACK, "Back", move || ui.act(|s| s.active_command("back", Value::Null)))
}

fn forward(ui: Ui) -> impl IntoElement {
    icon_button(icons::FORWARD, "Forward", move || ui.act(|s| s.active_command("forward", Value::Null)))
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
        .on_press(move || ui.show(Screen::Tabs))
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

fn cancel(ui: Ui) {
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

pub fn top_bar(ui: Ui) -> impl IntoElement {
    let field = ui.field;
    let typed = signal(String::new());
    let completed = signal(None::<(String, String)>);
    let resting = move || ui.read(|s| browser_core::command::display_host(&s.active_url()));
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .px_2()
        .min_h(px(TOOLBAR_HEIGHT))
        .child(themed(move || dynamic(move || ui.wide().then(|| view().flex_row().child(back(ui)).child(forward(ui)).child(reload(ui))))))
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
                .child(
                    input()
                        .element_ref(&field)
                        .value(move || if ui.editing.get() { ui.address.get() } else { resting() })
                        .on_focus(move || {
                            let current = ui.read(|s| s.active_url());
                            let end = current.encode_utf16().count() as u32;
                            ui.address.set(current);
                            typed.set(String::new());
                            completed.set(None);
                            ui.suggest.set(Vec::new());
                            ui.editing.set(true);
                            field.set_selection(0, end);
                        })
                        .on_change(move |value: &str| {
                            if !ui.editing.get_untracked() || value == ui.address.get_untracked() {
                                return;
                            }
                            let (rows, completion) = ui.read(|s| s.suggestions(&typed.get_untracked(), value));
                            typed.set(value.to_owned());
                            ui.suggest.set(rows);
                            let full = format!("{value}{}", completion.as_ref().map_or("", |c| c.suffix.as_str()));
                            completed.set(completion.map(|c| (full.clone(), c.url.to_string())));
                            ui.address.set(full.clone());
                            if full.len() > value.len() {
                                field.set_selection(value.encode_utf16().count() as u32, full.encode_utf16().count() as u32);
                            }
                        })
                        .on_submit(move |value: &str| {
                            let typed = completed.get_untracked().filter(|(full, _)| full == value).map_or_else(|| value.to_owned(), |(_, url)| url);
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
                        .text_color(move || match (ui.editing.get(), resting().starts_with("Not Secure")) {
                            (true, _) => Theme::text(),
                            (false, false) => Secure::text(),
                            (false, true) => Secure::warning(),
                        }),
                )
                .child(themed(move || {
                    dynamic(move || {
                        (!ui.editing.get() && ui.read(|s| s.active_page().is_some_and(|p| p.article)))
                            .then(|| icon_button(icons::BOOK_OPEN, "Reader", move || ui.act(|s| s.open_reader())))
                    })
                }))
                .child(themed(move || {
                    dynamic(move || {
                        (!ui.editing.get() && !ui.read(|s| s.active_url().is_empty()))
                            .then(|| icon_button(icons::SHARE, "Share", move || super::panels::share(ui)))
                    })
                })),
        )
        .child(dynamic(move || {
            ui.editing.get().then(|| {
                button()
                    .accessibility_label("Cancel")
                    .h(px(TOUCH_TARGET))
                    .px_2()
                    .justify_center()
                    .on_press(move || cancel(ui))
                    .child(text("Cancel").text_size(px(TEXT_TITLE)).text_color(Theme::accent))
            })
        }))
        .child(themed(move || {
            dynamic(move || {
                (ui.wide() && !ui.editing.get())
                    .then(|| view().flex_row().child(new_tab(ui)).child(tabs(ui)).child(icon_button(icons::MENU, "Menu", move || ui.show(Screen::Menu))))
            })
        }))
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

pub fn bottom_bar(ui: Ui) -> impl IntoElement {
    view()
        .flex_row()
        .justify_around()
        .items_center()
        .px_2()
        .pt_1()
        .pb_safe_area()
        .border_t_1()
        .border_color(Theme::hairline)
        .style_when(move || ui.wide(), |s| s.hidden())
        .child(back(ui))
        .child(forward(ui))
        .child(new_tab(ui))
        .child(tabs(ui))
        .child(icon_button(icons::MENU, "Menu", move || ui.show(Screen::Menu)))
}

pub fn suggestions(ui: Ui) -> impl IntoElement {
    let found = move || if ui.editing.get() { ui.suggest.get() } else { Vec::new() };
    scroll().flex_1().style_when(move || !ui.editing.get(), |s| s.hidden()).child(
        surface(Theme::radius_panel()).p_1().m_2().style_when(move || found().is_empty(), |s| s.hidden()).child(each(
            found,
            |h: &store::HistoryEntry| h.url.clone(),
            move |h, _| {
                let entry = h.get_untracked();
                let url = entry.url.clone();
                let name = if entry.title.is_empty() { entry.url.clone() } else { entry.title.clone() };
                let detail = entry.url.trim_start_matches("https://").trim_end_matches('/').to_owned();
                button()
                    .accessibility_label(SharedString::from(name.clone()))
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .min_h(px(TOUCH_TARGET))
                    .px_3()
                    .py_1()
                    .rounded(px(Theme::radius_row()))
                    .active(|s| s.bg(Theme::selected))
                    .on_press(move || {
                        ui.editing.set(false);
                        dismiss_keyboard();
                        ui.act(|s| s.navigate(&url));
                    })
                    .child(icon(icons::GLOBE, Theme::muted))
                    .child(view().flex_1().child(label(name).number_of_lines(1)).child(caption(detail).number_of_lines(1)))
            },
        )),
    )
}
