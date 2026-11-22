use browser_core::command::ENGINES;
use browser_core::layout::Layout;
use browser_core::settings::{SEARCH_SHORTCUTS, SEARCH_SHORTCUTS_NOTE};
use browser_core::tabs::TabKey;
use nsh_base::{alert_dialog, button, input};
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use serde_json::{Value, json};
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
use tarn_nsh_shared::icons as glyphs;
use tarn_nsh_shared::panes::{self, Pane};

const FIND_RADIUS: f32 = 12.;
const TAB_ICON: f32 = 20.;
const FIND_WIDTH: f32 = 420.;

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
        Screen::Extensions => "Extensions",
        other => sheets::heading_for(other).unwrap_or_default(),
    }
}

pub fn panel(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let screen = ui.screen.get();
        if ui.mac.get() && super::mac::owns(screen) {
            return None;
        }
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
            Screen::SpaceNew => sheets::space_card(ui, None).into_element(),
            Screen::SpaceEdit(ix) => sheets::space_card(ui, Some(ix)).into_element(),
            Screen::Licences => sheets::licences(ui).into_element(),
            Screen::Licence(ix) => sheets::licence(ix).into_element(),
            Screen::Extensions => super::extensions::pane(ui),
            Screen::ExtensionPopup => return Some(super::extensions::popup(ui).into_element()),
        };
        Some(sheet(ui, screen, body).into_element())
    })
}

pub(super) fn parent(screen: Screen) -> Option<Screen> {
    match screen {
        Screen::Move(_) => Some(Screen::Tabs),
        Screen::SpaceNew | Screen::SpaceEdit(_) => Some(Screen::Spaces),
        Screen::Licences => Some(Screen::Settings),
        Screen::Licence(_) => Some(Screen::Licences),
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
    let header = view().flex_row().items_center().pt_2();
    let header = match parent(screen) {
        Some(up) => {
            header.pl_1().child(tarn_nsh_shared::controls::icon_button(icon(icons::BACK, Theme::text), format!("Back to {}", title(up)), move || leave(ui, up)))
        }
        None => header.pl_3(),
    };
    surface(Theme::radius_card())
        .flex_1()
        .style_when(move || ui.wide(), |s| s.self_center().w(px(SHEET_WIDTH)))
        .mx_2()
        .mt_1()
        .mb_2()
        .overflow_hidden()
        .bind_motion(&tarn_nsh_shared::motion::pop_in())
        .child(dynamic(move || (!ui.wide()).then(|| view().items_center().pt_2().child(view().w(px(36.)).h(px(5.)).rounded_full().bg(Theme::faint)))))
        .child(
            header.child(view().flex_1().child(heading(name))).child(
                button()
                    .accessibility_label("Done")
                    .h(px(TOUCH_TARGET))
                    .px_3()
                    .justify_center()
                    .on_press(move || leave(ui, Screen::Browse))
                    .child(text("Done").text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::accent_text)),
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

/// A card of `panes` tabs side by side, as wide as that many cards and their gaps, within the sheet.
fn card_span(ui: Ui, panes: usize) -> Points {
    let card = card_width(ui).0;
    let n = panes as f32;
    px((card * n + GAP * (n - 1.0)).min(window().width.0.min(SHEET_WIDTH) - GAP * 4.0).max(card))
}

fn tabs(ui: Ui) -> impl IntoElement {
    let cards = move || {
        ui.read(|s| {
            let keys: Vec<TabKey> = s.model.tabs.iter().map(|t| s.model.key(t)).collect();
            let card = |key: TabKey| {
                let t = s.model.tab(key)?;
                let page = s.pages.get(&key);
                let url = page.map(|p| p.url.clone()).filter(|u| !u.is_empty()).unwrap_or(t.url.clone());
                Some((key, s.tab_title(key), url))
            };
            keys.iter()
                .filter_map(|k| s.splits.entry(*k, &keys))
                .map(|panes| panes.into_iter().filter_map(card).collect())
                .collect::<Vec<Vec<(TabKey, String, String)>>>()
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
            |c: &Vec<(TabKey, String, String)>| c.iter().map(|p| p.0).collect::<Vec<_>>(),
            move |card, _| {
                let panes = card.get_untracked();
                let keys: Vec<TabKey> = panes.iter().map(|p| p.0).collect();
                let count = panes.len();
                let current = move || ui.read(|s| s.model.active.is_some_and(|a| keys.contains(&a)));
                view()
                    .w(move || card_span(ui, count))
                    .rounded(px(Theme::radius_tile()))
                    .bg(Theme::surface)
                    .border_2()
                    .border_color(move || if current() { Theme::accent() } else { Theme::surface_edge() })
                    .shadow(Theme::elevation(2., 6.))
                    .child(view().flex_row().children(panes.into_iter().map(|(key, title, url)| tab_card(ui, key, title, url).flex_1().min_w_0())))
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

fn tab_card(ui: Ui, key: TabKey, title: String, url: String) -> View {
    let current = move || ui.read(|s| s.model.active == Some(key));
    let movable = move || ui.read(|s| !s.in_burner() && !s.move_targets(key).is_empty());
    view()
        .child(
            view()
                .flex_row()
                .items_center()
                .pl_3()
                .child(dynamic(move || ui.read(|s| s.in_burner()).then(|| icon(icons::FLAME, Secure::warning))))
                .child(dynamic({
                    let url = url.clone();
                    move || {
                        let host = url::Url::parse(&url).ok().and_then(|u| browser_core::command::page_host(&u)).unwrap_or_default();
                        tarn_nsh_shared::controls::site_tile(ui.read(|s| s.site_icon(&url)), &host, TAB_ICON).mr_2()
                    }
                }))
                .child(
                    label(title.clone())
                        .font_weight(FontWeight::SEMIBOLD)
                        .number_of_lines(1)
                        .flex_1()
                        .text_color(move || if ui.read(|s| s.model.lifecycle.live().any(|l| l == key)) { Theme::text() } else { Theme::asleep() }),
                )
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
}

fn menu(ui: Ui) -> impl IntoElement {
    let private = move || ui.read(|s| s.model.private());
    let zoom = move |delta: f64| ui.act(|s| s.zoom_by(delta));
    let level = computed(move || ui.read(|s| (s.active_page().map_or(1.0, |p| p.zoom) * 100.).round() as i64));
    let shareable = computed(move || ui.read(|s| s.share_url().is_some()));
    let tab = computed(move || ui.read(|s| s.model.active.is_some()));
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
                .child(row(icons::FOLDER_OPEN, "Open File…", move || open_file(ui)))
                .child(
                    when(private, move || {
                        row(icons::HAND, "Close Private Space", move || {
                            ui.act(|s| {
                                s.close_private();
                                Vec::new()
                            });
                            ui.show(Screen::Browse);
                        })
                    })
                    .otherwise(move || {
                        row(icons::HAND, "Private Space", move || {
                            ui.act(|s| {
                                s.enter_private();
                                Vec::new()
                            });
                            ui.show(Screen::Browse);
                        })
                    }),
                )
                .child(when(
                    move || !ui.read(|s| s.in_burner() || <crate::data::Data as crate::data::Platform>::private_process()),
                    move || {
                        row(icons::FLAME, "New Burner Tab", move || {
                            ui.act(|s| {
                                s.open_burner();
                                Vec::new()
                            });
                            ui.show(Screen::Browse);
                        })
                    },
                ))
                .child(when(
                    move || ui.read(|s| !s.model.closed_tabs.is_empty()),
                    move || {
                        row(icons::UNDO, "Reopen Closed Tab", move || {
                            ui.act(|s| {
                                if !s.reopen_closed() {
                                    s.notice = Some("No closed tab to reopen.".into());
                                }
                                Vec::new()
                            });
                            ui.show(Screen::Browse);
                        })
                    },
                )),
        ))
        .child(framed(
            group()
                .child(dynamic(move || tab.get().then(|| row(icons::SEARCH, "Find in Page", move || ui.show(Screen::Find)))))
                .child(row(icons::APP_WINDOW, "Find in Tabs", move || ui.show(Screen::FindTabs)))
                .child(when(move || ui.read(|s| s.offers_reader()), move || row(icons::BOOK_OPEN, "Reader", move || ui.act(|s| s.open_reader()))))
                .child(when(
                    move || tab.get(),
                    move || {
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
                            .child(pill("+", move || zoom(0.1)).accessibility_label("Zoom In"))
                    },
                ))
                .child(row(icons::SHIELD, "Shield and Settings for This Site", move || ui.show(Screen::Shield)))
                .child(when(
                    move || ui.read(|s| s.zappable()),
                    move || {
                        row(icons::CLOSE, "Zap Element", move || {
                            ui.show(Screen::Browse);
                            ui.act(|s| s.pick());
                        })
                    },
                ))
                .child(when(
                    move || ui.read(|s| s.offers_capture()),
                    move || {
                        group()
                            .child(row(icons::CAMERA, "Take Screenshot", move || {
                                ui.show(Screen::Browse);
                                ui.act(|s| s.take_screenshot());
                            }))
                            .child(row(icons::CAMERA, "Capture Area", move || super::capture::capture_area(ui)))
                            .child(row(icons::CAMERA, "Capture Element", move || super::capture::capture_element(ui)))
                            .child(row(icons::CAMERA, "Capture Full Page", move || {
                                ui.show(Screen::Browse);
                                ui.act(|s| s.capture_full_page());
                            }))
                    },
                )),
        ))
        .child(framed(
            group()
                .child(row(icons::HISTORY, "History", move || ui.show(Screen::History)))
                .child(row(icons::DOWNLOADS, "Downloads", move || ui.show(Screen::Downloads)))
                .child(row(icons::PERSON, "Spaces and Profiles", move || ui.show(Screen::Spaces)))
                .child(row(icons::GEAR, "Settings", move || ui.show(Screen::Settings)))
                .child(when(move || !private(), move || row(icons::APP_WINDOW, "Extensions", move || ui.show(Screen::Extensions)))),
        ))
        .child(super::extensions::actions(ui))
        .child(framed(
            group()
                .child(dynamic(move || {
                    shareable.get().then(|| {
                        view()
                            .child(row(icons::SHARE, "Share…", move || ui.show(Screen::Share)))
                            .child(row(icons::SMARTPHONE, "Show QR Code", move || ui.show(Screen::Qr)))
                            .child(row(icons::CHECK, "Copy Clean Link", move || {
                                ui.copy_link(None, true);
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

pub(super) fn default_levels(ui: Ui) -> View {
    levels(
        move || Some(ui.read_data(|s| s.default_level())),
        move |level| {
            let Some(level) = level else { return };
            let set = move || {
                ui.act(|s| {
                    s.set_default_level(level);
                    Vec::new()
                })
            };
            match ui.read(|s| s.shield_wipes_data(level)) {
                Some(profile) => confirm(browser_core::profiles::lockdown_question(&profile), browser_core::profiles::LOCKDOWN_WIPE, "Use Lockdown", set),
                None => set(),
            }
        },
        false,
    )
}

fn levels(current: impl Fn() -> Option<ShieldLevel> + Copy + 'static, set: impl Fn(Option<ShieldLevel>) + Copy + 'static, with_default: bool) -> View {
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

pub(super) fn open_file(ui: Ui) {
    ui.show(Screen::Browse);
    ui.root.run(|| {
        drop(spawn(async move {
            let Some(file) = pick_files(nsh_core::picker::FilePickRequest::new(false, 1)).await.into_iter().next() else { return };
            let staged = browser_core::open_file::stage(&ui.read(|s| s.opened_dir()), &file.name, file.data.bytes());
            ui.act(|s| {
                match staged {
                    Ok(url) => s.open_file(&url),
                    Err(e) => s.notice = Some(format!("The file could not be opened: {e}")),
                }
                Vec::new()
            });
        }))
    });
}

pub fn share(ui: Ui, url: String) {
    spawn(async move {
        if !crate::platform::share(url.clone()).await {
            set_clipboard(url);
            ui.act(|s| {
                s.toast_as(browser_core::toast::Tone::Success, "Link copied".into());
                Vec::new()
            });
        }
    });
}

fn layout_choice(ui: Ui) -> impl IntoElement {
    when(
        move || {
            let size = window();
            browser_core::layout::offered(size.width.0, size.height.0)
        },
        move || {
            view()
                .gap_2()
                .child(section("Layout"))
                .child(segmented(
                    Layout::ALL
                        .into_iter()
                        .map(|layout| {
                            (
                                layout.label().to_owned(),
                                Box::new(move || ui.read_data(|s| s.layout) == layout) as Box<dyn Fn() -> bool>,
                                Box::new(move || ui.act(|s| s.set_layout(layout))) as Box<dyn Fn()>,
                            )
                        })
                        .collect(),
                ))
                .child(
                    caption("Mac arranges Tarn like the Mac app, with a sidebar, a toolbar and desktop-sized controls for a pointer and keyboard.")
                        .number_of_lines(0)
                        .px_1(),
                )
        },
    )
}

fn pane_heading(pane: Pane) -> impl IntoElement {
    label(panes::name(pane)).text_size(px(TEXT_TITLE)).font_weight(FontWeight::BOLD).px_1().pt_4()
}

type Part = (Box<dyn Fn(&str) -> bool>, Box<dyn Fn() -> AnyElement>);

pub(super) fn settings(ui: Ui) -> impl IntoElement {
    let mut parts: Vec<Part> = TOUCH_PANES
        .iter()
        .map(|&pane| -> Part {
            (
                Box::new(move |q: &str| panes::matches(pane, q)),
                Box::new(move || view().gap_2().child(pane_heading(pane)).children(pane_body(ui, pane)).into_element()),
            )
        })
        .collect();
    parts.push((
        Box::new(|q: &str| browser_core::settings::words_match(q, &format!("{} {}", panes::EXTENSIONS.0, panes::EXTENSIONS.1))),
        Box::new(move || {
            view()
                .gap_2()
                .child(section("Extensions"))
                .child(framed(group().child(row(icons::APP_WINDOW, "Extensions", move || ui.show(Screen::Extensions)))))
                .into_element()
        }),
    ));
    parts.push((Box::new(|q: &str| panes::matches(Pane::Privacy, q)), Box::new(move || your_data(ui).into_element())));
    let (matchers, builders): (Vec<_>, Vec<_>) = parts.into_iter().unzip();
    let shown: Vec<Signal<bool>> = matchers.iter().map(|_| signal(true)).collect();
    let marks = shown.clone();
    let search = move |q: &str| {
        for (matches, on) in matchers.iter().zip(&marks) {
            if on.get_untracked() != matches(q) {
                on.set(matches(q));
            }
        }
    };
    let blocks: Vec<AnyElement> = builders
        .into_iter()
        .zip(&shown)
        .map(|(build, on)| {
            let on = *on;
            dynamic(move || on.get().then(&build)).into_element()
        })
        .collect();
    view()
        .gap_2()
        .child(text_field("Search settings", "Search settings").on_change(search).auto_capitalize(AutoCapitalize::None))
        .child(dynamic(move || shown.iter().all(|on| !on.get()).then(|| empty_state("No settings match."))))
        .child(blocks)
}

const TOUCH_PANES: [Pane; 6] = [Pane::Profile, Pane::General, Pane::Appearance, Pane::Search, Pane::Privacy, Pane::Permissions];

pub(super) fn pane_body(ui: Ui, pane: Pane) -> Option<View> {
    let body = view().gap_2();
    Some(match pane {
        Pane::Profile => body.child(sheets::shield_card(ui)),
        Pane::General => body
            .child(section("Downloads"))
        .child(framed(group().child(switch_row("Sort downloads into folders", move || ui.read_data(|s| s.setting_on(crate::scripts::SORT_DOWNLOADS)), move || {
            ui.act(|s| {
                let on = s.setting_on(crate::scripts::SORT_DOWNLOADS);
                s.set_setting_on(crate::scripts::SORT_DOWNLOADS, !on)
            })
        }))))
        .child(section("About"))
        .child(view().flex_row().flex_wrap().gap_2().child(link_pill(ui, browser_core::LICENCE, browser_core::LICENCE_URL)).child(link_pill(ui, "Source code", browser_core::SOURCE_URL)).child(link_pill(ui, "Privacy notice", browser_core::PRIVACY_URL)).child(link_pill(ui, "Imprint", browser_core::IMPRINT_URL)).child(pill("Open-source licences", move || ui.show(Screen::Licences)))),
        Pane::Appearance => body
            .child(layout_choice(ui))
        .child(super::appearance::pane(ui)),
        Pane::Search => body
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
        .child(section("Shortcuts"))
        .child(framed(group().child(switch_row("Search shortcuts", move || ui.read_data(|s| s.search_shortcuts()), move || {
            ui.act(|s| {
                let on = s.search_shortcuts();
                s.set_setting_on(SEARCH_SHORTCUTS, !on)
            })
        }))))
        .child(caption(SEARCH_SHORTCUTS_NOTE).number_of_lines(0).px_1()),
        Pane::Privacy => body
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
        .child(caption("No filter lists are installed yet; Shield's built-in rules always apply. Cookie banners uses EasyList Cookie and uBlock Cookie Notices once installed; nothing is accepted or refused for you.").number_of_lines(0).px_1()),
        Pane::Permissions => body.child(permissions(ui)),
        Pane::Profiles => body.child(spaces(ui)),
        Pane::Tabs | Pane::Developer | Pane::Shortcuts => return None,
    })
}

pub(super) fn your_data(ui: Ui) -> View {
    view().gap_2().child(section("Your data")).child(framed(group().child(row(icons::TRASH, "Clear Browsing Data…", move || {
        confirm("Clear browsing data?", "History, downloads list, closed tabs and this profile's website data are removed.", "Clear", move || {
            ui.act(|s| {
                s.clear_data();
                Vec::new()
            })
        })
    }))))
}

pub(super) use tarn_nsh_shared::controls::confirm;

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
        .child(dynamic(move || {
            let now = browser_core::now();
            let starts = ui.read(|s| s.day_starts(now));
            let mut days: Vec<(&'static str, Vec<store::HistoryEntry>)> = Vec::new();
            for h in rows() {
                let day = browser_core::library::history_day(h.visited_at, starts);
                match days.last_mut() {
                    Some((last, list)) if *last == day => list.push(h),
                    _ => days.push((day, vec![h])),
                }
            }
            let remove = move |url: String| {
                move || {
                    ui.act(|s| {
                        s.remove_history_url(&url);
                        Vec::new()
                    });
                    let q = query.get_untracked();
                    lookup(&q, browser_core::suggest::searches_text(q.trim()));
                }
            };
            Some(view().gap_2().children(days.into_iter().map(|(day, list)| {
                view().gap_1().child(section(day)).child(framed(group().children(list.into_iter().map(|entry| {
                    let url = entry.url.clone();
                    let name = browser_core::command::display_title("", &entry.title, &url);
                    let when = ui.read(|s| browser_core::library::ago(s.store(), entry.visited_at, now));
                    let host =
                        url::Url::parse(&entry.url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_owned())).unwrap_or(entry.url);
                    let open = url.clone();
                    view()
                        .flex_row()
                        .items_center()
                        .child(
                            detail_row(Some(icons::GLOBE), name, Some(caption(format!("{when} · {host}")).number_of_lines(1).into_element()), move || {
                                ui.show(Screen::Browse);
                                ui.act(|s| s.navigate(&open));
                            })
                            .flex_1(),
                        )
                        .child(icon_button(icons::TRASH, "Remove from History", remove(url)))
                }))))
            })))
        }))
        .child(dynamic(move || {
            (!found.with(Vec::is_empty) || !query.get().trim().is_empty()).then(|| {
                framed(group().child(row(icons::TRASH, "Clear History…", move || {
                    clear_history(ui, move || {
                        let q = query.get_untracked();
                        lookup(&q, false);
                    })
                })))
            })
        }))
}

fn clear_history(ui: Ui, then: impl FnOnce() + 'static) {
    use browser_core::library::HistoryRange;
    let mut dialog = alert_dialog()
        .title("Clear history?")
        .description("Visits in the chosen range are removed from this profile's history.")
        .action_sheet()
        .cancel("cancel", "Cancel");
    for (ix, range) in HistoryRange::ALL.iter().enumerate() {
        dialog = dialog.destructive(ix.to_string(), range.label());
    }
    let answer = dialog.show();
    spawn(async move {
        let picked = answer.await.ok().and_then(|a| a.action).and_then(|id| id.parse::<usize>().ok()).and_then(|ix| HistoryRange::ALL.get(ix).copied());
        if let Some(range) = picked {
            ui.act(|s| {
                s.clear_history(range);
                Vec::new()
            });
            then();
        }
    });
}

fn downloads(ui: Ui) -> impl IntoElement {
    let rows = move || ui.read(|s| s.downloads());
    view()
        .gap_2()
        .child(caption("Downloads are saved in the Files app under Tarn › Downloads and are never opened automatically.").px_1())
        .child(sheets::transfers(ui))
        .child(dynamic(move || rows().is_empty().then(|| empty_state("No downloads yet"))))
        .child(dynamic(move || {
            (!rows().is_empty()).then(|| {
                view().flex_row().items_center().px_1().child(section("Finished").flex_1()).child(
                    button()
                        .accessibility_label("Clear Downloads List")
                        .min_h(px(TOUCH_TARGET))
                        .px_2()
                        .justify_center()
                        .on_press(move || {
                            ui.act(|s| {
                                s.clear_downloads();
                                Vec::new()
                            })
                        })
                        .child(text("Clear").text_size(px(TEXT_TITLE)).text_color(Theme::accent_text)),
                )
            })
        }))
        .child(framed(group().child(each(
            rows,
            |d: &store::Download| d.id,
            move |d, _| {
                let d = d.get_untracked();
                let (status, failed) = match d.state.as_str() {
                    "finished" => ("Saved", false),
                    "cancelled" => ("Cancelled", false),
                    _ => ("Failed", true),
                };
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .min_h(px(TOUCH_TARGET))
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(Theme::separator)
                    .child(sheets::file_tile(&d.url, false))
                    .child(
                        view()
                            .flex_1()
                            .child(label(d.url.clone()).number_of_lines(1))
                            .child(caption(status).text_color(move || if failed { Theme::danger() } else { Theme::muted() })),
                    )
            },
        ))))
}

fn spaces(ui: Ui) -> impl IntoElement {
    let rows = move || ui.read_data(|s| s.spaces());
    let current = move || ui.read(|s| s.model.space_ix);
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
                        tarn_nsh_shared::motion::space_switch();
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
                    .child(icon_button(icons::PENCIL, "Edit Space", move || ui.show(Screen::SpaceEdit(ix))))
            },
        ))))
        .child(ui.read(|s| s.can_add_space()).then(|| framed(group().child(row(icons::PLUS, "New Space", move || ui.show(Screen::SpaceNew))))))
        .child(caption("A profile keeps its own cookies and website data; its spaces share them.").px_1())
        .child(dynamic(move || (!ui.read(|s| s.model.private())).then(|| sheets::snapshots(ui))))
}

fn find(ui: Ui) -> impl IntoElement {
    let query = signal(String::new());
    let search = move |backwards: bool| {
        let text = query.get_untracked();
        ui.act(|s| s.active_command("find", json!({ "text": text, "backwards": backwards })));
    };
    let count = move || ui.read(|s| s.found).filter(|_| !query.get().is_empty());
    let always = KeyOptions { when: When::Always, ..KeyOptions::default() };
    on_key_with("mod+g", always, move |_| search(false));
    on_key_with("shift+mod+g", always, move |_| search(true));
    let shown = tarn_nsh_shared::motion::pop_in();
    let close = move || {
        ui.act_on_screen(|s| s.active_command("clearFind", Value::Null));
        tarn_nsh_shared::motion::pop_out(shown, 1., move || ui.show(Screen::Browse));
    };
    on_key_with("escape", always, move |_| close());
    surface(FIND_RADIUS)
        .absolute()
        .top(px(8.))
        .right(px(8.))
        .style_with(move |s| if ui.wide() { s.w(px(FIND_WIDTH)) } else { s.left(px(8.)) })
        .z_index(2)
        .flex_row()
        .items_center()
        .pl_3()
        .shadow(Theme::elevation(4., 14.))
        .bind_motion(&shown)
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
                        if typed.is_empty() { s.active_command("clearFind", Value::Null) } else { Vec::new() }
                    });
                    set_timeout(browser_core::find::TYPING_DELAY, move || {
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
            small(move || count().map_or_else(String::new, |(n, at)| browser_core::find::count_label(Some(n), at)))
                .text_color(move || if count().is_some_and(|(n, _)| n == 0) { Theme::danger() } else { Theme::faint() }),
        )
        .child(view().w(px(1.)).h(px(20.)).mx_1().bg(Theme::separator))
        .child(tarn_nsh_shared::controls::icon_button(find_glyph(glyphs::CHEVRON_UP), "Previous", move || search(true)))
        .child(tarn_nsh_shared::controls::icon_button(find_glyph(glyphs::CHEVRON_DOWN), "Next", move || search(false)))
        .child(icon_button(icons::CLOSE, "Done", close))
}

fn find_glyph(source: &'static str) -> View {
    view().text_color(Theme::text).child(glyphs::icon(source, crate::theme::ICON))
}
