use browser_core::space_card::SpaceDraft;
use nsh_core::prelude::*;
use store::{Id, Profile, Schedule};
use tarn_nsh_shared::icons::SPACE_ICONS;
use tarn_nsh_shared::theme::PROFILE_COLORS;

use super::panels::{capitalised, heading, popover, section};
use super::tools::{line, note};
use super::{Panel, Ui, act, read, site_tile};
use crate::browser::Browser;
use crate::controls::{accent_pill, label, pill, pressable, small, title};
use crate::icons;
use crate::memory;
use crate::theme::Theme;

const FIELD_WIDTH: f32 = 388.;
const DAYS: [(&str, u8); 7] = [("Mon", 1), ("Tue", 2), ("Wed", 3), ("Thu", 4), ("Fri", 5), ("Sat", 6), ("Sun", 0)];

fn hour(value: u8, set: impl Fn(u8) + Copy + 'static) -> View {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(crate::controls::icon_button(icons::CHEVRON_LEFT, "Earlier", move || set((value + 23) % 24)))
        .child(small(format!("{value:02}:00")).w(px(44.)).text_center())
        .child(crate::controls::icon_button(icons::CHEVRON_RIGHT, "Later", move || set((value + 1) % 24)))
}

fn draft<R>(ui: Ui, f: impl FnOnce(&SpaceDraft) -> R) -> Option<R> {
    ui.space_draft.with(|d| d.as_ref().map(f))
}

fn edit_draft(ui: Ui, f: impl FnOnce(&mut SpaceDraft)) {
    ui.space_draft.update(|d| {
        if let Some(d) = d {
            f(d);
        }
    });
}

fn ready(ui: Ui) -> bool {
    let Some(d) = ui.space_draft.with(Clone::clone) else { return false };
    read(ui, |b| d.ready(&b.model.spaces, &b.core.disk.profiles().unwrap_or_default()).is_some()) == Some(true)
}

fn confirm(ui: Ui) {
    let Some(d) = ui.space_draft.get_untracked().filter(|_| ready(ui)) else { return };
    let saved = std::cell::Cell::new(false);
    act(ui, |b| saved.set(b.save_space(&d)));
    if saved.get() {
        super::show(ui, Panel::None);
    }
}

fn field(ui: Ui, name: &str, get: fn(&SpaceDraft) -> String, set: fn(&mut SpaceDraft, String)) -> nsh_base::Input {
    super::bound_field(move || draft(ui, get).unwrap_or_default(), move |v| edit_draft(ui, |d| set(d, v)), name, FIELD_WIDTH)
        .on_submit(move |_: &str| confirm(ui))
}

fn footer(ui: Ui, name: &str) -> View {
    view()
        .flex_row()
        .justify_end()
        .gap_2()
        .child(pill("Cancel", move || super::show(ui, Panel::None)))
        .child(accent_pill(name, move || confirm(ui)).disabled(move || !ready(ui)).disabled_style(|s| s.text_color(Theme::faint())))
}

fn swatches(ui: Ui, chosen: &str, none: bool, set: fn(&mut SpaceDraft, String)) -> View {
    let dots: Vec<AnyElement> = none
        .then_some("")
        .into_iter()
        .chain(PROFILE_COLORS.iter().map(|(name, _)| *name))
        .map(|name| {
            let label = if name.is_empty() { "No Color".to_owned() } else { capitalised(name) };
            pressable(label, move || edit_draft(ui, |d| set(d, name.to_owned())))
                .w(px(22.))
                .h(px(22.))
                .rounded_full()
                .items_center()
                .justify_center()
                .border_1()
                .border_color(Theme::hairline())
                .bg(Theme::profile_color(name).unwrap_or(Theme::control()))
                .text_color(if name.is_empty() { Theme::text() } else { rgb(0xffffff) })
                .children((name == chosen).then(|| icons::icon(icons::CHECK, 12.)))
                .into_element()
        })
        .collect();
    view().flex_row().flex_wrap().gap_2().children(dots)
}

fn icon_grid(ui: Ui, d: &SpaceDraft) -> View {
    let tint = Theme::profile_color(&d.color).unwrap_or(Theme::accent());
    let chosen = if d.icon.is_empty() { SPACE_ICONS[0].0 } else { d.icon.as_str() };
    let cells: Vec<AnyElement> = SPACE_ICONS
        .iter()
        .map(|&(name, glyph)| {
            let on = name == chosen;
            crate::controls::row(capitalised(name), move || edit_draft(ui, |d| d.icon = name.to_owned()))
                .w(px(30.))
                .h(px(30.))
                .px_0()
                .justify_center()
                .bg(if on { Theme::selected() } else { Theme::clear() })
                .text_color(if on { tint } else { Theme::muted() })
                .child(icons::icon(glyph, 16.))
                .into_element()
        })
        .collect();
    view().flex_row().flex_wrap().gap_1().children(cells)
}

fn profile_rows(ui: Ui, profiles: &[Profile], chosen: Option<Id>) -> View {
    let choice = |name: String, id: Option<Id>, lead: View| {
        crate::controls::row(name.clone(), move || edit_draft(ui, |d| d.profile = id))
            .bg(if chosen == id { Theme::selected() } else { Theme::clear() })
            .child(lead.w(px(16.)).items_center())
            .child(label(name).flex_1().truncate())
            .into_element()
    };
    let rows: Vec<AnyElement> = profiles
        .iter()
        .map(|p| {
            let dot = view().w(px(8.)).h(px(8.)).rounded_full().bg(Theme::profile_color(&p.color).unwrap_or(Theme::faint()));
            choice(p.name.clone(), Some(p.id), view().child(dot))
        })
        .chain([choice("New Profile…".to_owned(), None, view().text_color(Theme::muted()).child(icons::icon(icons::PLUS, 14.)))])
        .collect();
    view().flex_col().gap(px(2.)).children(rows)
}

fn schedule_rows(ui: Ui, schedule: Schedule) -> View {
    let set = move |f: fn(&mut Schedule, u8), value: u8| edit_draft(ui, |d| f(&mut d.schedule, value));
    let days: Vec<AnyElement> = DAYS
        .iter()
        .map(|(name, bit)| {
            let on = schedule.days & (1 << bit) != 0;
            let bit = *bit;
            crate::controls::row(Some(SharedString::from(*name)), move || set(|s, b| s.days ^= 1 << b, bit))
                .w(px(44.))
                .justify_center()
                .rounded(px(Theme::radius_small()))
                .bg(if on { Theme::accent_soft() } else { Theme::control() })
                .selected(on)
                .child(small(*name).text_color(if on { Theme::accent() } else { Theme::muted() }))
                .into_element()
        })
        .collect();
    let overnight = schedule.end <= schedule.start;
    view()
        .flex_col()
        .gap_2()
        .child(view().flex_row().gap_1().children(days))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(label("From"))
                .child(hour(schedule.start, move |h| set(|s, h| s.start = h, h)))
                .child(label("to"))
                .child(hour(schedule.end, move |h| set(|s, h| s.end = h, h)))
                .children(overnight.then(|| small("next day"))),
        )
        .child(note("On these days and hours each window switches to this space, unless you changed space in the last hour. No days, no schedule."))
}

pub fn new_space_panel(ui: Ui) -> AnyElement {
    if ui.space_draft.with_untracked(Option::is_none) {
        ui.space_draft.set(read(ui, Browser::new_space_draft));
    }
    let profiles = read(ui, |b| b.core.disk.profiles().unwrap_or_default()).unwrap_or_default();
    let new_profile = move || {
        view()
            .flex_col()
            .gap_2()
            .child(note("Starts empty, with its own logins, cookies, site data, history and bookmarks."))
            .child(section("Profile Name"))
            .child(field(ui, "Profile Name", |d| d.profile_name.clone(), |d, v| d.profile_name = v))
            .child(section("Profile Color"))
            .child(dynamic(move || draft(ui, |d| swatches(ui, &d.profile_color, false, |d, c| d.profile_color = c))))
    };
    popover()
        .child(heading(ui, title("New Space")))
        .child(note("A space keeps its own tabs, pinned tabs and folders; spaces of one profile share its logins, cookies and history."))
        .child(section("Profile"))
        .child(dynamic(move || draft(ui, |d| profile_rows(ui, &profiles, d.profile))))
        .child(when(move || draft(ui, |d| d.profile.is_none()) == Some(true), new_profile))
        .child(section("Space Name"))
        .child(field(ui, "Space Name", |d| d.name.clone(), |d, v| d.name = v))
        .child(section("Color"))
        .child(dynamic(move || draft(ui, |d| swatches(ui, &d.color, true, |d, c| d.color = c))))
        .child(section("Icon"))
        .child(dynamic(move || draft(ui, |d| icon_grid(ui, d))))
        .child(footer(ui, "Create Space"))
        .into_element()
}

pub fn space_panel(ui: Ui) -> AnyElement {
    let Some((current, snapshots)) = read(ui, |b| (b.model.current_space().map(SpaceDraft::edit), b.snapshots())) else {
        return view().into_element();
    };
    let target = |d: &SpaceDraft| (d.editing, d.private);
    if ui.space_draft.with_untracked(|d| d.as_ref().map(target)) != current.as_ref().map(target) {
        ui.space_draft.set(current);
    }
    let private = ui.space_draft.with_untracked(|d| d.as_ref().is_some_and(|d| d.private));
    let mut body = popover()
        .child(heading(ui, title("Edit Space")))
        .child(note("Rename the space, give it another colour and icon, or schedule it."))
        .child(section("Name"))
        .child(field(ui, "Name", |d| d.name.clone(), |d, v| d.name = v));
    if !private {
        body = body.child(section("Color")).child(dynamic(move || draft(ui, |d| swatches(ui, &d.color, true, |d, c| d.color = c))));
    }
    body = body.child(section("Icon")).child(dynamic(move || draft(ui, |d| icon_grid(ui, d))));
    body = if private {
        body.child(note("A private space has no schedule or snapshots."))
    } else {
        body.child(section("Schedule")).child(dynamic(move || draft(ui, |d| schedule_rows(ui, d.schedule))))
    };
    body = body.child(footer(ui, "Save"));
    let rows: Vec<AnyElement> = snapshots
        .into_iter()
        .map(|saved| {
            let (a, b, c) = (saved.clone(), saved.clone(), saved.clone());
            line(saved.name.clone())
                .child(small(crate::browser::spaces::date(saved.created_at)))
                .child(pill("Restore in New Space", move || act(ui, |br| br.restore_snapshot(&a, true))))
                .child(pill("Restore in This Space", move || act(ui, |br| br.restore_snapshot(&b, false))))
                .child(super::confirmed(ui, format!("snapshot{}", saved.id), "Delete", move || act(ui, |br| br.delete_snapshot(&c))))
                .into_element()
        })
        .collect();
    let empty = rows.is_empty().then(|| note("Save a space with “Save Space as Snapshot”"));
    body.child(
        view()
            .flex_row()
            .items_center()
            .child(section("Snapshots").flex_1())
            .child(accent_pill("Save Space as Snapshot", move || act(ui, Browser::save_snapshot))),
    )
    .children(empty)
    .child(view().flex_col().children(rows))
    .into_element()
}

pub fn memory_panel(ui: Ui) -> AnyElement {
    let Some(rows) = read(ui, |b| {
        let costs = b.tab_costs();
        let live: std::collections::HashSet<_> = b.model.lifecycle.live().collect();
        b.model
            .tabs
            .iter()
            .map(|t| {
                let key = b.model.key(t);
                let host = b.page_url(key).and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
                (
                    key,
                    crate::snapshot::tab_label(t, b, key),
                    host,
                    memory::detail(costs.get(&key).copied(), !live.contains(&key)),
                    live.contains(&key) && b.model.active != Some(key),
                )
            })
            .collect::<Vec<_>>()
    }) else {
        return view().into_element();
    };
    let items: Vec<AnyElement> = rows
        .into_iter()
        .map(|(key, name, host, detail, sleepable)| {
            view()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .h(px(44.))
                .px_2()
                .child(site_tile(ui, &host, 16.))
                .child(view().flex_col().flex_1().child(label(name).truncate()).child(small(detail)))
                .children(sleepable.then(|| pill("Sleep Tab", move || act(ui, |b| b.sleep_tab(key)))))
                .into_element()
        })
        .collect();
    popover()
        .child(super::panels::heading(ui, title("Memory")))
        .child(note("Memory and CPU of each tab's web content process over the last minute, read from /proc. Tabs WebKitGTK runs in one process each show its total; a tab whose process could not be told apart is not measured."))
        .child(view().flex_col().children(items))
        .into_element()
}

pub fn reading_list_panel(ui: Ui) -> AnyElement {
    let items: Vec<AnyElement> = read(ui, Browser::reading_list)
        .unwrap_or_default()
        .into_iter()
        .map(|b| {
            let url = b.url.clone().unwrap_or_default();
            let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
            let open = url.clone();
            let name = if b.title.is_empty() { url.clone() } else { b.title.clone() };
            super::panels::library_row(ui, Some(SharedString::from(b.title.clone())), &host, name, move || {
                ui.panel.set(Panel::None);
                let url = open.clone();
                act(ui, move |br| {
                    br.new_tab(&url);
                });
            })
            .child(super::panels::copy_link(ui, url))
            .child(crate::controls::icon_button(icons::TRASH, "Remove", move || act(ui, |br| br.remove_from_reading_list(b.id))))
            .into_element()
        })
        .collect();
    let count = items.len();
    let empty = items.is_empty().then(|| note("Nothing saved yet. Save for Later files the page here."));
    popover()
        .child(super::panels::heading(
            ui,
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_1()
                .child(super::panels::counted("Reading List", count))
                .child(pill("History", move || super::show(ui, Panel::History))),
        ))
        .children(empty)
        .child(view().flex_col().gap(px(2.)).children(items))
        .into_element()
}
