use std::time::Duration;

use browser_core::palette;
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use super::{Panel, Ui, act, read, show};
use crate::browser::spaces::SpaceCommand;
use crate::browser::{Browser, ToastAction};
use crate::controls::{body, icon_button, label, pill, small};
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

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Copy(String),
    CloseDuplicates,
    ZapElement,
    CaptureArea,
    CaptureElement,
    CaptureFullPage,
    Measure,
    InsertSnippet,
    Reader,
    SaveForLater,
    BurnerTab,
    FindInTabs,
    CopyLink,
    CopyCleanLink,
    CopyMarkdownLink,
    CopyTitle,
    SiteSettings,
    EditSpace,
    ShowQrCode,
    SavePdf,
    Print,
    BookmarkAllTabs,
    SaveSnapshot,
    Snapshots,
    Memory,
    ReadingList,
    Archived,
    ImportBrowserData,
    NewWindow,
    FullScreen,
    Key(Action),
    CloseSplit,
    Space(SpaceCommand),
    DuplicateTab,
    ToggleMute,
    RenameTab,
    ResetTitle,
    ReturnToPinned,
    SetPinnedUrl,
    SleepOthers,
    StopLoading,
    GlassDesign,
    ToastAction,
    Ask,
    AskPage,
}

fn command_for(command: palette::Command) -> Option<Command> {
    use palette::Command as C;
    Some(match command {
        C::Settings => Command::Key(Action::Settings),
        C::OpenFile => Command::Key(Action::OpenFile),
        C::NewGroup => Command::Key(Action::NewGroup),
        C::BookmarkTab => Command::Key(Action::BookmarkTab),
        C::BookmarkAllTabs => Command::BookmarkAllTabs,
        C::ManageBookmarks => Command::Key(Action::ManageBookmarks),
        C::NewSpace => Command::Key(Action::NewSpace),
        C::EditSpace => Command::EditSpace,
        C::PrivateSpace => Command::Key(Action::PrivateSpace),
        C::BurnerTab => Command::BurnerTab,
        C::ReopenClosedTab => Command::Key(Action::ReopenClosedTab),
        C::ToastAction => Command::ToastAction,
        C::ToggleSidebar => Command::Key(Action::ToggleSidebar),
        C::TabsOnTop => Command::Key(Action::ToggleTabsOnTop),
        C::CloseAllTabs => Command::Key(Action::CloseAllTabs),
        C::SaveSnapshot => Command::SaveSnapshot,
        C::Snapshots => Command::Snapshots,
        C::ShowDownloads => Command::Key(Action::ToggleDownloads),
        C::DuplicateTab => Command::DuplicateTab,
        C::CopyLink => Command::CopyLink,
        C::CopyCleanLink => Command::CopyCleanLink,
        C::TogglePin => Command::Key(Action::TogglePin),
        C::ToggleMute => Command::ToggleMute,
        C::ShowHistory => Command::Key(Action::ShowHistory),
        C::RenameTab => Command::RenameTab,
        C::ResetTitle => Command::ResetTitle,
        C::ReturnToPinned => Command::ReturnToPinned,
        C::SetPinnedUrl => Command::SetPinnedUrl,
        C::SleepOthers => Command::SleepOthers,
        C::Memory => Command::Memory,
        C::CloseDuplicates => Command::CloseDuplicates,
        C::SiteSettings => Command::SiteSettings,
        C::ZapElement => Command::ZapElement,
        C::ToggleSplitView => Command::Key(Action::ToggleSplitView),
        C::FocusNextPane => Command::Key(Action::FocusNextPane),
        C::FocusPreviousPane => Command::Key(Action::FocusPreviousPane),
        C::SwapPanes => Command::Key(Action::SwapPanes),
        C::RotateSplitLayout => Command::Key(Action::RotateSplitLayout),
        C::RemoveFromSplitView => Command::Key(Action::RemoveFromSplitView),
        C::CloseSplit => Command::CloseSplit,
        C::Reader => Command::Reader,
        C::SaveForLater => Command::SaveForLater,
        C::ReadingList => Command::ReadingList,
        C::Archived => Command::Archived,
        C::TakeScreenshot => Command::Key(Action::TakeScreenshot),
        C::CaptureArea => Command::CaptureArea,
        C::CaptureElement => Command::CaptureElement,
        C::CaptureFullPage => Command::CaptureFullPage,
        C::Measure => Command::Measure,
        C::InsertSnippet => Command::InsertSnippet,
        C::SavePdf => Command::SavePdf,
        C::CopyMarkdownLink => Command::CopyMarkdownLink,
        C::CopyTitle => Command::CopyTitle,
        C::ShowQrCode => Command::ShowQrCode,
        C::FindInTabs => Command::FindInTabs,
        C::ImportBrowserData => Command::ImportBrowserData,
        C::NewWindow => Command::NewWindow,
        C::FullScreen => Command::FullScreen,
        C::StopLoading => Command::StopLoading,
        C::GlassDesign => Command::GlassDesign,
        C::ReloadFromOrigin => Command::Key(Action::ReloadFromOrigin),
        C::CloseWindow => Command::Key(Action::CloseWindow),
        C::KeyboardShortcuts => Command::Key(Action::ToggleShortcuts),
        C::NewProfile
        | C::EditProfile
        | C::AutoHideSidebar
        | C::CompactToolbar
        | C::SidebarToolbar
        | C::SidebarChats
        | C::ShowLibrary
        | C::ReloadInChromium
        | C::ReloadInWebKit
        | C::PictureInPicture
        | C::ToggleInspector
        | C::MoveInspector
        | C::Share => return None,
        C::Ask => Command::Ask,
        C::AskPage => Command::AskPage,
    })
}

pub use palette::listing;

/// The commands `query` names, leaving out those with nothing to act on, and those that toggle
/// named by what they do now.
pub fn commands(ui: Ui, query: &str) -> Vec<(String, Command)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let Some((state, spaces)) = state(ui) else { return Vec::new() };
    let extra: Vec<(String, &str)> = spaces.iter().map(|(l, _)| (l.clone(), "space workspace")).collect();
    let answers = palette::answers(query).into_iter().map(|a| (a.label, Command::Copy(a.copy)));
    answers
        .chain(palette::rows(query, state, offered, &extra).into_iter().filter_map(|(label, row)| match row {
            palette::Row::Command(c) => Some((label, command_for(c)?)),
            palette::Row::Extra(ix) => Some((label, Command::Space(spaces[ix].1))),
        }))
        .collect()
}

/// The empty command bar's sections, one row each, the first of a section with its heading, and
/// the address a page or tab row's icon shows.
pub fn sections(ui: Ui, history: &[store::HistoryEntry]) -> Vec<SectionRow> {
    let Some((state, _)) = state(ui) else { return Vec::new() };
    read(ui, |b| {
        let url = |id| b.model.tabs.iter().find(|t| t.id == id).map(|t| t.url.clone()).unwrap_or_default();
        let mut rows = Vec::new();
        for (section, entries) in b.palette_sections(&state, offered, history) {
            for (ix, entry) in entries.into_iter().enumerate() {
                let address = match &entry.item {
                    palette::Item::Command(_) => String::new(),
                    palette::Item::Tab(id) => url(*id),
                    palette::Item::Bookmark(u) | palette::Item::History(u) => u.clone(),
                };
                rows.push(((ix == 0).then_some(section), entry, address));
            }
        }
        rows
    })
    .unwrap_or_default()
}

pub type SectionRow = (Option<browser_core::command::Section>, palette::Entry, String);

/// The command a section row runs, and its shortcut.
pub fn section_command(ui: Ui, command: palette::Command) -> Option<(Command, Option<String>)> {
    let command = command_for(command)?;
    let keys = action(&command).and_then(|a| super::shortcut_of(ui, a));
    Some((command, keys))
}

/// The palette command a command row runs; `None` for an answer or a space row.
pub fn palette_command(command: &Command) -> Option<palette::Command> {
    palette::COMMANDS.iter().map(|(_, _, c)| *c).find(|c| command_for(*c).as_ref() == Some(command))
}

fn offered(c: palette::Command) -> bool {
    command_for(c).is_some() && (c != palette::Command::ImportBrowserData || browser_core::import::available())
}

fn state(ui: Ui) -> Option<(palette::State, Vec<(String, SpaceCommand)>)> {
    let toast = ui.snap.with_untracked(|s| s.toast.as_ref().and_then(|t| Some(format!("{} \u{2014} {}", t.action.as_ref()?.label(), t.text))));
    let (spaces, pinned, split, reopenable, duplicates, private, tab, developer, assistant) = read(ui, |b| {
        let active = b.model.active;
        (
            b.space_commands(),
            active.and_then(|k| b.model.tab(k)).is_some_and(|t| t.kind != TabKind::Ephemeral),
            active.is_some_and(|k| b.splits.contains(k)),
            b.model.closed_tabs.newest().is_some(),
            !b.duplicate_tabs().is_empty(),
            b.model.private(),
            ActiveTab::of(b),
            b.devtools_on(),
            b.model.assistant_offered(&b.core, b.model.active.and_then(|k| b.pages.get(&k)).map(|p| p.url.as_str())),
        )
    })?;
    let state = palette::State {
        pinned,
        split,
        reopenable,
        duplicates,
        private,
        no_tab: !tab.open,
        no_page: ui.snap.with_untracked(super::no_page),
        muted: tab.muted,
        reader: Some(tab.reader),
        custom_title: tab.custom,
        renamable: tab.renamable,
        drifted: tab.drifted,
        sleep_others: tab.sleep_others,
        unpinned: tab.unpinned,
        loading: tab.loading,
        toast,
        developer,
        assistant,
        ..palette::State::default()
    };
    Some((state, spaces))
}

pub fn run(ui: Ui, command: Command) {
    ui.panel.set(Panel::None);
    match command {
        Command::Copy(text) => {
            set_clipboard(text);
            act(ui, |b| b.show_toast_as(browser_core::toast::Tone::Success, "Copied", None));
        }
        Command::CloseDuplicates => act(ui, Browser::close_duplicates),
        Command::ZapElement => act(ui, Browser::zap_element),
        Command::CaptureArea => super::capture::open(ui, tarn_nsh_shared::page_overlay::Purpose::Area),
        Command::CaptureElement => super::capture::capture_element(ui),
        Command::CaptureFullPage => act(ui, Browser::capture_full_page),
        Command::Measure => super::capture::open(ui, tarn_nsh_shared::page_overlay::Purpose::Measure),
        Command::InsertSnippet => open_snippets(ui),
        Command::Reader => act(ui, Browser::toggle_reader),
        Command::SaveForLater => act(ui, Browser::save_for_later),
        Command::BurnerTab => act(ui, Browser::open_burner),
        Command::FindInTabs => open_find_tabs(ui),
        Command::CopyLink => copy_link(ui, false),
        Command::CopyCleanLink => copy_link(ui, true),
        Command::CopyMarkdownLink => copy_page(ui, true),
        Command::CopyTitle => copy_page(ui, false),
        Command::SiteSettings => show(ui, Panel::Shield),
        Command::EditSpace => show(ui, Panel::Space),
        Command::ShowQrCode => show_qr(ui),
        Command::SavePdf => save_pdf(ui),
        Command::Print => act(ui, Browser::print),
        Command::BookmarkAllTabs => super::library::bookmark_all_tabs(ui),
        Command::SaveSnapshot => act(ui, Browser::save_snapshot),
        Command::Snapshots => show(ui, Panel::Space),
        Command::Memory => show(ui, Panel::Memory),
        Command::ReadingList => show(ui, Panel::ReadingList),
        Command::Archived => show(ui, Panel::Archived),
        Command::ImportBrowserData => super::import::open(ui),
        Command::NewWindow => super::new_window(ui),
        Command::FullScreen => act(ui, Browser::toggle_full_screen),
        Command::StopLoading => act(ui, Browser::stop),
        Command::GlassDesign => {
            act(ui, Browser::toggle_glass_design);
            super::themes_changed();
        }
        Command::ToastAction => take_toast_action(ui),
        Command::Ask => super::assistant::open(ui, false),
        Command::AskPage => super::assistant::open(ui, true),
        Command::Key(action) => super::run_action(ui, action),
        Command::CloseSplit => act(ui, |b| b.model.active.into_iter().for_each(|k| b.close_split(k))),
        Command::Space(c) => act(ui, |b| b.run_space_command(c)),
        Command::DuplicateTab => act(ui, |b| b.model.active.into_iter().for_each(|k| b.duplicate_tab(k))),
        Command::ToggleMute => act(ui, |b| b.model.active.into_iter().for_each(|k| b.toggle_site_muted(k))),
        Command::RenameTab => {
            ui.tab_menu.set(None);
            show(ui, Panel::Tab);
        }
        Command::ResetTitle => act(ui, |b| b.model.active.into_iter().for_each(|k| b.reset_tab_title(k))),
        Command::ReturnToPinned => act(ui, |b| b.model.active.into_iter().for_each(|k| b.return_to_pinned(k))),
        Command::SetPinnedUrl => act(ui, |b| b.model.active.into_iter().for_each(|k| b.set_pinned_url(k))),
        Command::SleepOthers => act(ui, |b| b.model.active.into_iter().for_each(|k| b.sleep_tabs(b.model.other_tabs(k)))),
    }
}

#[derive(Default)]
struct ActiveTab {
    open: bool,
    unpinned: bool,
    renamable: bool,
    muted: Option<bool>,
    custom: bool,
    drifted: bool,
    sleep_others: bool,
    loading: bool,
    reader: bool,
}

impl ActiveTab {
    fn of(b: &Browser) -> ActiveTab {
        let unpinned = !b.model.ephemeral_keys().is_empty();
        let Some((key, tab)) = b.model.active.and_then(|k| Some((k, b.model.tab(k)?))) else { return ActiveTab { unpinned, ..ActiveTab::default() } };
        ActiveTab {
            open: true,
            unpinned,
            renamable: tab.kind != TabKind::Tile,
            muted: b.can_mute(key).then(|| b.tab_muted(key)),
            custom: !tab.custom_title.is_empty(),
            drifted: b.pinned_drift(key).is_some(),
            sleep_others: b.can_sleep_others(key),
            loading: b.pages.get(&key).is_some_and(|p| p.loading),
            reader: b.reader.as_ref().is_some_and(|r| r.key == key),
        }
    }
}

/// Runs a command row of the address field or the start card.
pub fn pick(ui: Ui, command: Command) {
    ui.editing.set(false);
    ui.address_ref.blur();
    ui.command.set(String::new());
    run_from_menu(ui, command);
}

/// Runs a command picked from a menu or the command bar, then perhaps names its shortcut.
pub fn run_from_menu(ui: Ui, command: Command) {
    let shortcut = action(&command).and_then(|a| Some((a, super::shortcut_of(ui, a)?)));
    run(ui, command);
    let Some((action, keys)) = shortcut else { return };
    let typing = ui.editing.get_untracked() || ui.finding.get_untracked() || ui.find_tabs.get_untracked() || ui.panel.get_untracked() != Panel::None;
    act(ui, |b| b.offer_hint(browser_core::shortcuts::label(action), &keys, typing));
}

pub fn copy_link(ui: Ui, clean: bool) {
    let mut link = None;
    act(ui, |b| link = b.copy_link(clean));
    if let Some(link) = link {
        set_clipboard(link);
    }
}

pub fn copy_page(ui: Ui, markdown: bool) {
    let mut text = None;
    act(ui, |b| text = b.copy_page(markdown));
    if let Some(text) = text {
        set_clipboard(text);
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
    let Some((serial, held, action)) = ui.snap.with_untracked(|s| s.toast.as_ref().map(|t| (t.serial, t.held, t.action.is_some()))) else { return };
    if ui.toast_timer.get_untracked() == serial || held {
        return;
    }
    ui.toast_timer.set(serial);
    toast_timer(ui, serial, if action { ACTION_TIME } else { TOAST_TIME });
}

/// Ends toast `serial` after `time`, or waits again while the pointer is on it.
fn toast_timer(ui: Ui, serial: u64, time: Duration) {
    ui.root.run(|| {
        drop(set_timeout(time, move || {
            if ui.toast_hovered.get_untracked() {
                toast_timer(ui, serial, TOAST_TIME);
            } else {
                act(ui, |b| b.expire_toast(serial));
            }
        }))
    });
}

/// The toast's button, from the toast or its command row.
fn take_toast_action(ui: Ui) {
    if let Some(ToastAction::Reveal(path)) = ui.snap.with_untracked(|s| s.toast.as_ref().and_then(|t| t.action.clone())) {
        drop(reveal_path(std::path::PathBuf::from(&path)));
    }
    act(ui, Browser::take_toast_action);
}

pub fn toast_bar(ui: Ui) -> AnyElement {
    dynamic(move || {
        let toast = ui.snap.with(|s| s.toast.clone())?;
        let action = toast.action.map(|action| tarn_nsh_shared::controls::toast_action(action.label(), move || take_toast_action(ui)));
        Some(
            view().flex_row().justify_center().child(
                tarn_nsh_shared::controls::toast(toast.tone)
                    .mt_2()
                    .max_w(px(560.))
                    .on_mouse_enter(move |_: &MouseEvent| ui.toast_hovered.set(true))
                    .on_mouse_leave(move |_: &MouseEvent| ui.toast_hovered.set(false))
                    .child(label(toast.text).flex_shrink(1.).number_of_lines(2))
                    .children(action)
                    .child(crate::controls::icon_button_at(icons::CLOSE, "Dismiss", Placement::Left, move || act(ui, |b| b.expire_toast(toast.serial)))),
            ),
        )
    })
}

fn action(command: &Command) -> Option<Action> {
    match command {
        Command::Key(action) => Some(*action),
        Command::CopyLink => Some(Action::CopyLink),
        Command::CopyCleanLink => Some(Action::CopyCleanLink),
        Command::CopyMarkdownLink => Some(Action::CopyMarkdownLink),
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
        let shown = rows.iter().any(|(_, c)| !matches!(c, Command::Copy(_)));
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
                let keys = action(&command).and_then(|a| super::shortcut_of(ui, a));
                let (glyph, detail) = match command {
                    Command::Copy(_) => (icons::COPY, Some(palette::ANSWER_DETAIL)),
                    _ => (icons::glyph(browser_core::command::glyph(&name)), None),
                };
                super::suggestion_row(name.clone(), move || pick(ui, command.clone()))
                    .element_ref(&at)
                    .bg(move || super::picked_fill(ui, ix + 1))
                    .child(super::row_glyph(ui, glyph, move || ix + 1))
                    .child(label(name).flex_1().truncate())
                    .children(detail.map(super::detail))
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

/// Switches to the tab of `row`, and for a match in its text finds the query on it.
pub fn open_find_row(ui: Ui, row: &crate::browser::FindRow) {
    let mut query = String::new();
    act(ui, |b| query = b.open_find_row(row.key));
    close_find_tabs(ui);
    if row.text {
        ui.find.set(query.clone());
        ui.finding.set(true);
        act(ui, |b| b.find_typed(&query));
    }
}

pub fn find_rows(ui: Ui) -> View {
    let rows = each(
        move || ui.snap.with(|s| s.find_rows.iter().cloned().enumerate().collect::<Vec<_>>()),
        |(ix, r)| format!("{ix}-{}-{}", r.label, r.key.id),
        move |item, _| {
            let (_, row) = item.get_untracked();
            let picked = row.clone();
            super::suggestion_row(row.label.clone(), move || open_find_row(ui, &picked))
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

fn open_snippets(ui: Ui) {
    let mut target = None;
    act(ui, |b| target = b.snippet_target());
    if target.is_some() {
        show(ui, Panel::Snippets);
    }
}

/// Insert Snippet's list, filtered as the user types; a choice types the snippet into the page.
pub fn snippets_panel(ui: Ui) -> AnyElement {
    let query = signal(String::new());
    let all = read(ui, Browser::snippets).unwrap_or_default();
    let field = nsh_base::input()
        .value(move || query.get())
        .on_change(move |v: &str| query.set(v.to_owned()))
        .placeholder("Filter snippets")
        .label("Filter snippets")
        .auto_focus(true)
        .w_full()
        .px(px(8.))
        .py(px(4.))
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .border_1()
        .border_color(Theme::hairline())
        .text_color(Theme::text());
    let rows = dynamic(move || {
        if all.is_empty() {
            return Some(view().child(note(browser_core::snippets::EMPTY)));
        }
        let found = query.with(|q| browser_core::snippets::matching(q, &all).into_iter().cloned().collect::<Vec<_>>());
        let rows: Vec<AnyElement> = found
            .into_iter()
            .map(|s| {
                let body = s.body.clone();
                crate::controls::row(format!("Insert {}", s.name), move || {
                    let body = body.clone();
                    ui.panel.set(Panel::None);
                    act(ui, move |b| b.insert_snippet(&body));
                })
                .child(label(format!(";{}", s.name)).flex_none())
                .child(small(browser_core::snippets::preview(&s.body)).truncate())
                .into_element()
            })
            .collect();
        Some(view().flex_col().gap(px(2.)).children(rows))
    });
    super::panels::popover().child(super::panels::heading(ui, label("Insert Snippet"))).child(field).child(rows).into_element()
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
        super::panels::menu_row(name, action.and_then(|a| super::shortcut_of(ui, a)), move || run_from_menu(ui, command.clone()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_row_names_the_palette_command_it_runs() {
        for (_, _, c) in palette::COMMANDS {
            if let Some(command) = command_for(c) {
                assert_eq!(palette_command(&command), Some(c), "{c:?}");
            }
        }
        assert_eq!(palette_command(&Command::Copy("1".into())), None);
    }
}
