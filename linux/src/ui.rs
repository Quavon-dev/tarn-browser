mod appearance;
mod library;
mod panels;
mod settings;
mod tools;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use browser_core::tabs::TabKey;
use futures_util::StreamExt;
use nsh_base::input;
use nsh_core::JsonValue;
use nsh_core::desktop::{WindowAttributes, WindowPlacement};
use nsh_core::element::BoxShadow;
use nsh_core::element::desktop::{WindowRef, current_window, main_window, open_window, quit, window};
use nsh_core::prelude::*;
use nsh_core::{LayoutCurve, LayoutTransition};
use nsh_core::{Points, Size};
use serde_json::json;

use crate::browser::{Browser, Notice, START_URL};
use crate::controls::{self, body, icon_button, label, pill, row, small};
use crate::icons;
use crate::shortcuts::Action;
use crate::snapshot::{self, RowKind, Snapshot};
use crate::theme::{Paint, ROW_HEIGHT, SYSTEM_FONT, Secure, TEXT, TEXT_TITLE, TOOLBAR_HEIGHT, Theme, themed};
use browser_core::command::{ENGINES, Input, classify_input};
use browser_core::{Core, suggest};
use store::SavedWindow;
use tarn_nsh_shared::controls::{secure_button, secure_surface};

use crate::data::Data;

const SIDEBAR_WIDTH: f32 = 256.;
const TAB_ROW: f32 = 30.;
const CARD_WIDTH: f32 = 680.;
const ADDRESS_WIDTH: f32 = 560.;
const CLOSE_WIDTH: f32 = 28.;
const CLOSE_GAP: f32 = 10.;
const CLOSE_MS: f32 = 130.;
use crate::webview;

const PLACEMENT: &str = "linux.window_placement";
const SCHEDULE_TICK: std::time::Duration = std::time::Duration::from_secs(60);
const SAMPLE_TICK: std::time::Duration = std::time::Duration::from_secs(5);
const SUGGESTIONS: usize = 6;
const WINDOW_SIZE: (f32, f32) = (1280., 800.);

thread_local! {
    static CORE: RefCell<Option<Rc<Core<Data>>>> = const { RefCell::new(None) };
    static BROWSERS: RefCell<HashMap<u64, Browser>> = RefCell::default();
    static WINDOWS: RefCell<Vec<Ui>> = const { RefCell::new(Vec::new()) };
    static NEXT_WINDOW: Cell<u64> = const { Cell::new(0) };
    /// Each window's web views by tab, with the wake number the view was made for.
    static REFS: RefCell<HashMap<(u64, TabKey), (u64, ElementRef)>> = RefCell::default();
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    None,
    Shield,
    Settings,
    History,
    Downloads,
    Tab,
    Space,
    Memory,
    ReadingList,
    Qr,
}

#[derive(Clone, Copy)]
pub struct Ui {
    window: u64,
    snap: Signal<Snapshot>,
    panel: Signal<Panel>,
    rev: Signal<u64>,
    sidebar: Signal<bool>,
    address: Signal<String>,
    typed: Signal<String>,
    completed: Signal<Option<(String, String)>>,
    editing: Signal<bool>,
    command: Signal<String>,
    suggest: Signal<Vec<(String, String)>>,
    finding: Signal<bool>,
    find: Signal<String>,
    address_ref: ElementRef,
    command_ref: ElementRef,
    find_ref: ElementRef,
    answer_text: Signal<String>,
    answer_user: Signal<String>,
    answer_password: Signal<String>,
    find_tabs: Signal<bool>,
    css: Signal<Option<String>>,
    /// The host `css` was read for; the field follows the page to another host.
    css_host: Signal<String>,
    css_error: Signal<Option<String>>,
    qr: Signal<Option<(Vec<Vec<bool>>, String)>>,
    keys_rev: Signal<u64>,
    shortcut_edit: Signal<Option<crate::shortcuts::Action>>,
    shortcut_text: Signal<String>,
    shortcut_error: Signal<Option<String>>,
    card_back: Signal<bool>,
    reader_size: Signal<usize>,
    reader_width: Signal<usize>,
    reader_speed: Signal<usize>,
    speaking: Signal<bool>,
    toast_timer: Signal<u64>,
    reloaded: Signal<u64>,
    settings_pane: Signal<tarn_nsh_shared::panes::Pane>,
    open: Signal<bool>,
    root: Owner,
}

pub fn act(ui: Ui, f: impl FnOnce(&mut Browser)) {
    let mut f = Some(f);
    act_dyn(ui, &mut |b| f.take().into_iter().for_each(|f| f(b)));
}

fn act_dyn(ui: Ui, f: &mut dyn FnMut(&mut Browser)) {
    let shown = ui.panel.get_untracked() == Panel::None;
    let done = BROWSERS.with_borrow_mut(|windows| {
        let elsewhere = live_elsewhere(windows, ui.window);
        let b = windows.get_mut(&ui.window)?;
        b.elsewhere = elsewhere;
        f(b);
        let moved = b.model.active.filter(|k| b.elsewhere.contains(k));
        let forgotten = std::mem::take(&mut b.forgotten);
        let shown = shown && b.reader.as_ref().is_none_or(|r| b.model.active != Some(r.key));
        Some((
            std::mem::take(&mut b.outbox),
            std::mem::take(&mut b.released),
            std::mem::take(&mut b.cancelled_downloads),
            snapshot::snapshot(b, shown),
            moved,
            forgotten,
        ))
    });
    let Some((commands, released, cancelled, snap, moved, forgotten)) = done else { return };
    ui.snap.set(snap);
    if !(released.is_empty() && cancelled.is_empty()) {
        drop(run_on_main_thread(move || {
            released.iter().for_each(webview::session::release);
            cancelled.iter().for_each(|id| webview::session::cancel(id));
        }));
    }
    tools::expire_toast(ui);
    for (key, name, args) in commands {
        if let Some(r) = REFS.with_borrow(|refs| refs.get(&(ui.window, key)).map(|(_, r)| *r)) {
            r.command(name, args);
        }
    }
    ui.rev.update(|r| *r += 1);
    for other in WINDOWS.with_borrow(Vec::clone).into_iter().filter(|o| o.window != ui.window) {
        if let Some(key) = moved
            && read(other, |b| b.model.lifecycle.live().any(|k| k == key)) == Some(true)
        {
            act_dyn(other, &mut |b| b.give_up(key));
        }
        if read(other, |b| forgotten.iter().any(|k| b.model.tab(*k).is_some() || b.pages.contains_key(k))) == Some(true) {
            act_dyn(other, &mut Browser::sync_shared);
        }
    }
}

fn live_elsewhere(windows: &HashMap<u64, Browser>, here: u64) -> HashSet<TabKey> {
    windows.iter().filter(|(window, _)| **window != here).flat_map(|(_, b)| b.model.lifecycle.live()).collect()
}

pub fn read<R>(ui: Ui, f: impl FnOnce(&Browser) -> R) -> Option<R> {
    BROWSERS.with_borrow(|windows| windows.get(&ui.window).map(f))
}

pub fn show(ui: Ui, panel: Panel) {
    let next = if ui.panel.get_untracked() == panel { Panel::None } else { panel };
    ui.css.set(None);
    ui.css_error.set(None);
    ui.panel.set(next);
    act(ui, |_| {});
}

fn new_tab(ui: Ui) {
    ui.panel.set(Panel::None);
    act(ui, |b| {
        b.new_tab(START_URL);
    });
    ui.command.set(String::new());
    ui.command_ref.focus();
}

fn web_views(ui: Ui) -> View {
    view().flex_col().child(each(
        move || ui.snap.with(|s| s.views.clone()),
        |v| (v.key, v.born),
        move |item, _| {
            let (key, born) = item.with_untracked(|v| (v.key, v.born));
            let web = element_ref();
            REFS.with_borrow_mut(|refs| refs.insert((ui.window, key), (born, web)));
            cleanup(move || {
                REFS.with_borrow_mut(|refs| {
                    if refs.get(&(ui.window, key)).is_some_and(|(b, _)| *b == born) {
                        refs.remove(&(ui.window, key));
                    }
                });
            });
            native(webview::NAME)
                .prop("tab", JsonValue::from(format!("{}-{}", key.private, key.id)))
                .prop("profile", move || JsonValue::from(item.with(|v| v.profile.clone())))
                .prop("level", move || JsonValue::from(item.with(|v| v.level.clone())))
                .prop("levels", move || item.with(|v| v.levels.clone()))
                .prop("lists", move || JsonValue::from(item.with(|v| v.lists.clone())))
                .prop("styles", move || item.with(|v| v.styles.clone()))
                .prop("themeStyles", move || item.with(|v| v.theme_styles.clone()))
                .prop("zaps", move || item.with(|v| v.zaps.clone()))
                .prop("sortDownloads", move || JsonValue::from(item.with(|v| v.sort_downloads)))
                .prop("load", move || item.with(|v| v.load.as_ref().map_or(JsonValue::Null, |(u, s)| json!({ "url": u, "seq": s }))))
                .prop("active", move || JsonValue::from(item.with(|v| v.active)))
                .on_native_event(move |event: &NativeEvent| act(ui, |b| b.on_event(key, &event.name, &event.data)))
                .element_ref(&web)
                .flex_grow(move || if item.with(|v| v.active) { 1.0_f32 } else { 0.0 })
                .h(px(0.0))
        },
    ))
}

pub fn site_tile(host: &str, size: f32) -> View {
    let letter = host.trim_start_matches("www.").chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    let tile = view().w(px(size)).h(px(size)).flex_none().rounded(px(size * 0.28)).items_center().justify_center();
    if letter.is_empty() {
        return tile.bg(Theme::control());
    }
    tile.bg(Theme::monogram(host)).child(text(letter).text_size(px(size * 0.62)).font_weight(FontWeight::BOLD).text_color(Theme::text()))
}

fn space_tint(color: &str, alpha: f32) -> Option<Rgba> {
    Theme::profile_color(color).map(|c| Rgba { a: alpha, ..c })
}

fn tab_key(tab: &str) -> Option<TabKey> {
    let (private, id) = tab.split_once('-')?;
    Some(TabKey { private: private.parse().ok()?, id: id.parse().ok()? })
}

fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()
}

fn sidebar_rows(ui: Ui, pinned: bool) -> AnyElement {
    each(
        move || {
            ui.snap.with(|s| s.rows.iter().filter(|r| (r.pinned || r.depth > 0 || matches!(r.kind, RowKind::Folder(_))) == pinned).cloned().collect::<Vec<_>>())
        },
        |r| r.id(),
        move |item, _| {
            let kind = item.with_untracked(|r| r.kind.clone());
            let depth = item.with_untracked(|r| r.depth) as f32;
            let name = move || item.with(|r| r.label.clone());
            match kind {
                RowKind::Tab(key) => {
                    let active = move || item.with(|r| r.active);
                    let hovered = signal(false);
                    let slot = element_ref();
                    let close = (!pinned).then(|| {
                        let shown = move || hovered.get() as u8 as f32;
                        view()
                            .element_ref(&slot)
                            .flex_shrink_0()
                            .overflow_hidden()
                            .w(move || px(CLOSE_WIDTH * shown()))
                            .ml(move || px(-CLOSE_GAP * (1. - shown())))
                            .opacity(shown)
                            .transition_opacity()
                            .child(icon_button(icons::CLOSE, "Close tab", move || act(ui, |b| b.close_tab(key))))
                    });
                    let hover = move |on: bool| {
                        if !pinned && !reduce_motion() {
                            slot.animate_layout(LayoutTransition::new(LayoutCurve::EaseOut, CLOSE_MS));
                        }
                        hovered.set(on);
                    };
                    let tab = row(move || Some(SharedString::from(name())), move || act(ui, |b| b.activate(key)))
                        .h(px(TAB_ROW))
                        .relative()
                        .pl(px(10. + depth * 16.))
                        .pr_1()
                        .gap(px(CLOSE_GAP))
                        .bg(move || if active() { Theme::selected() } else { rgba(0) })
                        .child(when(active, || view().absolute().left(px(0.)).top(px(7.)).bottom(px(7.)).w(px(3.)).rounded_full().bg(Theme::accent())))
                        .child({
                            let mark = computed(move || item.with(|r| (r.loading && !reduce_motion(), r.host.clone())));
                            dynamic(move || match mark.get() {
                                (true, _) => view()
                                    .text_color(Theme::space_label())
                                    .child(tarn_nsh_shared::motion::spinner(14., icons::icon(icons::RELOAD, 14.)))
                                    .into_element(),
                                (false, host) => site_tile(&host, 16.).opacity(move || if item.with(|r| r.asleep) { 0.4_f32 } else { 1. }).into_element(),
                            })
                        })
                        .child(
                            body(name)
                                .text_color(move || match item.with(|r| (r.active, r.asleep)) {
                                    (true, _) => Theme::text(),
                                    (false, true) => Theme::asleep(),
                                    (false, false) => Theme::muted(),
                                })
                                .flex_1()
                                .truncate(),
                        )
                        .child(dynamic(move || {
                            let (burner, heavy) = item.with(|r| (r.burner, r.heavy));
                            let mark = if burner {
                                Some((icons::FLAME, Theme::warning(), "Burner tab: nothing is kept after it closes"))
                            } else if heavy {
                                Some((icons::GAUGE, Theme::muted(), "Awake in the background and using memory. Sleep Tab frees it."))
                            } else {
                                None
                            };
                            mark.map(|(icon, tint, name)| view().text_color(tint).accessible(true).label(name).child(icons::icon(icon, 13.)))
                        }))
                        .children(close);
                    view().on_mouse_enter(move |_: &MouseEvent| hover(true)).on_mouse_leave(move |_: &MouseEvent| hover(false)).child(tab).into_element()
                }
                RowKind::Folder(id) => row(move || Some(SharedString::from(name())), move || act(ui, |b| b.toggle_folder(id)))
                    .h(px(TAB_ROW))
                    .pl(px(8. + depth * 16.))
                    .child(
                        view()
                            .text_color(Theme::muted())
                            .child(dynamic(move || icons::icon(if item.with(|r| r.open) { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }, 12.))),
                    )
                    .child(view().text_color(Theme::space_label()).child(icons::icon(icons::FOLDER, 15.)))
                    .child(label(name).font_weight(FontWeight::SEMIBOLD).flex_1().truncate())
                    .into_element(),
            }
        },
    )
}

fn space_bar(ui: Ui) -> View {
    let spaces = each(
        move || ui.snap.with(|s| s.spaces.clone()),
        |s| s.ix,
        move |space, _| {
            let (ix, icon, name, private, color) = space.with_untracked(|s| (s.ix, s.icon.clone(), s.label.clone(), s.private, s.color.clone()));
            let source = if private { icons::PERSON } else { icons::space_icon(&icon) };
            controls::row(name, move || act(ui, |b| b.switch_space_by_user(ix)))
                .w(px(20.))
                .h(px(20.))
                .px_0()
                .justify_center()
                .rounded_full()
                .text_color(move || match (space.with(|s| s.current), private) {
                    (true, true) => Theme::warning(),
                    (true, false) => space_tint(&color, 1.).unwrap_or(Theme::text()),
                    (false, _) => space_tint(&color, 0.45).unwrap_or(Theme::faint()),
                })
                .child(icons::icon(source, 13.))
        },
    );
    view()
        .flex_row()
        .items_center()
        .h(px(40.))
        .px_1()
        .child(icon_button(icons::CHEVRON_UP, "History", move || show(ui, Panel::History)))
        .child(view().flex_row().flex_1().items_center().justify_center().gap(px(2.)).child(spaces))
        .child(icon_button(icons::GEAR, "This space", move || show(ui, Panel::Space)))
        .child(icon_button(icons::PLUS, "New space", move || act(ui, |b| b.new_space())))
}

fn downloads_button(ui: Ui) -> AnyElement {
    dynamic(move || {
        let progress = ui.snap.with(|s| s.downloading);
        let button = icon_button(icons::CIRCLE_ARROW_DOWN, "Downloads", move || show(ui, Panel::Downloads));
        match progress {
            Some(p) => view()
                .flex_row()
                .items_center()
                .text_color(Theme::accent())
                .child(button.text_color(Theme::accent()))
                .child(small(format!("{:.0}%", p * 100.)).text_color(Theme::accent()))
                .into_element(),
            None => button.into_element(),
        }
    })
}

fn sidebar(ui: Ui) -> View {
    let tint = move || ui.snap.with(|s| if s.private { Theme::warning() } else { Theme::profile_color(&s.profile_color).unwrap_or(Theme::space_label()) });
    let glyph = move || ui.snap.with(|s| s.spaces.iter().find(|x| x.current && !x.private).and_then(|x| space_tint(&x.color, 1.))).unwrap_or_else(tint);
    let header = view()
        .flex_row()
        .items_center()
        .gap_1()
        .h(px(TOOLBAR_HEIGHT - 8.))
        .pl_2()
        .child(view().text_color(glyph).child(dynamic(move || {
            let icon = ui.snap.with(|s| s.space_icon.clone());
            icons::icon(icons::space_icon(&icon), 14.)
        })))
        .child(
            text(move || ui.snap.with(|s| s.space_name.clone()))
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(tint)
                .ml(px(2.))
                .flex_1()
                .truncate(),
        )
        .child(icon_button(icons::FOLDER, "New folder", move || act(ui, |b| b.new_folder())))
        .child(icon_button(icons::BOOKMARK, "Reading List", move || show(ui, Panel::ReadingList)))
        .child(downloads_button(ui));
    let new = controls::pressable("New Tab", move || new_tab(ui))
        .flex_row()
        .items_center()
        .gap_2()
        .h(px(ROW_HEIGHT - 4.))
        .px(px(10.))
        .mb_1()
        .rounded_full()
        .bg(Theme::accent_soft())
        .border_1()
        .border_color(Theme::accent())
        .text_color(Theme::text())
        .hover(|s| s.bg(Theme::selected()).border_color(Theme::space_label()))
        .child(icons::icon(icons::PLUS, 14.))
        .child(body("New Tab").text_color(Theme::text()));
    let list = view()
        .flex_col()
        .gap(px(2.))
        .px_1()
        .pb_2()
        .child(sidebar_rows(ui, true))
        .child(view().h(px(1.)).mx_2().my_2().bg(Theme::hairline()))
        .child(new)
        .child(sidebar_rows(ui, false));
    view()
        .flex_col()
        .w(px(SIDEBAR_WIDTH))
        .p_1()
        .rounded(px(Theme::radius_tile()))
        .paint(Theme::sidebar())
        .border_1()
        .border_color(Theme::sidebar_edge())
        .shadow(Theme::elevation(10., 30.))
        .on_wheel(move |e: &WheelEvent| {
            if let WheelDelta::Points(p) = e.delta {
                let step = BROWSERS.with_borrow_mut(|w| w.get_mut(&ui.window)?.swipe.feed(p.x.0, p.y.0, std::time::Instant::now()));
                if let Some(step) = step {
                    act(ui, |b| b.swipe_to(step));
                }
            }
        })
        .child(header)
        .child(scroll().flex_1().child(list))
        .child(space_bar(ui))
}

fn history(ui: Ui) -> Vec<store::HistoryEntry> {
    read(ui, |b| b.model.profile().map(|(p, id)| b.core.store(p).history(id, 500).unwrap_or_default()).unwrap_or_default()).unwrap_or_default()
}

fn show_suggestions(ui: Ui, rows: Vec<(String, String)>, query: &str, current: Signal<String>) {
    let found = read(ui, |b| b.cached_text_matches(query)).flatten();
    let shown = crate::input::with_text(rows.clone(), found.as_deref().unwrap_or_default(), SUGGESTIONS);
    ui.suggest.set(shown.clone());
    if found.is_some() {
        return;
    }
    let query = query.to_owned();
    ui.root.run(|| {
        drop(set_timeout(suggest::TEXT_SEARCH_DELAY, move || {
            if current.get_untracked() != query || !ui.suggest.with_untracked(|s| *s == shown) {
                return;
            }
            let found = BROWSERS.with_borrow_mut(|windows| windows.get_mut(&ui.window).map(|b| b.search_text_matches(&query, SUGGESTIONS))).unwrap_or_default();
            ui.suggest.set(crate::input::with_text(rows, &found, SUGGESTIONS));
        }))
    });
}

fn open_tabs(ui: Ui) -> Vec<store::Tab> {
    read(ui, |b| b.model.tabs.clone()).unwrap_or_default()
}

fn go(ui: Ui, text: String) {
    if ui.find_tabs.get_untracked() {
        if let Some(row) = ui.snap.with_untracked(|s| s.find_rows.first().cloned()) {
            let mut query = String::new();
            act(ui, |b| query = b.open_find_row(row.key));
            tools::close_find_tabs(ui);
            ui.find.set(query.clone());
            ui.finding.set(true);
            act(ui, |b| b.find(&query, false));
        }
        return;
    }
    let text = ui.completed.get_untracked().filter(|(full, _)| *full == text).map_or(text, |(_, url)| url);
    ui.editing.set(false);
    ui.suggest.set(Vec::new());
    ui.address_ref.blur();
    act(ui, |b| b.navigate(&text));
}

fn shield_button(ui: Ui) -> nsh_base::ButtonElement {
    let standard = move || ui.snap.with(|s| s.level == "standard");
    let overridden = move || ui.snap.with(|s| s.overridden);
    let badge = move || if standard() && !overridden() { String::new() } else { ui.snap.with(|s| panels::capitalised(&s.level)) };
    let color = move || if standard() { Secure::muted() } else { Secure::shield_raised() };
    row("Shield", move || show(ui, Panel::Shield))
        .h(px(22.))
        .px(px(6.))
        .gap_1()
        .rounded(px(Theme::radius_small()))
        .bg(move || if overridden() { Theme::accent_soft() } else { Theme::clear() })
        .hover(|s| s.bg(Theme::hover()))
        .text_color(color)
        .child(dynamic(move || icons::icon(if standard() { icons::SHIELD } else { icons::SHIELD_CHECK }, 14.)))
        .child(small(badge).text_color(color).font_family(SYSTEM_FONT))
}

fn address(ui: Ui) -> View {
    let field = input()
        .value(move || {
            if ui.editing.get() || ui.find_tabs.get() {
                ui.address.get()
            } else {
                ui.snap.with(|s| if s.url == START_URL { String::new() } else { crate::input::display_host(&s.url) })
            }
        })
        .on_focus(move |_: &()| {
            let url = ui.snap.with_untracked(|s| if s.url == START_URL { String::new() } else { s.url.clone() });
            ui.address.set(url.clone());
            ui.typed.set(String::new());
            ui.completed.set(None);
            ui.editing.set(true);
            ui.address_ref.set_selection(0, url.chars().count() as u32);
        })
        .on_blur(move |_: &()| {
            if !ui.find_tabs.get_untracked() {
                ui.editing.set(false);
            }
            ui.suggest.set(Vec::new());
        })
        .on_change(move |value: &str| {
            if ui.find_tabs.get_untracked() {
                ui.address.set(value.to_owned());
                let query = value.to_owned();
                act(ui, |b| b.find_in_tabs(&query));
            } else if ui.editing.get_untracked() && value != ui.address.get_untracked() {
                let history = history(ui);
                let (rows, completion) = suggest::edit(&ui.typed.get_untracked(), value, &open_tabs(ui), &history, SUGGESTIONS, browser_core::now());
                ui.typed.set(value.to_owned());
                show_suggestions(ui, rows.into_iter().map(|h| (h.title.clone(), h.url.clone())).collect(), value, ui.typed);
                let full = format!("{value}{}", completion.as_ref().map_or("", |c| c.suffix.as_str()));
                ui.completed.set(completion.map(|c| (full.clone(), c.url.to_string())));
                ui.address.set(full.clone());
                if full.len() > value.len() {
                    ui.address_ref.set_selection(value.chars().count() as u32, full.chars().count() as u32);
                }
            }
        })
        .on_submit(move |value: &str| go(ui, value.to_owned()))
        .placeholder(move || if ui.find_tabs.get() { "Find in this space's tabs" } else { "Search or enter address" })
        .label("Address")
        .element_ref(&ui.address_ref)
        .flex_1()
        .text_size(px(TEXT))
        .text_color(move || match (ui.editing.get(), ui.snap.with(|s| s.secure)) {
            (true, _) => Theme::text(),
            (false, true) => Secure::text(),
            (false, false) => Secure::warning(),
        });
    let ring = || {
        vec![BoxShadow { offset_x: Points(0.), offset_y: Points(0.), blur: Points(0.), spread: Points(3.), color: Theme::accent_soft().into(), inset: false }]
    };
    let capsule = view()
        .flex_row()
        .flex_1()
        .max_w(px(ADDRESS_WIDTH))
        .items_center()
        .pl_1()
        .pr(px(4.))
        .h(px(30.))
        .rounded(move || px(Theme::radius_row()))
        .bg(move || if ui.editing.get() { Theme::raised() } else { Secure::chip() })
        .border_1()
        .border_color(move || if ui.editing.get() { Theme::accent() } else { Secure::hairline() })
        .font_family(SYSTEM_FONT)
        .style_when(move || ui.editing.get(), move |s| s.max_w(px(f32::MAX)).shadow(ring()))
        .text_color(Theme::faint)
        .child(view().w(px(28.)).items_center().child(icons::icon(icons::SEARCH, 13.)))
        .child(field)
        .child(themed(move || {
            when(
                move || ui.snap.with(|s| s.article) && !ui.editing.get(),
                move || {
                    icon_button(icons::BOOK_OPEN, "Reader", move || act(ui, |b| b.toggle_reader()))
                        .text_color(move || if ui.snap.with(|s| s.reader.is_some()) { Theme::accent() } else { Theme::muted() })
                },
            )
        }))
        .child(themed(move || when(move || !ui.editing.get() && ui.snap.with(|s| !s.host.is_empty()), move || shield_button(ui))));
    view().flex_row().flex_1().justify_center().child(capsule)
}

fn toolbar(ui: Ui) -> View {
    let navigation = move || {
        let enabled = |on: bool| if on { Theme::muted() } else { Theme::faint() };
        view()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .child(icon_button(icons::SIDEBAR, "Toggle sidebar", move || ui.sidebar.update(|o| *o = !*o)))
            .child(icon_button(icons::BACK, "Back", move || act(ui, |b| b.back())).text_color(move || enabled(ui.snap.with(|s| s.can_back))))
            .child(icon_button(icons::FORWARD, "Forward", move || act(ui, |b| b.forward())).text_color(move || enabled(ui.snap.with(|s| s.can_forward))))
            .child(reload_button(ui))
            .child(private_mark(ui))
    };
    let actions = move || {
        view().flex_row().items_center().gap(px(2.)).child(icon_button(icons::APP_WINDOW, "Tab", move || show(ui, Panel::Tab))).child(icon_button(
            icons::GEAR,
            "Settings",
            move || show(ui, Panel::Settings),
        ))
    };
    view().flex_row().items_center().gap_2().h(px(TOOLBAR_HEIGHT)).child(themed(navigation)).child(address(ui)).child(themed(actions))
}

fn reload(ui: Ui) {
    act(ui, |b| b.reload());
    ui.reloaded.update(|n| *n += 1);
    let serial = ui.reloaded.get_untracked();
    ui.root.run(|| {
        drop(set_timeout(tarn_nsh_shared::motion::RELOAD_SPIN, move || {
            if ui.reloaded.get_untracked() == serial {
                ui.reloaded.set(0);
            }
        }))
    });
}

fn reload_button(ui: Ui) -> AnyElement {
    let reload = move || reload(ui);
    let state = computed(move || (ui.snap.with(|s| s.loading), ui.reloaded.get() > 0));
    dynamic(move || {
        let (loading, reloaded) = state.get();
        if !loading && !reloaded {
            return icon_button(icons::RELOAD, "Reload", reload).into_element();
        }
        let press = move || if ui.snap.with_untracked(|s| s.loading) { act(ui, |b| b.stop()) } else { reload() };
        let name = if loading { "Stop Loading" } else { "Reload" };
        if reduce_motion() {
            return icon_button(icons::CLOSE, name, press).into_element();
        }
        tarn_nsh_shared::controls::icon_button(tarn_nsh_shared::motion::spinner(16., icons::icon(icons::RELOAD, 16.)), name, press)
            .text_color(Theme::text())
            .into_element()
    })
}

fn private_mark(ui: Ui) -> AnyElement {
    dynamic(move || {
        let (private, burner) = ui.snap.with(|s| (s.private, s.burner));
        (private && !ui.sidebar.get()).then(|| {
            let (glyph, name) =
                if burner { (icons::FLAME, "Burner tab: nothing is kept after it closes") } else { (icons::HAND, "Private space: kept in memory only") };
            view()
                .w(px(26.))
                .h(px(26.))
                .items_center()
                .justify_center()
                .text_color(Secure::warning())
                .accessible(true)
                .label(name)
                .child(icons::icon(glyph, 14.))
        })
    })
    .into_element()
}

pub(super) fn suggestion_row(name: String, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    controls::pressable(name, press).flex_row().items_center().gap_3().mx(px(6.)).h(px(ROW_HEIGHT)).px(px(10.)).rounded(px(Theme::radius_row()))
}

pub(super) fn detail(content: impl IntoReactive<SharedString>) -> Text {
    small(content).max_w(px(260.)).truncate()
}

fn input_row(ui: Ui, query: Signal<String>) -> AnyElement {
    dynamic(move || {
        let text = query.get();
        let engine = read(ui, |b| b.search_engine()).unwrap_or_default();
        let (name, note, rejected) = match classify_input(&text, &ENGINES[engine]) {
            Some(Input::Navigate(u)) => (u.to_string(), "Open".to_owned(), false),
            Some(Input::External(u)) => (u.to_string(), format!("Open in another app ({})", u.scheme()), false),
            Some(Input::Rejected) => (text.clone(), "This kind of address cannot be opened".to_owned(), true),
            _ => (text.clone(), format!("Search {}", ENGINES[engine].name), false),
        };
        suggestion_row(name.clone(), move || go(ui, query.get_untracked()))
            .bg(Theme::selected())
            .text_color(Theme::muted())
            .child(view().w(px(18.)).items_center().child(icons::icon(icons::SEARCH, 15.)))
            .child(body(name).text_color(if rejected { Theme::danger() } else { Theme::text() }).flex_1().truncate())
            .child(detail(note))
    })
}

fn suggestion_rows(ui: Ui, query: Signal<String>) -> View {
    view().flex_col().gap(px(2.)).py(px(6.)).child(input_row(ui, query)).child(tools::command_rows(ui, query)).child(each(
        move || ui.suggest.get(),
        |(_, url)| url.clone(),
        move |item, _| {
            let (title, url) = item.get_untracked();
            let host = host_of(&url);
            let target = url.clone();
            suggestion_row(url.clone(), move || go(ui, target.clone()))
                .child(view().w(px(18.)).items_center().child(site_tile(&host, 18.)))
                .child(label(if title.is_empty() { url.clone() } else { title }).flex_1().truncate())
                .child(detail(url.trim_start_matches("https://").trim_end_matches('/').to_owned()))
        },
    ))
}

fn address_suggestions(ui: Ui) -> AnyElement {
    dynamic(move || {
        if ui.find_tabs.get() {
            return Some(controls::card().mb_2().shadow(Theme::elevation(4., 14.)).child(tools::find_rows(ui)).into_element());
        }
        let shown = ui.editing.get() && ui.address.with(|a| !a.trim().is_empty() && ui.snap.with(|s| *a != s.url));
        shown.then(|| controls::card().mb_2().shadow(Theme::elevation(4., 14.)).child(suggestion_rows(ui, ui.address)).into_element())
    })
    .into_element()
}

fn chip() -> View {
    view().flex_row().items_center().gap(px(6.)).h(px(28.)).px(px(10.)).rounded_full().border_1().border_color(Theme::surface_edge()).text_color(Theme::muted())
}

fn start_card(ui: Ui) -> AnyElement {
    when(
        move || ui.snap.with(|s| s.blank) && ui.panel.get() == Panel::None,
        move || {
            let field = input()
                .value(move || ui.command.get())
                .on_change(move |v: &str| {
                    ui.command.set(v.to_owned());
                    show_suggestions(ui, crate::input::suggestions(&history(ui), &[], v, SUGGESTIONS), v, ui.command);
                })
                .on_submit(move |v: &str| {
                    let v = v.to_owned();
                    ui.command.set(String::new());
                    go(ui, v);
                })
                .placeholder("Search or enter address")
                .label("Search or enter address")
                .auto_focus(true)
                .element_ref(&ui.command_ref)
                .flex_1()
                .text_size(px(TEXT_TITLE))
                .text_color(Theme::text());
            let typing = move || !ui.command.with(|c| c.trim().is_empty());
            let dot = view().w(px(8.)).h(px(8.)).rounded_full().bg(move || ui.snap.with(|s| Theme::profile_color(&s.profile_color).unwrap_or(Theme::faint())));
            let footer = view()
                .flex_row()
                .items_center()
                .justify_between()
                .px_3()
                .pb_3()
                .child(chip().child(dot).child(icons::icon(icons::FOLDER, 12.)).child(small(move || ui.snap.with(|s| s.space_name.clone()))))
                .child(
                    chip()
                        .bg(Secure::chip())
                        .border_color(Secure::hairline())
                        .text_color(Secure::muted())
                        .font_family(SYSTEM_FONT)
                        .child(icons::icon(icons::SHIELD, 12.))
                        .child(small(move || ui.snap.with(|s| panels::capitalised(&s.level))).text_color(Secure::muted())),
                );
            let input_row =
                view().flex_row().items_center().gap_3().px_4().h(px(54.)).text_color(Theme::muted()).child(icons::icon(icons::SEARCH, 17.)).child(field);
            let card = view()
                .flex_col()
                .w(px(CARD_WIDTH))
                .rounded(px(Theme::radius_card()))
                .surface()
                .shadow(Theme::elevation(18., 48.))
                .child(input_row)
                .child(when(typing, move || suggestion_rows(ui, ui.command).mb_1().border_t_1().border_color(Theme::hairline())))
                .child(footer);
            let column = view()
                .flex_col()
                .flex_1()
                .items_center()
                .px_4()
                .child(view().flex_grow(0.22_f32))
                .child(icons::mark(64.))
                .child(view().relative().mt(px(28.)).child(tarn_nsh_shared::search_fx::search_fx(54.)).child(card))
                .child(view().flex_grow(0.78_f32));
            view().flex_col().flex_1().relative().child(fog()).child(column)
        },
    )
    .into_element()
}

fn fog() -> View {
    use browser_core::fog::{FRAME, frame, step};
    tarn_nsh_shared::fog::fog(
        |cols, rows, t| frame(cols, rows, step(t)).into_iter().map(|row| (row.text, row.shades)).collect(),
        std::time::Duration::from_secs_f32(FRAME),
    )
}

pub fn bar() -> View {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .min_h(px(42.))
        .pl(px(14.))
        .pr_2()
        .py_1()
        .mb_2()
        .rounded(px(Theme::radius_panel()))
        .surface()
        .shadow(Theme::elevation(4., 14.))
}

pub(super) fn text_field_multiline(ui: Ui, placeholder: &str) -> nsh_base::Input {
    input()
        .value(move || ui.css.get().unwrap_or_default())
        .on_change(move |v: &str| ui.css.set(Some(v.to_owned())))
        .multiline(true)
        .placeholder(placeholder.to_owned())
        .label("Site CSS")
        .h(px(96.))
        .px(px(10.))
        .py(px(6.))
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .border_1()
        .border_color(Theme::hairline())
        .focus(|s| s.border_color(Theme::accent()))
        .text_color(Theme::text())
}

pub(super) fn text_field(value: Signal<String>, name: &str, width: f32) -> nsh_base::Input {
    input()
        .value(move || value.get())
        .on_change(move |v: &str| value.set(v.to_owned()))
        .placeholder(name.to_owned())
        .label(name.to_owned())
        .w(px(width))
        .h(px(26.))
        .px(px(10.))
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .border_1()
        .border_color(Theme::hairline())
        .focus(|s| s.border_color(Theme::accent()))
        .text_color(Theme::text())
}

fn notice_bar(ui: Ui) -> AnyElement {
    dynamic(move || {
        let notice = ui.snap.with(|s| s.notice.clone())?;
        let dismiss = move || act(ui, |b| b.notice = None);
        let (message, buttons): (String, Vec<AnyElement>) = match notice {
            Notice::Message(m) => (m, vec![pill("Dismiss", dismiss).into_element()]),
            Notice::Open { message, .. } => (
                message,
                vec![
                    secure_button("Open Anyway", move || act(ui, Browser::open_anyway), true).into_element(),
                    secure_button("Dismiss", dismiss, false).into_element(),
                ],
            ),
            Notice::NewTab { message, .. } => (
                message,
                vec![
                    controls::accent_pill("Open in New Tab", move || act(ui, Browser::open_blocked_popup)).into_element(),
                    pill("Dismiss", dismiss).into_element(),
                ],
            ),
            Notice::External { url, origin } => {
                let message =
                    if origin.is_empty() { format!("Open “{url}” in another app?") } else { format!("{origin} wants to open “{url}” in another app") };
                let open = secure_button(
                    "Open",
                    move || {
                        drop(open_url(url.clone()));
                        dismiss();
                    },
                    true,
                );
                (message, vec![open.into_element(), secure_button("Cancel", dismiss, false).into_element()])
            }
        };
        if buttons.len() == 1 {
            return Some(bar().child(label(message).flex_1().truncate()).children(buttons));
        }
        let surface = secure_surface(Theme::radius_panel()).flex_row().items_center().gap_2().min_h(px(42.)).pl(px(14.)).pr_2().py_1().mb_2();
        Some(surface.shadow(Theme::elevation(4., 14.)).child(body(message).text_color(Secure::text()).flex_1().truncate()).children(buttons))
    })
}

fn prompt_bar(ui: Ui) -> AnyElement {
    dynamic(move || {
        let prompt = ui.snap.with(|s| s.prompt.clone())?;
        let id = prompt.id;
        let reply = move |accept: bool, remember: bool| {
            let (t, u, p) = (ui.answer_text.get_untracked(), ui.answer_user.get_untracked(), ui.answer_password.get_untracked());
            ui.answer_text.set(String::new());
            ui.answer_user.set(String::new());
            ui.answer_password.set(String::new());
            act(ui, |b| b.answer(id, accept, remember, &t, &u, &p));
        };
        let title = match prompt.kind.as_str() {
            "alert" | "confirm" | "text" => format!("{} says", prompt.origin),
            "credentials" => format!(
                "{} asks you to sign in{}",
                prompt.origin,
                prompt.realm.as_deref().filter(|r| !r.is_empty()).map(|r| format!(" ({r})")).unwrap_or_default()
            ),
            kind => format!("{} wants to use your {kind}", prompt.origin),
        };
        let message = prompt.message.clone().filter(|m| !m.is_empty());
        if prompt.kind == "text" && ui.answer_text.get_untracked().is_empty() {
            ui.answer_text.set(prompt.default.clone().unwrap_or_default());
        }
        let fields: Vec<AnyElement> = match prompt.kind.as_str() {
            "text" => vec![text_field(ui.answer_text, "Answer", 320.).into_element()],
            "credentials" => {
                vec![text_field(ui.answer_user, "User name", 320.).into_element(), text_field(ui.answer_password, "Password", 320.).masked(true).into_element()]
            }
            _ => Vec::new(),
        };
        let (accept, decline) = match prompt.kind.as_str() {
            "alert" => ("OK", None),
            "credentials" => ("Sign In", Some("Cancel")),
            "camera" | "microphone" | "location" => ("Allow", Some("Deny")),
            _ => ("OK", Some("Cancel")),
        };
        let buttons = view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(secure_button(accept, move || reply(true, false), true))
            .children(decline.map(|name| secure_button(name, move || reply(false, false), false)));
        let insecure = prompt.insecure.then(|| body("This connection is not secure: the password would be sent unencrypted.").text_color(Secure::warning()));
        Some(
            secure_surface(Theme::radius_panel())
                .flex_col()
                .gap_2()
                .px_4()
                .py_3()
                .mb_2()
                .shadow(Theme::elevation(4., 14.))
                .role(AccessibilityRole::Alert)
                .child(body(title).font_weight(FontWeight::SEMIBOLD).text_color(Secure::text()))
                .children(insecure)
                .children(message.map(|m| body(m).text_color(Secure::muted())))
                .children(fields)
                .child(buttons),
        )
    })
}

fn close_find(ui: Ui) {
    ui.finding.set(false);
    ui.find.set(String::new());
    act(ui, |b| b.find("", false));
}

fn find_bar(ui: Ui) -> AnyElement {
    let search = move |backwards: bool| {
        let text = ui.find.get_untracked();
        act(ui, |b| b.find(&text, backwards));
    };
    when(
        move || ui.finding.get(),
        move || {
            let field = text_field(ui.find, "Find in page", 260.)
                .on_change(move |v: &str| {
                    ui.find.set(v.to_owned());
                    let text = v.to_owned();
                    act(ui, |b| b.find(&text, false));
                })
                .on_submit(move |_: &str| search(false))
                .auto_focus(true)
                .element_ref(&ui.find_ref);
            bar()
                .h(px(44.))
                .pl_3()
                .text_color(Theme::muted())
                .child(icons::icon(icons::SEARCH, 14.))
                .child(field)
                .child(small(move || if ui.snap.with(|s| s.found == Some(0)) { "No matches" } else { "" }).text_color(Theme::danger()).flex_1())
                .child(pill("Previous", move || search(true)))
                .child(controls::accent_pill("Next", move || search(false)))
                .child(pill("Done", move || close_find(ui)))
        },
    )
    .into_element()
}

fn error_bar(ui: Ui) -> AnyElement {
    dynamic(move || {
        let error = ui.snap.with(|s| s.error.clone())?;
        Some(
            bar()
                .child(body(format!("This page could not be loaded: {error}")).text_color(Theme::danger()).flex_1().truncate())
                .child(controls::accent_pill("Retry", move || act(ui, |b| b.reload())))
                .child(pill("Dismiss", move || act(ui, |b| b.dismiss_error()))),
        )
    })
}

fn content(ui: Ui) -> View {
    let bare = move || ui.snap.with(|s| s.blank || s.reader.is_some()) || ui.panel.get() != Panel::None;
    view()
        .flex_col()
        .flex_1()
        .rounded(px(Theme::radius_page()))
        .overflow_hidden()
        .bg(move || if bare() { rgba(0) } else { Theme::page() })
        .border_1()
        .border_color(move || if bare() { rgba(0) } else { Theme::surface_edge() })
        .child(panels::panel_view(ui))
        .child(themed(move || tools::reader_view(ui)))
        .child(themed(move || start_card(ui)))
        .child(web_views(ui).flex_grow(move || if bare() { 0.0_f32 } else { 1.0 }).h(px(0.0)))
}

fn run_action(ui: Ui, action: Action) {
    match action {
        Action::NewTab => new_tab(ui),
        Action::OpenLocation => ui.address_ref.focus(),
        Action::CloseTab => act(ui, |b| b.model.active.into_iter().for_each(|k| b.close_tab(k))),
        Action::ReopenClosedTab => act(ui, |b| b.reopen()),
        Action::NextTab => act(ui, |b| b.cycle(true)),
        Action::PreviousTab => act(ui, |b| b.cycle(false)),
        Action::TogglePin => act(ui, |b| b.model.active.into_iter().for_each(|k| b.pin_toggle(k))),
        Action::BurnerTab => tools::run(ui, tools::Command::BurnerTab),
        Action::Reload => reload(ui),
        Action::Back => act(ui, |b| b.back()),
        Action::Forward => act(ui, |b| b.forward()),
        Action::Find => {
            ui.finding.set(true);
            ui.find_ref.focus();
        }
        Action::FindInTabs => tools::open_find_tabs(ui),
        Action::ZoomIn => act(ui, |b| b.zoom(1)),
        Action::ZoomOut => act(ui, |b| b.zoom(-1)),
        Action::ZoomReset => act(ui, |b| b.zoom(0)),
        Action::CopyLink => tools::copy_link(ui, false),
        Action::CopyCleanLink => tools::copy_link(ui, true),
        Action::ToggleReader => act(ui, |b| b.toggle_reader()),
        Action::ShowHistory => show(ui, Panel::History),
        Action::ToggleDownloads => show(ui, Panel::Downloads),
        Action::ToggleSidebar => ui.sidebar.update(|o| *o = !*o),
        Action::NewSpace => act(ui, |b| b.new_space()),
        Action::PrivateSpace => act(ui, |b| b.private_space()),
        Action::Settings => show(ui, Panel::Settings),
        Action::NewWindow => new_window(ui),
    }
}

fn bound_keys(ui: Ui) -> AnyElement {
    dynamic(move || {
        ui.keys_rev.get();
        let stored = untracked(|| read(ui, |b| crate::shortcuts::stored(&b.core.disk))).unwrap_or_default();
        for (keys, action) in crate::shortcuts::keymap(&stored) {
            on_key(&keys, move |_| run_action(ui, action));
        }
        bind_fixed(ui);
        Some(view())
    })
}

fn escape(ui: Ui) {
    if ui.find_tabs.get_untracked() {
        tools::close_find_tabs(ui);
    } else if ui.editing.get_untracked() {
        ui.editing.set(false);
        ui.suggest.set(Vec::new());
        ui.address_ref.blur();
    } else if ui.finding.get_untracked() {
        close_find(ui);
    } else if ui.panel.get_untracked() != Panel::None {
        show(ui, Panel::None);
    } else if ui.snap.with_untracked(|s| s.reader.is_some()) {
        tools::close_reader(ui);
    } else {
        act(ui, |b| {
            if b.toast.is_some() {
                b.toast = None;
            }
        });
        if let Some(key) = ui.snap.with_untracked(|s| s.views.iter().find(|v| v.active).map(|v| v.key))
            && let Some(r) = REFS.with_borrow(|refs| refs.get(&(ui.window, key)).map(|(_, r)| *r))
        {
            r.command("pickCancel", JsonValue::Null);
        }
    }
}

fn bind_fixed(ui: Ui) {
    on_key("f5", move |_| reload(ui));
    on_key(crate::shortcuts::ZOOM_IN_SHIFTED, move |_| run_action(ui, Action::ZoomIn));
    on_key_with("escape", KeyOptions { when: nsh_core::keymap::When::Always, pass_through: true, ..KeyOptions::default() }, move |_| escape(ui));
    for n in 1..=9usize {
        on_key(&format!("mod+{n}"), move |_| act(ui, |b| b.slot(n)));
    }
}

fn core() -> Rc<Core<Data>> {
    CORE.with_borrow_mut(|core| core.get_or_insert_with(|| Browser::open_core(&crate::data::root())).clone())
}

fn shift_held() -> bool {
    use webkit6::gdk;
    use webkit6::prelude::*;
    let keyboard = gdk::Display::default().and_then(|d| d.default_seat()).and_then(|s| s.keyboard());
    keyboard.is_some_and(|k| k.modifier_state().contains(gdk::ModifierType::SHIFT_MASK))
}

fn keep_placement(ui: Ui, here: WindowRef) {
    let saved = here.is_main().then(|| read(ui, |b| b.core.disk.setting(PLACEMENT).ok().flatten()).flatten()).flatten();
    if let Some(placement) = saved.and_then(|json| serde_json::from_str::<WindowPlacement>(&json).ok()) {
        here.set_placement(placement);
    }
    effect(move || {
        let Some(state) = here.state() else { return };
        let json = here.is_main().then(|| serde_json::to_string(&state.placement).ok()).flatten();
        let size = state.placement.bounds.size;
        untracked(|| {
            BROWSERS.with_borrow_mut(|windows| {
                let Some(b) = windows.get_mut(&ui.window) else { return };
                if let Some(json) = json
                    && b.core.disk.setting(PLACEMENT).ok().flatten().as_deref() != Some(json.as_str())
                {
                    b.core.disk.set_setting(PLACEMENT, &json).ok();
                }
                b.set_window_size(f64::from(size.width.0), f64::from(size.height.0));
            })
        });
    });
}

fn new_window(ui: Ui) {
    let (width, height) = read(ui, |b| (b.window().width, b.window().height)).unwrap_or_default();
    open_browser_window(Browser::window_row(&core(), SavedWindow { width, height, ..SavedWindow::default() }));
}

fn open_browser_window(row: SavedWindow) {
    let (width, height) = if row.width > 0. && row.height > 0. { (row.width as f32, row.height as f32) } else { WINDOW_SIZE };
    let mut attributes = WindowAttributes::default();
    attributes.close_on_press = Some(false);
    open_window(
        window()
            .title("Tarn")
            .default_size(Size { width: Points(width), height: Points(height) })
            .attributes(attributes)
            .content(move || browser_window(new_ui(row.clone()))),
    );
}

fn close_window(ui: Ui, here: &WindowRef) {
    if WINDOWS.with_borrow(|windows| windows.len()) <= 1 {
        return quit();
    }
    act(ui, Browser::close_window);
    BROWSERS.with_borrow_mut(|windows| windows.remove(&ui.window));
    WINDOWS.with_borrow_mut(|windows| windows.retain(|w| w.window != ui.window));
    ui.open.set(false);
    if here.is_main() {
        here.hide();
    } else {
        here.close();
    }
}

fn route_downloads() {
    let mut downloads = webview::session::route();
    drop(spawn(async move {
        while let Some((tab, data)) = downloads.next().await {
            let Some(key) = tab_key(&tab) else { continue };
            let mut recorded = false;
            for ui in WINDOWS.with_borrow(Vec::clone) {
                act(ui, |b| recorded |= b.on_download(key, &data, !recorded));
            }
        }
    }));
}

fn new_ui(row: SavedWindow) -> Ui {
    let window = NEXT_WINDOW.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    let elsewhere = BROWSERS.with_borrow(|windows| live_elsewhere(windows, window));
    let core = core();
    let themes = BROWSERS
        .with_borrow(|windows| windows.values().next().map(|b| b.themes.clone()))
        .unwrap_or_else(|| crate::browser::themes::Themes::open(&core.disk, &crate::data::root().join("filters")));
    let mut browser = Browser::new(core, row, elsewhere, themes);
    browser.downloads = BROWSERS.with_borrow(|windows| windows.values().next().map(|b| b.downloads.clone())).unwrap_or_default();
    BROWSERS.with_borrow_mut(|windows| windows.insert(window, browser));
    let ui = Ui {
        window,
        open: signal(true),
        snap: signal(Snapshot::default()),
        panel: signal(Panel::None),
        rev: signal(0),
        sidebar: signal(true),
        address: signal(String::new()),
        typed: signal(String::new()),
        completed: signal(None),
        editing: signal(false),
        command: signal(String::new()),
        suggest: signal(Vec::new()),
        finding: signal(false),
        find: signal(String::new()),
        address_ref: element_ref(),
        command_ref: element_ref(),
        find_ref: element_ref(),
        answer_text: signal(String::new()),
        answer_user: signal(String::new()),
        answer_password: signal(String::new()),
        find_tabs: signal(false),
        css: signal(None),
        css_host: signal(String::new()),
        css_error: signal(None),
        qr: signal(None),
        keys_rev: signal(0),
        shortcut_edit: signal(None),
        shortcut_text: signal(String::new()),
        shortcut_error: signal(None),
        card_back: signal(false),
        reader_size: signal(2),
        reader_width: signal(0),
        reader_speed: signal(0),
        speaking: signal(false),
        toast_timer: signal(0),
        reloaded: signal(0),
        settings_pane: signal(tarn_nsh_shared::panes::Pane::Profile),
        root: Owner::current(),
    };
    let shown_space = std::cell::Cell::new(None);
    effect(move || {
        let space = ui.snap.with(|s| s.space);
        if shown_space.replace(Some(space)).is_some_and(|before| before != space) {
            untracked(|| {
                ui.command.set(String::new());
                ui.suggest.set(Vec::new());
                if ui.editing.get() {
                    ui.editing.set(false);
                    ui.address_ref.blur();
                }
            });
        }
    });
    WINDOWS.with_borrow_mut(|windows| windows.push(ui));
    ui
}

fn browser_window(ui: Ui) -> AnyElement {
    let here = current_window();
    keep_placement(ui, here.clone());
    here.on_focus_change(move |focused| {
        if focused {
            act(ui, Browser::sync_shared);
        }
    });
    let closing = here.clone();
    here.on_close_press(move |_| close_window(ui, &closing));
    act(ui, |_| {});
    drop(set_interval(SCHEDULE_TICK, move || {
        act(ui, |b| {
            b.schedule_tick();
            b.idle_tick();
        })
    }));
    drop(set_interval(SAMPLE_TICK, move || {
        if ui.speaking.get_untracked() && !crate::speech::speaking() {
            ui.speaking.set(false);
        }
        act(ui, |b| b.sample_processes());
    }));
    act(ui, |b| b.schedule_tick());
    effect(move || {
        let light = !dark();
        untracked(|| act(ui, |b| b.follow_appearance(light)));
    });
    view()
        .flex_row()
        .size_full()
        .relative()
        .p_2()
        .gap_2()
        .text_color(Theme::text)
        .font_family(Theme::font_family)
        .child(themed(|| view().absolute().inset_0().paint(Theme::backdrop())))
        .child(when(move || ui.sidebar.get(), move || themed(move || sidebar(ui))))
        .child(
            view()
                .flex_col()
                .flex_1()
                .child(toolbar(ui))
                .child(address_suggestions(ui))
                .child(notice_bar(ui))
                .child(error_bar(ui))
                .child(prompt_bar(ui))
                .child(themed(move || find_bar(ui)))
                .child(content(ui))
                .child(themed(move || tools::toast_bar(ui))),
        )
        .child(bound_keys(ui))
        .into_element()
}

#[component]
pub fn App() -> impl IntoElement {
    let mut attributes = WindowAttributes::default();
    attributes.close_on_press = Some(false);
    main_window().set(attributes);
    route_downloads();
    let mut rows = Browser::saved_windows(&core()).into_iter();
    let ui = new_ui(rows.next().unwrap_or_default());
    if shift_held() {
        act(ui, Browser::enter_safe_mode);
    }
    rows.for_each(open_browser_window);
    view().size_full().child(when(move || ui.open.get(), move || browser_window(ui)))
}
