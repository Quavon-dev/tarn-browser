use nsh_core::prelude::*;
use store::Schedule;

use super::panels::{popover, section};
use super::tools::{line, note};
use super::{Panel, Ui, act, read, site_tile};
use crate::browser::Browser;
use crate::controls::{accent_pill, label, pill, small, title};
use crate::icons;
use crate::memory;
use crate::theme::Theme;

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

fn schedule_rows(ui: Ui, schedule: Schedule) -> View {
    let set = move |f: fn(&mut Schedule, u8), value: u8| {
        let mut next = schedule;
        f(&mut next, value);
        act(ui, move |b| b.set_space_schedule(next));
    };
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

pub fn space_panel(ui: Ui) -> AnyElement {
    let Some((name, schedule, snapshots)) = read(ui, |b| (ui.snap.with(|s| s.space_name.clone()), b.space_schedule(), b.snapshots())) else {
        return view().into_element();
    };
    let mut body = popover().child(super::panels::heading(ui, title(format!("Space “{name}”")).truncate())).child(super::panels::rename_row(
        ui,
        format!("space {:?}", ui.snap.with_untracked(|s| s.space)),
        &name,
        move |name| act(ui, |b| b.rename_space(&name)),
    ));
    let key = ui.snap.with_untracked(|s| s.space);
    body = body.children(key.map(|key| super::profiles::space_actions(ui, key)));
    body = match schedule {
        Some(schedule) => body.child(section("Schedule")).child(schedule_rows(ui, schedule)),
        None => body.child(note("A private space has no schedule or snapshots.")),
    };
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
    .child(super::profiles::switcher(ui))
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
