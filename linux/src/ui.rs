mod appearance;
mod blocking;
mod devtools;
mod extensions;
mod groups;
mod import;
mod library;
mod link_peek;
mod panels;
mod peek;
mod profiles;
mod settings;
mod split;
mod tools;

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use browser_core::tabs::TabKey;
use futures_util::StreamExt;
use nsh_base::input;
use nsh_core::JsonValue;
use nsh_core::desktop::{WindowAttributes, WindowPlacement};
use nsh_core::element::IntoProp;
use nsh_core::element::desktop::{WindowRef, current_window, main_window, open_window, quit, window};
use nsh_core::prelude::*;
use nsh_core::{LayoutCurve, LayoutTransition};
use nsh_core::{Points, Size};
use serde_json::json;
use tarn_nsh_shared::motion::{POP, SLIDE, animate_next};

use crate::browser::{Browser, Notice, START_URL};
use crate::controls::{self, body, icon_button, label, pill, row, small};
use crate::icons;
use crate::shortcuts::Action;
use crate::snapshot::{self, RowKind, Snapshot};
use crate::theme::{Paint, SYSTEM_FONT, Secure, TEXT_TITLE, TOOLBAR_HEIGHT, Theme, themed};
use browser_core::command::{ENGINES, Input, Section, classify_input};
use browser_core::{Core, suggest};
use store::SavedWindow;
use tarn_nsh_shared::controls::{icon_size, secure_button, secure_surface};

use crate::data::Data;

const SIDEBAR_WIDTH: f32 = 256.;
const TAB_ROW: f32 = 30.;
const CARD_WIDTH: f32 = 680.;
const ADDRESS_WIDTH: f32 = 560.;
const COMPACT_FIELD: f32 = 28.;
const COMPACT_TITLE: f32 = 360.;
const CLOSE_WIDTH: f32 = 28.;
const CLOSE_GAP: f32 = 10.;
const TILE_GAP: f32 = 6.;
/// A space button as big as the footer's icon buttons; more spaces than fit wrap to another row.
const SPACE_BUTTON: f32 = 28.;
const FIELD: f32 = 30.;
const CHIP_INSET: f32 = 3.;
const CHIP: f32 = FIELD - 2. - 2. * CHIP_INSET;
const CHIP_GLYPH: f32 = 14.;
/// The sidebar list's width inside its border and padding.
const LIST_WIDTH: f32 = SIDEBAR_WIDTH - 2. - 16.;
use crate::webview;

const PLACEMENT: &str = "linux.window_placement";
const SCHEDULE_TICK: std::time::Duration = std::time::Duration::from_secs(60);
const SAMPLE_TICK: std::time::Duration = std::time::Duration::from_secs(5);
const RETIRE_DELAY: std::time::Duration = std::time::Duration::from_secs(3);
const SUGGESTIONS: usize = 6;
const PALETTE_WIDTH: f32 = 640.;
const PALETTE_ROW: f32 = 40.;
const PROMPT_RADIUS: f32 = 12.;
const FIND_RADIUS: f32 = 12.;
const FIND_WIDTH: f32 = 380.;
const PALETTE_DIM: u32 = 0x0000_0059;
const WINDOW_SIZE: (f32, f32) = (1280., 800.);

thread_local! {
    static CORE: RefCell<Option<Rc<Core<Data>>>> = const { RefCell::new(None) };
    static BROWSERS: RefCell<HashMap<u64, Browser>> = RefCell::default();
    static WINDOWS: RefCell<Vec<Ui>> = const { RefCell::new(Vec::new()) };
    static NEXT_WINDOW: Cell<u64> = const { Cell::new(0) };
    /// Saved windows still to open at launch; the launch archive waits for all of them.
    static LAUNCHING: Cell<usize> = const { Cell::new(0) };
    /// Each window's web views by tab, with the wake number the view was made for.
    static REFS: RefCell<HashMap<(u64, TabKey), (u64, ElementRef)>> = RefCell::default();
    static WINDOW_REFS: RefCell<HashMap<u64, WindowRef>> = RefCell::default();
    static FOCUSED: Cell<u64> = const { Cell::new(0) };
    static INCOMING: RefCell<Vec<std::time::Instant>> = const { RefCell::new(Vec::new()) };
    static PENDING_LINKS: RefCell<Vec<(Vec<String>, usize)>> = const { RefCell::new(Vec::new()) };
    static LAUNCH_ARGS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    None,
    Shield,
    Settings,
    Library,
    History,
    Downloads,
    Tab,
    Space,
    NewSpace,
    Memory,
    ReadingList,
    Archived,
    Qr,
    Share,
    Address,
    Shortcuts,
    Import,
    Folder(store::Id),
    Bookmarks(Option<store::Id>),
    Group(store::Id),
    Sidebar,
    Card(Option<store::Id>),
    SpaceMenu(crate::browser::profiles::SpaceKey),
}

#[derive(Clone, Copy)]
pub struct Ui {
    window: u64,
    snap: Signal<Snapshot>,
    icons: Signal<crate::browser::icons::SiteIcons>,
    panel: Signal<Panel>,
    rev: Signal<u64>,
    sidebar: Signal<bool>,
    address: Signal<String>,
    typed: Signal<String>,
    completed: Signal<Option<(String, String)>>,
    editing: Signal<bool>,
    new_tab_card: Signal<bool>,
    command: Signal<String>,
    suggest: Signal<Vec<(String, String)>>,
    tabs: Signal<Vec<crate::browser::links::TabRow>>,
    pasteable: Signal<bool>,
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
    site_more: Signal<bool>,
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
    ext_popup: Signal<Option<(store::Id, String, u64)>>,
    ext_pane: Signal<bool>,
    remember: Signal<bool>,
    /// The suggestion row the arrow keys chose; 0 is what was typed.
    picked: Signal<usize>,
    /// The address history of the profile it was read for.
    history: Signal<(Option<Profile>, Rc<Vec<store::HistoryEntry>>)>,
    /// The tab the Tab panel is about; `None` is the active one.
    tab_menu: Signal<Option<TabKey>>,
    confirm: Signal<Option<String>>,
    /// The name field of the space or folder panel; empty until it shows the current name.
    rename: Signal<String>,
    /// What `rename` names, so a panel that changes target starts from the new name.
    rename_for: Signal<String>,
    mark_query: Signal<String>,
    mark_edit: Signal<Option<library::MarkEdit>>,
    library_query: Signal<String>,
    /// The emoji field of the folder panel and the folder it belongs to.
    emoji: Signal<String>,
    emoji_for: Signal<String>,
    space_draft: Signal<Option<browser_core::space_card::SpaceDraft>>,
    open: Signal<bool>,
    /// The page area's size, which places the panes of a split.
    area: Signal<(f32, f32)>,
    /// The tab whose row or pane header is being dragged, and what dropping it now does where.
    dragging: Signal<Option<TabKey>>,
    split_drop: Signal<Option<(TabKey, &'static str)>>,
    resizing: Signal<Option<browser_core::split::Divider>>,
    floating: Signal<peek::Floating>,
    hover_card: Signal<peek::HoverCard>,
    form: Signal<profiles::Form>,
    card_name: Signal<String>,
    card_space: Signal<String>,
    import: Signal<import::State>,
    root: Owner,
}

pub fn act(ui: Ui, f: impl FnOnce(&mut Browser)) {
    let mut f = Some(f);
    act_dyn(ui, &mut |b| f.take().into_iter().for_each(|f| f(b)), true);
}

/// `act` for timers and page events: the open panel is rebuilt only when what it shows may have changed.
fn act_quietly(ui: Ui, f: impl FnOnce(&mut Browser)) {
    let mut f = Some(f);
    act_dyn(ui, &mut |b| f.take().into_iter().for_each(|f| f(b)), false);
}

fn act_dyn(ui: Ui, f: &mut dyn FnMut(&mut Browser), rebuild: bool) {
    let shown = ui.panel.get_untracked() == Panel::None && !ui.editing.get_untracked();
    let overlay = ui.panel.get_untracked() != Panel::None || ui.ext_popup.with_untracked(Option::is_some) || ui.find_tabs.get_untracked();
    let (address, find) = (ui.editing.get_untracked(), ui.finding.get_untracked());
    let done = BROWSERS.with_borrow_mut(|windows| {
        let elsewhere = live_elsewhere(windows, ui.window);
        let b = windows.get_mut(&ui.window)?;
        b.elsewhere = elsewhere;
        f(b);
        b.keep_page_fullscreen(overlay, address, find);
        let moved = b.model.active.filter(|k| b.elsewhere.contains(k));
        let forgotten = std::mem::take(&mut b.forgotten);
        let groups = std::mem::take(&mut b.forgotten_groups);
        let regrouped = b.regrouped_space.take();
        let shown = shown && b.reader.as_ref().is_none_or(|r| b.model.active != Some(r.key)) && b.active_json().is_none();
        let snap = snapshot::snapshot(b, shown);
        if ui.snap.with_untracked(|s| s.notice != snap.notice || s.prompt != snap.prompt || s.error != snap.error) {
            b.settle_downloads();
        }
        let done = (
            ui.icons.with_untracked(|i| b.site_icons(i)),
            std::mem::take(&mut b.outbox),
            std::mem::take(&mut b.released),
            std::mem::take(&mut b.retired),
            std::mem::take(&mut b.cancelled_downloads),
            std::mem::take(&mut b.kept_downloads),
            snap,
            moved,
            forgotten,
            b.window_request.take(),
        );
        for (_, other) in windows.iter_mut().filter(|(w, _)| **w != ui.window) {
            groups.iter().for_each(|(private, id)| other.model.closed_tabs.forget_group(*private, *id));
        }
        Some((done, regrouped))
    });
    let Some(((icons, commands, released, retired, cancelled, kept, snap, moved, forgotten, mode), regrouped)) = done else { return };
    if let Some(mode) = mode
        && let Some(here) = WINDOW_REFS.with_borrow(|refs| refs.get(&ui.window).cloned())
    {
        here.set_mode(mode);
    }
    let rebuild = rebuild
        || ui.snap.with_untracked(|s| *s != snap)
        || match ui.panel.get_untracked() {
            Panel::Memory | Panel::Downloads => true,
            Panel::Settings => ui.settings_pane.get_untracked() == tarn_nsh_shared::panes::Pane::Profile,
            _ => false,
        };
    ui.snap.set(snap);
    if let Some(icons) = icons {
        ui.icons.set(icons);
    }
    if !(released.is_empty() && cancelled.is_empty() && kept.is_empty()) {
        drop(run_on_main_thread(move || {
            released.iter().for_each(webview::session::release);
            cancelled.iter().for_each(|id| webview::session::cancel(id));
            kept.iter().for_each(|(id, keep)| webview::session::keep(id, *keep));
        }));
    }
    if !retired.is_empty() {
        ui.root.run(|| drop(set_timeout(RETIRE_DELAY, || crate::data::wipe_retired(&crate::data::root(), &core().disk))));
    }
    tools::expire_toast(ui);
    for (key, name, args) in commands {
        if let Some(r) = REFS.with_borrow(|refs| refs.get(&(ui.window, key)).map(|(_, r)| *r)) {
            r.command(name, args);
        }
    }
    if rebuild {
        ui.rev.update(|r| *r += 1);
    }
    for other in WINDOWS.with_borrow(Vec::clone).into_iter().filter(|o| o.window != ui.window) {
        if let Some(key) = moved
            && read(other, |b| b.model.lifecycle.live().any(|k| k == key)) == Some(true)
        {
            act_dyn(other, &mut |b| b.give_up(key), true);
        }
        let shows = |b: &Browser| regrouped.is_some() && b.model.current_space().map(|s| (s.private, s.space.id)) == regrouped;
        if read(other, |b| shows(b) || forgotten.iter().any(|k| b.model.tab(*k).is_some() || b.pages.contains_key(k))) == Some(true) {
            act_dyn(other, &mut Browser::sync_shared, true);
        }
    }
}

fn sync_others(ui: Ui) {
    for other in WINDOWS.with_borrow(Vec::clone).into_iter().filter(|o| o.window != ui.window) {
        act(other, Browser::sync_shared);
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
    ui.shortcut_edit.set(None);
    ui.confirm.set(None);
    ui.rename.set(String::new());
    ui.rename_for.set(String::new());
    ui.mark_query.set(String::new());
    ui.mark_edit.set(None);
    ui.library_query.set(String::new());
    ui.emoji_for.set(String::new());
    ui.space_draft.set(None);
    ui.panel.set(next);
    act(ui, move |b| {
        if next == Panel::Downloads {
            b.settle_downloads();
        }
    });
}

fn tab_menu(ui: Ui, key: TabKey) {
    ui.tab_menu.set(Some(key));
    if ui.panel.get_untracked() == Panel::Tab {
        act(ui, |_| {});
    } else {
        show(ui, Panel::Tab);
    }
}

pub(super) fn confirmed(ui: Ui, id: String, name: &'static str, run: impl Fn() + Clone + 'static) -> AnyElement {
    dynamic(move || {
        let (id, run) = (id.clone(), run.clone());
        if ui.confirm.with(|c| c.as_deref() == Some(id.as_str())) {
            let confirm = controls::accent_pill(format!("Confirm {name}"), move || {
                ui.confirm.set(None);
                run();
            });
            view().flex_row().items_center().gap_2().child(confirm).child(pill("Cancel", move || ui.confirm.set(None))).into_element()
        } else {
            pill(name, move || ui.confirm.set(Some(id.clone()))).into_element()
        }
    })
}

fn act_moving(ui: Ui, f: impl FnOnce(&mut Browser)) {
    animate_next(POP);
    act(ui, f);
}

fn new_tab(ui: Ui) {
    ui.panel.set(Panel::None);
    if !read(ui, |b| b.pref(browser_core::prefs::NEW_TAB_START_PAGE)).unwrap_or(true) {
        if ui.find_tabs.get_untracked() {
            tools::close_find_tabs(ui);
        }
        open_palette(ui);
        ui.address.set(String::new());
        ui.typed.set(String::new());
        ui.completed.set(None);
        ui.picked.set(0);
        ui.suggest.set(Vec::new());
        ui.new_tab_card.set(true);
        return;
    }
    animate_next(POP);
    act(ui, |b| {
        b.new_tab(START_URL);
    });
    ui.command.set(String::new());
    ui.command_ref.focus();
}

fn open_file(ui: Ui) {
    let dialog = nsh_core::file_dialog::OpenDialog::files().button_label("Open");
    ui.root.run(|| {
        drop(spawn(async move {
            let path = match pick_paths(dialog).await {
                Ok(paths) => paths.and_then(|p| p.into_iter().next()),
                Err(_) => return act(ui, |b| b.show_toast("The file dialog could not open", None)),
            };
            let Some(url) = path.as_deref().and_then(browser_core::open_file::url_for) else { return };
            animate_next(POP);
            ui.panel.set(Panel::None);
            act(ui, |b| b.open_file(&url));
        }))
    });
}

fn web_views(ui: Ui) -> View {
    view()
        .relative()
        .on_layout(move |b: &Bounds<Points>| ui.area.set((b.size.width.0, b.size.height.0)))
        .on_mouse_move(move |e: &MouseEvent| split::pointer_moved(ui, e))
        .on_mouse_up(move |_: &MouseEvent| split::released(ui))
        .child(split::layer(ui))
        .child(each(
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
                let rect = move || item.with(|v| split::view_rect(ui, v.active, v.pane));
                native(webview::NAME)
                    .prop("tab", JsonValue::from(format!("{}-{}", key.private, key.id)))
                    .prop("profile", move || JsonValue::from(item.with(|v| v.profile.clone())))
                    .prop("level", move || JsonValue::from(item.with(|v| v.level.clone())))
                    .prop("levels", move || item.with(|v| v.levels.clone()))
                    .prop("lists", move || JsonValue::from(item.with(|v| v.lists.clone())))
                    .prop("youtube", move || JsonValue::from(item.with(|v| v.youtube)))
                    .prop("filtered", move || JsonValue::from(item.with(|v| v.filtered)))
                    .prop("blocking", move || item.with(|v| v.blocking.clone()))
                    .prop("styles", move || item.with(|v| v.styles.clone()))
                    .prop("themeStyles", move || item.with(|v| v.theme_styles.clone()))
                    .prop("zaps", move || item.with(|v| v.zaps.clone()))
                    .prop("burstsOf", move || JsonValue::from(item.with(|v| v.opener.clone())))
                    .prop("sortDownloads", move || JsonValue::from(item.with(|v| v.sort_downloads)))
                    .prop("muted", move || JsonValue::from(item.with(|v| v.muted)))
                    .prop("inspectable", move || JsonValue::from(item.with(|v| v.inspectable)))
                    .prop("load", move || {
                        item.with(|v| v.load.as_ref().map_or(JsonValue::Null, |(u, s)| json!({ "url": u, "seq": s, "link": v.link, "file": v.file })))
                    })
                    .prop("active", move || JsonValue::from(item.with(|v| v.active)))
                    .on_native_event(move |event: &NativeEvent| {
                        if &*event.name == browser_core::extensions::STORE_EVENT {
                            extensions::store_link(ui, key, event.data["id"].as_str().unwrap_or_default());
                        }
                        if &*event.name == "iconData" {
                            return icon_data(ui, key, event.data.clone());
                        }
                        if &*event.name == "snapshot" {
                            return peek::shot(ui, key, &event.data["png"]);
                        }
                        if &*event.name == "jsonText" {
                            return devtools::json_text(ui, key, event.data.clone());
                        }
                        if &*event.name != "pressed" || read(ui, |b| b.in_split(key) && b.model.active != Some(key)) == Some(true) {
                            act_quietly(ui, |b| b.on_event(key, &event.name, &event.data));
                        }
                    })
                    .element_ref(&web)
                    .absolute()
                    .left(move || px(rect().0))
                    .top(move || px(rect().1))
                    .w(move || px(rect().2))
                    .h(move || px(rect().3))
            },
        ))
        .child(link_peek::layer(ui))
}

pub fn site_tile(ui: Ui, host: &str, size: f32) -> View {
    let host = host.to_owned();
    view()
        .w(px(size))
        .h(px(size))
        .flex_none()
        .child(dynamic(move || tarn_nsh_shared::controls::site_tile(ui.icons.with(|i| i.find(&host)).map(|icon| icon.to_string()), &host, size).into_element()))
}

fn space_tint(color: &str, alpha: f32) -> Option<Rgba> {
    Theme::profile_color(color).map(|c| Rgba { a: alpha, ..c })
}

fn host_of(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()
}

fn tiles(ui: Ui) -> View {
    let side = browser_core::tabs::tile_grid(LIST_WIDTH, TILE_GAP).1;
    let width = move || px(browser_core::tabs::tile_row(LIST_WIDTH, TILE_GAP, ui.snap.with(|s| s.tiles.len())).1);
    view().flex_row().flex_wrap().gap(px(TILE_GAP)).mb_1().child(each(
        move || ui.snap.with(|s| s.tiles.clone()),
        |r| r.id(),
        move |item, _| {
            let RowKind::Tab(key) = item.with_untracked(|r| r.kind.clone()) else { return view().into_element() };
            let active = move || item.with(|r| r.active);
            let tile = row(move || Some(SharedString::from(item.with(|r| r.label.clone()))), move || act(ui, |b| b.activate(key)))
                .h(px(side - 2.))
                .px_0()
                .justify_center()
                .rounded(px(Theme::radius_tile()))
                .bg(move || if active() { Theme::selected() } else { Theme::raised() })
                .child(dynamic(move || {
                    let host = item.with(|r| r.host.clone());
                    site_tile(ui, &host, 20.).opacity(move || if item.with(|r| r.asleep) { 0.4_f32 } else { 1. }).into_element()
                }));
            view()
                .w(width)
                .rounded(px(Theme::radius_tile()))
                .border_1()
                .border_color(move || if active() { Theme::sidebar_edge() } else { Theme::hairline() })
                .on_context_menu(move |_: &ContextMenuEvent| tab_menu(ui, key))
                .child(tile)
                .into_element()
        },
    ))
}

fn sidebar_rows(ui: Ui, pinned: bool) -> AnyElement {
    each(
        move || {
            ui.snap.with(|s| s.rows.iter().filter(|r| (r.pinned || r.depth > 0 || matches!(r.kind, RowKind::Folder(_))) == pinned).cloned().collect::<Vec<_>>())
        },
        |r| r.id(),
        move |item, _| list_row(ui, pinned, item),
    )
}

fn list_row(ui: Ui, pinned: bool, item: ReadSignal<snapshot::Row>) -> AnyElement {
    let kind = item.with_untracked(|r| r.kind.clone());
    let depth = item.with_untracked(|r| r.depth) as f32;
    let name = move || item.with(|r| r.label.clone());
    match kind {
        RowKind::Tab(key) => tab_row(ui, pinned, key, item),
        RowKind::Group(id) => groups::group_card(ui, id, item),
        RowKind::Split(_) => view()
            .flex_row()
            .gap(px(2.))
            .child(each(
                move || item.with(|r| r.panes.clone()),
                |r| r.id(),
                move |pane, _| {
                    let RowKind::Tab(key) = pane.with_untracked(|r| r.kind.clone()) else { return view().into_element() };
                    view().flex_1().min_w_0().child(tab_row(ui, pinned, key, pane)).into_element()
                },
            ))
            .into_element(),
        RowKind::Folder(id) => view()
            .on_context_menu(move |_: &ContextMenuEvent| show(ui, Panel::Folder(id)))
            .child(
                row(move || Some(SharedString::from(name())), move || act(ui, |b| b.toggle_folder(id)))
                    .h(px(TAB_ROW))
                    .pl(px(8. + depth * 16.))
                    .child(
                        view()
                            .text_color(Theme::muted())
                            .child(dynamic(move || icons::icon(if item.with(|r| r.open) { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }, 12.))),
                    )
                    .child(view().text_color(Theme::space_label()).child(dynamic(move || folder_icon(&item.with(|r| r.icon.clone())))))
                    .child(label(name).font_weight(FontWeight::SEMIBOLD).flex_1().truncate()),
            )
            .into_element(),
    }
}

fn folder_icon(stored: &str) -> AnyElement {
    match browser_core::icon::parse(stored) {
        Some(browser_core::icon::Icon::Emoji(emoji)) => body(emoji.to_owned()).into_element(),
        Some(browser_core::icon::Icon::Symbol(name)) => icons::icon(icons::named(name).unwrap_or(icons::FOLDER), 15.).into_element(),
        None => icons::icon(icons::FOLDER, 15.).into_element(),
    }
}

fn tab_row(ui: Ui, pinned: bool, key: TabKey, item: ReadSignal<snapshot::Row>) -> AnyElement {
    let depth = item.with_untracked(|r| r.depth) as f32;
    let name = move || item.with(|r| r.label.clone());
    let active = move || item.with(|r| r.active);
    let hovered = signal(false);
    let slot = element_ref();
    let close = (!pinned).then(|| {
        let shown = move || (hovered.get() || active()) as u8 as f32;
        view()
            .element_ref(&slot)
            .flex_shrink_0()
            .overflow_hidden()
            .w(move || px(CLOSE_WIDTH * shown()))
            .ml(move || px(-CLOSE_GAP * (1. - shown())))
            .opacity(shown)
            .transition_opacity()
            .child(icon_button(icons::CLOSE, "Close tab", move || {
                animate_next(POP);
                act(ui, |b| b.close_tab(key));
            }))
    });
    let hover = move |on: bool| {
        if !pinned && !tarn_nsh_shared::motion::reduced() {
            slot.animate_layout(LayoutTransition::new(LayoutCurve::EaseOut, tarn_nsh_shared::motion::FADE.as_secs_f32() * 1000.));
        }
        hovered.set(on);
    };
    let dropping = move || ui.split_drop.with(|d| d.filter(|(k, _)| *k == key).map(|(_, does)| does));
    let edge = move || match item.with(|r| r.split) {
        Some(true) => Some(Theme::accent()),
        Some(false) => Some(Theme::faint()),
        None => None,
    };
    let closing = move || !pinned && (hovered.get() || active());
    let tab = row(move || Some(SharedString::from(name())), move || act(ui, |b| b.activate(key)))
        .on_press_down(move || ui.dragging.set(Some(key)))
        .h(px(TAB_ROW))
        .relative()
        .pl(px(10. + depth * 16.))
        .pr_1()
        .gap(px(CLOSE_GAP))
        .bg(move || match (dropping().is_some(), active()) {
            (true, _) => Theme::accent_soft(),
            (false, true) => Theme::selected(),
            (false, false) => rgba(0),
        })
        .border_1()
        .border_color(move || if active() && dropping().is_none() { Theme::surface_edge() } else { Theme::clear() })
        .child(dynamic(move || edge().map(|color| view().absolute().left(px(0.)).top(px(7.)).bottom(px(7.)).w(px(3.)).rounded_full().bg(color))))
        .child(when(move || dropping().is_some(), || view().absolute().inset_0().rounded(px(Theme::radius_row())).border_1().border_color(Theme::accent())))
        .child({
            let mark = computed(move || item.with(|r| (r.loading && !tarn_nsh_shared::motion::reduced(), r.host.clone())));
            dynamic(move || match mark.get() {
                (true, _) => {
                    view().text_color(Theme::space_label()).child(tarn_nsh_shared::motion::spinner(14., icons::icon(icons::RELOAD, 14.))).into_element()
                }
                (false, host) => site_tile(ui, &host, 16.).opacity(move || if item.with(|r| r.asleep) { 0.4_f32 } else { 1. }).into_element(),
            })
        })
        .child(
            body(name)
                .text_color(move || match item.with(|r| (r.active, r.asleep)) {
                    (true, _) => Theme::text(),
                    (false, true) => Theme::asleep(),
                    (false, false) => Theme::muted(),
                })
                .style_when(active, |s| s.font_weight(FontWeight::MEDIUM))
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
            mark.map(|(icon, tint, name)| {
                view().text_color(tint).accessible(true).label(name).tooltip(controls::tip(name, Placement::Top)).child(icons::icon(icon, 13.))
            })
        }))
        .child(dynamic(move || {
            let (playing, asleep, muted) = item.with(|r| (r.playing, r.asleep, r.muted));
            let state = if closing() || dropping().is_some() {
                None
            } else if muted {
                Some((icons::VOLUME_X, Theme::faint(), "Muted"))
            } else if playing {
                Some((icons::VOLUME_2, Theme::space_label(), "Playing audio"))
            } else if asleep {
                Some((icons::MOON, Theme::faint(), "Asleep"))
            } else if pinned {
                Some((icons::PIN, Theme::faint(), "Pinned"))
            } else {
                None
            };
            state.map(|(glyph, tint, name)| {
                tarn_nsh_shared::motion::popping(view().w(px(16.)).items_center().text_color(tint).accessible(true).label(name).child(icons::icon(glyph, 13.)))
            })
        }))
        .child(dynamic(move || dropping().map(|does| small(does).text_color(Theme::accent()))))
        .children(close);
    // Where the dragged tab lands when dropped here: after this row when it comes from above in the same zone.
    let landing = move || {
        let dragged = ui.dragging.get().filter(|d| *d != key && hovered.get() && dropping().is_none())?;
        read(ui, |b| match (b.model.tab(dragged), b.model.tab(key)) {
            (Some(d), Some(t)) => d.kind == t.kind && d.folder_id == t.folder_id && d.position < t.position,
            _ => false,
        })
    };
    view()
        .relative()
        .on_mouse_up(move |_: &MouseEvent| split::drop_on_row(ui, key))
        .on_mouse_enter(move |e: &MouseEvent| {
            hover(true);
            peek::hover_row(ui, key, true, e.page_position.y.0);
        })
        .on_mouse_leave(move |_: &MouseEvent| {
            hover(false);
            peek::hover_row(ui, key, false, 0.);
            split::drop_leave(ui, key);
        })
        .on_mouse_move(move |e: &MouseEvent| split::drop_over(ui, key, browser_core::split::drops_into(e.position.y.0 / TAB_ROW)))
        .on_context_menu(move |_: &ContextMenuEvent| tab_menu(ui, key))
        .child(tab)
        .child(dynamic(move || {
            let line = view().absolute().left(px(6.)).right(px(6.)).h(px(2.)).rounded_full().bg(Theme::accent());
            landing().map(|after| if after { line.bottom(px(-2.)) } else { line.top(px(-2.)) })
        }))
        .into_element()
}

fn space_bar(ui: Ui) -> View {
    let spaces = each(
        move || ui.snap.with(|s| s.spaces.clone()),
        |s| (s.ix, s.icon.clone(), s.color.clone(), s.label.clone()),
        move |space, _| {
            let (ix, icon, name, private, color) = space.with_untracked(|s| (s.ix, s.icon.clone(), s.label.clone(), s.private, s.color.clone()));
            let source = if private { icons::PERSON } else { icons::space_icon(&icon) };
            let current = move || space.with(|s| s.current);
            let button = controls::row(name, move || {
                animate_next(SLIDE);
                act(ui, |b| b.switch_space_by_user(ix));
            })
            .w(px(SPACE_BUTTON))
            .h(px(SPACE_BUTTON))
            .flex_none()
            .px_0()
            .justify_center()
            .rounded(px(Theme::radius_row()))
            .bg(move || if current() { Theme::selected() } else { Theme::clear() })
            .text_color(move || match (current(), private) {
                (true, true) => Theme::warning(),
                (true, false) => space_tint(&color, 1.).unwrap_or(Theme::text()),
                (false, _) => space_tint(&color, 0.45).unwrap_or(Theme::faint()),
            })
            .child(icons::icon(source, icon_size()));
            view()
                .on_context_menu(move |_: &ContextMenuEvent| {
                    if let Some(Some(key)) = read(ui, |b| b.model.spaces.get(ix).map(|s| (s.private, s.space.id))) {
                        show(ui, Panel::SpaceMenu(key));
                    }
                })
                .child(button)
        },
    );
    view()
        .flex_row()
        .items_center()
        .min_h(px(40.))
        .py(px(6.))
        .px_1()
        .child(view().flex_row().flex_none().w(px(2. * SPACE_BUTTON)).child(icon_button(icons::LIBRARY, "Library", move || show(ui, Panel::Library))))
        .child(view().flex_row().flex_wrap().flex_1().items_center().justify_center().gap(px(2.)).child(spaces))
        .child(
            view()
                .flex_row()
                .flex_none()
                .w(px(2. * SPACE_BUTTON))
                .child(icon_button(icons::PLUS, "New Space", move || show(ui, Panel::NewSpace)))
                .child(icon_button(icons::GEAR, "Settings", move || show(ui, Panel::Settings))),
        )
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

fn space_switcher(ui: Ui) -> nsh_base::ButtonElement {
    let tint = move || ui.snap.with(|s| if s.private { Theme::warning() } else { Theme::profile_color(&s.profile_color).unwrap_or(Theme::space_label()) });
    let glyph = move || ui.snap.with(|s| s.spaces.iter().find(|x| x.current && !x.private).and_then(|x| space_tint(&x.color, 1.))).unwrap_or_else(tint);
    let name = move || ui.snap.with(|s| s.space_name.clone());
    row(move || Some(SharedString::from(format!("Space {}", name()))), move || show(ui, Panel::Space))
        .flex_1()
        .min_w(px(0.))
        .px_1()
        .gap(px(6.))
        .child(
            view()
                .w(px(SPACE_BUTTON))
                .h(px(SPACE_BUTTON))
                .flex_none()
                .rounded(px(Theme::radius_row()))
                .bg(Theme::selected())
                .items_center()
                .justify_center()
                .text_color(glyph)
                .child(dynamic(move || {
                    let icon = ui.snap.with(|s| s.space_icon.clone());
                    icons::icon(icons::space_icon(&icon), icon_size())
                })),
        )
        .child(text(name).text_size(px(14.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).truncate())
}

fn new_tab_row(ui: Ui) -> View {
    let new = controls::pressable("New Tab", move || new_tab(ui))
        .flex_row()
        .flex_1()
        .items_center()
        .gap(px(CLOSE_GAP))
        .h(px(TAB_ROW))
        .pl(px(10.))
        .rounded(px(Theme::radius_row()))
        .child(
            view()
                .w(px(16.))
                .h(px(16.))
                .rounded(px(Theme::radius_small()))
                .bg(Theme::accent())
                .items_center()
                .justify_center()
                .text_color(Theme::on_fill(Theme::accent()))
                .child(icons::icon(icons::PLUS, 11.)),
        )
        .child(body("New Tab").text_color(Theme::muted()));
    view()
        .flex_row()
        .items_center()
        .mb_1()
        .child(new)
        .child(icon_button(icons::GROUP, "New Tab Group", move || groups::new_group(ui, None)))
        .child(icon_button(icons::FOLDER_PLUS, "New Folder", move || act(ui, |b| b.new_folder(None))))
        .child(when(
            move || ui.snap.with(|s| s.cleanable),
            move || icon_button(icons::BROOM_SPARKLES, browser_core::cleanup::DESCRIPTION, move || act(ui, Browser::clean_up_tabs)),
        ))
}

fn sidebar(ui: Ui) -> View {
    let header = view().flex_row().items_center().gap_1().h(px(TOOLBAR_HEIGHT - 8.)).pl_1().child(space_switcher(ui)).child(downloads_button(ui));
    let list = view()
        .flex_col()
        .gap(px(2.))
        .px_1()
        .pb_2()
        .child(when(move || ui.snap.with(|s| !s.tiles.is_empty()), move || tiles(ui)))
        .child(sidebar_rows(ui, true))
        .child(view().h(px(1.)).mx_2().my_2().bg(Theme::hairline()))
        .child(new_tab_row(ui))
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
        .on_context_menu(move |_: &ContextMenuEvent| show(ui, Panel::Sidebar))
        .on_wheel(move |e: &WheelEvent| {
            if let WheelDelta::Points(p) = e.delta {
                let step = BROWSERS.with_borrow_mut(|w| w.get_mut(&ui.window)?.swiped(p.x.0, p.y.0, std::time::Instant::now()));
                if let Some(step) = step {
                    animate_next(SLIDE);
                    act(ui, |b| b.swipe_to(step));
                }
            }
        })
        .child(header)
        .child(scroll().flex_1().child(list))
        .child(space_bar(ui))
}

/// Whether a space is private, and its profile.
type Profile = (bool, store::Id);

fn load_history(ui: Ui) {
    let (profile, rows) = read(ui, |b| {
        let profile = b.model.profile();
        (profile, profile.map(|(p, id)| b.core.store(p).history(id, 500).unwrap_or_default()).unwrap_or_default())
    })
    .unwrap_or_default();
    ui.history.set((profile, Rc::new(rows)));
}

pub(super) fn in_other_windows(ui: Ui, f: impl FnOnce(&mut Browser) + Clone) {
    for other in WINDOWS.with_borrow(Vec::clone).into_iter().filter(|o| o.window != ui.window) {
        act_quietly(other, f.clone());
    }
}

/// Clears the profile's history, and the cache that would still suggest it.
fn clear_history(ui: Ui) {
    act(ui, |b| b.clear_history());
    if let Some(Some(profile)) = read(ui, |b| b.model.profile()) {
        in_other_windows(ui, move |b| {
            b.favicons.forget(&profile);
            b.reset_zooms(profile.0, profile.1, None);
        });
    }
    ui.history.set(Default::default());
}

/// The cached history, read again when the space's profile changed since it was loaded.
fn history(ui: Ui) -> Rc<Vec<store::HistoryEntry>> {
    if read(ui, |b| b.model.profile()).flatten() != ui.history.with_untracked(|h| h.0) {
        load_history(ui);
    }
    ui.history.with_untracked(|h| h.1.clone())
}

fn show_suggestions(ui: Ui, rows: Vec<(String, String)>, query: &str, current: Signal<String>) {
    ui.picked.set(usize::from(tools::listing(query)));
    if read(ui, |b| !b.suggestions()).unwrap_or(false) {
        return clear_suggestions(ui);
    }
    let tabs = if tools::listing(query) { Vec::new() } else { read(ui, |b| b.tab_suggestions(query)).unwrap_or_default() };
    let open_tabs = read(ui, |b| b.model.tabs.iter().map(|t| t.url.clone()).collect::<Vec<_>>()).unwrap_or_default();
    let open: HashSet<String> = open_tabs.into_iter().chain(tabs.iter().map(|t| t.url.clone())).collect();
    let without_tabs = move |rows: Vec<(String, String)>| rows.into_iter().filter(|(_, url)| !open.contains(url)).collect::<Vec<_>>();
    ui.tabs.set(tabs);
    let rows = without_tabs(rows);
    let found = read(ui, |b| b.cached_text_matches(query)).flatten();
    let shown = without_tabs(crate::input::with_text(rows.clone(), found.as_deref().unwrap_or_default(), SUGGESTIONS));
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
            ui.suggest.set(without_tabs(crate::input::with_text(rows, &found, SUGGESTIONS)));
        }))
    });
}

/// Moves the picked row for `key`: up and down wrap at the ends, page up and page down jump to them.
/// A `>` list has only the command rows, from row 1.
fn step_pick(ui: Ui, query: Signal<String>, key: &str) {
    let query = query.get_untracked();
    let commands = tools::commands(ui, &query).len();
    let listed = ui.tabs.with_untracked(Vec::len) + ui.suggest.with_untracked(Vec::len);
    let (first, rows) = if tools::listing(&query) { (1, commands) } else { (0, 1 + commands + listed) };
    ui.picked.update(|p| {
        *p = first
            + match key {
                "pageup" => 0,
                "pagedown" => rows.saturating_sub(1),
                _ => browser_core::suggest::step_selection(p.saturating_sub(first), rows, key == "down"),
            }
    });
}

/// Enter in the address field or the start card: the picked row of `query`, or what was typed.
fn submit(ui: Ui, query: Signal<String>, text: String) {
    let picked = ui.picked.get_untracked();
    // The command list shows commands only: Enter runs the picked one, or nothing.
    if tools::listing(&query.get_untracked()) {
        if let Some((_, command)) = tools::commands(ui, &query.get_untracked()).get(picked.saturating_sub(1)) {
            tools::pick(ui, *command);
        }
        return;
    }
    if picked == 0 || ui.find_tabs.get_untracked() {
        return go(ui, text);
    }
    let commands = tools::commands(ui, &query.get_untracked());
    if let Some((_, command)) = commands.get(picked - 1) {
        return tools::pick(ui, *command);
    }
    let row = picked - 1 - commands.len();
    if let Some(key) = ui.tabs.with_untracked(|t| t.get(row).map(|t| t.key)) {
        return switch_to_tab(ui, key);
    }
    let url = ui.suggest.with_untracked(|s| s.get(row - ui.tabs.with_untracked(Vec::len)).map(|(_, url)| url.clone()));
    go(ui, url.unwrap_or(text));
}

fn clear_suggestions(ui: Ui) {
    ui.suggest.set(Vec::new());
    ui.tabs.set(Vec::new());
}

fn switch_to_tab(ui: Ui, key: TabKey) {
    ui.picked.set(0);
    ui.editing.set(false);
    clear_suggestions(ui);
    ui.address_ref.blur();
    ui.command.set(String::new());
    act(ui, |b| b.switch_to_tab(key));
}

fn go(ui: Ui, text: String) {
    ui.picked.set(0);
    if ui.find_tabs.get_untracked() {
        if let Some(row) = ui.snap.with_untracked(|s| s.find_rows.first().cloned()) {
            let mut query = String::new();
            act(ui, |b| query = b.open_find_row(row.key));
            tools::close_find_tabs(ui);
            ui.find.set(query.clone());
            ui.finding.set(true);
            act(ui, |b| b.find_typed(&query));
        }
        return;
    }
    let text = ui.completed.get_untracked().filter(|(full, _)| *full == text).map_or(text, |(_, url)| url);
    let new_tab = ui.new_tab_card.get_untracked();
    ui.new_tab_card.set(false);
    ui.editing.set(false);
    clear_suggestions(ui);
    ui.address_ref.blur();
    act(ui, |b| b.navigate_in(&text, new_tab));
}

fn address_menu(ui: Ui) {
    ui.pasteable.set(false);
    drop(spawn(async move {
        let has = nsh_core::prelude::clipboard_has_string().await;
        ui.pasteable.set(has);
    }));
    show(ui, Panel::Address);
}

pub(super) fn paste_and_go(ui: Ui) {
    drop(spawn(async move {
        let text = nsh_core::prelude::clipboard_string().await;
        let line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        if line.trim().is_empty() {
            return;
        }
        show(ui, Panel::None);
        ui.completed.set(None);
        go(ui, line);
    }));
}

fn shield_chip(ui: Ui) -> nsh_base::ButtonElement {
    let standard = move || ui.snap.with(|s| s.level == "standard");
    let color = move || if standard() { Secure::muted() } else { Secure::shield_raised() };
    let count = move || ui.snap.with(|s| if s.site_total > 0 { browser_core::shield_card::thousands(s.site_total as u64) } else { String::new() });
    let name = move || {
        let (level, total) = ui.snap.with(|s| (panels::capitalised(&s.level), s.site_total));
        Some(SharedString::from(format!("Site settings: Shield {level}, {total} stopped on this site")))
    };
    row(name, move || show(ui, Panel::Shield))
        .h(px(CHIP))
        .px(px(6.))
        .gap_1()
        .flex_none()
        .rounded(px(Theme::radius_row() - 1. - CHIP_INSET))
        .bg(Secure::control())
        .hover(|s| s.bg(Secure::hover()))
        .text_color(color)
        .child(dynamic(move || {
            tarn_nsh_shared::motion::popping(view().child(icons::icon(if standard() { icons::SHIELD } else { icons::SHIELD_CHECK }, CHIP_GLYPH)))
        }))
        .child(when(
            move || !standard(),
            move || small(move || ui.snap.with(|s| panels::capitalised(&s.level))).text_color(color).font_family(SYSTEM_FONT).font_weight(FontWeight::MEDIUM),
        ))
        .child(when(move || ui.snap.with(|s| s.site_total > 0), move || small(count).text_color(color).font_family(SYSTEM_FONT)))
}

fn open_palette(ui: Ui) {
    ui.new_tab_card.set(false);
    if ui.editing.get_untracked() {
        return ui.address_ref.focus();
    }
    let url = ui.snap.with_untracked(|s| if s.url == START_URL { String::new() } else { s.url.clone() });
    ui.address.set(url);
    ui.typed.set(String::new());
    ui.completed.set(None);
    ui.picked.set(0);
    clear_suggestions(ui);
    load_history(ui);
    ui.editing.set(true);
}

fn close_palette(ui: Ui) {
    if ui.find_tabs.get_untracked() {
        return tools::close_find_tabs(ui);
    }
    ui.editing.set(false);
    ui.new_tab_card.set(false);
    clear_suggestions(ui);
    ui.address_ref.blur();
}

fn address(ui: Ui) -> View {
    let host = move || ui.snap.with(|s| if s.url == START_URL || s.url.is_empty() { String::new() } else { crate::input::display_host(&s.url) });
    let shown = controls::pressable("Address", move || open_palette(ui)).flex_1().min_w(px(0.)).h_full().items_center().justify_center().px_2().child(dynamic(
        move || {
            let host = host();
            if host.is_empty() {
                return body("Search or enter address").text_color(Theme::faint()).truncate();
            }
            body(host).text_color(if ui.snap.with(|s| s.secure) { Secure::text() } else { Secure::warning() }).truncate()
        },
    ));
    let capsule = view()
        .flex_row()
        .flex_1()
        .max_w(px(ADDRESS_WIDTH))
        .items_center()
        .px(px(CHIP_INSET))
        .h(px(FIELD))
        .rounded(move || px(Theme::radius_row()))
        .bg(Secure::chip())
        .border_1()
        .border_color(move || if ui.editing.get() { Theme::accent() } else { Secure::hairline() })
        .font_family(SYSTEM_FONT)
        .text_color(Theme::faint)
        .child(themed(move || {
            if ui.snap.with(|s| s.host.is_empty()) {
                view().w(px(28.)).items_center().child(icons::icon(icons::SEARCH, CHIP_GLYPH)).into_element()
            } else {
                shield_chip(ui).into_element()
            }
        }))
        .child(view().flex_row().flex_1().min_w(px(0.)).h_full().on_context_menu(move |_: &ContextMenuEvent| address_menu(ui)).child(shown))
        .child(themed(move || {
            when(
                move || ui.snap.with(|s| s.article),
                move || {
                    icon_button(icons::BOOK_OPEN, "Reader", move || act(ui, |b| b.toggle_reader()))
                        .h(px(CHIP))
                        .rounded(px(Theme::radius_row() - 1. - CHIP_INSET))
                        .text_color(move || if ui.snap.with(|s| s.reader.is_some()) { Theme::accent() } else { Theme::muted() })
                },
            )
        }));
    view().flex_row().flex_1().justify_center().child(capsule)
}

pub(super) fn shortcut_of(ui: Ui, action: Action) -> Option<String> {
    let stored = read(ui, |b| crate::shortcuts::stored(&b.core.disk))?;
    crate::shortcuts::keymap(&stored).into_iter().find(|(_, a)| *a == action).map(|(k, _)| crate::shortcuts::display(&k))
}

fn toolbar(ui: Ui) -> View {
    let navigation = move || {
        let button = |icon: &str, name: &str, press: fn(Ui)| controls::icon_button_at(icon, name, Placement::Right, move || press(ui));
        view()
            .flex_row()
            .items_center()
            .gap(px(2.))
            .child(button(icons::SIDEBAR, "Toggle sidebar", toggle_sidebar))
            .child(
                button(icons::BACK, "Back", |ui| act(ui, |b| b.back()))
                    .disabled(move || !ui.snap.with(|s| s.can_back))
                    .disabled_style(|s| s.text_color(Theme::faint())),
            )
            .child(
                button(icons::FORWARD, "Forward", |ui| act(ui, |b| b.forward()))
                    .disabled(move || !ui.snap.with(|s| s.can_forward))
                    .disabled_style(|s| s.text_color(Theme::faint())),
            )
            .child(reload_button(ui))
            .child(private_mark(ui))
    };
    let split_shown = computed(move || ui.snap.with(|s| s.split.is_some()));
    let actions = move || {
        let name = if split_shown.get() { "Close Split View" } else { "Open Split View" };
        let tip = shortcut_of(ui, Action::ToggleSplitView).map_or_else(|| name.to_owned(), |keys| format!("{name} ({keys})"));
        let split = controls::icon_button_at(icons::SPLIT_ADD, &tip, Placement::Left, move || act(ui, Browser::toggle_split));
        let share = controls::icon_button_at(icons::SHARE, "Share", Placement::Left, move || show(ui, Panel::Share));
        view().flex_row().items_center().gap(px(2.)).child(devtools::responsive_button(ui)).child(extensions::buttons(ui)).child(split).child(share)
    };
    let compact = computed(move || ui.snap.with(|s| s.compact));
    let address = move || if compact.get() { compact_address(ui) } else { address(ui) };
    view().flex_row().items_center().gap_2().h(px(TOOLBAR_HEIGHT)).child(themed(navigation)).child(dynamic(address)).child(themed(actions))
}

fn compact_address(ui: Ui) -> View {
    let host = move || ui.snap.with(|s| if s.url == START_URL || s.url.is_empty() { String::new() } else { crate::input::display_host(&s.url) });
    let title = move || ui.snap.with(|s| if s.host.is_empty() { String::new() } else { browser_core::text::bounded_title(&s.title) });
    let field = controls::pressable("Address", move || open_palette(ui))
        .flex_1()
        .min_w(px(0.))
        .h(px(COMPACT_FIELD))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(Theme::radius_row()))
        .font_family(SYSTEM_FONT)
        .child(themed(move || (!ui.snap.with(|s| s.host.is_empty())).then(|| shield_chip(ui))))
        .child(dynamic(move || {
            let host = host();
            if host.is_empty() {
                return body("Search or enter address").text_color(Theme::faint()).truncate();
            }
            body(host).text_color(if ui.snap.with(|s| s.secure) { Secure::text() } else { Secure::warning() }).flex_shrink_0().truncate()
        }))
        .child(dynamic(move || {
            let title = title();
            (!title.is_empty()).then(|| {
                view()
                    .flex_row()
                    .flex_1()
                    .min_w(px(0.))
                    .items_center()
                    .gap_2()
                    .child(small("—").text_color(Theme::faint()))
                    .child(small(title).text_color(Theme::muted()).max_w(px(COMPACT_TITLE)).truncate())
            })
        }));
    view().flex_row().flex_1().min_w(px(0.)).items_center().gap_1().child(field).child(themed(move || {
        when(
            move || ui.snap.with(|s| s.article),
            move || {
                icon_button(icons::BOOK_OPEN, "Reader", move || act(ui, |b| b.toggle_reader()))
                    .text_color(move || if ui.snap.with(|s| s.reader.is_some()) { Theme::accent() } else { Theme::muted() })
            },
        )
    }))
}

fn toggle_sidebar(ui: Ui) {
    animate_next(SLIDE);
    ui.sidebar.update(|o| *o = !*o);
    peek::close(ui);
    peek::hide(ui);
}

pub(super) fn set_auto_hide_sidebar(ui: Ui) {
    act(ui, |b| b.toggle_off_setting(crate::browser::AUTO_HIDE_SIDEBAR));
    let hidden = read(ui, |b| b.auto_hide_sidebar()).unwrap_or_default();
    for other in WINDOWS.with_borrow(Vec::clone) {
        other.sidebar.set(!hidden);
        peek::close(other);
        peek::hide(other);
    }
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
            return controls::icon_button_at(icons::RELOAD, "Reload", Placement::Right, reload).into_element();
        }
        let press = move || if ui.snap.with_untracked(|s| s.loading) { act(ui, |b| b.stop()) } else { reload() };
        let name = if loading { "Stop Loading" } else { "Reload" };
        if tarn_nsh_shared::motion::reduced() {
            return controls::icon_button_at(icons::CLOSE, name, Placement::Right, press).into_element();
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
                .tooltip(controls::tip(name, Placement::Right))
                .child(icons::icon(glyph, 14.))
        })
    })
    .into_element()
}

pub(super) fn suggestion_row(name: String, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    controls::pressable(name, press).flex_row().items_center().gap_3().mx(px(6.)).h(px(PALETTE_ROW)).px(px(6.)).rounded(px(Theme::radius_row()))
}

pub(super) fn row_glyph(ui: Ui, glyph: &str, row: impl Fn() -> usize + Copy + 'static) -> View {
    let picked = move || ui.picked.get() == row();
    tarn_nsh_shared::controls::glyph_tile(
        glyph,
        28.,
        move || if picked() { Theme::accent() } else { Theme::control() },
        move || {
            if picked() { Theme::on_fill(Theme::accent()) } else { Theme::muted() }
        },
    )
}

pub(super) fn row_section(name: &str) -> View {
    view().px(px(12.)).pt_2().pb_1().child(tarn_nsh_shared::controls::caps_label(name))
}

pub(super) fn detail(content: impl IntoReactive<SharedString>) -> Text {
    small(content).max_w(px(260.)).truncate()
}

fn input_row(ui: Ui, query: Signal<String>) -> AnyElement {
    dynamic(move || {
        let text = query.get();
        if tools::listing(&text) {
            return None;
        }
        let engine = read(ui, |b| b.search_engine()).unwrap_or_default();
        let (name, note, glyph, rejected) = match classify_input(&text, &ENGINES[engine]) {
            Some(Input::Navigate(u)) => (u.to_string(), "Open".to_owned(), icons::GLOBE, false),
            Some(Input::External(u)) => (u.to_string(), format!("Open in another app ({})", u.scheme()), icons::EXTERNAL_LINK, false),
            Some(Input::Rejected) => (text.clone(), "This kind of address cannot be opened".to_owned(), icons::TRIANGLE_ALERT, true),
            _ => (text.clone(), format!("Search {}", ENGINES[engine].name), icons::SEARCH, false),
        };
        Some(
            suggestion_row(name.clone(), move || go(ui, query.get_untracked()))
                .bg(move || picked_fill(ui, 0))
                .text_color(Theme::muted())
                .child(row_glyph(ui, glyph, || 0))
                .child(body(name).text_color(if rejected { Theme::danger() } else { Theme::text() }).flex_1().truncate())
                .child(detail(note)),
        )
    })
}

pub(super) fn picked_fill(ui: Ui, row: usize) -> Rgba {
    if ui.picked.get() == row { Theme::selected() } else { rgba(0) }
}

fn suggestion_rows(ui: Ui, query: Signal<String>, address: bool) -> View {
    let first = computed(move || 1 + tools::commands(ui, &query.get()).len());
    for key in ["up", "down", "pageup", "pagedown"] {
        let options = KeyOptions { when: nsh_core::keymap::When::Always, repeat: true, ..KeyOptions::default() };
        // The start card's rows and the address field's can both be on screen; only the edited field's move.
        on_key_with(key, options, move |_| {
            if ui.editing.get_untracked() == address {
                step_pick(ui, query, key);
            }
        });
    }
    let listed = move || if query.with(|q| tools::listing(q)) { Vec::new() } else { ui.suggest.get() };
    let tabs = move || if query.with(|q| tools::listing(q)) { Vec::new() } else { ui.tabs.get() };
    let after_tabs = computed(move || first.get() + ui.tabs.with(Vec::len));
    view()
        .flex_col()
        .gap(px(2.))
        .py(px(6.))
        .child(input_row(ui, query))
        .child(tools::command_rows(ui, query))
        .child(when(move || !tabs().is_empty(), || row_section(Section::Tabs.title())))
        .child(each(
            tabs,
            |row| format!("{}-{}", row.key.private, row.key.id),
            move |item, ix| {
                let row = item.get_untracked();
                let (key, detail_text) = (row.key, row.detail());
                suggestion_row(row.title.clone(), move || switch_to_tab(ui, key))
                    .bg(move || picked_fill(ui, first.get() + ix.get()))
                    .child(view().w(px(28.)).items_center().child(site_tile(ui, &host_of(&row.url), 20.)))
                    .child(label(row.title.clone()).flex_1().truncate())
                    .child(detail(detail_text))
            },
        ))
        .child(when(move || !listed().is_empty(), || row_section(Section::History.title())))
        .child(each(
            listed,
            |(_, url)| url.clone(),
            move |item, ix| {
                let (title, url) = item.get_untracked();
                let host = host_of(&url);
                let target = url.clone();
                suggestion_row(url.clone(), move || go(ui, target.clone()))
                    .bg(move || picked_fill(ui, after_tabs.get() + ix.get()))
                    .child(view().w(px(28.)).items_center().child(site_tile(ui, &host, 20.)))
                    .child(label(if title.is_empty() { url.clone() } else { title }).flex_1().truncate())
                    .child(detail(url.trim_start_matches("https://").trim_end_matches('/').to_owned()))
            },
        ))
}

fn palette(ui: Ui) -> AnyElement {
    when(
        move || ui.editing.get(),
        move || {
            let field = input()
                .value(move || ui.address.get())
                .on_focus(move |_: &()| {
                    let length = ui.address.with_untracked(|a| a.chars().count() as u32);
                    ui.address_ref.set_selection(0, length);
                })
                .on_blur(move |_: &()| {
                    if !ui.find_tabs.get_untracked() {
                        ui.editing.set(false);
                    }
                    clear_suggestions(ui);
                })
                .on_change(move |value: &str| {
                    if ui.find_tabs.get_untracked() {
                        ui.address.set(value.to_owned());
                        let query = value.to_owned();
                        act(ui, |b| b.find_in_tabs(&query));
                    } else if value != ui.address.get_untracked() {
                        let (history, typed) = (history(ui), ui.typed.get_untracked());
                        let edit = read(ui, |b| b.address_edit(&typed, value, &history, SUGGESTIONS));
                        let Some((rows, completion)) = edit else { return };
                        ui.typed.set(value.to_owned());
                        show_suggestions(ui, rows, value, ui.typed);
                        let full = format!("{value}{}", completion.as_ref().map_or("", |c| c.suffix.as_str()));
                        ui.completed.set(completion.map(|c| (full.clone(), c.url.to_string())));
                        ui.address.set(full.clone());
                        if full.len() > value.len() {
                            ui.address_ref.set_selection(value.chars().count() as u32, full.chars().count() as u32);
                        }
                    }
                })
                .on_submit(move |value: &str| submit(ui, ui.address, value.to_owned()))
                .placeholder(move || if ui.find_tabs.get() { "Find in this space's tabs" } else { "Search or enter address" })
                .label("Address")
                .auto_focus(true)
                .element_ref(&ui.address_ref)
                .flex_1()
                .text_size(px(TEXT_TITLE))
                .text_color(Theme::text());
            let commands = || {
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .h(px(22.))
                    .px_2()
                    .rounded(px(Theme::radius_small()))
                    .bg(Theme::accent_soft())
                    .text_color(Theme::space_label())
                    .child(icons::icon(icons::COMMAND, 11.))
                    .child(small("Commands").text_color(Theme::space_label()))
            };
            let head = view()
                .flex_row()
                .items_center()
                .gap_3()
                .px_4()
                .h(px(52.))
                .border_b_1()
                .border_color(Theme::hairline())
                .text_color(Theme::muted())
                .child(icons::icon(icons::SEARCH, 16.))
                .child(field)
                .child(when(move || ui.address.with(|a| tools::listing(a)), commands));
            let rows = dynamic(move || {
                if ui.find_tabs.get() {
                    return tools::find_rows(ui).into_element();
                }
                let typed = ui.address.with(|a| !a.trim().is_empty() && ui.snap.with(|s| *a != s.url));
                if typed { suggestion_rows(ui, ui.address, true).into_element() } else { view().into_element() }
            });
            let hint = tarn_nsh_shared::controls::key_hint;
            let footer = view()
                .flex_row()
                .items_center()
                .gap_4()
                .px_4()
                .h(px(36.))
                .border_t_1()
                .border_color(Theme::hairline())
                .child(hint("↵", "Run"))
                .child(hint("↑+↓", "Move"))
                .child(view().flex_1())
                .child(hint("Esc", "Close"));
            let card = view()
                .flex_col()
                .w(px(PALETTE_WIDTH))
                .rounded(px(Theme::radius_card()))
                .surface()
                .shadow(Theme::elevation(18., 48.))
                .overflow_hidden()
                .child(head)
                .child(rows)
                .child(footer);
            let dim = nsh_base::button().accessibility_label("Close").on_press(move || close_palette(ui)).absolute().inset_0().bg(rgba(PALETTE_DIM));
            view()
                .flex_col()
                .flex_1()
                .relative()
                .items_center()
                .child(tarn_nsh_shared::motion::fading(dim))
                .child(view().flex_grow(1.))
                .child(tarn_nsh_shared::motion::popping(card))
                .child(view().flex_grow(6.))
        },
    )
    .into_element()
}

fn chip<M>(name: impl IntoProp<Option<SharedString>, M>, on: impl Fn() + 'static) -> nsh_base::ButtonElement {
    controls::pressable(name, on).h(px(28.)).px(px(10.)).rounded_full().border_1().border_color(Theme::surface_edge()).text_color(Theme::muted())
}

fn chip_content(name: &'static str) -> View {
    view().flex_row().items_center().gap(px(6.)).tooltip(controls::tip(name, Placement::Top))
}

fn start_card(ui: Ui) -> AnyElement {
    when(
        move || ui.snap.with(|s| s.blank) && ui.panel.get() == Panel::None && !ui.editing.get(),
        move || {
            if ui.snap.with_untracked(|s| s.devtools) {
                devtools::refresh_servers(ui);
            }
            let field = input()
                .value(move || ui.command.get())
                .on_change(move |v: &str| {
                    // A new query reads the history again, so visits since the last one show.
                    if ui.command.with_untracked(String::is_empty) {
                        load_history(ui);
                    }
                    ui.command.set(v.to_owned());
                    show_suggestions(ui, crate::input::suggestions(&history(ui), &[], v, SUGGESTIONS), v, ui.command);
                })
                .on_submit(move |v: &str| {
                    submit(ui, ui.command, v.to_owned());
                    ui.command.set(String::new());
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
                .child(
                    chip(move || Some(SharedString::from(ui.snap.with(|s| format!("Space: {}", s.space_name)))), move || show(ui, Panel::Space)).child(
                        chip_content("Space Options")
                            .child(dot)
                            .child(icons::icon(icons::FOLDER, 12.))
                            .child(small(move || ui.snap.with(|s| s.space_name.clone()))),
                    ),
                )
                .child(
                    chip(
                        move || Some(SharedString::from(ui.snap.with(|s| format!("Shield level: {}", panels::capitalised(&s.level))))),
                        move || show(ui, Panel::Shield),
                    )
                    .bg(Secure::chip())
                    .hover(|s| s.bg(Secure::hover()))
                    .active(|s| s.bg(Secure::pressed()))
                    .border_color(Secure::hairline())
                    .text_color(Secure::muted())
                    .font_family(SYSTEM_FONT)
                    .child(
                        chip_content("Shield Level")
                            .child(icons::icon(icons::SHIELD, 12.))
                            .child(small(move || ui.snap.with(|s| panels::capitalised(&s.level))).text_color(Secure::muted())),
                    ),
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
                .child(when(typing, move || suggestion_rows(ui, ui.command, false).mb_1().border_t_1().border_color(Theme::hairline())))
                .child(footer);
            let column = view()
                .flex_col()
                .flex_1()
                .items_center()
                .px_4()
                .child(view().flex_grow(0.22_f32))
                .child(icons::mark(64.))
                .child(view().relative().mt(px(28.)).child(tarn_nsh_shared::search_fx::search_fx(54.)).child(card))
                .child(view().w(px(CARD_WIDTH)).child(devtools::server_strip(ui)))
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
    view().flex_row().items_center().gap_2().min_h(px(42.)).pl(px(14.)).pr_2().py_1().mb_2().rounded(px(Theme::radius_panel())).surface()
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
    bound_field(move || value.get(), move |v| value.set(v), name, width)
}

pub(super) fn bound_field(value: impl Fn() -> String + 'static, change: impl Fn(String) + 'static, name: &str, width: f32) -> nsh_base::Input {
    input()
        .value(value)
        .on_change(move |v: &str| change(v.to_owned()))
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
        Some(surface.shadow(Theme::elevation(4., 12.)).child(body(message).text_color(Secure::text()).flex_1().truncate()).children(buttons))
    })
}

fn take_answers(ui: Ui) -> (String, String, String) {
    let answers = (ui.answer_text.get_untracked(), ui.answer_user.get_untracked(), ui.answer_password.get_untracked());
    ui.answer_text.set(String::new());
    ui.answer_user.set(String::new());
    ui.answer_password.set(String::new());
    answers
}

fn answer_prompt(ui: Ui, id: u64, accept: bool) {
    let (t, u, p) = take_answers(ui);
    act(ui, |b| b.answer(id, accept, false, &t, &u, &p));
}

fn always_allow(ui: Ui, id: u64) {
    take_answers(ui);
    act(ui, |b| b.answer(id, true, true, "", "", ""));
}

fn silence_prompt(ui: Ui, id: u64) {
    take_answers(ui);
    act(ui, |b| b.silence(id));
}

/// Enter, not held, accepts an alert or a confirm only while neither a field nor the page has the
/// keyboard, so typing cannot answer it; a permission prompt stays pointer-only.
fn prompt_enter(ui: Ui, prompt: Memo<Option<browser_core::prompts::Prompt>>) -> AnyElement {
    dynamic(move || {
        let id = prompt.with(|p| p.as_ref().filter(|p| matches!(p.kind.as_str(), "alert" | "confirm")).map(|p| p.id))?;
        if !ui.editing.get() && !ui.finding.get() && ui.panel.get() == Panel::None {
            on_key_with("enter", KeyOptions { when: nsh_core::keymap::When::NotEditing, ..KeyOptions::default() }, move |_| answer_prompt(ui, id, true));
        }
        None::<View>
    })
    .into_element()
}

fn prompt_bar(ui: Ui) -> AnyElement {
    let shown = computed(move || ui.snap.with(|s| s.prompt.clone()));
    let enter = prompt_enter(ui, shown);
    let card = dynamic(move || {
        let prompt = shown.get()?;
        let id = prompt.id;
        let reply = move |accept: bool| answer_prompt(ui, id, accept);
        if prompt.kind == crate::browser::extensions::PROMPT_KIND {
            return Some(extensions::site_prompt(ui, &prompt));
        }
        let ask = prompt.ask();
        let glyph = icons::glyph(browser_core::glyphs::glyph(&prompt.kind));
        let message = prompt.message.clone().filter(|m| !m.is_empty()).or_else(|| browser_core::prompts::meaning(&prompt.kind).map(str::to_owned));
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
            "camera" | "microphone" | "location" => ("Allow Once", Some("Deny")),
            _ => ("OK", Some("Cancel")),
        };
        let remember = prompt.is_permission() && read(ui, |b| b.remembers(&prompt)).unwrap_or(false);
        let buttons = view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(tarn_nsh_shared::controls::secure_ghost("Block requests from this page", move || silence_prompt(ui, id)))
            .children(decline.map(|name| tarn_nsh_shared::controls::secure_ghost(name, move || reply(false))))
            .child(secure_button(accept, move || reply(true), !remember))
            .children(remember.then(|| secure_button("Always Allow", move || always_allow(ui, id), true)));
        let insecure = prompt.insecure.then(|| body("This connection is not secure: the password would be sent unencrypted.").text_color(Secure::warning()));
        let tile = view()
            .w(px(30.))
            .h(px(30.))
            .flex_none()
            .rounded(px(8.))
            .items_center()
            .justify_center()
            .bg(Secure::control())
            .text_color(if glyph == icons::TRIANGLE_ALERT { Secure::warning() } else { Secure::accent() })
            .child(icons::icon(glyph, 16.));
        let words = view()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .gap(px(2.))
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .items_baseline()
                    .gap_1()
                    .child(body(prompt.origin.clone()).font_weight(FontWeight::BOLD).text_color(Secure::text()))
                    .child(body(ask).text_color(Secure::text())),
            )
            .children(insecure)
            .children(message.map(|m| small(m).text_color(Secure::muted())))
            .children(fields);
        Some(tarn_nsh_shared::motion::popping(
            secure_surface(PROMPT_RADIUS)
                .flex_row()
                .items_center()
                .gap_3()
                .px_3()
                .py(px(10.))
                .mb_2()
                .shadow(Theme::elevation(6., 18.))
                .role(AccessibilityRole::Alert)
                .child(tile)
                .child(words)
                .child(buttons),
        ))
    });
    view().child(enter).child(card).into_element()
}

fn close_find(ui: Ui) {
    ui.finding.set(false);
    ui.find.set(String::new());
    act(ui, |b| b.find_typed(""));
}

fn find_again(ui: Ui, backwards: bool) {
    if ui.finding.get_untracked() {
        let text = ui.find.get_untracked();
        act(ui, |b| b.find(&text, backwards));
    }
}

fn find_bar(ui: Ui) -> AnyElement {
    let matches = move || match ui.snap.with(|s| s.found).filter(|_| !ui.find.with(String::is_empty)) {
        None => String::new(),
        Some(0) => "No matches".to_owned(),
        Some(1) => "1 match".to_owned(),
        Some(n) => format!("{n} matches"),
    };
    when(
        move || ui.finding.get(),
        move || {
            let field = input()
                .value(move || ui.find.get())
                .on_change(move |v: &str| {
                    ui.find.set(v.to_owned());
                    let text = v.to_owned();
                    act(ui, |b| b.find_typed(&text));
                })
                .on_submit(move |_: &str| find_again(ui, false))
                .placeholder("Find in page")
                .label("Find in page")
                .auto_focus(true)
                .element_ref(&ui.find_ref)
                .flex_1()
                .text_color(Theme::text());
            let previous = format!("Previous match ({})", crate::shortcuts::display(crate::shortcuts::FIND_PREVIOUS));
            let next = format!("Next match ({})", crate::shortcuts::display(crate::shortcuts::FIND_NEXT));
            let pill = view()
                .flex_row()
                .items_center()
                .gap_2()
                .w(px(FIND_WIDTH))
                .h(px(36.))
                .pl_3()
                .pr_1()
                .rounded(px(FIND_RADIUS))
                .surface()
                .shadow(Theme::elevation(6., 18.))
                .text_color(Theme::muted())
                .child(icons::icon(icons::SEARCH, 14.))
                .child(field)
                .child(small(matches).text_color(move || if ui.snap.with(|s| s.found == Some(0)) { Theme::danger() } else { Theme::faint() }))
                .child(view().w(px(1.)).h(px(18.)).bg(Theme::hairline()))
                .child(controls::icon_button_at(icons::CHEVRON_UP, &previous, Placement::Left, move || find_again(ui, true)))
                .child(controls::icon_button_at(icons::CHEVRON_DOWN, &next, Placement::Left, move || find_again(ui, false)))
                .child(controls::icon_button_at(icons::CLOSE, "Close find bar (Esc)", Placement::Left, move || close_find(ui)));
            view().flex_row().justify_end().mb_2().child(tarn_nsh_shared::motion::popping(pill))
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
    let covered =
        move || ui.snap.with(|s| s.blank || s.reader.is_some() || s.blocked.is_some() || s.json.is_some()) || ui.panel.get() != Panel::None || ui.editing.get();
    let bare = move || covered() || ui.snap.with(|s| s.split.is_some() || s.fullscreen);
    view()
        .flex_col()
        .flex_1()
        .rounded(px(Theme::radius_page()))
        .overflow_hidden()
        .bg(move || if bare() { rgba(0) } else { Theme::page() })
        .border_1()
        .border_color(move || if bare() { rgba(0) } else { Theme::surface_edge() })
        .child(themed(move || palette(ui)))
        .child(when(move || !ui.editing.get(), move || panels::panel_view(ui)))
        .child(themed(move || tools::reader_view(ui)))
        .child(themed(move || devtools::json_view(ui)))
        .child(themed(move || start_card(ui)))
        .child(themed(move || blocking::page(ui)))
        .child(web_views(ui).flex_grow(move || if covered() { 0.0_f32 } else { 1.0 }).h(px(0.0)))
}

fn run_action(ui: Ui, action: Action) {
    match action {
        Action::NewTab => new_tab(ui),
        Action::OpenLocation => open_palette(ui),
        Action::OpenFile => open_file(ui),
        Action::CloseTab => act_moving(ui, |b| b.model.active.into_iter().for_each(|k| b.close_tab(k))),
        Action::ReopenClosedTab => {
            animate_next(POP);
            act(ui, |b| b.reopen());
        }
        Action::NextTab => act(ui, |b| b.cycle(true)),
        Action::PreviousTab => act(ui, |b| b.cycle(false)),
        Action::TogglePin => act(ui, |b| b.model.active.into_iter().for_each(|k| b.pin_toggle(k))),
        Action::BurnerTab => tools::run(ui, tools::Command::BurnerTab),
        Action::NewGroup => groups::new_group(ui, None),
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
        Action::TakeScreenshot => act(ui, Browser::take_screenshot),
        Action::ToggleReader => act(ui, |b| b.toggle_reader()),
        Action::Print => act(ui, Browser::print),
        Action::BookmarkTab => library::bookmark_tab(ui, None),
        Action::ManageBookmarks => show(ui, Panel::Bookmarks(None)),
        Action::ShowHistory => show(ui, Panel::History),
        Action::ToggleDownloads => show(ui, Panel::Downloads),
        Action::ToggleSidebar => toggle_sidebar(ui),
        Action::NewSpace => show(ui, Panel::NewSpace),
        Action::PrivateSpace => act(ui, |b| b.private_space()),
        Action::Settings => show(ui, Panel::Settings),
        Action::NewWindow => new_window(ui),
        Action::ToggleSplitView => act(ui, Browser::toggle_split),
        Action::FocusNextPane => act(ui, |b| b.focus_pane(true)),
        Action::FocusPreviousPane => act(ui, |b| b.focus_pane(false)),
        Action::SwapPanes => act(ui, Browser::swap_panes),
        Action::RotateSplitLayout => act(ui, |b| b.model.active.into_iter().for_each(|k| b.rotate_split(k))),
        Action::RemoveFromSplitView => act(ui, |b| b.model.active.into_iter().for_each(|k| b.leave_split(k))),
        Action::CloseAllTabs => act_moving(ui, Browser::close_all_tabs),
        Action::LastTab => act(ui, |b| b.slot(9)),
        Action::CopyMarkdownLink => tools::copy_page(ui, true),
        Action::ToggleShortcuts => show(ui, Panel::Shortcuts),
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
        close_palette(ui);
    } else if ui.snap.with_untracked(|s| s.link_peek.is_some()) {
        link_peek::close(ui);
    } else if ui.panel.get_untracked() == Panel::Settings && ui.shortcut_edit.get_untracked().is_some() {
        ui.shortcut_edit.set(None);
        act(ui, |_| {});
    } else if let Some(id) = ui.snap.with_untracked(|s| s.prompt.as_ref().map(|p| p.id)) {
        answer_prompt(ui, id, false);
    } else if ui.finding.get_untracked() {
        close_find(ui);
    } else if ui.ext_popup.get_untracked().is_some() {
        extensions::show_popup(ui, None);
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
    for (keys, backwards) in [(crate::shortcuts::FIND_NEXT, false), (crate::shortcuts::FIND_PREVIOUS, true)] {
        on_key(keys, move |_| find_again(ui, backwards));
    }
    for n in 1..=8usize {
        on_key(&format!("mod+{n}"), move |_| act(ui, |b| b.slot(n)));
    }
    for n in 1..=9usize {
        on_key(&format!("alt+{n}"), move |_| {
            animate_next(SLIDE);
            act(ui, |b| b.space_number(n));
        });
    }
}

fn core() -> Rc<Core<Data>> {
    CORE.with_borrow_mut(|core| {
        core.get_or_insert_with(|| {
            let opened = Browser::open_core(&crate::data::root());
            crate::data::wipe_retired(&crate::data::root(), &opened.disk);
            crate::filter_updates::start(opened.clone());
            opened
        })
        .clone()
    })
}

pub fn refresh_panels() {
    for ui in WINDOWS.with_borrow(Vec::clone) {
        act(ui, |_| {});
    }
}

pub fn themes_changed() {
    for ui in WINDOWS.with_borrow(Vec::clone) {
        act(ui, Browser::apply_theme);
    }
}

fn open_settings(ui: Ui, pane: tarn_nsh_shared::panes::Pane) {
    ui.settings_pane.set(pane);
    ui.ext_pane.set(false);
    if ui.panel.get_untracked() == Panel::Settings {
        act(ui, |_| {});
    } else {
        show(ui, Panel::Settings);
    }
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
                b.window_changed(state.mode);
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
    extensions::show_popup(ui, None);
    act(ui, Browser::close_window);
    BROWSERS.with_borrow_mut(|windows| windows.remove(&ui.window));
    WINDOWS.with_borrow_mut(|windows| windows.retain(|w| w.window != ui.window));
    if !here.is_main() {
        WINDOW_REFS.with_borrow_mut(|refs| refs.remove(&ui.window));
    }
    ui.open.set(false);
    if here.is_main() {
        here.hide();
    } else {
        here.close();
    }
}

fn icon_data(ui: Ui, key: TabKey, data: JsonValue) {
    let text = data["text"].as_str().unwrap_or_default().to_owned();
    drop(spawn(async move {
        let png = blocking(move || engine::favicon::normalize(&text)).await.ok().flatten();
        act_quietly(ui, |b| b.on_icon_event(key, &data, png));
    }));
}

fn route_downloads() {
    let mut downloads = webview::session::route();
    drop(spawn(async move {
        while let Some((tab, data)) = downloads.next().await {
            let Some(key) = snapshot::tab_key(&tab) else { continue };
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
    let auto_hide = browser.auto_hide_sidebar();
    BROWSERS.with_borrow_mut(|windows| windows.insert(window, browser));
    let ui = Ui {
        window,
        open: signal(true),
        snap: signal(Snapshot::default()),
        icons: signal(Default::default()),
        panel: signal(Panel::None),
        rev: signal(0),
        sidebar: signal(!auto_hide),
        address: signal(String::new()),
        typed: signal(String::new()),
        completed: signal(None),
        editing: signal(false),
        new_tab_card: signal(false),
        command: signal(String::new()),
        suggest: signal(Vec::new()),
        tabs: signal(Vec::new()),
        pasteable: signal(false),
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
        site_more: signal(false),
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
        ext_popup: signal(None),
        ext_pane: signal(false),
        remember: signal(false),
        picked: signal(0),
        history: signal(Default::default()),
        tab_menu: signal(None),
        confirm: signal(None),
        rename: signal(String::new()),
        rename_for: signal(String::new()),
        mark_query: signal(String::new()),
        mark_edit: signal(None),
        library_query: signal(String::new()),
        emoji: signal(String::new()),
        emoji_for: signal(String::new()),
        space_draft: signal(None),
        area: signal((0., 0.)),
        dragging: signal(None),
        split_drop: signal(None),
        resizing: signal(None),
        floating: signal(Default::default()),
        hover_card: signal(Default::default()),
        form: signal(profiles::Form::default()),
        card_name: signal(String::new()),
        card_space: signal(String::new()),
        import: signal(import::State::default()),
        root: Owner::current(),
    };
    let editing = std::cell::Cell::new(false);
    effect(move || {
        let now = ui.editing.get();
        if editing.replace(now) != now {
            untracked(|| act(ui, |_| {}));
        }
    });
    let finding = std::cell::Cell::new(false);
    effect(move || {
        let now = ui.finding.get();
        if finding.replace(now) != now {
            untracked(|| act_quietly(ui, |_| {}));
        }
    });
    let extensions_open = std::cell::Cell::new(false);
    effect(move || {
        let open = ui.panel.get() == Panel::Settings && ui.ext_pane.get();
        if extensions_open.replace(open) && !open {
            untracked(tarn_nsh_shared::extensions::close_store_sheet);
        }
    });
    let fitted = std::cell::Cell::new(None);
    effect(move || {
        let area = ui.area.get();
        let wanted = ui.snap.with(|s| (s.responsive, s.views.iter().find(|v| v.active && v.pane.is_none()).map(|v| v.key), s.url.clone()));
        if fitted.replace(Some((area, wanted.clone()))) != Some((area, wanted)) {
            untracked(|| act_quietly(ui, |b| b.fit_responsive(area.0, area.1)));
        }
    });
    let shown_space = std::cell::Cell::new(None);
    effect(move || {
        let space = ui.snap.with(|s| s.space);
        if shown_space.replace(Some(space)).is_some_and(|before| before != space) {
            untracked(|| {
                ui.command.set(String::new());
                clear_suggestions(ui);
                if ui.editing.get() {
                    ui.editing.set(false);
                    ui.address_ref.blur();
                }
            });
        }
    });
    WINDOWS.with_borrow_mut(|windows| windows.push(ui));
    if LAUNCHING.replace(LAUNCHING.get().saturating_sub(1)) == 1 {
        for ui in WINDOWS.with_borrow(Vec::clone) {
            act_quietly(ui, |b| {
                b.archive_unused_tabs();
            });
        }
    }
    ui
}

fn browser_window(ui: Ui) -> AnyElement {
    let here = current_window();
    keep_placement(ui, here.clone());
    WINDOW_REFS.with_borrow_mut(|refs| refs.insert(ui.window, here.clone()));
    here.on_focus_change(move |focused| {
        if focused {
            FOCUSED.with(|f| f.set(ui.window));
            act(ui, |b| {
                b.sync_shared();
                b.settle_downloads();
            });
        }
    });
    let closing = here.clone();
    here.on_close_press(move |_| close_window(ui, &closing));
    act(ui, |_| {});
    if read(ui, Browser::devtools_on) == Some(true) {
        devtools::refresh_servers(ui);
    }
    drop(set_interval(SCHEDULE_TICK, move || {
        let windows = WINDOWS.with_borrow(|windows| windows.len());
        act_quietly(ui, |b| {
            b.schedule_tick();
            b.idle_tick(windows);
        })
    }));
    drop(set_interval(SAMPLE_TICK, move || {
        if ui.speaking.get_untracked() && !crate::speech::speaking() {
            ui.speaking.set(false);
        }
        act_quietly(ui, |b| b.sample_processes());
    }));
    drop(set_interval(browser_core::closed::ARCHIVE_INTERVAL, move || {
        act_quietly(ui, |b| {
            b.archive_unused_tabs();
        })
    }));
    act(ui, |b| b.schedule_tick());
    for (urls, dropped) in PENDING_LINKS.take() {
        deliver_links(ui, &urls, dropped);
    }
    let launch = LAUNCH_ARGS.take();
    if !launch.is_empty() {
        open_app_links(launch);
    }
    effect(move || {
        let light = !dark();
        untracked(|| act(ui, |b| b.follow_appearance(light)));
    });
    let full = computed(move || ui.snap.with(|s| s.fullscreen));
    view()
        .flex_row()
        .size_full()
        .relative()
        .p(move || px(if full.get() { 0. } else { 8. }))
        .gap(move || px(if full.get() { 0. } else { 8. }))
        .text_color(Theme::text)
        .font_family(Theme::font_family)
        .child(themed(|| view().absolute().inset_0().paint(Theme::backdrop())))
        .on_layout(move |b: &Bounds<Points>| peek::window_height(ui, b.size.height.0))
        .child(when(move || ui.sidebar.get() && !full.get(), move || themed(move || sidebar(ui))))
        .child(
            view()
                .flex_col()
                .flex_1()
                .child(when(move || !full.get(), move || toolbar(ui)))
                .child(themed(move || extensions::popup_panel(ui)))
                .child(notice_bar(ui))
                .child(when(move || !full.get(), move || error_bar(ui)))
                .child(prompt_bar(ui))
                .child(themed(move || find_bar(ui)))
                .child(content(ui))
                .child(when(move || !full.get(), move || themed(move || tools::toast_bar(ui)))),
        )
        .child(peek::edge(ui))
        .child(peek::floating(ui))
        .child(peek::card(ui))
        .child(bound_keys(ui))
        .on_mouse_up(move |_: &MouseEvent| split::release(ui))
        .on_mouse_move(move |e: &MouseEvent| {
            if e.pressed_button.is_none() {
                split::release(ui);
            }
        })
        .into_element()
}

#[component]
pub fn App() -> impl IntoElement {
    let mut attributes = WindowAttributes::default();
    attributes.close_on_press = Some(false);
    main_window().set(attributes);
    route_downloads();
    extensions::route(core());
    let saved = Browser::saved_windows(&core());
    LAUNCHING.set(saved.len().max(1));
    let mut rows = saved.into_iter();
    let ui = new_ui(rows.next().unwrap_or_default());
    if shift_held() {
        act(ui, Browser::enter_safe_mode);
    }
    rows.for_each(open_browser_window);
    LAUNCH_ARGS.set(std::env::args().skip(1).collect());
    drop(spawn(async move {
        loop {
            let launch = next_second_instance().await;
            open_app_links(launch.args);
        }
    }));
    view().size_full().child(when(move || ui.open.get(), move || browser_window(ui)))
}

fn open_app_links(args: Vec<String>) {
    let windows = WINDOWS.with_borrow(Vec::clone);
    hide_closed_main(&windows);
    let now = std::time::Instant::now();
    let (urls, dropped) = if args.is_empty() {
        if INCOMING.with_borrow_mut(|seen| browser_core::incoming::admit(seen, now, 1)) == 0 {
            return;
        }
        (Vec::new(), 0)
    } else {
        let (urls, dropped) = INCOMING.with_borrow_mut(|seen| browser_core::incoming::admit_links(seen, now, args));
        if urls.is_empty() {
            return;
        }
        (urls, dropped)
    };
    let usable: Vec<u64> =
        windows.iter().filter(|ui| ui.open.get_untracked() && read(**ui, |b| !b.model.burner_shown()) == Some(true)).map(|ui| ui.window).collect();
    let target = link_window(&usable, FOCUSED.with(Cell::get)).and_then(|id| windows.iter().find(|w| w.window == id).copied());
    match target {
        Some(ui) => deliver_links(ui, &urls, dropped),
        None if urls.is_empty() => windows.first().into_iter().for_each(|ui| deliver_links(*ui, &[], 0)),
        None => {
            PENDING_LINKS.with_borrow_mut(|pending| pending.push((urls, dropped)));
            open_browser_window(Browser::window_row(&core(), SavedWindow::default()));
        }
    }
}

fn link_window(usable: &[u64], focused: u64) -> Option<u64> {
    usable.contains(&focused).then_some(focused).or_else(|| usable.first().copied())
}

fn deliver_links(ui: Ui, urls: &[String], dropped: usize) {
    if !urls.is_empty() {
        if ui.editing.get_untracked() {
            close_palette(ui);
        }
        act(ui, |b| {
            b.open_app_links(urls);
            if dropped > 0 {
                b.show_toast(browser_core::incoming::dropped_notice(dropped), None);
            }
        });
    }
    if let Some(here) = WINDOW_REFS.with_borrow(|refs| refs.get(&ui.window).cloned()) {
        here.show();
        here.focus();
    }
}

fn hide_closed_main(windows: &[Ui]) {
    let open: HashSet<u64> = windows.iter().map(|w| w.window).collect();
    let closed = WINDOW_REFS.with_borrow(|refs| refs.iter().filter(|(id, r)| r.is_main() && !open.contains(id)).map(|(_, r)| r.clone()).collect::<Vec<_>>());
    closed.iter().for_each(WindowRef::hide);
}

#[cfg(test)]
mod tests {
    #[test]
    fn links_go_to_the_focused_usable_window_else_the_first() {
        assert_eq!(super::link_window(&[1, 2, 3], 2), Some(2));
        assert_eq!(super::link_window(&[1, 3], 2), Some(1), "the focused window shows a burner tab or is closed");
        assert_eq!(super::link_window(&[], 2), None);
    }
}
