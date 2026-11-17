use std::time::Duration;

use browser_core::shield_card::{Card, H, Label, SHORT, W, back, day_totals, face, front, front_svg, thousands};
use browser_core::space_card::SpaceDraft;
use browser_core::tabs::TabKey;
use engine::reader::Block;
use nsh_base::button;
use nsh_core::prelude::*;
use store::Schedule;
use tarn_nsh_shared::card::{Words, day_chart, day_summary, picture, share_bar};
use tarn_nsh_shared::controls::group;

use super::{
    Screen, Ui, accent_pill, caption, detail_row, framed, icon_button, icons, label, pill, section, segmented, small, surface, switch_row, text_field,
};
use crate::scripts::{FindRow, Transfer, TransferState};
use tarn_nsh_shared::controls::glyph_tile;
use tarn_nsh_shared::icons as glyphs;

use crate::theme::{ICON, PROFILE_COLORS, Secure, TEXT_HEADING, TEXT_TITLE, TOUCH_TARGET, Theme, themed};
use tarn_nsh_shared::icons::SPACE_ICONS;

const FILE_TILE: f32 = 28.;
const CARD_FILE: &str = "Tarn Shield card.svg";

const QR_SIZE: f32 = 240.;
const SWATCH: f32 = 28.;
pub(super) const FIND_DELAY: Duration = Duration::from_millis(150);
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn row_button(name: String, detail: Option<String>, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    detail_row(None, name, detail.map(|d| caption(d).number_of_lines(2)), press)
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
                                            s.toast_as(browser_core::toast::Tone::Warning, "Read Aloud needs an on-device voice for this language.".into());
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
            text_field("Find in this space's tabs", "Find in Tabs")
                .auto_focus(true)
                .value(move || query.get())
                .on_change(move |v: &str| {
                    let typed = v.to_owned();
                    query.set(typed.clone());
                    set_timeout(FIND_DELAY, move || {
                        if query.get_untracked() == typed {
                            ui.act(|s| s.find_in_tabs(&typed));
                        }
                    });
                })
                .return_key_type(ReturnKeyType::Search)
                .test_id("find-tabs"),
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
    let rows = move || ui.read_data(|s| s.reading_list());
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
                .child(
                    text_field("Site CSS", "Site CSS")
                        .value(move || css.get())
                        .on_change(move |v: &str| css.set(v.to_owned()))
                        .auto_capitalize(AutoCapitalize::None)
                        .test_id("Site CSS"),
                )
                .child(view().flex_row().gap_2().child(pill("Save CSS", move || {
                    let value = css.get_untracked();
                    ui.act(|s| {
                        if s.set_site_css(&value) {
                            s.toast_as(browser_core::toast::Tone::Success, "Site CSS saved".into());
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

fn swatches(chosen: impl Fn() -> String + Copy + 'static, pick: impl Fn(&'static str) + Copy + 'static, none: bool) -> View {
    let options = none.then_some("").into_iter().chain(PROFILE_COLORS.iter().map(|(name, _)| *name));
    view().flex_row().flex_wrap().gap_1().children(
        options
            .map(|name| {
                button()
                    .accessibility_label(if name.is_empty() { "No Color" } else { name })
                    .w(px(TOUCH_TARGET))
                    .h(px(TOUCH_TARGET))
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_2()
                    .border_color(move || if chosen() == name { Theme::accent() } else { Theme::clear() })
                    .selected(move || chosen() == name)
                    .on_press(move || pick(name))
                    .child(
                        view()
                            .w(px(SWATCH))
                            .h(px(SWATCH))
                            .rounded_full()
                            .border_1()
                            .border_color(Theme::hairline)
                            .bg(move || Theme::profile_color(name).unwrap_or_else(Theme::control)),
                    )
                    .into_element()
            })
            .collect::<Vec<_>>(),
    )
}

pub fn space_card(ui: Ui, editing: Option<usize>) -> impl IntoElement {
    let draft = signal(ui.read(|s| s.space_draft(editing)));
    let profiles = ui.read_data(|s| s.profiles());
    let known = signal(profiles.clone());
    let ready = move || ui.read_data(|s| known.with(|p| draft.get().ready(&s.model.spaces, p).is_some()));
    let back = move || ui.show(Screen::Spaces);
    on_key_with("escape", KeyOptions { when: nsh_core::keymap::When::Always, ..KeyOptions::default() }, move |_| back());
    let confirm = move || {
        if !ready() {
            return;
        }
        let d = draft.get_untracked();
        let saved = std::cell::Cell::new(false);
        ui.act(|s| {
            saved.set(s.save_space(&d));
            Vec::new()
        });
        if saved.get() {
            ui.show(if d.editing.is_none() { Screen::Browse } else { Screen::Spaces });
        }
    };
    let private = draft.get_untracked().private;
    let new = editing.is_none();
    let field = move |name: &'static str, value: fn(&SpaceDraft) -> String, set: fn(&mut SpaceDraft, String)| {
        text_field(name, name)
            .test_id(name)
            .value(move || value(&draft.get()))
            .on_change(move |v: &str| draft.update(|d| set(d, v.to_owned())))
            .on_submit(move |_: &str| confirm())
    };
    let profile_rows = profiles
        .iter()
        .map(|p| (Some(p.id), p.name.clone(), Some(p.color.clone())))
        .chain([(None, "New Profile…".to_owned(), None)])
        .map(|(id, name, color)| {
            button()
                .accessibility_label(SharedString::from(name.clone()))
                .flex_row()
                .items_center()
                .gap_3()
                .min_h(px(TOUCH_TARGET))
                .px_3()
                .border_b_1()
                .border_color(Theme::separator)
                .bg(move || if draft.get().profile == id { Theme::selected() } else { Theme::clear() })
                .selected(move || draft.get().profile == id)
                .active(|s| s.bg(Theme::pressed))
                .on_press(move || draft.update(|d| d.profile = id))
                .child(match color {
                    Some(color) => view()
                        .w(px(ICON))
                        .h(px(ICON))
                        .items_center()
                        .justify_center()
                        .child(view().w(px(10.)).h(px(10.)).rounded_full().bg(move || Theme::profile_color(&color).unwrap_or_else(Theme::muted)))
                        .into_element(),
                    None => super::controls::icon(icons::PLUS, Theme::muted).into_element(),
                })
                .child(label(name).flex_1())
                .into_element()
        })
        .collect::<Vec<_>>();
    let icons_grid = SPACE_ICONS
        .iter()
        .enumerate()
        .map(|(ix, (name, source))| {
            let chosen = move || {
                let icon = draft.get().icon;
                icon == *name || (icon.is_empty() && ix == 0)
            };
            button()
                .accessibility_label(*name)
                .w(px(TOUCH_TARGET))
                .h(px(TOUCH_TARGET))
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_row()))
                .bg(move || if chosen() { Theme::selected() } else { Theme::clear() })
                .text_color(move || if chosen() { Theme::profile_color(&draft.get().color).unwrap_or_else(Theme::text) } else { Theme::muted() })
                .selected(chosen)
                .active(|s| s.bg(Theme::pressed))
                .on_press(move || draft.update(|d| d.icon = (*name).to_owned()))
                .child(glyphs::icon(source, ICON))
                .into_element()
        })
        .collect::<Vec<_>>();
    view()
        .gap_2()
        .child(
            caption(if new {
                "A space keeps its own tabs, pinned tabs and folders; spaces of one profile share its logins, cookies and history."
            } else {
                "Rename the space, give it another colour and icon, or schedule it."
            })
            .px_1(),
        )
        .child(new.then(|| view().gap_2().child(section("Profile")).child(framed(view().children(profile_rows)))))
        .child(dynamic(move || {
            (draft.get().profile.is_none()).then(|| {
                view()
                    .gap_2()
                    .child(caption("Starts empty, with its own logins, cookies, site data, history and bookmarks.").px_1())
                    .child(section("Profile Name"))
                    .child(field("Profile Name", |d| d.profile_name.clone(), |d, v| d.profile_name = v))
                    .child(section("Profile Color"))
                    .child(swatches(move || draft.get().profile_color, move |c| draft.update(|d| d.profile_color = c.to_owned()), false))
            })
        }))
        .child(section(if new { "Space Name" } else { "Name" }))
        .child(field(if new { "Space Name" } else { "Name" }, |d| d.name.clone(), |d, v| d.name = v))
        .child((!private).then(|| {
            view().gap_2().child(section("Color")).child(swatches(move || draft.get().color, move |c| draft.update(|d| d.color = c.to_owned()), true))
        }))
        .child(section("Icon"))
        .child(view().flex_row().flex_wrap().gap_1().children(icons_grid))
        .child((!new && !private).then(|| schedule(draft)))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .pt_2()
                .child(pill("Cancel", back))
                .child(accent_pill(if new { "Create Space" } else { "Save" }, confirm).disabled(move || !ready()).disabled_style(|s| s.opacity(0.35))),
        )
}

fn schedule(draft: Signal<SpaceDraft>) -> View {
    let schedule = move || draft.get().schedule;
    let save = move |next: Schedule| draft.update(|d| d.schedule = next);
    let days: Vec<tarn_nsh_shared::controls::Segment> = [1u8, 2, 3, 4, 5, 6, 0]
        .into_iter()
        .map(|day| {
            let bit = 1u8 << day;
            (
                WEEKDAYS[day as usize].to_owned(),
                Box::new(move || schedule().days & bit != 0) as Box<dyn Fn() -> bool>,
                Box::new(move || {
                    let s = draft.get_untracked().schedule;
                    save(Schedule { days: s.days ^ bit, ..s });
                }) as Box<dyn Fn()>,
            )
        })
        .collect();
    let stepper = move |name: &'static str, start: bool| {
        let value = move || {
            let s = schedule();
            if start { s.start } else { s.end }
        };
        let step = move |delta: i8| {
            let s = draft.get_untracked().schedule;
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
            .child(caption(move || if !start && schedule().end <= schedule().start { "next day" } else { "" }))
    };
    view()
        .gap_2()
        .child(section("Schedule"))
        .child(segmented(days))
        .child(stepper("From", true))
        .child(stepper("To", false))
        .child(caption("On these days and hours each window switches to this space, unless you changed space in the last hour. No days, no schedule.").px_1())
}

pub fn snapshots(ui: Ui) -> impl IntoElement {
    let name = signal(String::new());
    let rows = move || ui.read_data(|s| s.snapshots());
    let none = move || rows().is_empty();
    view()
        .gap_2()
        .child(section("Snapshots"))
        .child(text_field("Snapshot Name", "Snapshot Name").value(move || name.get()).on_change(move |v: &str| name.set(v.to_owned())).test_id("Snapshot Name"))
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
                            let remove = remove.clone();
                            super::panels::confirm("Delete this snapshot?", "Its saved tabs cannot be restored afterwards.", "Delete", move || {
                                ui.act(|s| {
                                    s.delete_snapshot(&remove);
                                    Vec::new()
                                })
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
            let flagged = matches!(t.state, TransferState::Flagged(_));
            let share = t.total.filter(|total| *total > 0 && !flagged).map(|total| (t.done as f32 / total as f32).clamp(0., 1.));
            view()
                .border_b_1()
                .border_color(Theme::separator)
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .min_h(px(TOUCH_TARGET))
                        .pl_3()
                        .py_1()
                        .child(file_tile(&t.name, flagged))
                        .child(
                            view()
                                .flex_1()
                                .child(label(t.name.clone()).number_of_lines(1))
                                .child(caption(progress).number_of_lines(2).style_when(move || flagged, |s| s.text_color(Secure::warning()))),
                        )
                        .child(buttons),
                )
                .child(share.map(|share| {
                    view()
                        .h(px(3.))
                        .mx_3()
                        .mb_2()
                        .rounded_full()
                        .bg(Theme::separator)
                        .overflow_hidden()
                        .child(view().h_full().w(relative(share)).bg(Theme::accent))
                }))
        },
    )))
}

pub fn file_tile(name: &str, flagged: bool) -> View {
    let source = if flagged { glyphs::TRIANGLE_ALERT } else { glyphs::named(browser_core::glyphs::file_glyph(name)).unwrap_or(glyphs::FILE_PLAIN) };
    glyph_tile(
        source,
        FILE_TILE,
        move || if flagged { Secure::control() } else { Theme::control() },
        move || if flagged { Secure::warning() } else { Theme::muted() },
    )
}

fn shield_words(labels: &[Label]) -> Vec<Words<'_>> {
    labels.iter().map(|l| Words { x: l.x, y: l.y, anchor: l.anchor, size: l.size, weight: l.weight, fill: l.fill, opacity: l.opacity, text: &l.text }).collect()
}

fn numbers(card: &Card) -> View {
    let cells: Vec<AnyElement> = card
        .counters
        .iter()
        .zip(SHORT)
        .map(|((_, n), (first, second))| {
            view()
                .flex_1()
                .items_center()
                .child(text(thousands((*n).max(0) as u64)).text_size(px(TEXT_HEADING)).font_weight(FontWeight::BOLD).text_color(Theme::text))
                .child(small(format!("{first} {second}")).text_center().number_of_lines(2))
                .into_element()
        })
        .collect();
    group().flex_row().py_3().px_1().children(cells)
}

fn last_week(card: &Card) -> View {
    let days = day_totals(&card.week);
    let total: i64 = days.iter().map(|(_, n)| n).sum();
    group()
        .p_3()
        .gap_2()
        .accessible(true)
        .label(format!("Last 7 days: {} protected. {}", thousands(total as u64), day_summary(&days)))
        .child(
            view()
                .flex_row()
                .items_center()
                .child(label("Last 7 days").font_weight(FontWeight::SEMIBOLD).flex_1())
                .child(caption(format!("{} protected", thousands(total as u64)))),
        )
        .child(day_chart(&days))
}

fn top_sites(card: &Card) -> Option<View> {
    let most = card.sites.iter().map(|(_, n)| *n).max()?;
    let rows: Vec<AnyElement> = card
        .sites
        .iter()
        .map(|(host, n)| {
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .min_h(px(32.))
                .child(label(browser_core::command::host_tail(&engine::view_policy::title(host), 40)).flex_1().number_of_lines(1))
                .child(view().w(relative(0.3)).child(share_bar(*n, most)))
                .child(label(thousands((*n).max(0) as u64)).w(px(48.)).text_right())
                .into_element()
        })
        .collect();
    Some(group().p_3().gap_1().child(label("Most protected sites").font_weight(FontWeight::SEMIBOLD)).children(rows))
}

fn share_card(ui: Ui, card: &Card, dark: bool) {
    let drawing = front_svg(card, true, dark);
    spawn(async move {
        let shared = browser_core::shield_card::export_path(CARD_FILE);
        if let Some(shared) = shared.ok().filter(|path| std::fs::write(path, &drawing).is_ok())
            && crate::platform::share_file(shared).await
        {
            return;
        }
        ui.act(|s| {
            s.notice = Some("Tarn could not share the card.".to_owned());
            Vec::new()
        });
    });
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
    themed(move || {
        dynamic(move || {
            let (card, private) = card.get()?;
            let turned = flipped.get();
            let dark = !Theme::is_light();
            let scale = ((window().width.0.min(crate::theme::SHEET_WIDTH) - 56.) / W as f32).min(1.);
            let (sky, shapes, labels) = if turned { back(&card, dark) } else { front(&card, true, dark) };
            let counts = card.counters.iter().map(|(name, n)| format!("{name}: {}", thousands((*n).max(0) as u64))).collect::<Vec<_>>().join(", ");
            let name = if turned {
                format!("Shield card, stamps side: {}. Tap to turn.", card.stamps.join(", "))
            } else {
                format!("Shield card of {}: {counts}. Tap to turn.", card.name)
            };
            let kept = if private {
                "Counts stay in memory and are gone once the private space closes."
            } else {
                "Counted on this device only and never sent anywhere."
            };
            let flip = move || flipped.update(|b| *b = !*b);
            let shared = card.clone();
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
                                .bind_motion(&tarn_nsh_shared::motion::fade_in())
                                .child(picture(&face(sky, &shapes), W as f32, H as f32, &shield_words(&labels), scale)),
                        ),
                    )
                    .child(numbers(&card))
                    .child(
                        view()
                            .flex_row()
                            .flex_wrap()
                            .justify_center()
                            .gap_2()
                            .child(pill("Flip", flip))
                            .child(pill("Change design", move || {
                                ui.act(|s| {
                                    if let Some((private, id)) = s.model.profile() {
                                        s.core.store(private).new_shield_design(id).ok();
                                    }
                                    Vec::new()
                                })
                            }))
                            .children((!private).then(|| pill("Share card", move || share_card(ui, &shared, dark)))),
                    )
                    .child(caption(kept).number_of_lines(0).px_1())
                    .child(last_week(&card))
                    .child(top_sites(&card))
                    .into_element(),
            )
        })
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
                .shadow(Theme::elevation(4., 14.))
                .child(label("Tap an element to hide it on this site.").flex_1())
                .child(pill("Cancel", move || ui.act(|s| s.cancel_pick())))
        })
    })
}

#[cfg(target_os = "android")]
const LICENCES_TEXT: &str = include_str!("../../../../packaging/third-party-licenses-android.txt");
#[cfg(not(target_os = "android"))]
const LICENCES_TEXT: &str = include_str!("../../../../packaging/third-party-licenses-ios.txt");

fn licence_sections() -> &'static [browser_core::licences::Section] {
    static SECTIONS: std::sync::LazyLock<Vec<browser_core::licences::Section>> = std::sync::LazyLock::new(|| browser_core::licences::sections(LICENCES_TEXT));
    &SECTIONS
}

pub fn licences(ui: Ui) -> impl IntoElement {
    let query = signal(String::new());
    let rows = move || browser_core::licences::matching(licence_sections(), &query.get()).into_iter().map(|(ix, _)| ix).collect::<Vec<_>>();
    view()
        .gap_2()
        .child(caption("The software Tarn is built from and its licences, as shipped with this version.").px_1())
        .child(
            text_field("Search licences", "Search licences")
                .value(move || query.get())
                .on_change(move |v: &str| query.set(v.to_owned()))
                .auto_capitalize(AutoCapitalize::None),
        )
        .child(framed(view().child(each(
            rows,
            |ix: &usize| *ix,
            move |ix, _| {
                let ix = ix.get_untracked();
                row_button(licence_sections()[ix].title.clone(), None, move || ui.show(Screen::Licence(ix)))
            },
        ))))
}

pub fn licence(ix: usize) -> impl IntoElement {
    let section = licence_sections().get(ix);
    view().gap_3().px_1().children(section.map(|s| {
        view()
            .gap_3()
            .child(text(s.title.clone()).text_size(px(TEXT_HEADING)).font_weight(FontWeight::BOLD).text_color(Theme::text))
            .child(text(s.body.clone()).text_color(Theme::text).selectable(true))
    }))
}

pub fn heading_for(screen: Screen) -> Option<&'static str> {
    Some(match screen {
        Screen::Reader => "Reader",
        Screen::FindTabs => "Find in Tabs",
        Screen::Qr => "QR Code",
        Screen::ReadingList => "Reading List",
        Screen::Move(_) => "Move Tab",
        Screen::SpaceNew => "New Space",
        Screen::SpaceEdit(_) => "Edit Space",
        Screen::Licences => "Open-Source Licences",
        Screen::Licence(_) => "Licence",
        _ => return None,
    })
}
