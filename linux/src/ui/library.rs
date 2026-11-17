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
    let renaming = ui
        .rename_for
        .with_untracked(|t| t.strip_prefix("snapshot ").and_then(|id| id.parse::<store::Id>().ok()))
        .filter(|id| snapshots.iter().any(|s| s.id == *id));
    let mut body = popover().child(super::panels::heading(ui, title(format!("Space “{name}”")).truncate()));
    if renaming.is_none() {
        body = body.child(super::panels::rename_row(ui, format!("space {:?}", ui.snap.with_untracked(|s| s.space)), &name, move |name| {
            act(ui, |b| b.rename_space(&name))
        }));
    }
    body = match schedule {
        Some(schedule) => body.child(section("Schedule")).child(schedule_rows(ui, schedule)),
        None => body.child(note("A private space has no schedule or snapshots.")),
    };
    let rows: Vec<AnyElement> = snapshots
        .into_iter()
        .map(|saved| {
            let id = saved.id;
            if renaming == Some(id) {
                return snapshot_rename(ui, id, &saved.name).into_element();
            }
            let (a, b, c, name) = (saved.clone(), saved.clone(), saved.clone(), saved.name.clone());
            line(saved.name.clone())
                .child(small(crate::browser::spaces::date(saved.created_at)))
                .child(pill("Restore in New Space", move || act(ui, |br| br.restore_snapshot(&a, true))))
                .child(pill("Restore in This Space", move || act(ui, |br| br.restore_snapshot(&b, false))))
                .child(pill("Rename", move || {
                    ui.rename_for.set(format!("snapshot {id}"));
                    ui.rename.set(name.clone());
                    act(ui, |_| {});
                }))
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

fn snapshot_rename(ui: Ui, id: store::Id, name: &str) -> View {
    let done = move || {
        ui.rename_for.set(String::new());
        ui.rename.set(String::new());
    };
    super::panels::rename_row(ui, format!("snapshot {id}"), name, move |name| {
        done();
        act(ui, |b| b.rename_snapshot(id, &name));
    })
    .child(pill("Cancel", move || {
        done();
        act(ui, |_| {});
    }))
}

fn section_rows(name: &'static str, rows: Vec<AnyElement>) -> Option<View> {
    (!rows.is_empty()).then(|| view().flex_col().gap(px(1.)).child(section(name)).children(rows))
}

fn library_links(ui: Ui, reading_list: bool, private: bool) -> View {
    let link = |name: &'static str, panel: Panel| Some(super::panels::menu_row(name, None, move || super::show(ui, panel)));
    let links = [
        link("History", Panel::History),
        reading_list.then(|| link("Reading List", Panel::ReadingList)).flatten(),
        link("Downloads", Panel::Downloads),
        (!private).then(|| link("Snapshots", Panel::Space)).flatten(),
        link("Memory", Panel::Memory),
    ];
    view().flex_col().child(section("Library")).child(crate::controls::group().children(links.into_iter().flatten()))
}

fn library_results(ui: Ui, history: &[store::HistoryEntry], query: &str) -> View {
    let Some((found, reading_list, private)) = read(ui, |b| (b.library_search(query, history), b.has_reading_list(), b.model.private())) else {
        return view();
    };
    let nothing = !query.is_empty() && found.is_empty();
    let tabs: Vec<AnyElement> = found
        .tabs
        .into_iter()
        .map(|(key, name, url)| {
            let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
            super::panels::library_row(ui, Some(SharedString::from(name.clone())), &host, name, move || {
                ui.panel.set(Panel::None);
                act(ui, |b| b.activate(key));
            })
            .into_element()
        })
        .collect();
    let open = (query.is_empty() || !tabs.is_empty()).then(|| {
        let empty = (query.is_empty() && tabs.is_empty()).then(|| note("No open tabs"));
        view()
            .flex_col()
            .gap(px(1.))
            .child(view().flex_row().items_baseline().gap_2().child(section("Open Tabs")).child(small(found.tab_count.to_string()).text_color(Theme::faint())))
            .children(empty)
            .children(tabs)
    });
    let links = |rows: Vec<crate::browser::library::Link>| -> Vec<AnyElement> { rows.into_iter().map(|link| super::panels::link_row(ui, link)).collect() };
    let downloads: Vec<AnyElement> = found.downloads.into_iter().map(|d| super::panels::saved_download(ui, d)).collect();
    view()
        .flex_col()
        .gap_3()
        .children(open)
        .children(section_rows("Bookmarks", links(found.bookmarks)))
        .children(section_rows("History", links(found.history)))
        .children(section_rows("Downloads", downloads))
        .children(nothing.then(|| super::panels::no_results(query)))
        .child(library_links(ui, reading_list, private))
}

pub fn library_panel(ui: Ui) -> AnyElement {
    let history = std::rc::Rc::new(read(ui, Browser::library_history).unwrap_or_default());
    let results = dynamic(move || {
        let query = ui.library_query.get();
        Some(library_results(ui, &history, query.trim()))
    });
    popover()
        .child(super::panels::heading(
            ui,
            view().flex_row().items_center().gap_2().flex_1().child(title("Library").flex_1()).child(super::panels::search_field(ui)),
        ))
        .child(results)
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
            .child(super::panels::copy_link(ui, "Copy Link", url))
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
