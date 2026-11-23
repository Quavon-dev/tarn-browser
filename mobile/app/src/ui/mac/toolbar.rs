use std::rc::Rc;

use browser_core::command::Section;
use browser_core::command::{ENGINES, Input, address, classify_input, search_engine};
use browser_core::palette::{self, Command, Data, Entry, Item, Row, State, listing};
use browser_core::shortcuts;
use browser_core::suggest::step_selection;
use nsh_base::input;
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use serde_json::Value;
use shield::ShieldLevel;
use tarn_nsh_shared::controls::key_caps;
use tarn_nsh_shared::icons as glyphs;
use tarn_nsh_shared::toolbar::{
    address_text, badge_color, capsule, chip_button, extension_button, picked_fill, row, row_address, row_detail, row_glyph, row_name, row_section, row_site,
    rows, search_slot, shield_chip, tool_button, tools,
};
use url::Url;

use super::super::{Screen, Ui, bars};
use crate::theme::{TEXT_TITLE, TOOLBAR_HEIGHT, Theme, themed};

const OFFERED: [Command; 29] = [
    Command::OpenFile,
    Command::ReopenClosedTab,
    Command::TogglePin,
    Command::CloseDuplicates,
    Command::ZapElement,
    Command::TakeScreenshot,
    Command::CaptureArea,
    Command::CaptureElement,
    Command::CaptureFullPage,
    Command::InsertSnippet,
    Command::Reader,
    Command::SaveForLater,
    Command::BurnerTab,
    Command::FindInTabs,
    Command::CopyLink,
    Command::CopyCleanLink,
    Command::ShowQrCode,
    Command::Snapshots,
    Command::ReadingList,
    Command::ToggleSplitView,
    Command::RotateSplitLayout,
    Command::RemoveFromSplitView,
    Command::CloseSplit,
    Command::TabsOnTop,
    Command::Ask,
    Command::AskPage,
    Command::ShowHistory,
    Command::ShowDownloads,
    Command::Settings,
];

fn level_name(level: ShieldLevel) -> &'static str {
    match level {
        ShieldLevel::Standard => "Standard",
        ShieldLevel::Strict => "Strict",
        ShieldLevel::Lockdown => "Lockdown",
    }
}

fn host(ui: Ui) -> (String, bool) {
    ui.read(|s| Url::parse(&s.active_url()).ok().map(|u| address(&u)).unwrap_or_default())
}

fn reload_button(ui: Ui) -> impl IntoElement {
    let reloaded = signal(0u64);
    let again = move || {
        ui.act(|s| s.active_command("reload", Value::Null));
        reloaded.update(|n| *n += 1);
        let serial = reloaded.get_untracked();
        set_timeout(tarn_nsh_shared::motion::RELOAD_SPIN, move || {
            if reloaded.get_untracked() == serial {
                reloaded.set(0);
            }
        });
    };
    let state = computed(move || (ui.read(|s| s.active_page().is_some_and(|p| p.loading)), reloaded.get() > 0));
    dynamic(move || {
        let (loading, spinning) = state.get();
        if !loading && !spinning {
            return tool_button(glyphs::RELOAD, "Reload", Placement::Right, again).into_element();
        }
        let press = move || {
            if untracked(|| ui.read(|s| s.active_page().is_some_and(|p| p.loading))) {
                ui.act(|s| s.active_command("stop", Value::Null));
            } else {
                again();
            }
        };
        let name = if loading { "Stop Loading" } else { "Reload" };
        if reduce_motion() {
            return tool_button(glyphs::CLOSE, name, Placement::Right, press).into_element();
        }
        tarn_nsh_shared::controls::icon_button(tarn_nsh_shared::motion::spinner(16., glyphs::icon(glyphs::RELOAD, 16.)), name, press)
            .text_color(Theme::text())
            .into_element()
    })
}

fn history_button(ui: Ui, glyph: &'static str, name: &'static str, command: &'static str, can: fn(&crate::shell::Page) -> bool) -> impl IntoElement {
    tool_button(glyph, name, Placement::Right, move || ui.act_on_screen(|s| s.active_command(command, Value::Null)))
        .disabled(move || !ui.read(|s| s.active_page().is_some_and(can)))
        .disabled_style(|s| s.text_color(Theme::faint()))
}

fn sidebar_toggle(ui: Ui) -> impl IntoElement {
    let open = computed(move || ui.read_data(|s| s.sidebar_open));
    dynamic(move || {
        let name = if open.get() { "Hide Sidebar" } else { "Show Sidebar" };
        tool_button(glyphs::SIDEBAR, name, Placement::Right, move || {
            ui.act(|s| {
                s.toggle_sidebar();
                Vec::new()
            })
        })
        .element_ref(&ui.popovers.toggle)
    })
}

#[cfg(target_os = "android")]
const PRIVATE_MARK: &str = "Private space: deleted when it closes";
#[cfg(not(target_os = "android"))]
const PRIVATE_MARK: &str = "Private space: kept in memory only";

fn private_mark(ui: Ui) -> impl IntoElement {
    let state = computed(move || ui.read(|s| (s.model.current_space().is_some_and(|c| c.private), s.model.burner_shown(), s.sidebar_open)));
    dynamic(move || {
        let (private, burner, open) = state.get();
        (private && !open).then(|| {
            let (glyph, name) = if burner { (glyphs::FLAME, "Burner tab: nothing is kept after it closes") } else { (glyphs::HAND, PRIVATE_MARK) };
            view()
                .w(px(tarn_nsh_shared::controls::target_size()))
                .h(px(tarn_nsh_shared::controls::target_size()))
                .items_center()
                .justify_center()
                .accessible(true)
                .label(name)
                .text_color(Theme::warning)
                .tooltip(tarn_nsh_shared::controls::tip(name, Placement::Right))
                .child(glyphs::icon(glyph, 14.))
        })
    })
}

fn navigation(ui: Ui) -> View {
    tools()
        .child(sidebar_toggle(ui))
        .child(history_button(ui, glyphs::BACK, "Back", "back", |p| p.back))
        .child(history_button(ui, glyphs::FORWARD, "Forward", "forward", |p| p.forward))
        .child(reload_button(ui))
        .child(private_mark(ui))
}

fn shield(ui: Ui) -> impl IntoElement {
    let level = move || ui.read(|s| s.site_level()).unwrap_or(ShieldLevel::Standard);
    let raised = move || (level() != ShieldLevel::Standard).then(|| level_name(level()).to_owned());
    let total = move || ui.read(|s| s.site_shield_total());
    let count = move || (total() > 0).then(|| browser_core::shield_card::thousands(total() as u64));
    let name = move || SharedString::from(format!("Site settings: Shield {}, {} stopped on this site", level_name(level()), total().max(0)));
    shield_chip(name, raised, count, move || ui.toggle(Screen::Shield)).element_ref(&ui.popovers.shield)
}

fn address_field(ui: Ui) -> impl IntoElement {
    let open = move || ui.editing.get() || ui.screen.get() == Screen::Shield;
    let capsule = capsule(open)
        .test_id("capsule")
        .relative()
        .child(themed(move || {
            dynamic(move || {
                if ui.read(|s| s.active_host().is_none()) {
                    return search_slot().into_element();
                }
                shield(ui).into_element()
            })
        }))
        .child(address_text("Search or enter address", move || host(ui), move || open_palette(ui)))
        .child(themed(move || {
            dynamic(move || {
                ui.read(|s| s.active_page().is_some_and(|p| p.article))
                    .then(|| chip_button(glyphs::BOOK_OPEN, "Reader", move || ui.read(|s| s.reader.is_some()), move || ui.act(|s| s.open_reader())))
            })
        }));
    view()
        .flex_row()
        .justify_center()
        .w(px(tarn_nsh_shared::toolbar::ADDRESS_WIDTH))
        .flex_shrink(1.)
        .min_w(px(tarn_nsh_shared::toolbar::ADDRESS_MIN))
        .child(capsule)
}

fn extensions(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let items = ui.read(|s| s.action_items());
        tools().children(items.into_iter().map(|item| {
            let id = item.install_id.clone();
            let name = if item.state.title.is_empty() { item.name.clone() } else { item.state.title.clone() };
            let badge = (!item.state.badge_text.is_empty()).then(|| (item.state.badge_text.clone(), badge_color(item.state.badge_color)));
            extension_button(name, None, badge, move || {
                ui.act(|s| s.click_action(&id, std::time::Instant::now()));
                let opened = ui.read(|s| s.popup().is_some());
                ui.show(if opened { Screen::ExtensionPopup } else { Screen::Browse });
            })
        }))
    })
}

fn in_split(ui: Ui) -> bool {
    ui.read(|s| s.model.active.is_some_and(|k| s.splits.contains(k)))
}

fn toggle_split(ui: Ui) {
    ui.act(|s| {
        if let Some(active) = s.model.active {
            if s.splits.contains(active) {
                s.splits.close(active);
            } else {
                s.split_with(active);
            }
        }
        Vec::new()
    });
}

fn page_actions(ui: Ui) -> View {
    let split = move || {
        let name = if in_split(ui) { "Close Split View" } else { "Open Split View" };
        tool_button(glyphs::SPLIT_ADD, name, Placement::Left, move || toggle_split(ui)).disabled(move || ui.read(|s| s.model.active.is_none()))
    };
    let share = tool_button(glyphs::SHARE, "Share", Placement::Left, move || ui.toggle(Screen::Share))
        .disabled(move || ui.read(|s| s.share_url().is_none()))
        .element_ref(&ui.popovers.share);
    tools().child(dynamic(split)).child(share)
}

pub(super) fn toolbar(ui: Ui) -> impl IntoElement {
    let side = || view().flex_row().items_center().flex_1().min_w(nsh_core::geometry::auto());
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(TOOLBAR_HEIGHT))
        .flex_none()
        .px_2()
        .child(side().child(themed(move || navigation(ui))))
        .child(address_field(ui))
        .child(side().justify_end().gap_2().child(themed(move || extensions(ui))).child(themed(move || page_actions(ui))))
}

pub(super) fn open_palette(ui: Ui) {
    if ui.editing.get_untracked() {
        return;
    }
    ui.address.set(if ui.new_tab.get_untracked() { String::new() } else { ui.read(|s| s.active_url()) });
    ui.suggest.set(Vec::new());
    ui.picked.set(Some(0));
    ui.actions.set(None);
    ui.rejected.set(0);
    if ui.screen.get_untracked() != Screen::Browse {
        ui.show(Screen::Browse);
    }
    ui.editing.set(true);
}

fn close_palette(ui: Ui) {
    ui.suggest.set(Vec::new());
    bars::cancel(ui);
}

fn state(ui: Ui) -> State {
    ui.read(|s| State {
        pinned: s.model.active.and_then(|k| s.model.tab(k)).is_some_and(|t| t.kind == store::TabKind::Pinned),
        pin_blocked: !s.model.active.and_then(|k| s.model.tab(k)).is_some_and(|t| browser_core::tab_menu::pinnable(t.kind, s.model.burner_shown())),
        split: s.model.active.is_some_and(|k| s.splits.contains(k)),
        reopenable: !s.model.closed_tabs.is_empty(),
        duplicates: !s.duplicate_tabs().is_empty(),
        private: s.model.private(),
        assistant: s.model.assistant_offered(&s.core, Some(&s.active_url()).filter(|u| !u.is_empty()).map(String::as_str)),
        ..State::default()
    })
}

fn offered(ui: Ui, command: Command) -> bool {
    OFFERED.contains(&command)
        && match command {
            Command::ToggleSplitView | Command::TabsOnTop => ui.wide(),
            Command::Reader => ui.read(|s| s.offers_reader()),
            Command::ZapElement => ui.read(|s| s.zappable()),
            Command::TakeScreenshot | Command::CaptureArea | Command::CaptureElement | Command::CaptureFullPage | Command::InsertSnippet => {
                ui.read(|s| s.offers_capture())
            }
            Command::BurnerTab => !ui.read(|s| s.in_burner() || <crate::data::Data as crate::data::Platform>::private_process()),
            Command::CopyLink | Command::CopyCleanLink | Command::ShowQrCode | Command::SaveForLater => ui.read(|s| s.share_url().is_some()),
            _ => true,
        }
}

pub(in crate::ui) type Sectioned = Rc<Vec<(Section, Entry)>>;

/// The empty command bar's rows (`palette::sections`), each with its section, read once per open.
pub(in crate::ui) fn sections(ui: Ui) -> Sectioned {
    untracked(|| {
        let state = state(ui);
        let found = ui.read(|s| {
            let private = s.model.private();
            let bookmarks = s.profile_id().filter(|_| !private).and_then(|id| s.store().bookmarks(id).ok()).unwrap_or_default();
            let history = if private { Vec::new() } else { s.suggestion_history() };
            let data = Data { tabs: &s.model.tabs, active: s.model.active.map(|k| k.id), bookmarks: &bookmarks, history: &history, now: browser_core::now() };
            palette::sections(&state, |c| offered(ui, c), &data)
        });
        Rc::new(found.into_iter().flat_map(|(section, rows)| rows.into_iter().map(move |e| (section, e))).collect())
    })
}

/// Opens a row of `sections`: runs its command, switches to its tab or opens its page.
pub(in crate::ui) fn open_entry(ui: Ui, item: Item) {
    match item {
        Item::Command(c) => pick(ui, Pick::Run(c)),
        Item::Tab(id) => {
            close_palette(ui);
            ui.act(|s| {
                s.activate(browser_core::tabs::TabKey { private: s.model.private(), id });
                Vec::new()
            });
            ui.show(Screen::Browse);
        }
        Item::Bookmark(url) | Item::History(url) => go(ui, url),
    }
}

fn move_targets(ui: Ui, item: &Item) -> Vec<(usize, String)> {
    match item {
        Item::Tab(id) => ui.read(|s| s.move_targets(browser_core::tabs::TabKey { private: s.model.private(), id: *id })),
        _ => Vec::new(),
    }
}

/// The actions page of `item` (`palette::actions`), with the spaces its tab can move to.
pub(in crate::ui) fn item_actions(ui: Ui, item: &Item) -> Vec<(String, palette::Action)> {
    let targets = untracked(|| move_targets(ui, item));
    palette::actions(item, &targets.iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>())
}

pub(in crate::ui) fn run_action(ui: Ui, item: Item, action: palette::Action) {
    use palette::Action as A;
    let key = match item {
        Item::Tab(id) => Some(browser_core::tabs::TabKey { private: ui.read(|s| s.model.private()), id }),
        _ => None,
    };
    let url = match &item {
        Item::Bookmark(u) | Item::History(u) => Some(u.clone()),
        Item::Command(_) | Item::Tab(_) => None,
    };
    let target = match action {
        A::MoveToSpace(ix) => move_targets(ui, &item).get(ix).map(|(ix, _)| *ix),
        _ => None,
    };
    if matches!(action, A::Run | A::Switch | A::Open) {
        return open_entry(ui, item);
    }
    close_palette(ui);
    match (action, key, url) {
        (A::Close, Some(key), _) => ui.act(|s| {
            s.close_tab(key);
            Vec::new()
        }),
        (A::CopyLink, Some(key), _) => ui.copy_link(Some(key), false),
        (A::CopyLink, None, Some(url)) => ui.copy_answer(url),
        (A::MoveToSpace(_), Some(key), _) => {
            if let Some(ix) = target {
                ui.act(|s| {
                    s.move_tab(key, ix);
                    Vec::new()
                });
            }
        }
        (A::OpenInNewTab, _, Some(url)) => {
            ui.act(|s| {
                s.new_tab(Some(&url));
                Vec::new()
            });
            ui.show(Screen::Browse);
        }
        (A::RemoveFromHistory, _, Some(url)) => ui.act(|s| {
            s.remove_history_url(&url);
            Vec::new()
        }),
        _ => {}
    }
}

/// The picked row as an item with an actions page, and its name.
fn picked_item(ui: Ui, sectioned: &Sectioned) -> Option<(Item, String)> {
    let query = ui.address.get_untracked();
    let at = ui.picked.get_untracked()?;
    if palette::sectioned(&query) {
        return sectioned.get(at).map(|(_, e)| (e.item.clone(), e.label.clone()));
    }
    let commands = commands(ui, &query);
    if let Some((name, Pick::Run(c))) = at.checked_sub(1).and_then(|ix| commands.get(ix)) {
        return Some((Item::Command(*c), name.clone()));
    }
    if listing(&query) {
        return None;
    }
    let ix = at.checked_sub(1 + commands.len())?;
    ui.suggest.with_untracked(|rows| rows.get(ix).map(|h| (Item::History(h.url.clone()), browser_core::command::display_title("", &h.title, &h.url))))
}

fn open_actions(ui: Ui, sectioned: &Sectioned) {
    if let Some((item, name)) = picked_item(ui, sectioned) {
        ui.actions.set(Some((item, name, ui.picked.get_untracked())));
        ui.picked.set(Some(0));
    }
}

fn close_actions(ui: Ui) -> bool {
    let Some((_, _, picked)) = ui.actions.get_untracked() else { return false };
    ui.actions.set(None);
    ui.picked.set(picked);
    true
}

#[derive(Clone, PartialEq)]
enum Pick {
    Copy(String),
    Run(Command),
}

/// The answer rows of `query`, then the commands it names.
fn commands(ui: Ui, query: &str) -> Vec<(String, Pick)> {
    let state = state(ui);
    let answers = palette::answers(query).into_iter().map(|a| (a.label, Pick::Copy(a.copy)));
    answers
        .chain(palette::rows(query, state, |c| offered(ui, c), &[]).into_iter().filter_map(|(name, row)| match row {
            Row::Command(c) => Some((name, Pick::Run(c))),
            Row::Extra(_) => None,
        }))
        .collect()
}

fn copy(ui: Ui, clean: bool) {
    ui.copy_link(None, clean);
}

fn on_active(ui: Ui, f: impl FnOnce(&mut crate::shell::Shell<crate::data::Data>, browser_core::tabs::TabKey)) {
    ui.act(|s| {
        if let Some(active) = s.model.active {
            f(s, active);
        }
        Vec::new()
    });
}

fn run(ui: Ui, command: Command) {
    let quiet = |f: fn(&mut crate::shell::Shell<crate::data::Data>)| {
        ui.act(|s| {
            f(s);
            Vec::new()
        })
    };
    match command {
        Command::OpenFile => super::super::panels::open_file(ui),
        Command::ReopenClosedTab => quiet(|s| {
            s.reopen_closed();
        }),
        Command::CloseDuplicates => quiet(|s| s.close_duplicates()),
        Command::ZapElement => ui.act(|s| s.pick()),
        Command::TakeScreenshot => ui.act(|s| s.take_screenshot()),
        Command::CaptureArea => super::super::capture::capture_area(ui),
        Command::CaptureElement => super::super::capture::capture_element(ui),
        Command::CaptureFullPage => ui.act(|s| s.capture_full_page()),
        Command::InsertSnippet => super::super::sheets::open_insert_snippet(ui),
        Command::Reader => ui.act(|s| s.open_reader()),
        Command::SaveForLater => quiet(|s| {
            s.save_for_later(browser_core::now());
        }),
        Command::BurnerTab => quiet(|s| s.open_burner()),
        Command::FindInTabs => ui.show(Screen::FindTabs),
        Command::CopyLink => copy(ui, false),
        Command::CopyCleanLink => copy(ui, true),
        Command::ShowQrCode => ui.show(Screen::Qr),
        Command::Snapshots => ui.show(Screen::Spaces),
        Command::ReadingList => ui.show(Screen::ReadingList),
        Command::ShowHistory => ui.show(Screen::History),
        Command::ShowDownloads => ui.show(Screen::Downloads),
        Command::Settings => ui.show(Screen::Settings),
        Command::ToggleSplitView => toggle_split(ui),
        Command::RotateSplitLayout => on_active(ui, |s, k| {
            s.splits.rotate(k);
        }),
        Command::RemoveFromSplitView => on_active(ui, |s, k| s.leave_split(k)),
        Command::CloseSplit => on_active(ui, |s, k| {
            s.splits.close(k);
        }),
        Command::TogglePin => on_active(ui, |s, k| s.toggle_pin(k)),
        Command::TabsOnTop => ui.act(|s| {
            s.toggle_tabs_on_top();
            Vec::new()
        }),
        Command::Ask => super::super::assistant::open(ui, false),
        Command::AskPage => super::super::assistant::open(ui, true),
        _ => {}
    }
}

fn pick(ui: Ui, pick: Pick) {
    close_palette(ui);
    match pick {
        Pick::Copy(text) => ui.copy_answer(text),
        Pick::Run(command) => {
            run(ui, command);
            let action = shortcuts::for_command(command).filter(|a| super::input::offered(*a));
            if let Some((action, keys)) = action.and_then(|a| Some((a, shortcuts::keys_for(a, super::input::keyboard())?))) {
                let typing = ui.editing.get_untracked();
                ui.act(|s| {
                    s.offer_hint(shortcuts::label(action), &keys, typing);
                    Vec::new()
                });
            }
        }
    }
}

fn go(ui: Ui, text: String) {
    if ui.rejects(&text) {
        return;
    }
    ui.suggest.set(Vec::new());
    ui.editing.set(false);
    dismiss_keyboard();
    ui.open_typed(&text);
}

fn submit(ui: Ui, text: String, completed: Option<(String, String)>, sectioned: &Sectioned) {
    if let Some((item, _, _)) = ui.actions.get_untracked() {
        if let Some((_, action)) = ui.picked.get_untracked().and_then(|ix| item_actions(ui, &item).get(ix).cloned()) {
            run_action(ui, item, action);
        }
        return;
    }
    if palette::sectioned(&text) {
        if let Some((_, entry)) = ui.picked.get_untracked().and_then(|ix| sectioned.get(ix)) {
            open_entry(ui, entry.item.clone());
        }
        return;
    }
    let picked = ui.picked.get_untracked().unwrap_or(0);
    let commands = commands(ui, &text);
    if listing(&text) {
        if let Some((_, command)) = commands.get(picked.saturating_sub(1)) {
            pick(ui, command.clone());
        }
        return;
    }
    if picked > 0 {
        if let Some((_, command)) = commands.get(picked - 1) {
            return pick(ui, command.clone());
        }
        if let Some(url) = ui.suggest.with_untracked(|s| s.get(picked - 1 - commands.len()).map(|h| h.url.clone())) {
            return go(ui, url);
        }
    }
    go(ui, typed_target(text, completed));
}

fn typed_target(text: String, completed: Option<(String, String)>) -> String {
    completed.filter(|(full, _)| *full == text).map_or(text, |(_, url)| url)
}

fn step(ui: Ui, key: &str, sectioned: &Sectioned) {
    let query = ui.address.get_untracked();
    let (first, count) = if let Some((item, _, _)) = ui.actions.get_untracked() {
        (0, item_actions(ui, &item).len())
    } else if palette::sectioned(&query) {
        (0, sectioned.len())
    } else if listing(&query) {
        (1, commands(ui, &query).len())
    } else {
        (0, 1 + commands(ui, &query).len() + ui.suggest.with_untracked(Vec::len))
    };
    if count == 0 {
        return;
    }
    let at = ui.picked.get_untracked().unwrap_or(first).saturating_sub(first);
    let next = match key {
        "pageup" => 0,
        "pagedown" => count - 1,
        _ => step_selection(at, count, key == "down"),
    };
    ui.picked.set(Some(first + next));
}

fn bind_keys(ui: Ui, sectioned: &Sectioned, caret: Signal<(u32, u32)>) {
    for key in ["up", "down", "pageup", "pagedown"] {
        let sectioned = sectioned.clone();
        on_key_with(key, KeyOptions { when: When::Always, repeat: true, ..KeyOptions::default() }, move |_| step(ui, key, &sectioned));
    }
    on_key_with("escape", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
        if !close_actions(ui) {
            close_palette(ui);
        }
    });
    let sectioned = sectioned.clone();
    // Right opens the picked row's actions only with the caret at the end, where it moves nothing.
    on_key_with("right", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
        let end = ui.address.with_untracked(|a| a.encode_utf16().count() as u32);
        if ui.actions.get_untracked().is_none() && caret.get_untracked() == (end, end) {
            open_actions(ui, &sectioned);
        }
    });
    on_key_with("left", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| {
        close_actions(ui);
    });
}

fn picked(ui: Ui, row: usize) -> bool {
    ui.picked.get().unwrap_or(0) == row
}

fn input_row(ui: Ui, completed: Signal<Option<(String, String)>>) -> impl IntoElement {
    dynamic(move || {
        if ui.address.with(|a| listing(a)) {
            return None;
        }
        let text = typed_target(ui.address.get(), completed.get());
        let (engine, shortcuts) = ui.read(|s| (&ENGINES[s.search_engine()], s.search_shortcuts()));
        let (name, note, glyph, rejected) = match classify_input(&text, engine, shortcuts) {
            Some(Input::Navigate(u)) => (u.to_string(), "Open".to_owned(), glyphs::GLOBE, false),
            Some(Input::External(u)) => (u.to_string(), format!("Open in another app ({})", u.scheme()), glyphs::EXTERNAL_LINK, false),
            Some(Input::Rejected) => (text.clone(), "This kind of address cannot be opened".to_owned(), glyphs::TRIANGLE_ALERT, true),
            _ => (text.clone(), format!("Search {}", search_engine(&text, engine, shortcuts).name), glyphs::SEARCH, false),
        };
        Some(
            row(name.clone(), move || go(ui, typed_target(ui.address.get_untracked(), completed.get_untracked())))
                .bg(move || picked_fill(picked(ui, 0)))
                .child(row_glyph(glyph, move || picked(ui, 0)))
                .child(row_name(name, if rejected { Theme::danger() } else { Theme::text() }))
                .child(row_detail(note)),
        )
    })
}

fn command_rows(ui: Ui) -> impl IntoElement {
    let listed = computed(move || commands(ui, &ui.address.get()));
    dynamic(move || {
        let found = listed.get();
        let shown = found.iter().any(|(_, p)| matches!(p, Pick::Run(_)));
        let items = found.into_iter().enumerate().map(move |(ix, (name, pick_row))| {
            let (glyph, command) = match &pick_row {
                Pick::Copy(_) => (glyphs::COPY, None),
                Pick::Run(c) => (glyphs::glyph(browser_core::command::glyph(&name)), Some(*c)),
            };
            row(name.clone(), move || pick(ui, pick_row.clone()))
                .bg(move || picked_fill(picked(ui, ix + 1)))
                .child(row_glyph(glyph, move || picked(ui, ix + 1)))
                .child(row_name(name, Theme::text()))
                .children(command.is_none().then(|| row_detail(palette::ANSWER_DETAIL.to_owned())))
                .children(
                    command
                        .and_then(shortcuts::for_command)
                        .filter(|a| super::input::offered(*a))
                        .and_then(|a| shortcuts::keys_for(a, super::input::keyboard()))
                        .map(|k| key_caps(&k)),
                )
        });
        view().flex_col().gap(px(tarn_nsh_shared::toolbar::TOOL_GAP)).children(shown.then(|| row_section("Commands"))).children(items)
    })
}

fn suggestion_rows(ui: Ui) -> impl IntoElement {
    let first = computed(move || 1 + commands(ui, &ui.address.get()).len());
    let listed = move || if ui.address.with(|a| listing(a)) { Vec::new() } else { ui.suggest.get() };
    view().flex_col().gap(px(tarn_nsh_shared::toolbar::TOOL_GAP)).child(when(move || !listed().is_empty(), || row_section("Suggestions"))).child(each(
        listed,
        |h: &store::HistoryEntry| h.url.clone(),
        move |item, ix| {
            let entry = item.get_untracked();
            let url = entry.url.clone();
            let site = Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
            let target = url.clone();
            let name = browser_core::command::display_title("", &entry.title, &url);
            row(name.clone(), move || go(ui, target.clone()))
                .bg(move || picked_fill(picked(ui, first.get() + ix.get())))
                .child(row_site(None, &site))
                .child(row_name(name, Theme::text()))
                .child({
                    let (host, rest) = browser_core::command::history_detail(&url);
                    row_address(host, rest)
                })
        },
    ))
}

/// The actions page of the row `item` named `name`; Run shows the command's keys.
fn action_rows(ui: Ui, item: Item, name: String) -> impl IntoElement {
    let keys = match item {
        Item::Command(c) => shortcuts::for_command(c).filter(|a| super::input::offered(*a)).and_then(|a| shortcuts::keys_for(a, super::input::keyboard())),
        _ => None,
    };
    let rows_ = item_actions(ui, &item).into_iter().enumerate().map(move |(ix, (label, action))| {
        let item = item.clone();
        let glyph = glyphs::glyph(browser_core::command::glyph(&label));
        row(label.clone(), move || run_action(ui, item.clone(), action))
            .bg(move || picked_fill(ui.picked.get() == Some(ix)))
            .child(row_glyph(glyph, move || ui.picked.get() == Some(ix)))
            .child(row_name(label, Theme::text()))
            .children(keys.as_deref().filter(|_| action == palette::Action::Run).map(key_caps))
    });
    view().flex_col().gap(px(tarn_nsh_shared::toolbar::TOOL_GAP)).child(row_section(&name)).children(rows_.collect::<Vec<_>>())
}

/// The empty bar's sections; Enter opens nothing until a row is picked.
fn section_rows(ui: Ui, sectioned: &Sectioned) -> impl IntoElement {
    let mut list = view().flex_col().gap(px(tarn_nsh_shared::toolbar::TOOL_GAP));
    for (ix, (section, entry)) in sectioned.iter().enumerate() {
        if ix == 0 || sectioned[ix - 1].0 != *section {
            list = list.child(row_section(section.title()));
        }
        let item = entry.item.clone();
        let row_ = row(entry.label.clone(), move || open_entry(ui, item.clone())).bg(move || picked_fill(ui.picked.get() == Some(ix)));
        list = list.child(match &entry.item {
            Item::Command(c) => row_
                .child(row_glyph(glyphs::glyph(browser_core::command::glyph(&entry.label)), move || ui.picked.get() == Some(ix)))
                .child(row_name(entry.label.clone(), Theme::text()))
                .children(
                    shortcuts::for_command(*c)
                        .filter(|a| super::input::offered(*a))
                        .and_then(|a| shortcuts::keys_for(a, super::input::keyboard()))
                        .map(|k| key_caps(&k)),
                ),
            Item::Tab(_) | Item::Bookmark(_) | Item::History(_) => {
                let url = match &entry.item {
                    Item::Tab(id) => ui.read(|s| s.model.tabs.iter().find(|t| t.id == *id).map(|t| t.url.clone())).unwrap_or_default(),
                    Item::Bookmark(u) | Item::History(u) => u.clone(),
                    Item::Command(_) => String::new(),
                };
                let site = Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
                row_.child(row_site(None, &site)).child(row_name(entry.label.clone(), Theme::text())).child(row_detail(entry.detail.clone()))
            }
        });
    }
    list
}

fn palette_field(ui: Ui, completed: Signal<Option<(String, String)>>, sectioned: Sectioned, caret: Signal<(u32, u32)>) -> impl IntoElement {
    let typed = signal(String::new());
    let history = Rc::new(ui.read(|s| s.suggestion_history()));
    let at = element_ref();
    ui.shaking(input())
        .element_ref(&at)
        .value(move || ui.address.get())
        .on_focus(move || at.set_selection(0, ui.address.with_untracked(|a| a.encode_utf16().count() as u32)))
        .on_change(move |value: &str| {
            if value == ui.address.get_untracked() {
                return;
            }
            ui.actions.set(None);
            ui.rejected.set(0);
            if let Some(full) = ui.read(|s| s.expand_snippet(value)) {
                typed.set(full.clone());
                completed.set(None);
                ui.suggest.set(Vec::new());
                return super::super::caret_at_end(ui.address, at, full);
            }
            let (found, completion) = ui.read(|s| s.suggest_from(&history, &typed.get_untracked(), value));
            typed.set(value.to_owned());
            ui.suggest.set(found);
            ui.picked.set((!palette::sectioned(value)).then(|| usize::from(listing(value))));
            let full = format!("{value}{}", completion.as_ref().map_or("", |c| c.suffix.as_str()));
            completed.set(completion.map(|c| (full.clone(), c.url.to_string())));
            ui.address.set(full.clone());
            if full.len() > value.len() {
                at.set_selection(value.encode_utf16().count() as u32, full.encode_utf16().count() as u32);
            }
        })
        .on_selection_change(move |s: &Selection| caret.set((s.start, s.end)))
        .on_submit(move |value: &str| submit(ui, value.to_owned(), completed.get_untracked(), &sectioned))
        .keyboard_type(KeyboardType::Url)
        .return_key_type(ReturnKeyType::Go)
        .auto_capitalize(AutoCapitalize::None)
        .auto_correct(false)
        .auto_focus(true)
        .placeholder("Search or enter address")
        .placeholder_color(Theme::faint)
        .label("Search or enter address")
        .test_id("palette-field")
        .flex_1()
        .text_size(px(TEXT_TITLE))
        .text_color(Theme::text)
}

pub(super) fn palette(ui: Ui) -> impl IntoElement {
    when(
        move || ui.editing.get(),
        move || {
            let sectioned = sections(ui);
            let caret = signal((0u32, 0u32));
            bind_keys(ui, &sectioned, caret);
            let completed = signal(None::<(String, String)>);
            let typed = move || ui.address.with(|a| !a.trim().is_empty()) && ui.read(|s| ui.address.with(|a| *a != s.active_url()));
            themed(move || {
                let shown = sectioned.clone();
                let list = dynamic(move || {
                    if let Some((item, name, _)) = ui.actions.get() {
                        return Some(rows().child(action_rows(ui, item, name)).into_element());
                    }
                    if ui.address.with(|a| palette::sectioned(a)) {
                        return (!shown.is_empty()).then(|| rows().child(section_rows(ui, &shown)).into_element());
                    }
                    typed().then(|| {
                        rows().child(ui.invalid_note()).child(input_row(ui, completed)).child(command_rows(ui)).child(suggestion_rows(ui)).into_element()
                    })
                });
                tarn_nsh_shared::toolbar::palette(
                    palette_field(ui, completed, sectioned.clone(), caret),
                    move || ui.address.with(|a| listing(a)),
                    list,
                    move || close_palette(ui),
                )
                .top(px(TOOLBAR_HEIGHT))
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use super::typed_target;

    #[test]
    fn the_input_row_opens_the_inline_completion_it_shows() {
        let completed = Some(("neverssl.test/".to_owned(), "http://neverssl.test/".to_owned()));
        assert_eq!(typed_target("neverssl.test/".to_owned(), completed.clone()), "http://neverssl.test/");
        assert_eq!(typed_target("neverssl.te".to_owned(), completed), "neverssl.te", "a completion of other text is stale");
        assert_eq!(typed_target("example.com".to_owned(), None), "example.com");
    }
}
