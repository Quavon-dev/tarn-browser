use browser_core::bookmarks;
use browser_core::text::bounded_title;
use nsh_core::prelude::*;
use store::{Bookmark, Schedule};

use super::panels::{popover, section};
use super::tools::{line, note};
use super::{Panel, Ui, act, read, site_tile};
use crate::browser::Browser;
use crate::browser::tools::READING_LIST;
use crate::controls::{accent_pill, label, pill, small, title};
use crate::icons;
use crate::memory;
use crate::theme::Theme;
use browser_core::closed::ARCHIVE_FOLDER;

const MAX_FOUND: usize = 200;
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
    let key = ui.snap.with_untracked(|s| s.space);
    body = body.children(key.map(|key| super::profiles::space_actions(ui, key)));
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
    .child(super::profiles::switcher(ui))
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
        link("Bookmarks", Panel::Bookmarks(None)),
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
        .child(super::panels::heading(
            ui,
            view().flex_row().items_center().flex_1().child(title("Memory")).child(view().flex_1()).child(pill("Sleep All Inactive Tabs", move || act(ui, Browser::sleep_inactive_tabs))),
        ))
        .child(note("Memory and CPU of each tab's web content process over the last minute, read from /proc. Tabs WebKitGTK runs in one process each show its total; a tab whose process could not be told apart is not measured."))
        .child(view().flex_col().children(items))
        .into_element()
}

pub fn reading_list_panel(ui: Ui) -> AnyElement {
    folder_panel(ui, READING_LIST, "Nothing saved yet. Save for Later files the page here.", ("Archived Tabs", Panel::Archived))
}

pub fn archived_panel(ui: Ui) -> AnyElement {
    folder_panel(ui, ARCHIVE_FOLDER, "Nothing archived. Settings › Tabs can close tabs you have not opened for a while.", ("Reading List", Panel::ReadingList))
}

/// The pages in the bookmark folder `name`, to open or remove; `other` names the sibling panel.
fn folder_panel(ui: Ui, name: &'static str, empty: &'static str, other: (&'static str, Panel)) -> AnyElement {
    let items: Vec<AnyElement> = read(ui, |b| b.folder_pages(name))
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
            .child(crate::controls::icon_button(icons::TRASH, "Remove", move || act(ui, |br| br.remove_bookmark(b.id))))
            .into_element()
        })
        .collect();
    let count = items.len();
    let empty = items.is_empty().then(|| note(empty));
    popover()
        .child(super::panels::heading(
            ui,
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_1()
                .child(super::panels::counted(name, count))
                .child(pill(other.0, move || super::show(ui, other.1)))
                .child(pill("Bookmarks", move || super::show(ui, Panel::Bookmarks(None))))
                .child(pill("History", move || super::show(ui, Panel::History))),
        ))
        .children(empty)
        .child(view().flex_col().gap(px(2.)).children(items))
        .into_element()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MarkEdit {
    Rename(store::Id),
    Move(store::Id),
}

pub fn open_bookmarks(ui: Ui, folder: Option<store::Id>, edit: Option<store::Id>) {
    if ui.panel.get_untracked() != Panel::Bookmarks(folder) {
        super::show(ui, Panel::Bookmarks(folder));
    }
    ui.mark_query.set(String::new());
    ui.mark_edit.set(edit.map(MarkEdit::Rename));
    act(ui, |_| {});
}

pub fn bookmark_tab(ui: Ui, key: Option<browser_core::tabs::TabKey>) {
    let mut id = None;
    act(ui, |b| id = b.bookmark_tab(key));
    let Some(id) = id else { return };
    let parent = read(ui, |b| b.bookmarks().into_iter().find(|m| m.id == id).and_then(|m| m.parent_id)).flatten();
    open_bookmarks(ui, parent, Some(id));
}

pub fn bookmark_all_tabs(ui: Ui) {
    let mut folder = None;
    act(ui, |b| folder = b.bookmark_all_tabs());
    if folder.is_some() {
        open_bookmarks(ui, None, folder);
    }
}

fn mark_row(ui: Ui, marks: &[Bookmark], mark: &Bookmark, edit: Option<MarkEdit>, actions: bool) -> AnyElement {
    let (id, parent) = (mark.id, mark.parent_id);
    let name = bookmarks::title_or_host(&mark.title, mark.url.as_deref().unwrap_or_default());
    let edit_button = |glyph: &'static str, tip: &'static str, next: MarkEdit| {
        crate::controls::icon_button(glyph, tip, move || {
            ui.mark_edit.set(if ui.mark_edit.get_untracked() == Some(next) { None } else { Some(next) });
        })
    };
    let row = match &mark.url {
        Some(url) => {
            let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
            let open = url.clone();
            super::panels::library_row(ui, Some(SharedString::from(name.clone())), &host, name.clone(), move || {
                ui.panel.set(Panel::None);
                let url = open.clone();
                act(ui, move |b| b.open_bookmark(&url));
            })
            .child(super::panels::copy_link(ui, "Copy Link", url.clone()))
        }
        None => crate::controls::row(Some(SharedString::from(name.clone())), move || super::show(ui, Panel::Bookmarks(Some(id))))
            .h(px(30.))
            .child(view().w(px(16.)).items_center().text_color(Theme::muted()).child(icons::icon(icons::FOLDER, 15.)))
            .child(label(name.clone()).flex_1().truncate()),
    };
    if !actions {
        return row.into_element();
    }
    let alone = mark.url.is_some() || !marks.iter().any(|m| m.parent_id == Some(id));
    let deletion = crate::controls::icon_button(icons::TRASH, "Delete", move || match alone {
        true => act(ui, |b| b.delete_bookmark(id)),
        false => {
            ui.confirm.set(Some(format!("bookmark {id}")));
            act(ui, |_| {});
        }
    });
    let asked = (!alone && ui.confirm.with_untracked(|c| c.as_deref() == Some(format!("bookmark {id}").as_str()))).then(|| {
        view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(note(format!("Delete “{name}” and everything in it?")))
            .child(crate::controls::accent_pill("Delete", move || {
                ui.confirm.set(None);
                act(ui, |b| b.delete_bookmark(id));
            }))
            .child(pill("Cancel", move || {
                ui.confirm.set(None);
                act(ui, |_| {});
            }))
    });
    let row = row
        .child(edit_button(icons::PENCIL, "Rename", MarkEdit::Rename(id)))
        .child(edit_button(icons::FOLDER_OPEN, "Move to Folder", MarkEdit::Move(id)))
        .child(deletion);
    let editor = match edit {
        Some(MarkEdit::Rename(e)) if e == id => Some(super::panels::rename_row(ui, format!("bookmark {id}"), &bounded_title(&mark.title), move |title| {
            ui.mark_edit.set(None);
            act(ui, |b| {
                let shown = b.bookmarks().into_iter().find(|m| m.id == id).map(|m| bounded_title(&m.title));
                if shown.as_deref() != Some(title.trim()) {
                    b.rename_bookmark(id, &title);
                }
            });
        })),
        Some(MarkEdit::Move(e)) if e == id => {
            let to = move |target: Option<store::Id>| {
                move || {
                    ui.mark_edit.set(None);
                    act(ui, |b| b.move_bookmark(id, target));
                }
            };
            let mut targets = vec![pill("Bookmarks", to(None)).disabled(parent.is_none()).into_element()];
            targets.extend(
                bookmarks::move_targets(marks, id)
                    .into_iter()
                    .map(|f| pill(bounded_title(&f.title), to(Some(f.id))).disabled(parent == Some(f.id)).into_element()),
            );
            Some(view().flex_col().gap_1().child(note("Move to folder")).child(view().flex_row().flex_wrap().gap_2().children(targets)))
        }
        _ => asked,
    };
    view().flex_col().child(row).children(editor.map(|e| e.pl(px(34.)).py_1())).into_element()
}

fn mark_list(ui: Ui, folder: Option<store::Id>) -> AnyElement {
    dynamic(move || {
        ui.rev.get();
        let query = ui.mark_query.get();
        let edit = ui.mark_edit.get();
        let marks = untracked(|| read(ui, Browser::bookmarks)).unwrap_or_default();
        let rows = |list: Vec<&Bookmark>, actions: bool| list.into_iter().map(|m| mark_row(ui, &marks, m, edit, actions)).collect::<Vec<_>>();
        let query = query.trim();
        let mut out = view().flex_col().gap(px(2.));
        if !query.is_empty() {
            let found = rows(bookmarks::search(&marks, query, MAX_FOUND), true);
            let none = found.is_empty().then(|| note(format!("No results for “{}”", bounded_title(query))));
            return Some(out.children(none).children(found));
        }
        let recent = if folder.is_none() { rows(bookmarks::recent(&marks, bookmarks::RECENT), false) } else { Vec::new() };
        if !recent.is_empty() {
            out = out.child(section("Recent")).children(recent).child(section("All Bookmarks"));
        }
        let inside = rows(marks.iter().filter(|m| m.parent_id == folder).collect(), true);
        let empty = inside.is_empty().then(|| note(if folder.is_none() { "No bookmarks yet" } else { "This folder is empty" }));
        Some(out.children(empty).children(inside))
    })
}

pub fn bookmarks_panel(ui: Ui, folder: Option<store::Id>) -> AnyElement {
    let marks = read(ui, Browser::bookmarks).unwrap_or_default();
    let current = folder.and_then(|id| marks.iter().find(|m| m.id == id));
    let name = current.map_or_else(|| "Bookmarks".to_owned(), |m| bounded_title(&m.title));
    let parent = current.and_then(|m| m.parent_id);
    let back = folder.is_some().then(|| crate::controls::icon_button(icons::CHEVRON_LEFT, "Back", move || super::show(ui, Panel::Bookmarks(parent))));
    let count = marks.iter().filter(|m| m.parent_id == folder).count();
    let new_folder = pill(bookmarks::NEW_FOLDER, move || {
        let mut id = None;
        act(ui, |b| id = b.new_bookmark_folder(folder));
        ui.mark_query.set(String::new());
        ui.mark_edit.set(id.map(MarkEdit::Rename));
        act(ui, |_| {});
    });
    popover()
        .child(super::panels::heading(
            ui,
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_1()
                .children(back)
                .child(super::panels::counted(&name, count))
                .child(new_folder)
                .child(pill("Bookmark All Tabs", move || bookmark_all_tabs(ui)))
                .child(pill("History", move || super::show(ui, Panel::History))),
        ))
        .child(super::text_field(ui.mark_query, "Search Bookmarks", 260.))
        .child(mark_list(ui, folder))
        .into_element()
}
