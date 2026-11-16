use browser_core::shield_card::{Card, H, W, back, face, front, thousands};
use nsh_core::prelude::*;
use tarn_nsh_shared::panes::{self, PANES, Pane};

use super::panels::{levels, section};
use super::tools::{line, note};
use super::{Panel, Ui, act, read};
use crate::browser::pages::{HISTORY_TEXT, SORT_DOWNLOADS};
use crate::browser::{Browser, Notice};
use crate::controls::{accent_pill, group, group_row, label, panel, pill, segmented, small, switch, title};
use crate::shortcuts;
use crate::theme::Theme;
use crate::webview;
use browser_core::command::ENGINES;

const NAV_WIDTH: f32 = 180.;

fn card_of(b: &Browser) -> Option<(Card, bool)> {
    let space = b.model.current_space()?;
    let store = b.core.store(space.private);
    let name = if space.private { "Private".to_owned() } else { engine::view_policy::title(&store.profile(space.space.profile_id).ok()?.name) };
    Some((browser_core::shield_card::read(store, space.space.profile_id, space.private, name, browser_core::now())?, space.private))
}

fn words(labels: &[browser_core::shield_card::Label]) -> Vec<tarn_nsh_shared::card::Words<'_>> {
    labels
        .iter()
        .map(|l| tarn_nsh_shared::card::Words {
            x: l.x,
            y: l.y,
            anchor: l.anchor,
            size: l.size,
            weight: l.weight,
            fill: l.fill,
            opacity: l.opacity,
            text: &l.text,
        })
        .collect()
}

fn shield_card(ui: Ui, card: &Card) -> View {
    let flipped = ui.card_back.get_untracked();
    let (sky, shapes, labels) = if flipped { back(card) } else { front(card, true) };
    let counts = card.counters.iter().map(|(name, n)| format!("{name}: {}", thousands((*n).max(0) as u64))).collect::<Vec<_>>().join(", ");
    let name = if flipped { format!("Shield card, stamps side: {}", card.stamps.join(", ")) } else { format!("Shield card of {}: {counts}", card.name) };
    let face = crate::controls::pressable(name, move || {
        ui.card_back.update(|b| *b = !*b);
        act(ui, |_| {});
    })
    .w(px(W as f32))
    .h(px(H as f32))
    .rounded(px(Theme::radius_card()))
    .shadow(Theme::elevation(10., 30.))
    .child(tarn_nsh_shared::card::picture(&face(sky, &shapes), W as f32, H as f32, &words(&labels), 1.));
    let buttons = view()
        .flex_row()
        .gap_2()
        .child(pill("Flip", move || {
            ui.card_back.update(|b| *b = !*b);
            act(ui, |_| {});
        }))
        .child(pill("New Design", move || act(ui, Browser::new_shield_design)));
    view().flex_col().items_center().gap_3().w_full().child(face).child(buttons)
}

fn week(card: &Card) -> View {
    let rows: Vec<AnyElement> = browser_core::shield_card::week_rows(&card.week)
        .into_iter()
        .map(|(name, days)| {
            view()
                .flex_row()
                .items_end()
                .gap_3()
                .child(small(name).flex_1())
                .child(tarn_nsh_shared::card::week_bars(&days))
                .child(label(thousands(days.iter().sum::<i64>() as u64)).w(px(44.)).text_right().font_weight(FontWeight::SEMIBOLD))
                .into_element()
        })
        .collect();
    let sites: Vec<AnyElement> = card
        .sites
        .iter()
        .map(|(host, n)| {
            line(browser_core::command::host_tail(&engine::view_policy::title(host), 40)).child(label(thousands((*n).max(0) as u64))).into_element()
        })
        .collect();
    view()
        .flex_col()
        .gap_2()
        .child(section("Last 7 days"))
        .children(rows)
        .children((!sites.is_empty()).then(|| section("Sites protected most")))
        .children(sites)
}

fn shortcut_rows(ui: Ui) -> View {
    let Some(stored) = read(ui, |b| shortcuts::stored(&b.core.disk)) else { return view() };
    let map = shortcuts::keymap(&stored);
    let mut out = view().flex_col().gap_2().child(note("Click a shortcut and type the new keys with Ctrl, such as Ctrl+Shift+Y; Enter saves, Esc cancels."));
    if let Some(error) = ui.shortcut_error.get_untracked() {
        out = out.child(note(error).text_color(Theme::warning()));
    }
    for (name, rows) in shortcuts::SECTIONS {
        let mut grp = group();
        for (row_label, action, _) in rows {
            let keys = map.iter().find(|(_, a)| a == action).map(|(k, _)| k.clone()).unwrap_or_default();
            let custom = stored.contains_key(&action.setting_key());
            let action = *action;
            let editing = ui.shortcut_edit.get_untracked() == Some(action);
            let control: AnyElement = if editing {
                super::text_field(ui.shortcut_text, "ctrl+shift+y", 160.)
                    .auto_focus(true)
                    .on_submit(move |v: &str| {
                        let raw = v.to_owned();
                        let result = read(ui, |b| shortcuts::save(&b.core.disk, action, &raw)).unwrap_or(Ok(()));
                        ui.shortcut_error.set(result.err());
                        ui.shortcut_edit.set(None);
                        ui.keys_rev.update(|r| *r += 1);
                        act(ui, |_| {});
                    })
                    .into_element()
            } else {
                pill(shortcuts::display(&keys), move || {
                    ui.shortcut_text.set(String::new());
                    ui.shortcut_error.set(None);
                    ui.shortcut_edit.set(Some(action));
                    act(ui, |_| {});
                })
                .into_element()
            };
            let reset = custom.then(|| {
                pill("Reset", move || {
                    let result = read(ui, |b| shortcuts::reset(&b.core.disk, Some(action))).unwrap_or(Ok(()));
                    ui.shortcut_error.set(result.err());
                    ui.keys_rev.update(|r| *r += 1);
                    act(ui, |_| {});
                })
            });
            grp = grp.child(group_row().child(label(*row_label).flex_1()).children(reset).child(control));
        }
        out = out.child(small(name).pt_2()).child(grp);
    }
    if !stored.is_empty() {
        out = out.child(view().flex_row().justify_end().child(pill("Reset All", move || {
            let result = read(ui, |b| shortcuts::reset(&b.core.disk, None)).unwrap_or(Ok(()));
            ui.shortcut_error.set(result.err());
            ui.keys_rev.update(|r| *r += 1);
            act(ui, |_| {});
        })));
    }
    out
}

fn rules(ui: Ui) -> View {
    let rows = read(ui, Browser::space_rule_rows).unwrap_or_default();
    let mut out = view().flex_col();
    if rows.is_empty() {
        return out.child(note("No rules. “Always open in” in a site's settings opens its new tabs in the space you pick."));
    }
    for (id, host, space) in rows {
        out = out.child(line(format!("{host} → {space}")).child(pill("Remove", move || act(ui, |b| b.delete_space_rule(id)))));
    }
    out
}

fn hidden(ui: Ui, private: bool) -> View {
    if private {
        return view().child(note("A private space keeps no hidden elements."));
    }
    let zaps = read(ui, |b| b.zapped(None)).unwrap_or_default();
    if zaps.is_empty() {
        return view().child(note("No hidden elements. Zap Element in the command bar hides an element on a site in this profile."));
    }
    let rows: Vec<AnyElement> = zaps
        .into_iter()
        .map(|(host, selector)| {
            let (h, s) = (host.clone(), selector.clone());
            line(format!("{host}: {selector}")).child(pill("Undo", move || act(ui, |b| b.unzap(&h, Some(&s))))).into_element()
        })
        .collect();
    view().flex_col().children(rows)
}

fn permissions(ui: Ui) -> View {
    let rows = read(ui, |b| b.permissions()).unwrap_or_default();
    if rows.is_empty() {
        return view()
            .child(note("No site has asked yet. When a site asks for the camera, microphone or location and you choose Remember, the answer shows here."));
    }
    let rows: Vec<AnyElement> = rows
        .into_iter()
        .map(|p| {
            let set = |allow: Option<bool>| {
                let (origin, kind) = (p.origin.clone(), p.kind.clone());
                move || act(ui, |b| b.set_permission(&origin, &kind, allow))
            };
            let mark = |on: bool, name: &'static str, allow: bool| {
                if on { accent_pill(name, set(Some(allow))).into_element() } else { pill(name, set(Some(allow))).into_element() }
            };
            line(format!("{} · {}", p.site, p.label))
                .child(mark(p.allowed, "Allow", true))
                .child(mark(!p.allowed, "Block", false))
                .child(pill("Forget", set(None)))
                .into_element()
        })
        .collect();
    view().flex_col().children(rows)
}

fn nav(ui: Ui, current: Pane) -> View {
    let rows: Vec<AnyElement> = PANES
        .iter()
        .map(|(pane, name)| {
            let pane = *pane;
            let selected = pane == current;
            crate::controls::row(*name, move || {
                ui.settings_pane.set(pane);
                act(ui, |_| {});
            })
            .h(px(30.))
            .px(px(10.))
            .rounded(px(Theme::radius_row()))
            .bg(if selected { Theme::selected() } else { rgba(0) })
            .hover(|s| s.bg(Theme::hover()))
            .text_color(if selected { Theme::text() } else { Theme::muted() })
            .child(crate::icons::icon(panes::icon(pane), 14.))
            .child(label(*name).text_color(if selected { Theme::text() } else { Theme::muted() }))
            .into_element()
        })
        .collect();
    view().flex_col().gap(px(2.)).w(px(NAV_WIDTH)).flex_none().pt_2().children(rows)
}

pub fn settings_panel(ui: Ui) -> AnyElement {
    let pane = ui.settings_pane.get_untracked();
    let body = pane_body(ui, pane).unwrap_or_else(panel);
    view().flex_row().flex_1().gap_4().child(nav(ui, pane)).child(scroll().flex_1().child(body.child(view().h(px(12.))))).into_element()
}

fn pane_body(ui: Ui, pane: Pane) -> Option<View> {
    let (level, engine, lists, private, card, history_text, sort) = read(ui, |b| {
        (b.default_level(), b.search_engine(), b.lists(), b.model.private(), card_of(b), !b.setting_off(HISTORY_TEXT), !b.setting_off(SORT_DOWNLOADS))
    })?;
    let on = |group: &str| lists.iter().any(|l| l == group);
    let toggle = move |setting: &'static str| move || act(ui, move |b| b.toggle_setting(setting));
    let toggle_off = move |setting: &'static str| move || act(ui, move |b| b.toggle_off_setting(setting));
    let body = panel().gap_0().child(super::panels::heading(ui, title(panes::name(pane))));
    Some(match pane {
        Pane::Profile => match card {
            Some((card, private)) => {
                let kept = if private {
                    "Counts stay in memory and are gone once the private space closes."
                } else {
                    "Counted on this computer only and never sent anywhere."
                };
                body.child(view().flex_col().gap_2().pt_3().child(shield_card(ui, &card)).child(note(kept)).child(week(&card)))
            }
            None => body.child(note("No profile is open.")),
        },
        Pane::General => body
            .child(section("Downloads"))
            .child(group().child(switch(sort, "Sort downloads into folders", toggle_off(SORT_DOWNLOADS))))
            .child(note("New files go into Images, Videos, Audio, Documents, Archives or Installers inside Downloads; other types stay at the top."))
            .child(section("About"))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(link_pill(ui, browser_core::LICENCE, browser_core::LICENCE_URL))
                    .child(link_pill(ui, "Source code", browser_core::SOURCE_URL))
                    .child(link_pill(ui, "Privacy notice", browser_core::PRIVACY_URL))
                    .child(link_pill(ui, "Imprint", browser_core::IMPRINT_URL))
                    .child(link_pill(ui, "Open-source licences", browser_core::LINUX_LICENCES_URL)),
            ),
        Pane::Appearance => body.child(view().pt_3().child(super::appearance::pane(ui))),
        Pane::Search => {
            let engines: Vec<(usize, String)> = ENGINES.iter().enumerate().map(|(ix, e)| (ix, e.name.to_owned())).collect();
            body.child(section("Search engine"))
                .child(group().child(group_row().child(label("Search engine").flex_1()).child(segmented(engines, engine, move |ix| act(ui, move |b| b.set_search_engine(ix))))))
                .child(section("History"))
                .child(group().child(switch(history_text, "Search the text of visited pages", toggle_off(HISTORY_TEXT))))
                .child(note("Search the text of visited pages keeps up to 4 KB of each page's text on this computer so history suggestions match what a page said. Never for the private space or Lockdown pages; cleared with history, and when you turn this off."))
        }
        Pane::Privacy => {
            let clear_data = move || {
                if let Some(key) = read(ui, |b| b.store_key()).flatten() {
                    run_on_main_thread(move || webview::session::clear(&key));
                    act(ui, |b| b.notice = Some(Notice::Message("Website data of this profile was cleared.".into())));
                }
            };
            body.child(section("Shield"))
                .child(
                    group()
                        .child(
                            group_row()
                                .child(label(if private { "Shield level (private space)" } else { "Default Shield level" }).flex_1())
                                .child(segmented(levels(), level, move |l| act(ui, move |b| b.set_default_level(l)))),
                        )
                        .child(switch(on("ads"), "Block ads", toggle("block_ads")))
                        .child(switch(on("trackers"), "Block trackers", toggle("block_trackers")))
                        .child(switch(on("annoyances"), "Block cookie banners", toggle("block_annoyances")))
                        .child(switch(on("malware"), "Block malware sites", toggle("block_malware"))),
                )
                .child(note("EasyList Cookie and uBlock Cookie Notices hide consent pop-ups and stop the scripts that show them. Nothing is accepted or refused for you."))
                .child(note("Filter lists apply to pages loaded after a change; they need the signed list bundle."))
                .child(section("Hidden elements"))
                .child(hidden(ui, private))
                .child(section("Clear data"))
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .gap_2()
                        .child(super::confirmed(ui, "history".to_owned(), "Clear History", move || super::clear_history(ui)))
                        .child(super::confirmed(ui, "download list".to_owned(), "Clear Download List", move || act(ui, |b| b.clear_downloads())))
                        .child(super::confirmed(ui, "data".to_owned(), "Clear Website Data", clear_data)),
                )
        }
        Pane::Profiles => body
            .child(
                view()
                    .flex_row()
                    .gap_2()
                    .pt_3()
                    .child(pill("New Profile", move || act(ui, |b| b.new_profile())))
                    .child(pill("Memory", move || super::show(ui, Panel::Memory)))
                    .child(accent_pill("Open Private Space", move || {
                        ui.panel.set(Panel::None);
                        act(ui, |b| b.private_space());
                    })),
            )
            .child(section("Space Rules"))
            .child(rules(ui)),
        Pane::Permissions => body
            .child(note("This profile's answers to sites. A site you forget asks again next time; Lockdown sites never ask."))
            .child(view().pt_2().child(permissions(ui))),
        Pane::Shortcuts => body.child(view().pt_3().child(shortcut_rows(ui))),
    })
}

fn link_pill(ui: Ui, name: &'static str, url: &'static str) -> impl IntoElement {
    pill(name, move || {
        ui.panel.set(Panel::None);
        act(ui, |b| {
            b.new_tab(url);
        });
    })
}
