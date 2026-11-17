use std::time::Duration;

use browser_core::command::rank_commands;
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use super::{Panel, Ui, act, read, show};
use crate::browser::Browser;
use crate::browser::spaces::SpaceCommand;
use crate::controls::{accent_pill, body, icon_button, label, pill, small};
use crate::icons;
use crate::shortcuts::Action;
use crate::speech::{self, SPEEDS};
use crate::theme::{ROW_HEIGHT, Theme};
use nsh_core::{RevealOptions, ScrollAlignment};
use store::TabKind;

const TOAST_TIME: Duration = Duration::from_secs(5);
/// A toast with a button stays longer, so there is time to reach it.
const ACTION_TIME: Duration = Duration::from_secs(10);
/// How many command rows the field lists before a `>` list scrolls.
const LISTED: f32 = 8.;
const READER_SIZES: [f32; 6] = [14., 16., 18., 20., 23., 26.];
const READER_WIDTHS: [(f32, &str); 2] = [(620., "Narrow"), (820., "Wide")];
const QR_SIZE: f32 = 204.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    CloseDuplicates,
    ZapElement,
    Reader,
    SaveForLater,
    BurnerTab,
    FindInTabs,
    CopyLink,
    CopyCleanLink,
    ShowQrCode,
    SavePdf,
    Print,
    SaveSnapshot,
    Snapshots,
    Memory,
    ReadingList,
    NewWindow,
    Key(Action),
    CloseSplit,
    Space(SpaceCommand),
}

const COMMANDS: [(&str, &str, Command); 24] = [
    ("Reopen Closed Tab", "undo restore", Command::Key(Action::ReopenClosedTab)),
    ("Pin or Unpin Tab", "", Command::Key(Action::TogglePin)),
    ("Close Duplicate Tabs", "same page dedupe clean up", Command::CloseDuplicates),
    ("Zap Element", "hide remove element block", Command::ZapElement),
    ("Reader", "article reading view text", Command::Reader),
    ("Save for Later", "reading list read later bookmark", Command::SaveForLater),
    ("Burner Tab", "throwaway incognito temporary private", Command::BurnerTab),
    ("Find in Tabs", "search text all pages", Command::FindInTabs),
    ("Copy Link", "url address", Command::CopyLink),
    ("Copy Clean Link", "url tracking parameters utm strip", Command::CopyCleanLink),
    ("Show QR Code", "phone scan link", Command::ShowQrCode),
    ("Save Page as PDF", "export print", Command::SavePdf),
    ("Save Space as Snapshot", "session tabs save", Command::SaveSnapshot),
    ("Restore Snapshot…", "session tabs open", Command::Snapshots),
    ("Memory", "process cpu ram usage sleep", Command::Memory),
    ("Reading List", "read later saved pages", Command::ReadingList),
    ("New Window", "open another browser window", Command::NewWindow),
    ("Open or Close Split View", "side by side tabs grid four", Command::Key(Action::ToggleSplitView)),
    ("Focus Next Pane", "split view switch", Command::Key(Action::FocusNextPane)),
    ("Focus Previous Pane", "split view switch", Command::Key(Action::FocusPreviousPane)),
    ("Swap Split Panes", "split view exchange move", Command::Key(Action::SwapPanes)),
    ("Rotate Split Layout", "split view stacked side by side rows columns", Command::Key(Action::RotateSplitLayout)),
    ("Remove Tab from Split View", "split pane leave", Command::Key(Action::RemoveFromSplitView)),
    ("Close Split View", "split panes end separate", Command::CloseSplit),
];

/// Whether `query` asks for the command list: every command it names, and nothing else.
pub fn listing(query: &str) -> bool {
    query.trim_start().starts_with('>')
}

/// The commands `query` names, leaving out those with nothing to act on, and those that toggle
/// named by what they do now.
pub fn commands(ui: Ui, query: &str) -> Vec<(String, Command)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let Some((spaces, pinned, split, reopenable, duplicates)) = read(ui, |b| {
        let active = b.model.active;
        (
            b.space_commands(),
            active.and_then(|k| b.model.tab(k)).is_some_and(|t| t.kind != TabKind::Ephemeral),
            active.is_some_and(|k| b.splits.contains(k)),
            b.model.closed_tabs.newest().is_some(),
            !b.duplicate_tabs().is_empty(),
        )
    }) else {
        return Vec::new();
    };
    let shown = |command: &Command| match command {
        Command::CloseDuplicates => duplicates,
        Command::Key(Action::ReopenClosedTab) => reopenable,
        Command::CloseSplit
        | Command::Key(Action::FocusNextPane | Action::FocusPreviousPane | Action::SwapPanes | Action::RotateSplitLayout | Action::RemoveFromSplitView) => {
            split
        }
        _ => true,
    };
    let mut rows: Vec<(String, &str, Command)> = COMMANDS.iter().filter(|(_, _, c)| shown(c)).map(|(l, k, c)| ((*l).to_owned(), *k, *c)).collect();
    rows.extend(spaces.into_iter().map(|(l, c)| (l, "space workspace", Command::Space(c))));
    let ranked = rank_commands(query, rows.iter().map(|(l, k, _)| (l.as_str(), *k)));
    let named = |(label, _, command): &(String, &str, Command)| match command {
        Command::Key(Action::TogglePin) => (if pinned { "Unpin Tab" } else { "Pin Tab" }).to_owned(),
        Command::Key(Action::ToggleSplitView) => (if split { "Close Split View" } else { "Open in Split View" }).to_owned(),
        _ => label.clone(),
    };
    let room = if listing(query) { rows.len() } else { 4 };
    ranked.into_iter().take(room).map(|ix| (named(&rows[ix]), rows[ix].2)).collect()
}

pub fn run(ui: Ui, command: Command) {
    ui.panel.set(Panel::None);
    match command {
        Command::CloseDuplicates => act(ui, Browser::close_duplicates),
        Command::ZapElement => act(ui, Browser::zap_element),
        Command::Reader => act(ui, Browser::toggle_reader),
        Command::SaveForLater => act(ui, Browser::save_for_later),
        Command::BurnerTab => act(ui, Browser::open_burner),
        Command::FindInTabs => open_find_tabs(ui),
        Command::CopyLink => copy_link(ui, false),
        Command::CopyCleanLink => copy_link(ui, true),
        Command::ShowQrCode => show_qr(ui),
        Command::SavePdf => save_pdf(ui),
        Command::Print => act(ui, Browser::print),
        Command::SaveSnapshot => act(ui, Browser::save_snapshot),
        Command::Snapshots => show(ui, Panel::Space),
        Command::Memory => show(ui, Panel::Memory),
        Command::ReadingList => show(ui, Panel::ReadingList),
        Command::NewWindow => super::new_window(ui),
        Command::Key(action) => super::run_action(ui, action),
        Command::CloseSplit => act(ui, |b| b.model.active.into_iter().for_each(|k| b.close_split(k))),
        Command::Space(c) => act(ui, |b| b.run_space_command(c)),
    }
}

/// Runs a command row of the address field or the start card.
pub fn pick(ui: Ui, command: Command) {
    ui.editing.set(false);
    ui.address_ref.blur();
    ui.command.set(String::new());
    run(ui, command);
}

pub fn copy_link(ui: Ui, clean: bool) {
    let mut link = None;
    act(ui, |b| link = b.copy_link(clean));
    if let Some(link) = link {
        set_clipboard(link);
    }
}

pub fn show_qr(ui: Ui) {
    let mut code = None;
    act(ui, |b| code = b.qr_code());
    if code.is_some() {
        ui.qr.set(code);
        ui.panel.set(Panel::Qr);
        act(ui, |_| {});
    }
}

fn save_pdf(ui: Ui) {
    let Some((key, url, name)) = read(ui, Browser::pdf_name).flatten() else { return };
    let dialog = SaveDialog::new().file_name(name).filter(FileFilter::new("PDF document", ["pdf"])).directory(crate::webview::session::downloads_dir());
    ui.root.run(|| {
        drop(spawn(async move {
            if let Ok(Some(path)) = pick_save_path(dialog).await {
                act(ui, |b| b.save_pdf(key, &url, &path));
            }
        }))
    });
}

pub fn open_find_tabs(ui: Ui) {
    ui.find_tabs.set(true);
    ui.address.set(String::new());
    ui.editing.set(true);
    ui.address_ref.focus();
    act(ui, |b| b.find_in_tabs(""));
}

pub fn expire_toast(ui: Ui) {
    let Some(serial) = ui.snap.with_untracked(|s| s.toast.as_ref().map(|t| t.serial)) else { return };
    if ui.toast_timer.get_untracked() == serial {
        return;
    }
    ui.toast_timer.set(serial);
    let time = if ui.snap.with_untracked(|s| s.toast.as_ref().is_some_and(|t| t.action.is_some())) { ACTION_TIME } else { TOAST_TIME };
    ui.root.run(|| drop(set_timeout(time, move || act(ui, |b| b.expire_toast(serial)))));
}

pub fn toast_bar(ui: Ui) -> AnyElement {
    dynamic(move || {
        let toast = ui.snap.with(|s| s.toast.clone())?;
        let action = toast.action.map(|_| accent_pill("Undo", move || act(ui, Browser::take_toast_action)));
        Some(
            view().flex_row().justify_center().child(
                tarn_nsh_shared::controls::toast(toast.tone)
                    .mt_2()
                    .pr_1()
                    .max_w(px(560.))
                    .child(label(toast.text).flex_shrink(1.).truncate())
                    .children(action)
                    .child(crate::controls::icon_button_at(icons::CLOSE, "Dismiss", Placement::Left, move || act(ui, |b| b.expire_toast(toast.serial)))),
            ),
        )
    })
}

fn action(command: Command) -> Option<Action> {
    match command {
        Command::Key(action) => Some(action),
        Command::CopyLink => Some(Action::CopyLink),
        Command::CopyCleanLink => Some(Action::CopyCleanLink),
        Command::Reader => Some(Action::ToggleReader),
        Command::Print => Some(Action::Print),
        Command::BurnerTab => Some(Action::BurnerTab),
        Command::FindInTabs => Some(Action::FindInTabs),
        Command::NewWindow => Some(Action::NewWindow),
        Command::CloseSplit => Some(Action::ToggleSplitView),
        _ => None,
    }
}

pub fn command_rows(ui: Ui, query: Signal<String>) -> AnyElement {
    dynamic(move || {
        let rows = commands(ui, &query.get());
        let shown = !rows.is_empty();
        let items: Vec<AnyElement> = rows
            .into_iter()
            .enumerate()
            .map(|(ix, (name, command))| {
                let at = element_ref();
                effect(move || {
                    if ui.picked.get() == ix + 1 {
                        at.reveal(RevealOptions::default().vertical(ScrollAlignment::Nearest));
                    }
                });
                let keys = action(command).and_then(|a| super::shortcut_of(ui, a));
                super::suggestion_row(name.clone(), move || pick(ui, command))
                    .element_ref(&at)
                    .bg(move || super::picked_fill(ui, ix + 1))
                    .child(super::row_glyph(ui, icons::glyph(browser_core::command::glyph(&name)), move || ix + 1))
                    .child(label(name).flex_1().truncate())
                    .children(keys.map(|k| tarn_nsh_shared::controls::key_caps(&k)))
                    .into_element()
            })
            .collect();
        view()
            .flex_col()
            .children(shown.then(|| super::row_section("Commands")))
            .child(scroll().max_h(px(LISTED * (ROW_HEIGHT + 8.))).child(view().flex_col().gap(px(2.)).children(items)))
    })
}

pub fn find_rows(ui: Ui) -> View {
    let rows = each(
        move || ui.snap.with(|s| s.find_rows.iter().cloned().enumerate().collect::<Vec<_>>()),
        |(ix, r)| format!("{ix}-{}-{}", r.label, r.key.id),
        move |item, _| {
            let (_, row) = item.get_untracked();
            let key = row.key;
            super::suggestion_row(row.label.clone(), move || {
                let mut query = String::new();
                act(ui, |b| query = b.open_find_row(key));
                close_find_tabs(ui);
                ui.find.set(query.clone());
                ui.finding.set(true);
                act(ui, |b| b.find_typed(&query));
            })
            .child(tarn_nsh_shared::controls::glyph_tile(icons::SEARCH, 28., Theme::control, Theme::muted))
            .child(label(row.label.clone()).flex_1().truncate())
            .child(super::detail(row.detail.clone()))
        },
    );
    let empty = move || ui.snap.with(|s| s.find_rows.is_empty()) && !ui.address.with(|a| a.trim().is_empty());
    view()
        .flex_col()
        .gap(px(2.))
        .py(px(6.))
        .child(rows)
        .child(when(empty, || view().h(px(ROW_HEIGHT)).px_4().justify_center().child(small("No tab mentions this"))))
}

pub fn close_find_tabs(ui: Ui) {
    ui.find_tabs.set(false);
    ui.editing.set(false);
    ui.address.set(String::new());
    ui.address_ref.blur();
    act(ui, |b| b.find_in_tabs(""));
}

pub fn qr_panel(ui: Ui) -> AnyElement {
    let Some((code, host)) = ui.qr.get_untracked() else { return view().into_element() };
    super::panels::popover()
        .items_center()
        .w(px(260.))
        .self_center()
        .child(super::panels::heading(ui, small("Scan to open on a phone")).w_full())
        .child(view().p_2().rounded(px(Theme::radius_row())).bg(rgb(0xffffff)).child(tarn_nsh_shared::qr::qr_code(
            &code,
            crate::browser::tools::QUIET,
            QR_SIZE,
        )))
        .child(small(host).truncate())
        .into_element()
}

pub fn share_panel(ui: Ui) -> AnyElement {
    let (title, url, host) = ui.snap.with(|s| (s.title.clone(), s.url.clone(), s.host.clone()));
    if host.is_empty() {
        return super::panels::popover().child(super::panels::heading(ui, note("This page has no link to share."))).into_element();
    }
    let header = view()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .pb_2()
        .border_b_1()
        .border_color(Theme::hairline())
        .child(super::site_tile(ui, &host, 28.))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(label(if title.is_empty() { host.clone() } else { title }).font_weight(FontWeight::SEMIBOLD).truncate())
                .child(small(url.trim_start_matches("https://").to_owned()).text_color(Theme::faint()).truncate()),
        )
        .child(icon_button(icons::CLOSE, "Close", move || show(ui, Panel::None)));
    let item = |name: &'static str, command: Command, action: Option<Action>| {
        super::panels::menu_row(name, action.and_then(|a| super::shortcut_of(ui, a)), move || run(ui, command))
    };
    super::panels::popover()
        .gap_1()
        .p_2()
        .child(header.px_2().pt_1())
        .child(item("Copy Link", Command::CopyLink, Some(Action::CopyLink)))
        .child(item("Copy Clean Link", Command::CopyCleanLink, Some(Action::CopyCleanLink)))
        .child(item("Show QR Code", Command::ShowQrCode, None))
        .child(
            view().flex_col().gap_1().pt_1().border_t_1().border_color(Theme::hairline()).child(item("Save Page as PDF", Command::SavePdf, None)).child(item(
                "Print…",
                Command::Print,
                Some(Action::Print),
            )),
        )
        .into_element()
}

pub fn reader_view(ui: Ui) -> AnyElement {
    when(
        move || ui.snap.with(|s| s.reader.is_some()) && ui.panel.get() == Panel::None && !ui.editing.get(),
        move || {
            dynamic(move || {
                let reader = ui.snap.with(|s| s.reader.clone())?;
                let (size, width, speed) = (ui.reader_size.get(), ui.reader_width.get(), ui.reader_speed.get());
                let size_px = READER_SIZES[size];
                let speaking = ui.speaking.get();
                let mut tools = view()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(pill("A−", move || ui.reader_size.update(|s| *s = s.saturating_sub(1))))
                    .child(pill("A+", move || ui.reader_size.update(|s| *s = (*s + 1).min(READER_SIZES.len() - 1))))
                    .child(crate::controls::segmented(
                        READER_WIDTHS.iter().enumerate().map(|(ix, (_, n))| (ix, (*n).to_owned())).collect(),
                        width,
                        move |ix| ui.reader_width.set(ix),
                    ));
                if speech::available() {
                    let full = reader.text();
                    tools = tools
                        .child(pill(if speaking { "Stop" } else { "Read Aloud" }, move || {
                            if ui.speaking.get_untracked() {
                                speech::stop();
                                ui.speaking.set(false);
                            } else if speech::speak(&full, SPEEDS[ui.reader_speed.get_untracked()]) {
                                ui.speaking.set(true);
                            } else {
                                act(ui, |b| b.show_toast_as(browser_core::toast::Tone::Warning, "This computer could not start speaking", None));
                            }
                        }))
                        .child(pill(format!("{}×", SPEEDS[speed]), move || ui.reader_speed.update(|s| *s = (*s + 1) % SPEEDS.len())));
                }
                tools = tools
                    .child(pill("Save for Later", move || act(ui, Browser::save_for_later)))
                    .child(view().flex_1())
                    .child(small("Esc to close"))
                    .child(icon_button(icons::CLOSE, "Close Reader", move || close_reader(ui)));
                let blocks: Vec<AnyElement> = reader
                    .blocks
                    .iter()
                    .map(|(t, heading)| {
                        let block = text(t.clone()).text_size(px(if *heading { size_px * 1.2 } else { size_px })).text_color(Theme::text());
                        if *heading { block.font_weight(FontWeight::SEMIBOLD).pt_2().into_element() } else { block.into_element() }
                    })
                    .collect();
                let article = view()
                    .flex_col()
                    .gap_3()
                    .w(px(READER_WIDTHS[width].0))
                    .self_center()
                    .py_6()
                    .child(text(reader.title.clone()).text_size(px(size_px * 1.6)).font_weight(FontWeight::BOLD).text_color(Theme::text()))
                    .children((!reader.byline.is_empty()).then(|| small(reader.byline.clone())))
                    .children(blocks);
                Some(
                    view()
                        .flex_col()
                        .flex_1()
                        .bg(Theme::page())
                        .child(view().px_4().py_2().border_b_1().border_color(Theme::hairline()).child(tools))
                        .child(scroll().flex_1().child(view().flex_col().px_6().child(article))),
                )
            })
        },
    )
    .into_element()
}

pub fn close_reader(ui: Ui) {
    ui.speaking.set(false);
    act(ui, Browser::close_reader);
}

pub fn note(content: impl Into<String>) -> Text {
    small(content.into()).text_color(Theme::muted())
}

pub fn line(name: impl Into<String>) -> View {
    view().flex_row().items_center().gap_2().min_h(px(32.)).child(body(name.into()).flex_1().truncate())
}
