use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use browser_core::bookmarks::{self, title_or_host};
use browser_core::import::{self as data, Kinds, Source};
use browser_core::library::{self, HISTORY_LOAD, LIST_ROWS as MAX_LIST, SECTION_ROWS as MAX_ROWS, ago};
use browser_core::snapshot::{self, LoadError, SpaceSnapshot};
use engine::menu::{self, Item as MenuItem};
use futures::channel::oneshot;
use gpui::{
    AnyElement, App, AppContext, Bounds, ClipboardItem, Context, Entity, Focusable, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    PromptLevel, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, WindowHandle, actions, div, point,
    prelude::FluentBuilder, px, size,
};
use store::{Bookmark, Download, HistoryEntry, Id, Snapshot, Tab};
use ui::controls::{button, caption, empty, icon_button, key_hint, surface, tooltip};
use ui::icon::{ICON_SMALL, Icon, icon};
use ui::motion::{EXIT, POP, SETTLE, animate, animate_linear, arrive};
use ui::text::OneLine;
use ui::text::bounded_title;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{TEXT, TEXT_SMALL, TOOLBAR_HEIGHT, Theme};

use crate::ToggleLibrary;
use crate::browser::{Browser, CommandMode, TabKey, now, tab_host};
use crate::peek::{popup_options, remove_after_exit};
use crate::prefs::Pref;
use crate::settings::thousands;
use crate::sidebar::{SIDEBAR_MARGIN, site_icon};

const CARD_WIDTH: f32 = 300.;
const SEARCH_HEIGHT: f32 = 44.;
const HEADER_ROW: f32 = 26.;
const BACK_ROW: f32 = 32.;
const EMPTY_ROW: f32 = 44.;
const DIVIDER: f32 = 13.;
const ROW: f32 = 30.;
const DETAIL_ROW: f32 = 40.;
const PAGE_SHIFT: f32 = 14.;
const LIST_PAD: f32 = 6.;
pub(crate) const SPACE_ROW: f32 = 40.;
const IMPORT_TIMEOUT: Duration = Duration::from_secs(120);
const MEMORY_REFRESH: Duration = Duration::from_secs(2);
const DELETE_KEY: &str = if cfg!(windows) { "Delete" } else { "⌘⌫" };

mod actions;
mod browser_api;
mod entries;
use entries::*;

actions!(library, [SaveSnapshot, RestoreSnapshot]);

pub(crate) use browser_core::shield_card::date;

/// The state a finished download is shown with once its file is gone; never stored.
const MISSING: &str = "missing";

fn download_row(store: &store::Store, d: &Download, now: i64) -> Entry {
    let state = match d.state.as_str() {
        "failed" => "Failed",
        "cancelled" => "Cancelled",
        MISSING => "Missing",
        s if s.starts_with("finished") => "Finished",
        _ => "In Progress",
    };
    let detail = tab_host(&d.url).into_iter().chain([state.to_owned(), ago(store, d.started_at, now)]).collect::<Vec<_>>().join(" · ");
    Entry::Row(Row { item: Item::Download(d.clone()), title: library::download_name(d), named: None, detail: Some(detail) })
}

fn visit_row(store: &store::Store, h: &HistoryEntry, now: i64) -> Entry {
    Entry::Row(Row { item: Item::Url(h.url.clone()), title: title_or_host(&h.title, &h.url), named: None, detail: Some(ago(store, h.visited_at, now)) })
}

fn visit(r: &Row) -> Option<&str> {
    match (&r.item, r.named) {
        (Item::Url(url), None) => Some(url),
        _ => None,
    }
}

fn detect_sources() -> Vec<Source> {
    std::env::home_dir().map(|home| data::sources(&home)).unwrap_or_default()
}

fn snapshot_error(e: LoadError) -> &'static str {
    match e {
        LoadError::Missing => "This snapshot no longer exists.",
        LoadError::Unreadable => "This snapshot could not be read.",
        LoadError::NewerVersion => "This snapshot was saved by a newer version of Tarn.",
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Page {
    Home,
    History,
    Bookmarks(Option<Id>),
    Downloads,
    Snapshots,
    Import,
    Memory,
}

/// A row whose name can be edited and which can be deleted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Named {
    Bookmark(Id),
    Snapshot(Id),
}

#[derive(Clone, Debug)]
enum Item {
    Tab(TabKey),
    Url(String),
    Folder(Id),
    Page(Page),
    Download(Download),
    Snapshot(Snapshot),
    ClearDownloads,
    SleepInactive,
    /// An index into the library's detected import sources.
    Source(usize),
}

#[derive(Clone)]
enum Cmd {
    Open(Item),
    OpenInNewTab(String),
    Rename(Named),
    Move(Id, Option<Id>),
    Delete(Named),
    Restore(Snapshot, bool),
    NewFolder,
    ToggleBookmark(TabKey),
    CopyLink(String),
    RemoveDownload(Id),
    RemoveHistory(String),
}

#[derive(Clone)]
struct Row {
    item: Item,
    title: String,
    named: Option<Named>,
    detail: Option<String>,
}

enum Entry {
    Header(&'static str, Option<usize>),
    Back(String),
    Divider,
    Empty(String),
    Row(Row),
}

pub struct LibraryWindow {
    browser: WeakEntity<Browser>,
    main: WindowHandle<Browser>,
    query: Entity<TextInput>,
    page: Page,
    selected: usize,
    /// The row under the pointer, which shows its actions in place of its host, and the list's
    /// scroll offset it was found at: a scroll moves another row under the pointer.
    hovered: Option<usize>,
    hovered_at: gpui::Point<gpui::Pixels>,
    history: Vec<HistoryEntry>,
    /// The query and the indices of the history entries it matches, kept until either changes.
    history_hits: RefCell<Option<(String, Rc<[usize]>)>>,
    /// The selectable rows the last render showed, with their child index in the list.
    shown: Vec<(usize, Row)>,
    downloads: Vec<Download>,
    /// Finished downloads whose file is gone, checked off the main thread when `downloads` is read.
    missing: HashSet<Id>,
    /// The browser's download change count and profile `downloads` was read for.
    downloads_seen: Option<(u64, Option<(bool, Id)>)>,
    snapshots: Vec<Snapshot>,
    sources: Vec<Source>,
    renaming: Option<(Named, Entity<TextInput>, [Subscription; 2])>,
    scroll: ScrollHandle,
    /// Counts page changes so each one replays the list's slide.
    turn: usize,
    backward: bool,
    closing: bool,
    /// A context menu or alert is up; the popover stays open although it is not the key window.
    holding: bool,
    fit: Option<Fit>,
    _subscriptions: Vec<Subscription>,
}

/// Where the popover sits in its browser window, so each page can refit its height to its rows.
struct Fit {
    x: f32,
    bottom: f32,
    width: f32,
    max_list: f32,
}

impl LibraryWindow {
    fn new(
        browser: WeakEntity<Browser>,
        main: WindowHandle<Browser>,
        history: Vec<HistoryEntry>,
        downloads: Vec<Download>,
        edit: Option<Named>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| TextInput::new("Search", cx));
        let mut subscriptions = vec![
            cx.subscribe_in(&query, window, Self::on_query),
            cx.observe_window_activation(window, |this, window, cx| {
                let menu = this.query.read(cx).menu_open() || this.renaming.as_ref().is_some_and(|(_, input, _)| input.read(cx).menu_open());
                if !window.is_window_active() && !this.holding && !menu {
                    this.close(false, window, cx);
                }
            }),
        ];
        subscriptions.extend(browser.upgrade().map(|b| {
            cx.observe(&b, |this, b, cx| {
                let b = b.read(cx);
                let seen = (b.downloads_popover.changed, b.model.current_space().map(|s| (s.private, s.space.profile_id)));
                if this.downloads_seen != Some(seen) {
                    this.downloads_seen = Some(seen);
                    this.downloads = b.downloads();
                    this.check_files(cx);
                }
                cx.notify();
            })
        }));
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while this.update(cx, |this, cx| this.sample_on_memory_page(cx)).is_ok() {
                cx.background_executor().timer(MEMORY_REFRESH).await;
            }
        })
        .detach();
        window.focus(&query.focus_handle(cx));
        let history = library::unique_history(history);
        let mut library = LibraryWindow {
            browser,
            main,
            query,
            page: Page::Home,
            selected: 0,
            hovered: None,
            hovered_at: Default::default(),
            history,
            history_hits: RefCell::new(None),
            shown: Vec::new(),
            downloads,
            missing: HashSet::new(),
            downloads_seen: None,
            snapshots: Vec::new(),
            sources: Vec::new(),
            renaming: None,
            scroll: ScrollHandle::new(),
            turn: 0,
            backward: false,
            closing: false,
            holding: false,
            fit: None,
            _subscriptions: subscriptions,
        };
        library.reload_snapshots(cx);
        library.check_files(cx);
        let found = match edit {
            Some(Named::Bookmark(id)) => {
                library.browser.upgrade().and_then(|b| b.read(cx).bookmarks.iter().find(|b| b.id == id).map(|bm| Page::Bookmarks(bm.parent_id)))
            }
            Some(Named::Snapshot(id)) => library.snapshots.iter().any(|s| s.id == id).then_some(Page::Snapshots),
            None => None,
        };
        if let (Some(page), Some(named)) = (found, edit) {
            library.page = page;
            library.select(named, cx);
            library.begin_rename(named, window, cx);
        }
        library
    }

    fn entries(&self, b: &Browser, cx: &App) -> Vec<Entry> {
        let query = self.query.read(cx).text().trim();
        let lists_history = self.page == Page::History || self.page == Page::Home && !query.is_empty();
        let hits = if lists_history { self.history_hits(query) } else { Rc::from([]) };
        let history: Vec<&HistoryEntry> = hits.iter().take(MAX_LIST).map(|&ix| &self.history[ix]).collect();
        let downloads: Vec<Download> =
            self.downloads.iter().map(|d| if self.missing.contains(&d.id) { Download { state: MISSING.into(), ..d.clone() } } else { d.clone() }).collect();
        entries(b, self.page, query, &history, &downloads, &self.snapshots, &self.sources)
    }

    fn history_hits(&self, query: &str) -> Rc<[usize]> {
        let mut memo = self.history_hits.borrow_mut();
        if let Some((_, hits)) = memo.as_ref().filter(|(q, _)| q == query) {
            return hits.clone();
        }
        let hits: Rc<[usize]> = library::history_hits(&self.history, query).into();
        *memo = Some((query.to_owned(), hits.clone()));
        hits
    }

    /// Each selectable row with its index among all entries, which is its child index in the list.
    fn rows(&self, cx: &App) -> Vec<(usize, Row)> {
        let Some(b) = self.browser.upgrade() else { return Vec::new() };
        self.entries(b.read(cx), cx).into_iter().enumerate().filter_map(|(ix, e)| if let Entry::Row(r) = e { Some((ix, r)) } else { None }).collect()
    }

    fn reveal(&self) {
        if let Some((ix, _)) = self.shown.get(self.selected) {
            self.scroll.scroll_to_item(*ix);
        }
    }

    fn select(&mut self, named: Named, cx: &App) {
        self.shown = self.rows(cx);
        if let Some(pos) = self.shown.iter().position(|(_, r)| r.named == Some(named)) {
            self.selected = pos;
            self.reveal();
        }
    }

    /// Up, Down and Enter act on the rows shown; an edit refilters them at once, and the render after
    /// reuses the memoised history filter.
    fn on_query(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Changed => {
                self.selected = 0;
                self.shown = self.rows(cx);
                self.scroll.scroll_to_item(0);
            }
            TextInputEvent::Up => self.selected = self.selected.saturating_sub(1),
            TextInputEvent::Down => self.selected = (self.selected + 1).min(self.shown.len().saturating_sub(1)),
            TextInputEvent::Submit => {
                if let Some((_, r)) = self.shown.get(self.selected).cloned() {
                    self.open(r.item, window, cx);
                }
            }
            TextInputEvent::Cancel => self.back(window, cx),
            TextInputEvent::PastEnd => return,
            TextInputEvent::MenuClosed => {
                if !window.is_window_active() && !self.holding {
                    self.close(false, window, cx);
                }
                return;
            }
        }
        if !matches!(event, TextInputEvent::Changed) {
            self.reveal();
        }
        cx.notify();
    }

    fn sample_on_memory_page(&self, cx: &mut Context<Self>) {
        if self.page == Page::Memory {
            self.browser.update(cx, |b, cx| b.sample_processes(cx)).ok();
        }
    }

    fn sleep(&self, keys: Vec<TabKey>, cx: &mut Context<Self>) {
        self.browser.update(cx, |b, cx| b.sleep_tabs(keys, cx)).ok();
    }

    fn go(&mut self, page: Page, window: &Window, cx: &mut Context<Self>) {
        self.hovered = None;
        if page == Page::Import {
            self.sources = detect_sources();
        }
        if page == Page::Snapshots {
            self.reload_snapshots(cx);
        }
        self.page = page;
        self.turn += 1;
        self.backward = false;
        self.sample_on_memory_page(cx);
        self.selected = 0;
        self.query.update(cx, |q, cx| q.set_text("", cx));
        self.shown = self.rows(cx);
        self.scroll.scroll_to_item(0);
        self.refit(window, cx);
        cx.notify();
    }

    fn refit(&self, window: &Window, cx: &mut Context<Self>) {
        let (Some(fit), Some(browser)) = (&self.fit, self.browser.upgrade()) else { return };
        let listed = self.entries(browser.read(cx), cx);
        self.place(card_height(&listed, row_boundary(&listed, fit.max_list)), window, cx);
    }

    fn place(&self, height: f32, window: &Window, cx: &mut Context<Self>) {
        let (Some(fit), Some(browser)) = (&self.fit, self.browser.upgrade()) else { return };
        let (Some(main), Some(popup)) = (browser.read(cx).host.clone(), engine::Host::from_window(window)) else { return };
        let (x, bottom, width) = (fit.x, fit.bottom, fit.width);
        cx.spawn(async move |_, _| main.place(&popup, x.into(), (bottom - height).into(), width.into(), height.into())).detach();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = |id| self.browser.upgrade().and_then(|b| b.read(cx).bookmarks.iter().find(|bm| bm.id == id).and_then(|bm| bm.parent_id));
        match self.page {
            Page::Home => return self.close(true, window, cx),
            Page::History | Page::Bookmarks(None) | Page::Downloads | Page::Snapshots | Page::Import | Page::Memory => self.go(Page::Home, window, cx),
            Page::Bookmarks(Some(id)) => self.go(Page::Bookmarks(parent(id)), window, cx),
        }
        self.backward = true;
    }

    fn open(&mut self, item: Item, window: &mut Window, cx: &mut Context<Self>) {
        match item {
            Item::Tab(key) => {
                self.main.update(cx, |b, window, cx| b.switch_to(key, window, cx)).ok();
                self.close(true, window, cx);
            }
            Item::Url(url) => {
                self.main.update(cx, |b, window, cx| b.open_url(&url, window, cx)).ok();
                self.close(true, window, cx);
            }
            Item::Folder(id) => self.go(Page::Bookmarks(Some(id)), window, cx),
            Item::Page(page) => self.go(page, window, cx),
            Item::Download(d) => engine::reveal_in_finder(std::path::Path::new(&d.path)),
            Item::Snapshot(saved) => self.restore(saved, true, window, cx),
            Item::Source(ix) => self.start_import(ix, window, cx),
            Item::SleepInactive => {
                let keys = self.browser.upgrade().map(|b| b.read(cx).model.tabs.iter().map(|t| b.read(cx).model.key(t)).collect());
                self.sleep(keys.unwrap_or_default(), cx);
            }
            Item::ClearDownloads => self.confirm("Clear the downloads list?", Some("The files stay where they are."), "Clear List", window, cx, |this, cx| {
                this.remove_download(None, cx)
            }),
        }
    }

    fn restore(&mut self, saved: Snapshot, new_space: bool, window: &mut Window, cx: &mut Context<Self>) {
        match self.main.update(cx, |b, window, cx| b.restore_snapshot(&saved, new_space, window, cx)) {
            Ok(Err(message)) => self.alert(message, None, window, cx),
            _ => self.close(true, window, cx),
        }
    }

    fn reload_snapshots(&mut self, cx: &mut Context<Self>) {
        self.snapshots = self.browser.upgrade().map(|b| b.read(cx).snapshots()).unwrap_or_default();
        cx.notify();
    }

    fn remove_download(&mut self, id: Option<Id>, cx: &mut Context<Self>) {
        self.downloads = self
            .browser
            .update(cx, |b, _| {
                b.remove_download(id);
                b.downloads()
            })
            .unwrap_or_default();
        self.check_files(cx);
        cx.notify();
    }

    fn check_files(&mut self, cx: &mut Context<Self>) {
        let finished: Vec<(Id, String)> = self.downloads.iter().filter(|d| d.state.starts_with("finished")).map(|d| (d.id, d.path.clone())).collect();
        cx.spawn(async move |this, cx| {
            let missing: HashSet<Id> = cx
                .background_executor()
                .spawn(async move { finished.into_iter().filter(|(_, p)| crate::downloads::file_size(p).is_none()).map(|(id, _)| id).collect() })
                .await;
            this.update(cx, |this, cx| {
                if this.missing != missing {
                    this.missing = missing;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn close(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.end_rename(true, window, cx);
        let reduce = self.browser.upgrade().is_some_and(|b| b.read(cx).pref(Pref::ReduceMotion));
        remove_after_exit(reduce, window, cx);
        self.browser
            .update(cx, |b, cx| {
                b.library = None;
                b.library_closed = (!refocus).then(Instant::now);
                cx.notify();
            })
            .ok();
        if refocus {
            self.main.update(cx, |_, window, _| window.activate_window()).ok();
        }
        cx.notify();
    }

    fn back_row(&self, title: String, tally: Option<String>, on_bookmarks: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("library-back")
            .h(px(BACK_ROW))
            .px_1()
            .flex()
            .flex_none()
            .items_center()
            .gap_1()
            .rounded(px(Theme::radius_row()))
            .cursor_pointer()
            .font_weight(FontWeight::SEMIBOLD)
            .hover(|s| s.bg(Theme::hover()))
            .child(icon(Icon::ChevronLeft, 14., Theme::muted()))
            .child(div().flex_1().one_line(SharedString::from(title)))
            .children(tally.map(|n| div().flex_none().px_1().text_size(px(TEXT_SMALL)).font_weight(FontWeight::NORMAL).text_color(Theme::muted()).child(n)))
            .when(on_bookmarks, |d| {
                d.child(icon_button("library-new-folder", Icon::FolderPlus, true).tooltip(tooltip("New Folder", None)).on_click(cx.listener(
                    |this, _, window, cx| {
                        cx.stop_propagation();
                        this.new_folder(window, cx);
                    },
                )))
            })
            .on_click(cx.listener(|this, _, window, cx| this.back(window, cx)))
            .into_any_element()
    }

    fn search_box(&self, empty_query: bool, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .m_1p5()
            .h(px(SEARCH_HEIGHT - 12.))
            .px_2()
            .flex()
            .flex_none()
            .items_center()
            .gap_2()
            .rounded(px(Theme::radius_row()))
            .bg(Theme::raised())
            .child(icon(Icon::Search, 14., Theme::muted()))
            .child(div().flex_1().min_w_0().child(self.query.clone()))
            .when(empty_query, |d| d.child(key_hint(crate::shortcuts::shown(&crate::ToggleLibrary, cx))))
    }

    fn row_title(&self, r: &Row, editing: Option<Entity<TextInput>>, detail: Option<String>, cx: &mut Context<Self>) -> AnyElement {
        match (editing, detail) {
            (Some(input), _) => div()
                .flex_1()
                .min_w_0()
                .px_1()
                .rounded(px(Theme::radius_row()))
                .bg(Theme::raised())
                .on_mouse_down_out(cx.listener(|this, _, window, cx| this.end_rename(true, window, cx)))
                .child(input)
                .into_any_element(),
            (None, Some(detail)) => div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().one_line(SharedString::from(r.title.clone())))
                .child(div().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).one_line(SharedString::from(detail)))
                .into_any_element(),
            (None, None) => div().flex_1().one_line(SharedString::from(r.title.clone())).into_any_element(),
        }
    }

    fn row_aside(&self, r: &Row, b: Option<&Browser>) -> (Option<String>, Option<String>) {
        let inside = |folder: Option<Id>| b.map_or(0, |b| b.bookmarks.iter().filter(|bm| bm.parent_id == folder).count());
        // Shortened from the left like the toolbar's host (S5), so the registrable domain stays visible.
        let host =
            |url: Option<String>| url.and_then(|url| tab_host(&url)).filter(|host| *host != r.title).map(|host| browser_core::command::host_tail(&host, 16));
        let aside = match &r.item {
            Item::Url(url) => host(Some(url.clone())),
            Item::Tab(key) if r.detail.is_none() => host(b.and_then(|b| b.tab_url(*key))),
            Item::Page(Page::Bookmarks(None)) => tally(b.map_or(0, |b| b.bookmarks.iter().filter(|bm| bm.url.is_some()).count())),
            Item::Page(Page::Bookmarks(Some(id))) | Item::Folder(id) => tally(inside(Some(*id))),
            Item::Page(Page::Downloads) => tally(self.downloads.len()),
            Item::Page(Page::Snapshots) => tally(self.snapshots.len()),
            _ => None,
        };
        // A row with a detail line shows its host there, so the title keeps the row's width.
        match (&r.detail, aside) {
            (Some(detail), Some(host)) if matches!(r.item, Item::Url(_)) => (None, Some(format!("{detail} · {host}"))),
            (detail, aside) => (aside, detail.clone()),
        }
    }

    fn render_row(&self, ix: usize, r: Row, leading: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let selected = ix == self.selected;
        let drills = matches!(r.item, Item::Folder(_) | Item::Page(_));
        let editing = r.named.zip(self.renaming.as_ref()).filter(|(named, (renamed, _, _))| named == renamed).map(|(_, (_, input, _))| input.clone());
        let editing_now = editing.is_some();
        let item = r.item.clone();
        let menu_item = r.item.clone();
        let named = r.named;
        let visited = visit(&r).map(str::to_owned);
        let acting = selected || self.hovered == Some(ix);
        let on_hover = move |d: gpui::Div| if acting { d } else { d.hidden() };
        let browser = self.browser.upgrade();
        let b = browser.as_ref().map(|b| b.read(cx));
        let sleepable = match r.item {
            Item::Tab(key) if self.page == Page::Memory => {
                b.is_some_and(|b| b.model.active != Some(key) && !b.sleeping(key) && !b.in_split(key)).then_some(key)
            }
            _ => None,
        };
        let has_actions = r.named.is_some() || visited.is_some() || sleepable.is_some();
        let (aside, detail) = self.row_aside(&r, b);
        let title = self.row_title(&r, editing, detail, cx);
        div()
            .id(("library-row", ix))
            .h(px(if r.detail.is_some() { DETAIL_ROW } else { ROW }))
            .px_2()
            .flex()
            .flex_none()
            .items_center()
            .gap_2p5()
            .rounded(px(Theme::radius_row()))
            .cursor_pointer()
            .when(selected, |d| d.bg(Theme::selected()))
            .when(!selected, |d| d.hover(|s| s.bg(Theme::hover())))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let next = if *hovered { Some(ix) } else { this.hovered.filter(|h| *h != ix) };
                if next != this.hovered {
                    this.hovered = next;
                    cx.notify();
                }
            }))
            .on_mouse_move(cx.listener(move |this, _: &gpui::MouseMoveEvent, _, cx| {
                if this.hovered != Some(ix) {
                    this.hovered = Some(ix);
                    cx.notify();
                }
            }))
            .child(leading)
            .child(title)
            .when_some(aside.filter(|_| !editing_now), |d, aside| {
                let aside = div().flex_none().whitespace_nowrap().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child(SharedString::from(aside));
                d.child(aside.when(has_actions && acting, |a| a.hidden()))
            })
            .when_some(r.named.filter(|_| !editing_now), |d, named| {
                let id = match named {
                    Named::Bookmark(id) | Named::Snapshot(id) => id as u64,
                };
                d.child(
                    on_hover(div().flex().flex_none())
                        .child(icon_button(("rename", id), Icon::Pencil, true).tooltip(tooltip("Rename", Some("F2".into()))).on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.begin_rename(named, window, cx);
                            },
                        )))
                        .child(icon_button(("delete", id), Icon::Trash, true).tooltip(tooltip("Delete", Some(DELETE_KEY.into()))).on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.delete(named, window, cx);
                            },
                        ))),
                )
            })
            .when_some(visited, |d, url| {
                d.child(on_hover(div().flex().flex_none()).child(
                    icon_button(("forget-visit", ix), Icon::Trash, true).tooltip(tooltip("Remove from History", Some(DELETE_KEY.into()))).on_click(
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.remove_history(url.clone(), cx);
                        }),
                    ),
                ))
            })
            .when_some(sleepable, |d, key| {
                d.child(on_hover(div()).child(button(("library-sleep", ix), "Sleep", false).on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.sleep(vec![key], cx);
                }))))
            })
            .when(drills, |d| d.child(icon(Icon::ChevronRight, ICON_SMALL, Theme::muted())))
            .when(!matches!(r.item, Item::Page(_) | Item::ClearDownloads | Item::SleepInactive | Item::Source(_)), |d| {
                d.on_mouse_down(MouseButton::Right, cx.listener(move |this, _, window, cx| this.row_menu(menu_item.clone(), named, window, cx)))
            })
            .on_click(cx.listener(move |this, _, window, cx| this.open(item.clone(), window, cx)))
            .into_any_element()
    }
}

fn leading_icon(b: &Browser, item: &Item) -> AnyElement {
    match item {
        Item::Tab(key) => b.model.tab(*key).map_or_else(|| icon(Icon::Search, 16., Theme::muted()).into_any_element(), |t| site_icon(b, &t.url, 16.)),
        Item::Url(url) => site_icon(b, url, 16.),
        Item::Folder(_) => icon(Icon::Folder, 16., Theme::muted()).into_any_element(),
        Item::Page(Page::History) => icon(Icon::History, 16., Theme::muted()).into_any_element(),
        Item::Page(Page::Downloads) => icon(Icon::Download, 16., Theme::muted()).into_any_element(),
        Item::Download(d) => site_icon(b, &d.url, 16.),
        Item::Snapshot(_) | Item::Page(Page::Snapshots) => icon(Icon::Group, 16., Theme::muted()).into_any_element(),
        Item::ClearDownloads => icon(Icon::Trash, 16., Theme::muted()).into_any_element(),
        Item::Page(Page::Import) | Item::Source(_) => icon(Icon::AppWindow, 16., Theme::muted()).into_any_element(),
        Item::Page(Page::Memory) => icon(Icon::Gauge, 16., Theme::muted()).into_any_element(),
        Item::SleepInactive => div().size(px(16.)).flex_none().into_any_element(),
        Item::Page(_) => icon(Icon::Bookmark, 16., Theme::muted()).into_any_element(),
    }
}

impl Render for LibraryWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.scroll.offset() != self.hovered_at {
            self.hovered_at = self.scroll.offset();
            self.hovered = None;
        }
        let Some(browser) = self.browser.upgrade() else { return div().into_any_element() };
        let b = browser.read(cx);
        let reduce = b.pref(Pref::ReduceMotion);
        let on_bookmarks = matches!(self.page, Page::Bookmarks(_));
        let entries = self.entries(b, cx);
        let tab_at = |shown: &[(usize, Row)], ix: usize| match shown.get(ix).map(|(_, r)| &r.item) {
            Some(Item::Tab(key)) => Some(*key),
            _ => None,
        };
        let selected_tab = tab_at(&self.shown, self.selected);
        self.shown = entries.iter().enumerate().filter_map(|(ix, e)| if let Entry::Row(r) = e { Some((ix, r.clone())) } else { None }).collect();
        // The Memory page re-sorts by use; the highlight stays on the tab it was on.
        if let Some(ix) = selected_tab.and_then(|key| (0..self.shown.len()).find(|ix| tab_at(&self.shown, *ix) == Some(key))) {
            self.selected = ix;
        }
        let entries: Vec<(Entry, Option<AnyElement>)> = entries
            .into_iter()
            .map(|e| {
                let leading = match &e {
                    Entry::Row(r) => Some(leading_icon(b, &r.item)),
                    _ => None,
                };
                (e, leading)
            })
            .collect();
        let count = entries.iter().filter(|(e, _)| matches!(e, Entry::Row(_))).count();
        self.selected = self.selected.min(count.saturating_sub(1));
        let listed = entries.iter().filter(|(e, _)| matches!(e, Entry::Row(r) if !matches!(r.item, Item::ClearDownloads | Item::SleepInactive))).count();
        let capped = matches!(self.page, Page::History | Page::Downloads) || on_bookmarks && !self.query.read(cx).text().trim().is_empty();
        let mut ix = 0;
        let mut list =
            div().id("library-list").track_scroll(&self.scroll).flex_1().min_h_0().overflow_y_scroll().px_1p5().pb(px(LIST_PAD)).flex().flex_col().gap(px(1.));
        for (entry, leading) in entries {
            list = list.child(match entry {
                Entry::Header(text, n) => caption(text)
                    .h(px(HEADER_ROW))
                    .flex_none()
                    .flex()
                    .items_end()
                    .justify_between()
                    .px_2()
                    .pb_1()
                    .children(n.and_then(tally).map(|n| div().font_weight(FontWeight::NORMAL).text_color(Theme::faint()).child(n)))
                    .into_any_element(),
                Entry::Back(title) => self.back_row(title, listed_tally(listed, capped), on_bookmarks, cx),
                Entry::Divider => div().mx_2().my_1p5().h(px(1.)).flex_none().bg(Theme::hairline()).into_any_element(),
                Entry::Empty(text) => empty(text).h(px(EMPTY_ROW)).px_2().into_any_element(),
                Entry::Row(r) => {
                    ix += 1;
                    self.render_row(ix - 1, r, leading.unwrap_or_else(|| div().into_any_element()), cx)
                }
            });
        }
        let empty_query = self.query.read(cx).text().is_empty();
        let search = self.search_box(empty_query, cx);
        let list = match (self.turn, self.backward) {
            (0, _) => list.into_any_element(),
            (turn, backward) => {
                let shift = if backward { -PAGE_SHIFT } else { PAGE_SHIFT };
                animate_linear(list, ("library-page", turn), SETTLE, reduce, move |el, t| {
                    let (shown, ahead) = arrive(t);
                    el.opacity(shown).relative().left(px(ahead * shift))
                })
            }
        };
        // The card grows up from its bottom edge, the anchor, over content laid out at full height.
        let height = f32::from(window.bounds().size.height);
        let body = div().absolute().left_0().right_0().bottom_0().h(px(height - 2.)).flex().flex_col().child(search).child(list);
        let card =
            surface(Theme::radius_panel()).absolute().left_0().right_0().bottom_0().h(px(height)).text_size(px(TEXT)).text_color(Theme::text()).child(body);
        let reveal = move |el: gpui::Div, t: f32| el.opacity(t).h(px(height * (0.9 + 0.1 * t)));
        let card = if self.closing {
            animate(card, "library-out", EXIT, reduce, move |el, t| reveal(el, 1. - t))
        } else {
            animate(card, "library-in", POP, reduce, reveal)
        };
        div()
            .key_context("Library")
            .capture_key_down(cx.listener(Self::on_key))
            .capture_action(cx.listener(|this, _: &ui::text_input::Delete, window, cx| this.delete_selected(window, cx)))
            .capture_action(cx.listener(|this, _: &ui::text_input::DeleteToBeginning, window, cx| this.delete_selected(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleLibrary, window, cx| this.close(true, window, cx)))
            .on_action(cx.listener(|this, _: &crate::OpenLocation, window, cx| {
                this.close(true, window, cx);
                this.main.update(cx, |b, window, cx| b.open_command(CommandMode::Current, window, cx)).ok();
            }))
            .on_action(cx.listener(|this, _: &crate::Settings, window, cx| {
                this.close(false, window, cx);
                this.main.update(cx, |b, window, cx| b.open_settings(window, cx)).ok();
            }))
            .on_action(cx.listener(|this, _: &crate::menubar::ShowHistory, window, cx| this.go(Page::History, window, cx)))
            .on_action(cx.listener(|this, _: &crate::menubar::ManageBookmarks, window, cx| this.go(Page::Bookmarks(None), window, cx)))
            .on_action(cx.listener(|this, _: &crate::menubar::ShowMemory, window, cx| this.go(Page::Memory, window, cx)))
            .size_full()
            .child(card)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_list_ends_below_a_row() {
        let entries = [row(Item::Page(Page::History), String::new(), None), Entry::Divider, Entry::Header("Open Tabs", Some(0)), Entry::Empty(String::new())];
        assert_eq!(row_boundary(&entries, 1000.), 1000.);
        assert_eq!(row_boundary(&entries, LIST_PAD + ROW + 1. + DIVIDER + 1. + HEADER_ROW + 10.), LIST_PAD + ROW);
        assert_eq!(row_boundary(&entries[1..], 20.), 20., "nothing fits, so the cap stands");
    }

    #[test]
    fn card_fits_its_entries_up_to_the_cap() {
        let entries = [Entry::Header("Open Tabs", Some(0)), Entry::Empty(String::new()), Entry::Divider, row(Item::Page(Page::History), String::new(), None)];
        let list = HEADER_ROW + EMPTY_ROW + DIVIDER + ROW + 3. + LIST_PAD;
        assert_eq!(card_height(&entries, 1000.), SEARCH_HEIGHT + 2. + list);
        assert_eq!(card_height(&entries, 50.), SEARCH_HEIGHT + 2. + 50.);
    }

    #[test]
    fn empty_lists_say_why() {
        assert_eq!(empty_text(Page::History, ""), "No history yet");
        assert_eq!(empty_text(Page::Bookmarks(Some(1)), ""), "This folder is empty");
        assert_eq!(empty_text(Page::Home, "zz"), "No results for “zz”");
        assert_eq!((tally(0), tally(1200)), (None, Some("1,200".into())));
        assert_eq!(
            (listed_tally(12, true), listed_tally(MAX_LIST, true), listed_tally(350, false)),
            (Some("12".into()), Some("200+".into()), Some("350".into()))
        );
    }

    #[test]
    fn download_rows_show_name_host_state_and_age() {
        let d = Download {
            id: 1,
            url: "https://Files.test/x/a.zip".into(),
            path: "/Users/u/Downloads/a.zip".into(),
            state: "finished-unquarantined".into(),
            started_at: 0,
        };
        let store = store::Store::open_in_memory().unwrap();
        let Entry::Row(r) = download_row(&store, &d, 3 * 3_600_000) else { panic!() };
        assert_eq!((r.title.as_str(), r.detail.as_deref()), ("a.zip", Some("files.test · Finished · 3 h ago")));
        let blob = Download { url: String::new(), state: "failed".into(), ..d };
        let Entry::Row(r) = download_row(&store, &blob, 0) else { panic!() };
        assert_eq!(r.detail.as_deref(), Some("Failed · Just now"));
        let Entry::Row(r) = download_row(&store, &Download { state: MISSING.into(), ..blob }, 0) else { panic!() };
        assert_eq!(r.detail.as_deref(), Some("Missing · Just now"));
    }
}
