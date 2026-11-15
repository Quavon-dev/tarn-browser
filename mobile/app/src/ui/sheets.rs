use std::time::Duration;

use browser_core::shield_card::{Card, H, Label, W, back, face, front, thousands, week_rows};
use browser_core::tabs::TabKey;
use engine::reader::Block;
use nsh_base::{button, input};
use nsh_core::prelude::*;
use store::Schedule;
use tarn_nsh_shared::card::{Words, picture, week_bars};

use super::{Screen, Ui, accent_pill, caption, icon_button, icons, label, pill, segmented, small, surface, switch_row};
use crate::scripts::{FindRow, Transfer, TransferState};
use crate::theme::{TEXT_HEADING, TEXT_TITLE, TOUCH_TARGET, Theme};

const QR_SIZE: f32 = 240.;
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn section(name: &'static str) -> impl IntoElement {
    label(name).font_weight(FontWeight::SEMIBOLD).px_1().pt_3()
}

fn framed(rows: View) -> View {
    tarn_nsh_shared::controls::group().overflow_hidden().child(rows)
}

fn field(value: Signal<String>, name: &'static str) -> impl IntoElement {
    input()
        .value(move || value.get())
        .on_change(move |v: &str| value.set(v.to_owned()))
        .auto_capitalize(AutoCapitalize::None)
        .placeholder(name)
        .placeholder_color(Theme::faint)
        .label(name)
        .test_id(name)
        .h(px(TOUCH_TARGET))
        .px_3()
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised)
        .border_1()
        .border_color(Theme::hairline)
        .text_size(px(TEXT_TITLE))
        .text_color(Theme::text)
}

fn row_button(name: String, detail: Option<String>, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    button()
        .accessibility_label(SharedString::from(name.clone()))
        .flex_row()
        .items_center()
        .gap_3()
        .min_h(px(TOUCH_TARGET))
        .px_3()
        .py_1()
        .border_b_1()
        .border_color(Theme::separator)
        .active(|s| s.bg(Theme::pressed))
        .on_press(press)
        .child(view().flex_1().child(label(name).number_of_lines(1)).child(detail.map(|d| caption(d).number_of_lines(2))))
}

pub fn reader(ui: Ui) -> impl IntoElement {
    let size = signal(TEXT_TITLE + 2.);
    let speaking = signal(false);
    cleanup(crate::platform::stop_speaking);
    let article = move || ui.read(|s| s.reader.clone());
    view()
        .gap_3()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(pill("A−", move || size.update(|s| *s = (*s - 2.).max(TEXT_TITLE - 2.))).accessibility_label("Smaller Text"))
                .child(pill("A+", move || size.update(|s| *s = (*s + 2.).min(TEXT_TITLE + 12.))).accessibility_label("Larger Text"))
                .child(dynamic(move || {
                    if speaking.get() {
                        accent_pill("Stop Reading", move || {
                            crate::platform::stop_speaking();
                            speaking.set(false);
                        })
                    } else {
                        pill("Read Aloud", move || {
                            if let Some(reader) = article() {
                                spawn(async move {
                                    let started = crate::platform::speak(reader.spoken()).await;
                                    speaking.set(started);
                                    if !started {
                                        ui.act(|s| {
                                            s.notice = Some("Read Aloud needs an on-device voice for this language.".into());
                                            Vec::new()
                                        });
                                    }
                                });
                            }
                        })
                    }
                }))
                .child(pill("Save for Later", move || {
                    ui.act(|s| {
                        s.save_for_later(browser_core::now());
                        Vec::new()
                    })
                })),
        )
        .child(dynamic(move || {
            article().map(|reader| {
                let body = size.get();
                view()
                    .gap_3()
                    .px_1()
                    .child(text(reader.article.title.clone()).text_size(px(TEXT_HEADING + 4.)).font_weight(FontWeight::BOLD).text_color(Theme::text))
                    .child((!reader.article.byline.is_empty()).then(|| caption(reader.article.byline.clone())))
                    .children(
                        reader
                            .article
                            .blocks
                            .iter()
                            .map(|b| match b {
                                Block::Heading(t) => {
                                    text(t.clone()).text_size(px(body + 3.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text).into_element()
                                }
                                Block::Paragraph(t) => text(t.clone()).text_size(px(body)).line_height(px(body * 1.5)).text_color(Theme::text).into_element(),
                            })
                            .collect::<Vec<_>>(),
                    )
            })
        }))
}

pub fn find_tabs(ui: Ui) -> impl IntoElement {
    let query = signal(ui.read(|s| s.find_tabs.query.clone()));
    let rows = move || ui.read(|s| s.find_tabs.rows.clone());
    view()
        .gap_2()
        .child(
            input()
                .auto_focus(true)
                .value(move || query.get())
                .on_change(move |v: &str| {
                    let typed = v.to_owned();
                    query.set(typed.clone());
                    set_timeout(Duration::from_millis(150), move || {
                        if query.get_untracked() == typed {
                            ui.act(|s| s.find_in_tabs(&typed));
                        }
                    });
                })
                .return_key_type(ReturnKeyType::Search)
                .placeholder("Find in this space's tabs")
                .placeholder_color(Theme::faint)
                .label("Find in Tabs")
                .test_id("find-tabs")
                .h(px(TOUCH_TARGET))
                .px_3()
                .rounded(px(Theme::radius_row()))
                .bg(Theme::raised)
                .border_1()
                .border_color(Theme::hairline)
                .text_size(px(TEXT_TITLE))
                .text_color(Theme::text),
        )
        .child(framed(view().child(each(
            rows,
            |r: &FindRow| (r.key, r.snippet.clone()),
            move |r, _| {
                let row = r.get_untracked();
                let key = row.key;
                let detail = Some(row.snippet.clone().unwrap_or_else(|| "Title or address".into()));
                row_button(row.title, detail, move || {
                    let q = query.get_untracked();
                    ui.act(|s| {
                        s.activate(key);
                        s.found = None;
                        s.active_command("find", serde_json::json!({ "text": q, "backwards": false }))
                    });
                    ui.show(Screen::Find);
                })
            },
        ))))
}

fn qr_painter(code: &[Vec<bool>]) -> Painter {
    let module = QR_SIZE / code.len().max(1) as f32;
    let mut painter = Painter::new().fill_color(Theme::text()).fill_rect(0., 0., QR_SIZE, QR_SIZE).fill_color(Theme::page());
    for (y, row) in code.iter().enumerate() {
        for (x, dark) in row.iter().enumerate() {
            if *dark {
                painter = painter.fill_rect(x as f32 * module, y as f32 * module, module + 0.5, module + 0.5);
            }
        }
    }
    painter
}

pub fn qr(ui: Ui) -> impl IntoElement {
    let code = ui.read(|s| s.qr_code());
    view().gap_3().items_center().child(match code {
        Some((code, host)) => view()
            .gap_3()
            .items_center()
            .child(
                view()
                    .p_3()
                    .rounded(px(Theme::radius_panel()))
                    .bg(Theme::text)
                    .accessible(true)
                    .label("QR code")
                    .child(canvas(qr_painter(&code)).w(px(QR_SIZE)).h(px(QR_SIZE))),
            )
            .child(label(host).number_of_lines(1))
            .child(caption("Scan with another device's camera to open this page."))
            .into_element(),
        None => caption("Open a web page to show its QR code.").into_element(),
    })
}

pub fn reading_list(ui: Ui) -> impl IntoElement {
    let rows = move || ui.read(|s| s.reading_list());
    view().gap_2().child(caption("Pages you saved for later, kept as bookmarks in the Reading List folder.").px_1()).child(framed(view().child(each(
        rows,
        |b: &store::Bookmark| b.id,
        move |b, _| {
            let b = b.get_untracked();
            let (id, url) = (b.id, b.url.clone().unwrap_or_default());
            view()
                .flex_row()
                .items_center()
                .child(
                    row_button(b.title.clone(), Some(url.clone()), move || {
                        ui.show(Screen::Browse);
                        ui.act(|s| s.navigate(&url));
                    })
                    .flex_1(),
                )
                .child(icon_button(icons::TRASH, "Remove", move || {
                    ui.act(|s| {
                        s.remove_from_reading_list(id);
                        Vec::new()
                    })
                }))
        },
    ))))
}

pub fn site(ui: Ui) -> impl IntoElement {
    let css = signal(ui.read(|s| s.site_css()));
    // The field follows the page to another host, so a save never lands on the wrong site.
    let css_host = std::cell::RefCell::new(ui.read(|s| s.active_host()));
    effect(move || {
        let host = ui.read(|s| s.active_host());
        if host.is_some() && *css_host.borrow() != host {
            css_host.replace(host);
            css.set(untracked(|| ui.read(|s| s.site_css())));
        }
    });
    let has_site = move || ui.read(|s| s.active_host().is_some());
    view().gap_2().child(dynamic(move || {
        has_site().then(|| {
            view()
                .gap_2()
                .child(section("Appearance"))
                .child(framed(view().child(switch_row(
                    "Dark mode for this site",
                    move || ui.read(|s| s.site_dark()),
                    move || {
                        ui.act(|s| {
                            s.set_site_dark(!s.site_dark());
                            Vec::new()
                        })
                    },
                ))))
                .child(caption("Inverts the page's colours for sites without a dark mode of their own. Off under Lockdown.").px_1())
                .child(section("Site CSS"))
                .child(field(css, "Site CSS"))
                .child(view().flex_row().gap_2().child(pill("Save CSS", move || {
                    let value = css.get_untracked();
                    ui.act(|s| {
                        if s.set_site_css(&value) {
                            s.notice = Some("Site CSS saved".into());
                        }
                        Vec::new()
                    })
                })))
                .child(section("Hidden elements"))
                .child(label(move || match ui.read(|s| s.site_zaps()) {
                    0 => "No hidden elements on this site.".to_owned(),
                    1 => "1 element hidden on this site.".to_owned(),
                    n => format!("{n} elements hidden on this site."),
                }))
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(pill("Zap Element", move || {
                            ui.show(Screen::Browse);
                            ui.act(|s| s.pick());
                        }))
                        .child(dynamic(move || {
                            (ui.read(|s| s.site_zaps()) > 0).then(|| {
                                pill("Show All", move || {
                                    ui.act(|s| {
                                        s.clear_site_zaps();
                                        Vec::new()
                                    })
                                })
                            })
                        })),
                )
                .child(dynamic(move || {
                    ui.read(|s| s.site_rule()).map(|rule| {
                        let mut options: Vec<tarn_nsh_shared::controls::Segment> = vec![(
                            "Any space".to_owned(),
                            Box::new(move || ui.read(|s| s.site_rule().and_then(|r| r.space).is_none())),
                            Box::new(move || {
                                ui.act(|s| {
                                    s.set_site_rule(None);
                                    Vec::new()
                                })
                            }),
                        )];
                        options.extend(rule.choices.into_iter().map(|(ix, name)| {
                            (
                                name,
                                Box::new(move || ui.read(|s| s.site_rule().and_then(|r| r.space) == Some(ix))) as Box<dyn Fn() -> bool>,
                                Box::new(move || {
                                    ui.act(|s| {
                                        s.set_site_rule(Some(ix));
                                        Vec::new()
                                    })
                                }) as Box<dyn Fn()>,
                            )
                        }));
                        view().gap_2().child(section("Open new tabs of this site in")).child(segmented(options))
                    })
                }))
        })
    }))
}

pub fn move_tab(ui: Ui, key: TabKey) -> impl IntoElement {
    let targets = ui.read(|s| s.move_targets(key));
    view()
        .gap_2()
        .child(caption(if targets.is_empty() { "No other space of this profile. Add one under Spaces and Profiles." } else { "Move this tab to:" }).px_1())
        .child(framed(
            view().children(
                targets
                    .into_iter()
                    .map(|(ix, name)| {
                        row_button(name, None, move || {
                            ui.act(|s| {
                                s.move_tab(key, ix);
                                Vec::new()
                            });
                            ui.show(Screen::Tabs);
                        })
                        .into_element()
                    })
                    .collect::<Vec<_>>(),
            ),
        ))
}

fn hour_name(hour: u8) -> String {
    format!("{hour:02}:00")
}

pub fn space_edit(ui: Ui, ix: usize) -> impl IntoElement {
    let schedule = signal(ui.read(|s| s.schedule(ix)));
    let save = move |next: Schedule| {
        schedule.set(next);
        ui.act(|s| {
            s.set_schedule(ix, next);
            Vec::new()
        });
    };
    let name = ui.read(|s| s.model.spaces.get(ix).map(|s| s.space.name.clone()).unwrap_or_default());
    let days: Vec<tarn_nsh_shared::controls::Segment> = (0..7u8)
        .map(|day| {
            let bit = 1u8 << day;
            (
                WEEKDAYS[day as usize].to_owned(),
                Box::new(move || schedule.get().days & bit != 0) as Box<dyn Fn() -> bool>,
                Box::new(move || {
                    let s = schedule.get_untracked();
                    save(Schedule { days: s.days ^ bit, ..s });
                }) as Box<dyn Fn()>,
            )
        })
        .collect();
    let stepper = move |name: &'static str, start: bool| {
        let value = move || {
            let s = schedule.get();
            if start { s.start } else { s.end }
        };
        let step = move |delta: i8| {
            let s = schedule.get_untracked();
            let next = |h: u8| (h as i8 + delta).rem_euclid(24) as u8;
            save(if start { Schedule { start: next(s.start), ..s } } else { Schedule { end: next(s.end), ..s } });
        };
        view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(label(name).w(px(56.)))
            .child(pill("‹", move || step(-1)).accessibility_label(format!("Earlier {name}")))
            .child(label(move || hour_name(value())).w(px(64.)))
            .child(pill("›", move || step(1)).accessibility_label(format!("Later {name}")))
    };
    view()
        .gap_2()
        .child(label(format!("Schedule for {name}")).font_weight(FontWeight::SEMIBOLD).px_1())
        .child(
            caption(
                "Tarn switches to this space during these hours, unless you changed space yourself within the last hour. No day selected means no schedule.",
            )
            .px_1(),
        )
        .child(segmented(days))
        .child(stepper("From", true))
        .child(stepper("To", false))
        .child(
            caption(move || {
                let s = schedule.get();
                if s.days == 0 {
                    "No schedule".to_owned()
                } else if s.end <= s.start {
                    format!("{}–{} runs past midnight.", hour_name(s.start), hour_name(s.end))
                } else {
                    format!("{}–{}", hour_name(s.start), hour_name(s.end))
                }
            })
            .px_1(),
        )
}

pub fn snapshots(ui: Ui, name: Signal<String>) -> impl IntoElement {
    let rows = move || ui.read(|s| s.snapshots());
    let none = move || rows().is_empty();
    view()
        .gap_2()
        .child(section("Snapshots"))
        .child(pill("Save Snapshot of This Space", move || {
            let n = name.get_untracked();
            let n =
                if n.trim().is_empty() { ui.read(|s| s.model.current_space().map(|c| c.space.name.clone()).unwrap_or_default()) } else { n.trim().to_owned() };
            ui.act(|s| {
                s.save_snapshot(&n, browser_core::now());
                Vec::new()
            });
            name.set(String::new());
        }))
        .child(
            framed(view().child(each(
                rows,
                |s: &store::Snapshot| s.id,
                move |snap, _| {
                    let snap = snap.get_untracked();
                    let restore = snap.clone();
                    let remove = snap.clone();
                    view()
                        .flex_row()
                        .items_center()
                        .child(
                            row_button(snap.name.clone(), Some("Restore in a new space".into()), move || {
                                ui.act(|s| {
                                    s.restore_snapshot(&restore);
                                    Vec::new()
                                });
                                ui.show(Screen::Browse);
                            })
                            .flex_1(),
                        )
                        .child(icon_button(icons::TRASH, "Delete Snapshot", move || {
                            ui.act(|s| {
                                s.delete_snapshot(&remove);
                                Vec::new()
                            })
                        }))
                },
            )))
            .style_when(none, |s| s.hidden()),
        )
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.)
}

pub fn transfers(ui: Ui) -> impl IntoElement {
    let rows = move || ui.read(|s| s.transfers.clone());
    framed(view().child(each(
        rows,
        |t: &Transfer| (t.id.clone(), t.state.clone(), t.done),
        move |t, _| {
            let t = t.get_untracked();
            let id = t.id.clone();
            let progress = match (&t.state, t.total) {
                (TransferState::Flagged(host), _) => format!("From {host}, a site on the malware list. Not saved until you keep it."),
                (TransferState::Paused, _) => format!("Paused at {}", megabytes(t.done)),
                (TransferState::Saving, Some(total)) => format!("{} of {}", megabytes(t.done), megabytes(total)),
                (TransferState::Saving, None) => megabytes(t.done),
            };
            let command = move |name: &'static str, keep: bool| {
                let id = id.clone();
                move || ui.act(|s| s.transfer_command(&id, name, keep))
            };
            let buttons: AnyElement = match t.state {
                TransferState::Flagged(_) => view()
                    .flex_row()
                    .gap_1()
                    .child(pill("Keep", command("downloadKeep", true)))
                    .child(pill("Delete", command("downloadKeep", false)))
                    .into_element(),
                TransferState::Paused => view()
                    .flex_row()
                    .child(icon_button(icons::PLAY, "Resume", command("downloadResume", false)))
                    .child(icon_button(icons::CLOSE, "Cancel Download", command("downloadCancel", false)))
                    .into_element(),
                TransferState::Saving if t.pausable => view()
                    .flex_row()
                    .child(icon_button(icons::PAUSE, "Pause", command("downloadPause", false)))
                    .child(icon_button(icons::CLOSE, "Cancel Download", command("downloadCancel", false)))
                    .into_element(),
                TransferState::Saving => icon_button(icons::CLOSE, "Cancel Download", command("downloadCancel", false)).into_element(),
            };
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .min_h(px(TOUCH_TARGET))
                .pl_3()
                .border_b_1()
                .border_color(Theme::separator)
                .child(view().flex_1().child(label(t.name.clone()).number_of_lines(1)).child(caption(progress).number_of_lines(2)))
                .child(buttons)
        },
    )))
}

fn shield_words(labels: &[Label]) -> Vec<Words<'_>> {
    labels.iter().map(|l| Words { x: l.x, y: l.y, anchor: l.anchor, size: l.size, weight: l.weight, fill: l.fill, opacity: l.opacity, text: &l.text }).collect()
}

fn week(card: &Card) -> View {
    let rows: Vec<AnyElement> = week_rows(&card.week)
        .into_iter()
        .map(|(name, days)| {
            view()
                .flex_row()
                .items_end()
                .gap_3()
                .child(small(name).flex_1().number_of_lines(2))
                .child(week_bars(&days))
                .child(label(thousands(days.iter().sum::<i64>() as u64)).w(px(44.)).text_right().font_weight(FontWeight::SEMIBOLD))
                .into_element()
        })
        .collect();
    let sites: Vec<AnyElement> = card
        .sites
        .iter()
        .map(|(host, n)| {
            view()
                .flex_row()
                .child(label(browser_core::command::host_tail(&engine::view_policy::title(host), 40)).flex_1().number_of_lines(1))
                .child(label(thousands((*n).max(0) as u64)))
                .into_element()
        })
        .collect();
    view().gap_2().child(section("Last 7 days")).children(rows).children((!sites.is_empty()).then(|| section("Sites protected most"))).children(sites)
}

pub fn shield_card(ui: Ui) -> impl IntoElement {
    let flipped = signal(false);
    let card = computed(move || {
        ui.read(|s| {
            let (private, id) = s.model.profile()?;
            let store = s.core.store(private);
            let name = if private { "Private".to_owned() } else { engine::view_policy::title(&store.profile(id).ok()?.name) };
            browser_core::shield_card::read(store, id, private, name, browser_core::now()).map(|c| (c, private))
        })
    });
    dynamic(move || {
        let (card, private) = card.get()?;
        let turned = flipped.get();
        let scale = ((window().width.0.min(crate::theme::SHEET_WIDTH) - 56.) / W as f32).min(1.);
        let (sky, shapes, labels) = if turned { back(&card) } else { front(&card, true) };
        let counts = card.counters.iter().map(|(name, n)| format!("{name}: {}", thousands((*n).max(0) as u64))).collect::<Vec<_>>().join(", ");
        let name = if turned {
            format!("Shield card, stamps side: {}. Tap to turn.", card.stamps.join(", "))
        } else {
            format!("Shield card of {}: {counts}. Tap to turn.", card.name)
        };
        let kept =
            if private { "Counts stay in memory and are gone once the private space closes." } else { "Counted on this device only and never sent anywhere." };
        let flip = move || flipped.update(|b| *b = !*b);
        Some(
            view()
                .gap_3()
                .child(
                    view().items_center().child(
                        button()
                            .accessibility_label(SharedString::from(name))
                            .on_press(flip)
                            .rounded(px(Theme::radius_card() * scale))
                            .overflow_hidden()
                            .shadow(Theme::elevation(6., 18.))
                            .active(|s| s.opacity(0.85))
                            .child(picture(&face(sky, &shapes), W as f32, H as f32, &shield_words(&labels), scale)),
                    ),
                )
                .child(view().flex_row().justify_center().gap_2().child(pill("Flip", flip)).child(pill("New Design", move || {
                    ui.act(|s| {
                        if let Some((private, id)) = s.model.profile() {
                            s.core.store(private).new_shield_design(id).ok();
                        }
                        Vec::new()
                    })
                })))
                .child(caption(kept).number_of_lines(0).px_1())
                .child(week(&card))
                .into_element(),
        )
    })
}

pub fn picking_banner(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        ui.read(|s| s.picking.is_some()).then(|| {
            surface(Theme::radius_panel())
                .flex_row()
                .items_center()
                .gap_2()
                .mx_3()
                .my_1()
                .pl_4()
                .pr_1()
                .py_1()
                .child(label("Tap an element to hide it on this site.").flex_1())
                .child(pill("Cancel", move || ui.act(|s| s.cancel_pick())))
        })
    })
}

pub fn heading_for(screen: Screen) -> Option<&'static str> {
    Some(match screen {
        Screen::Reader => "Reader",
        Screen::FindTabs => "Find in Tabs",
        Screen::Qr => "QR Code",
        Screen::ReadingList => "Reading List",
        Screen::Move(_) => "Move Tab",
        Screen::SpaceEdit(_) => "Edit Space",
        _ => return None,
    })
}
