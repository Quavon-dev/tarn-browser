use std::rc::Rc;

use browser_core::command::{ENGINES, Input, address, classify_input};
use browser_core::palette::{self, Command, Row, State, listing};
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
    address_text, badge_color, capsule, chip_button, extension_button, picked_fill, row, row_detail, row_glyph, row_name, row_section, row_site, rows,
    search_slot, shield_chip, tool_button, tools,
};
use url::Url;

use super::super::{Screen, Ui, bars};
use crate::theme::{TEXT_TITLE, TOOLBAR_HEIGHT, Theme, themed};

const OFFERED: [Command; 17] = [
    Command::ReopenClosedTab,
    Command::TogglePin,
    Command::CloseDuplicates,
    Command::ZapElement,
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

fn private_mark(ui: Ui) -> impl IntoElement {
    let state = computed(move || ui.read(|s| (s.model.current_space().is_some_and(|c| c.private), s.model.burner_shown(), s.sidebar_open)));
    dynamic(move || {
        let (private, burner, open) = state.get();
        (private && !open).then(|| {
            let (glyph, name) =
                if burner { (glyphs::FLAME, "Burner tab: nothing is kept after it closes") } else { (glyphs::HAND, "Private space: kept in memory only") };
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
    ui.address.set(ui.read(|s| s.active_url()));
    ui.suggest.set(Vec::new());
    ui.picked.set(Some(0));
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
    })
}

fn offered(ui: Ui, command: Command) -> bool {
    OFFERED.contains(&command)
        && match command {
            Command::Reader => ui.read(|s| s.offers_reader()),
            Command::ZapElement => ui.read(|s| s.zappable()),
            Command::BurnerTab => !ui.read(|s| s.in_burner() || <crate::data::Data as crate::data::Platform>::private_process()),
            Command::CopyLink | Command::CopyCleanLink | Command::ShowQrCode | Command::SaveForLater => ui.read(|s| s.share_url().is_some()),
            _ => true,
        }
}

fn commands(ui: Ui, query: &str) -> Vec<(String, Command)> {
    let state = state(ui);
    palette::rows(query, state, |c| offered(ui, c), &[])
        .into_iter()
        .filter_map(|(name, row)| match row {
            Row::Command(c) => Some((name, c)),
            Row::Extra(_) => None,
        })
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
        Command::ReopenClosedTab => quiet(|s| {
            s.reopen_closed();
        }),
        Command::CloseDuplicates => quiet(|s| s.close_duplicates()),
        Command::ZapElement => ui.act(|s| s.pick()),
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
        Command::ToggleSplitView => toggle_split(ui),
        Command::RotateSplitLayout => on_active(ui, |s, k| {
            s.splits.rotate(k);
        }),
        Command::RemoveFromSplitView => on_active(ui, |s, k| s.leave_split(k)),
        Command::CloseSplit => on_active(ui, |s, k| {
            s.splits.close(k);
        }),
        Command::TogglePin => on_active(ui, |s, k| s.toggle_pin(k)),
        Command::SaveSnapshot | Command::Memory | Command::NewWindow | Command::FocusNextPane | Command::FocusPreviousPane | Command::SwapPanes => {}
    }
}

fn pick(ui: Ui, command: Command) {
    ui.fresh.set(None);
    close_palette(ui);
    run(ui, command);
}

fn go(ui: Ui, text: String) {
    ui.suggest.set(Vec::new());
    ui.fresh.set(None);
    ui.editing.set(false);
    dismiss_keyboard();
    ui.act(|s| s.navigate(&text));
}

fn submit(ui: Ui, text: String, completed: Option<(String, String)>) {
    let picked = ui.picked.get_untracked().unwrap_or(0);
    let commands = commands(ui, &text);
    if listing(&text) {
        if let Some((_, command)) = commands.get(picked.saturating_sub(1)) {
            pick(ui, *command);
        }
        return;
    }
    if picked > 0 {
        if let Some((_, command)) = commands.get(picked - 1) {
            return pick(ui, *command);
        }
        if let Some(url) = ui.suggest.with_untracked(|s| s.get(picked - 1 - commands.len()).map(|h| h.url.clone())) {
            return go(ui, url);
        }
    }
    go(ui, completed.filter(|(full, _)| *full == text).map_or(text, |(_, url)| url));
}

fn step(ui: Ui, key: &str) {
    let query = ui.address.get_untracked();
    let commands = commands(ui, &query).len();
    let (first, count) = if listing(&query) { (1, commands) } else { (0, 1 + commands + ui.suggest.with_untracked(Vec::len)) };
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

fn bind_keys(ui: Ui) {
    for key in ["up", "down", "pageup", "pagedown"] {
        on_key_with(key, KeyOptions { when: When::Always, repeat: true, ..KeyOptions::default() }, move |_| step(ui, key));
    }
    on_key_with("escape", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| close_palette(ui));
}

fn picked(ui: Ui, row: usize) -> bool {
    ui.picked.get().unwrap_or(0) == row
}

fn input_row(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let text = ui.address.get();
        if listing(&text) {
            return None;
        }
        let engine = &ENGINES[ui.read(|s| s.search_engine())];
        let (name, note, glyph, rejected) = match classify_input(&text, engine) {
            Some(Input::Navigate(u)) => (u.to_string(), "Open".to_owned(), glyphs::GLOBE, false),
            Some(Input::External(u)) => (u.to_string(), format!("Open in another app ({})", u.scheme()), glyphs::EXTERNAL_LINK, false),
            Some(Input::Rejected) => (text.clone(), "This kind of address cannot be opened".to_owned(), glyphs::TRIANGLE_ALERT, true),
            _ => (text.clone(), format!("Search {}", engine.name), glyphs::SEARCH, false),
        };
        Some(
            row(name.clone(), move || submit(ui, ui.address.get_untracked(), None))
                .bg(move || picked_fill(picked(ui, 0)))
                .child(row_glyph(glyph, move || picked(ui, 0)))
                .child(row_name(name, if rejected { Theme::danger() } else { Theme::text() }))
                .child(row_detail(note)),
        )
    })
}

fn command_rows(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let query = ui.address.get();
        let found = untracked(|| commands(ui, &query));
        let shown = !found.is_empty();
        let items = found.into_iter().enumerate().map(move |(ix, (name, command))| {
            row(name.clone(), move || pick(ui, command))
                .bg(move || picked_fill(picked(ui, ix + 1)))
                .child(row_glyph(glyphs::glyph(browser_core::command::glyph(&name)), move || picked(ui, ix + 1)))
                .child(row_name(name, Theme::text()))
                .children(shortcuts::for_command(command).and_then(|a| shortcuts::keys_for(a, apple_keys())).map(|k| key_caps(&k)))
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
            row(url.clone(), move || go(ui, target.clone()))
                .bg(move || picked_fill(picked(ui, first.get() + ix.get())))
                .child(row_site(None, &site))
                .child(row_name(if entry.title.is_empty() { url.clone() } else { entry.title.clone() }, Theme::text()))
                .child(row_detail(url.trim_start_matches("https://").trim_end_matches('/').to_owned()))
        },
    ))
}

fn palette_field(ui: Ui) -> impl IntoElement {
    let typed = signal(String::new());
    let completed = signal(None::<(String, String)>);
    let history = Rc::new(ui.read(|s| s.suggestion_history()));
    let at = element_ref();
    input()
        .element_ref(&at)
        .value(move || ui.address.get())
        .on_focus(move || at.set_selection(0, ui.address.with_untracked(|a| a.encode_utf16().count() as u32)))
        .on_change(move |value: &str| {
            if value == ui.address.get_untracked() {
                return;
            }
            let (found, completion) = ui.read(|s| s.suggest_from(&history, &typed.get_untracked(), value));
            typed.set(value.to_owned());
            ui.suggest.set(found);
            ui.picked.set(Some(usize::from(listing(value))));
            let full = format!("{value}{}", completion.as_ref().map_or("", |c| c.suffix.as_str()));
            completed.set(completion.map(|c| (full.clone(), c.url.to_string())));
            ui.address.set(full.clone());
            if full.len() > value.len() {
                at.set_selection(value.encode_utf16().count() as u32, full.encode_utf16().count() as u32);
            }
        })
        .on_submit(move |value: &str| submit(ui, value.to_owned(), completed.get_untracked()))
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
            bind_keys(ui);
            let typed = move || ui.address.with(|a| !a.trim().is_empty()) && ui.read(|s| ui.address.with(|a| *a != s.active_url()));
            themed(move || {
                let list = dynamic(move || typed().then(|| rows().child(input_row(ui)).child(command_rows(ui)).child(suggestion_rows(ui))));
                tarn_nsh_shared::toolbar::palette(palette_field(ui), move || ui.address.with(|a| listing(a)), list, move || close_palette(ui))
                    .top(px(TOOLBAR_HEIGHT))
            })
        },
    )
}
