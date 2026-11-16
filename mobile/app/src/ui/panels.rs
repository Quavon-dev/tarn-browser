use browser_core::command::ENGINES;
use browser_core::tabs::TabKey;
use nsh_base::{alert_dialog, button, input};
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use serde_json::json;
use shield::ShieldLevel;

use super::controls::icon;
use super::sheets;
use super::{
    Screen, Ui, accent_pill, caption, detail_row, framed, heading, icon_button, icons, label, pill, row, section, segmented, small, surface, switch_row,
    text_field,
};
use crate::shell::{FILTER_SWITCHES, SpaceItem};
use crate::theme::{GAP, SHEET_WIDTH, Secure, TAB_MIN_WIDTH, TAB_PREVIEW_HEIGHT, TEXT_TITLE, TOUCH_TARGET, Theme};
use tarn_nsh_shared::controls::empty_state;
use tarn_nsh_shared::panes::{self, Pane};

fn title(screen: Screen) -> &'static str {
    match screen {
        Screen::Browse | Screen::Find | Screen::Share => "",
        Screen::Tabs => "Tabs",
        Screen::Menu => "Menu",
        Screen::Shield => "Shield",
        Screen::Settings => "Settings",
        Screen::History => "History",
        Screen::Downloads => "Downloads",
        Screen::Spaces => "Spaces and Profiles",
        other => sheets::heading_for(other).unwrap_or_default(),
    }
}

pub fn panel(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let screen = ui.screen.get();
        let body = match screen {
            Screen::Browse | Screen::Share => return None,
            Screen::Find => return Some(find(ui).into_element()),
            Screen::Tabs => tabs(ui).into_element(),
            Screen::Menu => menu(ui).into_element(),
            Screen::Shield => site_shield(ui).into_element(),
            Screen::Settings => settings(ui).into_element(),
            Screen::History => history(ui).into_element(),
            Screen::Downloads => downloads(ui).into_element(),
            Screen::Spaces => spaces(ui).into_element(),
            Screen::Reader => sheets::reader(ui).into_element(),
            Screen::FindTabs => sheets::find_tabs(ui).into_element(),
            Screen::Qr => sheets::qr(ui).into_element(),
            Screen::ReadingList => sheets::reading_list(ui).into_element(),
            Screen::Move(key) => sheets::move_tab(ui, key).into_element(),
            Screen::SpaceEdit(ix) => sheets::space_edit(ui, ix).into_element(),
        };
        Some(sheet(ui, screen, body).into_element())
    })
}

pub(super) fn parent(screen: Screen) -> Option<Screen> {
    match screen {
        Screen::Move(_) => Some(Screen::Tabs),
        Screen::SpaceEdit(_) => Some(Screen::Spaces),
        Screen::History | Screen::Downloads | Screen::Spaces | Screen::Settings | Screen::Qr | Screen::ReadingList | Screen::FindTabs => Some(Screen::Menu),
        _ => None,
    }
}

pub(super) fn leave(ui: Ui, to: Screen) {
    ui.act(|s| {
        s.close_reader();
        Vec::new()
    });
    ui.show(to);
}

fn sheet(ui: Ui, screen: Screen, body: AnyElement) -> impl IntoElement {
    let name = title(screen);
    let header = view().flex_row().items_center().pr_2().pt_2();
    let header = match parent(screen) {
        Some(up) => {
            header.pl_1().child(tarn_nsh_shared::controls::icon_button(icon(icons::BACK, Theme::text), format!("Back to {}", title(up)), move || leave(ui, up)))
        }
        None => header.pl_4(),
    };
    surface(Theme::radius_card())
        .flex_1()
        .style_when(move || ui.wide(), |s| s.self_center().w(px(SHEET_WIDTH)))
        .mx_2()
        .mt_1()
        .mb_2()
        .overflow_hidden()
        .child(
            header.child(view().flex_1().child(heading(name))).child(
                button()
                    .accessibility_label("Done")
                    .h(px(TOUCH_TARGET))
                    .px_3()
                    .justify_center()
                    .on_press(move || leave(ui, Screen::Browse))
                    .child(text("Done").text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::accent)),
            ),
        )
        .child(scroll().key(name).flex_1().child(view().gap_3().px_3().pt_2().pb_6().child(body)))
}

fn group() -> View {
    view().mb(px(-1.))
}

fn card_width(ui: Ui) -> Points {
    let columns = if ui.wide() { 3.0 } else { 2.0 };
    let sheet = window().width.0.min(SHEET_WIDTH);
    px(((sheet - GAP * 4.0 - (columns - 1.0) * GAP) / columns).max(TAB_MIN_WIDTH))
}

fn tabs(ui: Ui) -> impl IntoElement {
    let cards = move || {
        ui.read(|s| {
            s.model
                .tabs
                .iter()
                .map(|t| {
                    let key = s.model.key(t);
                    let page = s.pages.get(&key);
                    let url = page.map(|p| p.url.clone()).filter(|u| !u.is_empty()).unwrap_or(t.url.clone());
                    let title = page.map(|p| p.title.clone()).filter(|t| !t.is_empty()).unwrap_or(t.title.clone());
                    (key, if title.is_empty() { "New Tab".to_owned() } else { title }, url)
                })
                .collect::<Vec<(TabKey, String, String)>>()
        })
    };
    let split = move |f: fn(&mut super::AppShell, TabKey) -> bool| {
        move || {
            ui.act(|s| {
                if let Some(active) = s.model.active {
                    f(s, active);
                }
                Vec::new()
            })
        }
    };
    view()
        .gap_3()
        .child(caption(move || ui.read(|s| s.model.current_space().map(|c| c.space.name.clone()).unwrap_or_default())).text_color(Theme::space_label).px_1())
        .child(view().flex_row().flex_wrap().gap_3().child(each(
            cards,
            |c: &(TabKey, String, String)| c.0,
            move |card, _| {
                let (key, title, url) = card.get_untracked();
                let current = move || ui.read(|s| s.model.active == Some(key));
                let movable = move || ui.read(|s| !s.in_burner() && !s.move_targets(key).is_empty());
                view()
                    .w(move || card_width(ui))
                    .rounded(px(Theme::radius_tile()))
                    .bg(Theme::surface)
                    .border_2()
                    .border_color(move || if current() { Theme::accent() } else { Theme::surface_edge() })
                    .shadow(Theme::elevation(2., 6.))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .pl_3()
                            .child(dynamic(move || ui.read(|s| s.in_burner()).then(|| icon(icons::FLAME, Secure::warning))))
                            .child(label(title.clone()).font_weight(FontWeight::SEMIBOLD).number_of_lines(1).flex_1())
                            .child(dynamic(move || movable().then(|| icon_button(icons::CHEVRON_RIGHT, "Move to Space", move || ui.show(Screen::Move(key))))))
                            .child(dynamic(move || {
                                let menu = ui.read(|s| s.splits.menu(s.model.active, key));
                                (ui.wide() && menu.enabled).then(|| {
                                    icon_button(icons::APP_WINDOW, menu.entry.label(), move || {
                                        ui.act(|s| {
                                            s.split_with(key);
                                            Vec::new()
                                        });
                                        ui.show(Screen::Browse);
                                    })
                                })
                            }))
                            .child(dynamic(move || {
                                (ui.wide() && ui.read(|s| s.splits.contains(key))).then(|| {
                                    icon_button(icons::SHARE, "Remove from Split View", move || {
                                        ui.act(|s| {
                                            s.leave_split(key);
                                            Vec::new()
                                        })
                                    })
                                })
                            }))
                            .child(icon_button(icons::CLOSE, "Close Tab", move || {
                                ui.act(|s| {
                                    s.close_tab(key);
                                    Vec::new()
                                })
                            })),
                    )
                    .child(
                        button()
                            .accessibility_label(title.clone())
                            .h(px(TAB_PREVIEW_HEIGHT))
                            .mx_2()
                            .mb_2()
                            .p_2()
                            .rounded(px(Theme::radius_row()))
                            .bg(Theme::page)
                            .active(|s| s.bg(Theme::panel_row))
                            .selected(current)
                            .on_press(move || {
                                ui.act(|s| {
                                    s.activate(key);
                                    Vec::new()
                                });
                                ui.show(Screen::Browse);
                            })
                            .child(caption(browser_core::command::display_host(&url)).number_of_lines(1)),
                    )
            },
        )))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(pill("New Tab", move || {
                    ui.act(|s| {
                        s.new_tab(None);
                        Vec::new()
                    });
                    ui.show(Screen::Browse);
                }))
                .child(pill("Burner Tab", move || {
                    ui.act(|s| {
                        s.open_burner();
                        Vec::new()
                    });
                    ui.show(Screen::Browse);
                }))
                .child(dynamic(move || {
                    ui.read(|s| !s.duplicate_tabs().is_empty()).then(|| {
                        pill("Close Duplicates", move || {
                            ui.act(|s| {
                                s.close_duplicates();
                                Vec::new()
                            })
                        })
                    })
                }))
                .child(dynamic(move || {
                    (ui.wide() && ui.read(|s| s.shown_split().is_some())).then(|| {
                        view()
                            .flex_row()
                            .gap_2()
                            .child(pill("Rotate Layout", split(|s, k| s.splits.rotate(k))))
                            .child(pill("Close Split View", split(|s, k| s.splits.close(k))))
                    })
                }))
                .child(pill("Find in Tabs", move || ui.show(Screen::FindTabs)))
                .child(pill("Spaces", move || ui.show(Screen::Spaces))),
        )
}

fn menu(ui: Ui) -> impl IntoElement {
    let private = move || ui.read(|s| s.model.private());
    let zoom = move |delta: f64| {
        ui.act(|s| {
            let z = s.active_page().map_or(1.0, |p| p.zoom);
            s.zoom(if delta == 0.0 { 1.0 } else { z + delta })
        })
    };
    let level = computed(move || ui.read(|s| (s.active_page().map_or(1.0, |p| p.zoom) * 100.).round() as i64));
    let shareable = computed(move || ui.read(|s| s.share_url().is_some()));
    view()
        .gap_2()
        .child(framed(
            group()
                .child(row(icons::PLUS, "New Tab", move || {
                    ui.act(|s| {
                        s.new_tab(None);
                        Vec::new()
                    });
                    ui.show(Screen::Browse);
                }))
                .child(dynamic(move || {
                    if private() {
                        row(icons::HAND, "Close Private Space", move || {
                            ui.act(|s| {
                                s.close_private();
                                Vec::new()
                            });
                            ui.show(Screen::Browse);
                        })
                        .into_element()
                    } else {
                        row(icons::HAND, "Private Space", move || {
                            ui.act(|s| {
                                s.enter_private();
                                Vec::new()
                            });
                            ui.show(Screen::Browse);
                        })
                        .into_element()
                    }
                }))
                .child(dynamic(move || {
                    (!ui.read(|s| s.in_burner() || <crate::data::Data as crate::data::Platform>::private_process())).then(|| {
                        row(icons::FLAME, "New Burner Tab", move || {
                            ui.act(|s| {
                                s.open_burner();
                                Vec::new()
                            });
                            ui.show(Screen::Browse);
                        })
                    })
                }))
                .child(dynamic(move || {
                    ui.read(|s| !s.model.closed_tabs.is_empty()).then(|| {
                        row(icons::UNDO, "Reopen Closed Tab", move || {
                            ui.act(|s| {
                                if !s.reopen_closed() {
                                    s.notice = Some("No closed tab to reopen.".into());
                                }
                                Vec::new()
                            });
                            ui.show(Screen::Browse);
                        })
                    })
                })),
        ))
        .child(framed(
            group()
                .child(row(icons::SEARCH, "Find in Page", move || ui.show(Screen::Find)))
                .child(row(icons::APP_WINDOW, "Find in Tabs", move || ui.show(Screen::FindTabs)))
                .child(dynamic(move || ui.read(|s| s.offers_reader()).then(|| row(icons::BOOK_OPEN, "Reader", move || ui.act(|s| s.open_reader())))))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .min_h(px(TOUCH_TARGET))
                        .py_1()
                        .border_b_1()
                        .border_color(Theme::separator)
                        .child(label("Zoom").flex_1())
                        .child(pill("−", move || zoom(-0.1)).accessibility_label("Zoom Out"))
                        .child(dynamic(move || Some(pill(format!("{}%", level.get()), move || zoom(0.0)).accessibility_label("Actual Size"))))
                        .child(pill("+", move || zoom(0.1)).accessibility_label("Zoom In")),
                )
                .child(row(icons::SHIELD, "Shield and Settings for This Site", move || ui.show(Screen::Shield)))
                .child(dynamic(move || {
                    ui.read(|s| s.zappable()).then(|| {
                        row(icons::CLOSE, "Zap Element", move || {
                            ui.show(Screen::Browse);
                            ui.act(|s| s.pick());
                        })
                    })
                })),
        ))
        .child(framed(
            group()
                .child(row(icons::HISTORY, "History", move || ui.show(Screen::History)))
                .child(row(icons::DOWNLOADS, "Downloads", move || ui.show(Screen::Downloads)))
                .child(row(icons::PERSON, "Spaces and Profiles", move || ui.show(Screen::Spaces)))
                .child(row(icons::GEAR, "Settings", move || ui.show(Screen::Settings))),
        ))
        .child(framed(
            group()
                .child(dynamic(move || {
                    shareable.get().then(|| {
                        view()
                            .child(row(icons::SHARE, "Share…", move || ui.show(Screen::Share)))
                            .child(row(icons::SMARTPHONE, "Show QR Code", move || ui.show(Screen::Qr)))
                            .child(row(icons::CHECK, "Copy Clean Link", move || {
                                let mut copied = None;
                                ui.act(|s| {
                                    copied = s.copy_clean_link();
                                    Vec::new()
                                });
                                if let Some(url) = copied {
                                    set_clipboard(url);
                                }
                                ui.show(Screen::Browse);
                            }))
                            .child(row(icons::BOOKMARK, "Save to Reading List", move || {
                                ui.act(|s| {
                                    s.save_for_later(browser_core::now());
                                    Vec::new()
                                });
                                ui.show(Screen::Browse);
                            }))
                    })
                }))
                .child(row(icons::BOOK_OPEN, "Reading List", move || ui.show(Screen::ReadingList))),
        ))
}

fn levels(
    current: impl Fn() -> Option<ShieldLevel> + Copy + 'static,
    set: impl Fn(Option<ShieldLevel>) + Copy + 'static,
    with_default: bool,
) -> impl IntoElement {
    let choices: Vec<(&str, Option<ShieldLevel>)> = [
        ("Profile Default", None),
        ("Standard", Some(ShieldLevel::Standard)),
        ("Strict", Some(ShieldLevel::Strict)),
        ("Lockdown", Some(ShieldLevel::Lockdown)),
    ]
    .into_iter()
    .filter(|(_, l)| with_default || l.is_some())
    .collect();
    segmented(
        choices
            .into_iter()
            .map(|(name, level)| {
                (name.to_owned(), Box::new(move || current() == level) as Box<dyn Fn() -> bool>, Box::new(move || set(level)) as Box<dyn Fn()>)
            })
            .collect(),
    )
}

fn site_shield(ui: Ui) -> impl IntoElement {
    let host = move || ui.read(|s| s.active_host());
    let own = move || {
        ui.read(|s| {
            let (private, id) = s.model.profile()?;
            let host = s.active_host()?;
            s.core.has_override(private, id, &host).then(|| s.site_level()).flatten()
        })
    };
    view()
        .gap_3()
        .child(label(move || match host() {
            Some(host) => format!("Level for {host}"),
            None => "Open a site to choose its level.".into(),
        }))
        .child(dynamic(move || {
            host().map(|_| {
                levels(own, move |level| {
                    ui.act(|s| {
                        s.set_site_level(level);
                        Vec::new()
                    })
                }, true)
            })
        }))
        .child(caption(move || format!("Profile default: {}", ui.read_data(|s| s.default_level().as_str()))))
        .child(caption("Standard blocks trackers and private-network requests. Strict adds HTTPS-only. Lockdown also turns JavaScript off and keeps website data in memory."))
        .child(sheets::site(ui))
}

pub fn share(ui: Ui) {
    let Some(url) = ui.read(|s| s.share_url()) else { return };
    spawn(async move {
        if !crate::platform::share(url.clone()).await {
            set_clipboard(url);
            ui.act(|s| {
                s.notice = Some("Link copied".into());
                Vec::new()
            });
        }
    });
}

fn pane_heading(pane: Pane) -> impl IntoElement {
    label(panes::name(pane)).text_size(px(TEXT_TITLE)).font_weight(FontWeight::BOLD).px_1().pt_4()
}

fn settings(ui: Ui) -> impl IntoElement {
    view()
        .gap_2()
        .child(pane_heading(Pane::Profile))
        .child(sheets::shield_card(ui))
        .child(pane_heading(Pane::General))
        .child(section("Downloads"))
        .child(framed(group().child(switch_row("Sort downloads into folders", move || ui.read_data(|s| s.setting_on(crate::scripts::SORT_DOWNLOADS)), move || {
            ui.act(|s| {
                let on = s.setting_on(crate::scripts::SORT_DOWNLOADS);
                s.set_setting_on(crate::scripts::SORT_DOWNLOADS, !on)
            })
        }))))
        .child(section("About"))
        .child(view().flex_row().flex_wrap().gap_2().child(link_pill(ui, browser_core::LICENCE, browser_core::LICENCE_URL)).child(link_pill(ui, "Source code", browser_core::SOURCE_URL)))
        .child(pane_heading(Pane::Appearance))
        .child(super::appearance::pane(ui))
        .child(pane_heading(Pane::Search))
        .child(section("Search engine"))
        .child(segmented(
            ENGINES
                .iter()
                .enumerate()
                .map(|(ix, engine)| {
                    (
                        engine.name.to_owned(),
                        Box::new(move || ui.read_data(|s| s.search_engine()) == ix) as Box<dyn Fn() -> bool>,
                        Box::new(move || ui.act(|s| {
                            s.set_search_engine(ix);
                            Vec::new()
                        })) as Box<dyn Fn()>,
                    )
                })
                .collect(),
        ))
        .child(section("History"))
        .child(framed(group().child(switch_row("Search the text of visited pages", move || ui.read_data(|s| s.setting_on(crate::scripts::HISTORY_TEXT)), move || {
            ui.act(|s| {
                let on = s.setting_on(crate::scripts::HISTORY_TEXT);
                s.set_setting_on(crate::scripts::HISTORY_TEXT, !on)
            })
        }))))
        .child(caption("Keeps up to 4 KB of each visited page's text on this device so History search finds it. Never in a private space. Turning it off deletes the kept text.").number_of_lines(0).px_1())
        .child(pane_heading(Pane::Privacy))
        .child(section("Shield"))
        .child(levels(move || Some(ui.read_data(|s| s.default_level())), move |level| {
            ui.act(|s| {
                if let Some(level) = level {
                    s.set_default_level(level);
                }
                Vec::new()
            })
        }, false))
        .child(section("Filter lists"))
        .child(framed(group().child(FILTER_SWITCHES.iter().map(|(key, name, _)| {
            switch_row(*name, move || ui.read_data(|s| s.filter_switch(key)), move || {
                ui.act(|s| {
                    s.set_filter_switch(key, !s.filter_switch(key));
                    Vec::new()
                })
            })
            .border_b_1()
            .border_color(Theme::separator)
        }).collect::<Vec<_>>())))
        .child(caption("No filter lists are installed yet; Shield's built-in rules always apply. Cookie banners uses EasyList Cookie and uBlock Cookie Notices once installed; nothing is accepted or refused for you.").number_of_lines(0).px_1())
        .child(pane_heading(Pane::Permissions))
        .child(permissions(ui))
        .child(section("Your data"))
        .child(framed(group().child(row(icons::TRASH, "Clear Browsing Data…", move || {
            confirm("Clear browsing data?", "History, downloads list, closed tabs and this profile's website data are removed.", "Clear", move || {
                ui.act(|s| {
                    s.clear_data();
                    Vec::new()
                })
            })
        }))))
}

/// Asks before a destructive action, which runs only on `action`.
pub(super) fn confirm(title: &'static str, description: &'static str, action: &'static str, then: impl FnOnce() + 'static) {
    let answer = alert_dialog().title(title).description(description).cancel("cancel", "Cancel").destructive("confirm", action).show();
    spawn(async move {
        if answer.await.is_ok_and(|a| a.action.as_deref() == Some("confirm")) {
            then();
        }
    });
}

/// The profile's remembered site permissions, each with Allow, Block and Forget.
fn permissions(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let rows = ui.read_data(|s| s.permissions());
        if rows.is_empty() {
            return empty_state("No site has asked yet. A camera, microphone or location answer you remember shows here.").into_element();
        }
        let set = move |origin: String, kind: String, allow: Option<bool>| {
            move || {
                ui.act(|s| {
                    if let Some((private, profile)) = s.model.profile() {
                        s.core.set_permission(private, profile, &origin, &kind, allow).ok();
                    }
                    Vec::new()
                })
            }
        };
        let items: Vec<AnyElement> = rows
            .into_iter()
            .map(|p| {
                let choice = |on: bool, name: &str, allow: bool| {
                    let press = set(p.origin.clone(), p.kind.clone(), Some(allow));
                    if on { accent_pill(name, press).into_element() } else { pill(name, press).into_element() }
                };
                view()
                    .gap_1()
                    .py_2()
                    .border_b_1()
                    .border_color(Theme::separator)
                    .child(label(format!("{} · {}", p.site, p.label)).number_of_lines(1))
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(choice(p.allowed, "Allow", true))
                            .child(choice(!p.allowed, "Block", false))
                            .child(pill("Forget", set(p.origin.clone(), p.kind.clone(), None))),
                    )
                    .into_element()
            })
            .collect();
        framed(group().px_3().child(items)).into_element()
    })
}

fn link_pill(ui: Ui, name: &'static str, url: &'static str) -> impl IntoElement {
    pill(name, move || {
        ui.act(|s| {
            s.new_tab(Some(url));
            Vec::new()
        });
        ui.show(Screen::Browse);
    })
}

fn history(ui: Ui) -> impl IntoElement {
    let query = signal(String::new());
    let found = signal(Vec::new());
    let lookup = move |v: &str, text: bool| found.set(untracked(|| ui.read(|s| if text { s.search_history(v) } else { s.filter_history(v) })));
    lookup("", false);
    let search = move |v: &str| {
        let v = v.to_owned();
        query.set(v.clone());
        lookup(&v, false);
        if browser_core::suggest::searches_text(v.trim()) {
            set_timeout(browser_core::suggest::TEXT_SEARCH_DELAY, move || {
                if query.get_untracked() == v {
                    lookup(&v, true);
                }
            });
        }
    };
    let rows = move || found.get();
    view()
        .gap_2()
        .child(text_field("Search history", "Search history").value(move || query.get()).on_change(search).auto_capitalize(AutoCapitalize::None))
        .child(dynamic(move || {
            let (count, q) = (found.with(Vec::len), query.get());
            Some(match (count, q.trim().is_empty()) {
                (0, true) => empty_state("No history yet").into_element(),
                (0, false) => empty_state(format!("No results for “{}”", q.trim())).into_element(),
                (n, _) => small(if n == 1 { "1 page".to_owned() } else { format!("{n} pages") }).px_1().into_element(),
            })
        }))
        .child(framed(group().child(each(
            rows,
            |h: &store::HistoryEntry| (h.url.clone(), h.visited_at),
            move |h, _| {
                let entry = h.get_untracked();
                let url = entry.url.clone();
                let name = if entry.title.is_empty() { entry.url.clone() } else { entry.title };
                let host = url::Url::parse(&entry.url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_owned())).unwrap_or(entry.url);
                detail_row(Some(icons::GLOBE), name, Some(caption(host).number_of_lines(1)), move || {
                    ui.show(Screen::Browse);
                    ui.act(|s| s.navigate(&url));
                })
            },
        ))))
}

fn downloads(ui: Ui) -> impl IntoElement {
    let rows = move || ui.read(|s| s.downloads());
    view()
        .gap_2()
        .child(caption("Downloads are saved in the Files app under Tarn › Downloads and are never opened automatically.").px_1())
        .child(sheets::transfers(ui))
        .child(dynamic(move || rows().is_empty().then(|| empty_state("No downloads yet"))))
        .child(framed(group().child(each(
            rows,
            |d: &store::Download| d.id,
            move |d, _| {
                let d = d.get_untracked();
                let finished = d.state == "finished";
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .min_h(px(TOUCH_TARGET))
                    .px_3()
                    .border_b_1()
                    .border_color(Theme::separator)
                    .child(icon(icons::DOWNLOADS, move || if finished { Theme::success() } else { Theme::danger() }))
                    .child(view().flex_1().child(label(d.url.clone()).number_of_lines(1)).child(caption(if finished { "Saved" } else { "Failed" })))
            },
        ))))
}

fn spaces(ui: Ui) -> impl IntoElement {
    let name = signal(String::new());
    let rows = move || ui.read_data(|s| s.spaces());
    let current = move || ui.read(|s| s.model.space_ix);
    let add = move |profile: bool| {
        let n = name.get_untracked();
        if n.trim().is_empty() {
            return;
        }
        ui.act(|s| {
            if profile {
                s.new_profile(n.trim());
            } else {
                s.new_space(n.trim());
            }
            Vec::new()
        });
        name.set(String::new());
        ui.show(Screen::Browse);
    };
    view()
        .gap_2()
        .child(framed(group().child(each(
            rows,
            |s: &SpaceItem| (s.ix, s.private, s.name.clone()),
            move |item, _| {
                let item = item.get_untracked();
                let ix = item.ix;
                let name = if item.burner {
                    "Burner".to_owned()
                } else if item.private {
                    "Private".to_owned()
                } else {
                    format!("{} — {}", item.name, item.profile)
                };
                button()
                    .accessibility_label(SharedString::from(name.clone()))
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .min_h(px(TOUCH_TARGET))
                    .px_3()
                    .border_b_1()
                    .border_color(Theme::separator)
                    .bg(move || if current() == ix { Theme::selected() } else { Theme::clear() })
                    .selected(move || current() == ix)
                    .active(|s| s.bg(Theme::pressed))
                    .on_press(move || {
                        ui.act(|s| {
                            s.choose_space(ix, browser_core::now());
                            Vec::new()
                        });
                        ui.show(Screen::Browse);
                    })
                    .child(icon(
                        if item.private { icons::HAND } else { icons::PERSON },
                        move || if item.private { Theme::warning() } else { Theme::space_label() },
                    ))
                    .child(label(name).flex_1())
                    .child((!item.private).then(|| icon_button(icons::PENCIL, "Edit Schedule", move || ui.show(Screen::SpaceEdit(ix)))))
            },
        ))))
        .child(section("New"))
        .child(text_field("Name", "Name").value(move || name.get()).on_change(move |v: &str| name.set(v.to_owned())).test_id("Name"))
        .child(view().flex_row().gap_2().child(pill("Add Space", move || add(false))).child(pill("Add Profile", move || add(true))))
        .child(caption("A profile keeps its own cookies and website data; its spaces share them.").px_1())
        .child(dynamic(move || (!ui.read(|s| s.model.private())).then(|| sheets::snapshots(ui, name))))
}

fn find(ui: Ui) -> impl IntoElement {
    let query = signal(String::new());
    let search = move |backwards: bool| {
        let text = query.get_untracked();
        ui.act(|s| s.active_command("find", json!({ "text": text, "backwards": backwards })));
    };
    let count = move || ui.read(|s| s.found).filter(|_| !query.get().is_empty());
    let always = KeyOptions { when: When::Always, ..KeyOptions::default() };
    on_key_with("escape", always, move |_| ui.show(Screen::Browse));
    on_key_with("mod+g", always, move |_| search(false));
    on_key_with("shift+mod+g", always, move |_| search(true));
    surface(Theme::radius_panel())
        .flex_row()
        .items_center()
        .gap_1()
        .mx_2()
        .mb_1()
        .pl_3()
        .child(icon(icons::SEARCH, Theme::muted))
        .child(
            input()
                .flex_1()
                .auto_focus(true)
                .value(move || query.get())
                .on_change(move |v: &str| {
                    let typed = v.to_owned();
                    query.set(typed.clone());
                    ui.act_on_screen(|s| {
                        s.found = None;
                        Vec::new()
                    });
                    set_timeout(sheets::FIND_DELAY, move || {
                        if query.get_untracked() == typed {
                            search(false);
                        }
                    });
                })
                .on_submit(move |_: &str| search(false))
                .return_key_type(ReturnKeyType::Search)
                .placeholder("Find in page")
                .placeholder_color(Theme::faint)
                .label("Find")
                .h(px(TOUCH_TARGET))
                .px_2()
                .text_size(px(TEXT_TITLE))
                .text_color(Theme::text),
        )
        .child(
            small(move || match count() {
                Some(0) => "No matches".to_owned(),
                Some(1) => "1 match".to_owned(),
                Some(n) => format!("{n} matches"),
                None => String::new(),
            })
            .text_color(move || if count() == Some(0) { Theme::danger() } else { Theme::faint() }),
        )
        .child(icon_button(icons::BACK, "Previous", move || search(true)))
        .child(icon_button(icons::FORWARD, "Next", move || search(false)))
        .child(icon_button(icons::CLOSE, "Done", move || ui.show(Screen::Browse)))
}
