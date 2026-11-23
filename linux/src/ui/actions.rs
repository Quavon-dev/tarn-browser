use browser_core::palette::{self, Action, Item};
use browser_core::tabs::TabKey;
use nsh_core::prelude::*;

use super::{Ui, act, read, tools};
use crate::controls::{icon_button, label};
use crate::icons;

/// The actions page of a command bar row.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    label: String,
    item: Item,
    tab: Option<TabKey>,
    /// The `model.spaces` index of each `Action::MoveToSpace` target.
    targets: Vec<usize>,
    rows: Vec<(String, Action)>,
    keys: Option<String>,
    /// The list row picked again on the way back.
    back: usize,
}

/// The picked row of the list `query` shows: its name, item and tab.
fn picked_row(ui: Ui, query: &str) -> Option<(String, Item, Option<TabKey>)> {
    let row = ui.picked.get_untracked().checked_sub(1)?;
    if palette::sectioned(query) {
        let (_, entry, _) = ui.sections.with_untracked(|s| s.get(row).cloned())?;
        let tab = match entry.item {
            Item::Tab(id) => Some(TabKey { private: read(ui, |b| b.model.private())?, id }),
            _ => None,
        };
        return Some((entry.label, entry.item, tab));
    }
    let commands = tools::commands(ui, query);
    if let Some((name, command)) = commands.get(row) {
        return Some((name.clone(), Item::Command(tools::palette_command(command)?), None));
    }
    if tools::listing(query) {
        return None;
    }
    let row = row - commands.len();
    if let Some(t) = ui.tabs.with_untracked(|t| t.get(row).cloned()) {
        return Some((t.title, Item::Tab(t.key.id), Some(t.key)));
    }
    let (title, url) = ui.suggest.with_untracked(|s| s.get(row - ui.tabs.with_untracked(Vec::len)).cloned())?;
    Some((if title.is_empty() { url.clone() } else { title }, Item::History(url), None))
}

/// Opens the actions page of the picked row, if it has one.
pub fn open(ui: Ui, query: Signal<String>) {
    let Some((label, item, tab)) = picked_row(ui, &query.get_untracked()) else { return };
    // A tab of another space can be switched to and copied, but closed or moved only from its space.
    let here = tab.and_then(|key| read(ui, |b| b.model.tab(key).is_some())).unwrap_or(true);
    let targets = tab.filter(|_| here).and_then(|key| read(ui, |b| b.move_targets(key))).unwrap_or_default();
    let names: Vec<&str> = targets.iter().map(|(_, name)| name.as_str()).collect();
    let mut rows = palette::actions(&item, &names);
    rows.retain(|(_, a)| here || *a != Action::Close);
    let keys = match item {
        Item::Command(c) => tools::section_command(ui, c).and_then(|(_, keys)| keys),
        _ => None,
    };
    let back = ui.picked.get_untracked();
    ui.actions.set(Some(Page { label, item, tab, targets: targets.into_iter().map(|(ix, _)| ix).collect(), rows, keys, back }));
    ui.picked.set(0);
}

/// Leaves the actions page for the list; false when none is open.
pub fn close(ui: Ui) -> bool {
    let Some(page) = ui.actions.get_untracked() else { return false };
    ui.actions.set(None);
    ui.picked.set(page.back);
    true
}

fn step(ui: Ui, key: &str) {
    let rows = ui.actions.with_untracked(|p| p.as_ref().map_or(0, |p| p.rows.len()));
    ui.picked.update(|p| {
        *p = match key {
            "pageup" => 0,
            "pagedown" => rows.saturating_sub(1),
            _ => browser_core::suggest::step_selection(*p, rows, key == "down"),
        }
    });
}

/// Runs row `ix` of the open actions page.
pub fn run(ui: Ui, ix: usize) {
    let Some(page) = ui.actions.get_untracked() else { return };
    let Some(&(_, action)) = page.rows.get(ix) else { return };
    let url = match &page.item {
        Item::Bookmark(url) | Item::History(url) => Some(url.clone()),
        Item::Tab(_) => page.tab.and_then(|k| read(ui, |b| b.core.store(k.private).tab(k.id).ok().map(|t| t.url)).flatten()),
        Item::Command(_) => None,
    };
    ui.actions.set(None);
    match action {
        Action::Run => super::run_entry(ui, &page.item),
        Action::Switch => page.tab.into_iter().for_each(|key| super::switch_to_tab(ui, key)),
        Action::Open => url.into_iter().for_each(|url| super::go(ui, url)),
        Action::OpenInNewTab => url.into_iter().for_each(|url| {
            ui.new_tab_card.set(true);
            super::go(ui, url);
        }),
        Action::CopyLink => {
            super::end_palette(ui);
            url.into_iter().for_each(|url| tools::run(ui, tools::Command::Copy(url)));
        }
        Action::Close => {
            super::end_palette(ui);
            page.tab.into_iter().for_each(|key| act(ui, |b| b.close_tab(key)));
        }
        Action::MoveToSpace(target) => {
            super::end_palette(ui);
            if let (Some(key), Some(&ix)) = (page.tab, page.targets.get(target)) {
                act(ui, |b| b.move_to_space(key, ix));
            }
        }
        Action::RemoveFromHistory => {
            let Some(url) = url else { return };
            super::remove_history(ui, url.clone());
            ui.suggest.update(|rows| rows.retain(|(_, u)| *u != url));
            if !ui.sections.with_untracked(Vec::is_empty) {
                super::fill_sections(ui);
            }
            ui.picked.set(0);
        }
    }
}

/// The open actions page: the row's name with a way back, then its actions.
pub fn rows(ui: Ui, address: bool) -> AnyElement {
    for key in ["up", "down", "pageup", "pagedown", "left"] {
        let options = KeyOptions { when: nsh_core::keymap::When::Always, repeat: key != "left", pass_through: key == "left" };
        on_key_with(key, options, move |_| {
            if ui.editing.get_untracked() == address && ui.actions.with_untracked(Option::is_some) {
                if key == "left" {
                    close(ui);
                } else {
                    step(ui, key);
                }
            }
        });
    }
    dynamic(move || {
        let page = ui.actions.get()?;
        let head = view()
            .flex_row()
            .items_center()
            .gap_2()
            .px(px(8.))
            .pb_1()
            .child(icon_button(icons::CHEVRON_LEFT, "Back to Results", move || {
                close(ui);
            }))
            .child(label(page.label.clone()).flex_1().truncate());
        let items = page.rows.into_iter().enumerate().map(move |(ix, (name, action))| {
            let keys = page.keys.clone().filter(|_| action == Action::Run);
            super::suggestion_row(name.clone(), move || run(ui, ix))
                .bg(move || super::picked_fill(ui, ix))
                .child(super::row_glyph(ui, icons::glyph(browser_core::command::glyph(&name)), move || ix))
                .child(label(name).flex_1().truncate())
                .children(keys.map(|k| tarn_nsh_shared::controls::key_caps(&k)))
        });
        Some(view().flex_col().gap(px(2.)).py(px(6.)).child(head).children(items))
    })
    .into_element()
}
