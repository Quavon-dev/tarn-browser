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
use ui::motion::{EXIT, POP, animate};
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

impl Browser {
    pub(crate) fn reload_bookmarks(&mut self, cx: &mut Context<Self>) {
        self.bookmarks_of = self.model.profile();
        if let Some(space) = self.model.current_space() {
            self.set_bookmarks(self.store(space.private).bookmarks(space.space.profile_id).unwrap_or_default());
        }
        cx.notify();
    }

    pub fn is_bookmarked(&self, url: &str) -> bool {
        bookmarks::find(&self.bookmarks, url).is_some()
    }

    fn bookmark_target(&self, key: Option<TabKey>) -> Option<(String, String)> {
        let tab = key.or(self.model.active).and_then(|k| self.model.tab(k))?;
        let url = self.tab_url(self.model.key(tab)).unwrap_or_else(|| tab.url.clone());
        (!url.is_empty()).then(|| (browser_core::text::stored_title_or_host(&tab.title, &url), url))
    }

    /// The top-level folder Save for Later files pages into, if it exists.
    pub(crate) fn reading_list(&self) -> Option<Id> {
        bookmarks::reading_list(&self.bookmarks)
    }

    /// Files the active page into the Reading List bookmark folder, made on first use, so it is
    /// kept, exported and deleted like any bookmark.
    pub fn save_for_later(&mut self, cx: &mut Context<Self>) {
        let (Some((title, url)), Some(space)) = (self.bookmark_target(None), self.model.current_space()) else { return };
        let (private, profile_id) = (space.private, space.space.profile_id);
        self.reload_bookmarks(cx);
        let saved = bookmarks::save_for_later(self.store(private), profile_id, &self.bookmarks, &url, &title, now());
        self.reload_bookmarks(cx);
        let (tone, text) = browser_core::toast::reading_list(saved);
        self.show_toast_as(tone, text, None, cx);
    }

    /// Bookmarks the tab, or returns its bookmark when it already has one.
    pub fn bookmark_tab(&mut self, key: Option<TabKey>, cx: &mut Context<Self>) -> Option<Id> {
        let (title, url) = self.bookmark_target(key)?;
        let space = self.model.current_space()?;
        let id = bookmarks::bookmark(self.store(space.private), space.space.profile_id, &self.bookmarks, &url, &title, now())?;
        self.reload_bookmarks(cx);
        Some(id)
    }

    pub fn unbookmark_tab(&mut self, key: Option<TabKey>, cx: &mut Context<Self>) {
        let Some((_, url)) = self.bookmark_target(key) else { return };
        bookmarks::unbookmark(self.store(self.model.private()), &self.bookmarks, &url);
        self.reload_bookmarks(cx);
    }

    /// Saves every tab into a new dated folder and opens the library on its name field.
    pub fn bookmark_all_tabs(&mut self, cx: &mut Context<Self>) {
        let Some(space) = self.model.current_space().filter(|_| !self.model.tabs.is_empty()) else { return };
        let pages: Vec<(&str, String)> = self
            .model
            .tabs
            .iter()
            .filter(|t| !t.url.is_empty())
            .map(|t| (t.title.as_str(), self.tab_url(self.model.key(t)).unwrap_or_else(|| t.url.clone())))
            .collect();
        let Some(folder) = bookmarks::bookmark_all(self.store(space.private), space.space.profile_id, pages.iter().map(|(t, u)| (*t, u.as_str())), now())
        else {
            return;
        };
        self.reload_bookmarks(cx);
        if let Some(main) = self.peek.main {
            cx.defer(move |cx| {
                main.update(cx, |b, window, cx| b.show_library(Some(folder), window, cx)).ok();
            });
        }
    }

    pub fn new_bookmark_folder(&mut self, parent: Option<Id>, cx: &mut Context<Self>) -> Option<Id> {
        let space = self.model.current_space()?;
        let id = bookmarks::new_folder(self.store(space.private), space.space.profile_id, parent, now())?;
        self.reload_bookmarks(cx);
        Some(id)
    }

    pub fn rename_bookmark(&mut self, id: Id, title: &str, cx: &mut Context<Self>) {
        self.store(self.model.private()).rename_bookmark(id, title).ok();
        self.reload_bookmarks(cx);
    }

    pub fn move_bookmark(&mut self, id: Id, parent: Option<Id>, cx: &mut Context<Self>) {
        self.store(self.model.private()).move_bookmark(id, parent).ok();
        self.reload_bookmarks(cx);
    }

    pub fn delete_bookmark(&mut self, id: Id, cx: &mut Context<Self>) {
        self.store(self.model.private()).delete_bookmark(id).ok();
        self.reload_bookmarks(cx);
    }

    /// Switches to a tab already showing `url`, otherwise opens it in a new one.
    pub fn open_url(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.command_mode.is_some() {
            self.close_command(window, cx);
        }
        if let Some(key) = self.model.tabs.iter().find(|t| t.url == url).map(|t| self.model.key(t)) {
            return self.activate(key, window, cx);
        }
        if let Ok(url) = url::Url::parse(url) {
            let before = self.model.active;
            self.new_tab(&url, window, cx);
            if self.model.active != before && self.is_bookmarked(url.as_str()) {
                self.mark_typed(&url);
            }
        }
    }

    /// Saves the current space's tabs as a snapshot named after it and opens the library on its name.
    pub fn save_snapshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.model.current_space() else { return };
        let name = format!("{} {}", space.space.name, date(now()));
        if let Some(saved) = snapshot::save(self.store(space.private), space, &name, now()) {
            self.open_library(Page::Snapshots, Some(Named::Snapshot(saved.id)), window, cx);
        }
    }

    /// The current profile's snapshots, newest first; a private space has none.
    pub(crate) fn snapshots(&self) -> Vec<Snapshot> {
        self.model.current_space().filter(|s| !s.private).and_then(|s| self.store(false).snapshots(s.space.profile_id).ok()).unwrap_or_default()
    }

    pub fn rename_snapshot(&mut self, saved: &Snapshot, name: &str) {
        self.store(false).rename_snapshot(saved.profile_id, saved.id, name).ok();
    }

    pub fn delete_snapshot(&mut self, saved: &Snapshot) {
        self.store(false).delete_snapshot(saved.profile_id, saved.id).ok();
    }

    /// Opens the snapshot's tabs in a new space of its profile, or at the end of the current space.
    pub fn restore_snapshot(&mut self, saved: &Snapshot, new_space: bool, window: &mut Window, cx: &mut Context<Self>) -> Result<(), &'static str> {
        let Some(current) = self.model.current_space().filter(|s| !s.private && s.space.profile_id == saved.profile_id).map(|s| s.space.id) else {
            return Err("Switch to a space of this snapshot's profile to restore it.");
        };
        let store = self.store(false);
        let snapshot = SpaceSnapshot::load(store, saved.profile_id, saved.id).map_err(snapshot_error)?;
        let target = if new_space {
            let names = std::iter::once(saved.name.clone()).chain((2..50).map(|n| format!("{} {n}", saved.name)));
            let name = names.filter_map(|n| crate::spaces::space_name(&n, &self.model.spaces, false, Some(saved.profile_id), None)).next();
            let space = store.create_space(saved.profile_id, name.as_deref().unwrap_or("Snapshot"), "");
            space.map_err(|_| "Tarn could not create a space for this snapshot.")?.id
        } else {
            current
        };
        let last = snapshot.restore(store, target);
        if new_space {
            self.reload_spaces();
            if let Some(ix) = self.model.spaces.iter().position(|s| !s.private && s.space.id == target) {
                self.switch_space(ix, window, cx);
            }
        } else {
            self.reload_space();
        }
        if let Some(id) = last {
            self.activate(TabKey { private: false, id }, window, cx);
        }
        Self::sync_other_windows(cx);
        cx.notify();
        Ok(())
    }

    pub(crate) fn downloads(&self) -> Vec<Download> {
        self.model.current_space().and_then(|s| self.store(s.private).downloads(s.space.profile_id).ok()).unwrap_or_default()
    }

    /// Removes one entry, or with `None` the whole list; the files stay.
    pub fn remove_download(&mut self, id: Option<Id>) {
        let Some(space) = self.model.current_space() else { return };
        let (store, profile_id) = (self.store(space.private), space.space.profile_id);
        match id {
            Some(id) => store.delete_download(profile_id, id),
            None => store.clear_downloads(profile_id),
        }
        .ok();
    }

    pub(crate) fn close_library(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.library.take() {
            cx.defer(move |cx| drop(handle.update(cx, |library, window, cx| library.close(true, window, cx))));
            cx.notify();
        }
    }

    pub fn toggle_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.library {
            Some(_) => self.close_library(cx),
            // The click that took focus from the library already closed it; it must not reopen it.
            None if self.library_closed.is_some_and(|t| t.elapsed() < ui::motion::REOPEN_GUARD) => {}
            None => self.show_library(None, window, cx),
        }
    }

    /// Opens the library above the space row; with `edit`, on that bookmark with its name field open.
    pub fn show_library(&mut self, edit: Option<Id>, window: &mut Window, cx: &mut Context<Self>) {
        let page = match edit.and_then(|id| self.bookmarks.iter().find(|b| b.id == id)) {
            Some(bm) => Page::Bookmarks(bm.parent_id),
            None => Page::Home,
        };
        self.open_library(page, edit.map(Named::Bookmark), window, cx);
    }

    /// Opens the library on `page`; with `edit`, that row's name field is open.
    pub(crate) fn open_library(&mut self, page: Page, edit: Option<Named>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(handle) = self.library {
            cx.defer(move |cx| drop(handle.update(cx, |library, window, cx| library.go(page, window, cx))));
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>().or(self.peek.main) else { return };
        let inset = SIDEBAR_MARGIN;
        self.peek.owns[0] = self.peek.is_window(window);
        if !self.peek.owns[0] {
            self.close_peek(cx);
        }
        let anchor = self.main_bounds(window);
        let room = f32::from(anchor.size.height) - 2. * inset - SPACE_ROW - TOOLBAR_HEIGHT - SEARCH_HEIGHT - 2.;
        let max_list = room.max(ROW * 3.);
        let downloads = self.downloads();
        let snapshots = self.snapshots();
        let sources = if page == Page::Import { detect_sources() } else { Vec::new() };
        let history = self.model.current_space().and_then(|s| self.store(s.private).history(s.space.profile_id, HISTORY_LOAD).ok()).unwrap_or_default();
        let rows: Vec<&HistoryEntry> = history.iter().collect();
        let listed = entries(self, page, "", &rows, &downloads, &snapshots, &sources);
        let height = px(card_height(&listed, row_boundary(&listed, max_list)));
        let (x, bottom, width) = (inset + 4., f32::from(anchor.size.height) - inset - SPACE_ROW, CARD_WIDTH.min(self.sidebar_width - 8.));
        let bounds = Bounds::new(point(anchor.origin.x + px(x), anchor.origin.y + px(bottom) - height), size(px(width), height));
        let display = window.display(cx).map(|d| d.id());
        let browser = cx.entity().downgrade();
        // The new window renders from the browser at once, so it must open after this update ends.
        cx.defer(move |cx| {
            let handle = cx.open_window(popup_options(bounds, display, true), |window, cx| {
                if let Some(host) = engine::Host::from_window(window) {
                    host.hide_on_deactivate();
                    host.round_corners(0., 0., 0., 0., f64::from(ui::theme::Theme::radius_panel()));
                }
                cx.new(|cx| LibraryWindow {
                    page,
                    fit: Some(Fit { x, bottom, width, max_list }),
                    sources: sources.clone(),
                    ..LibraryWindow::new(browser.clone(), main, history, downloads.clone(), edit, window, cx)
                })
            });
            browser
                .update(cx, |b, cx| {
                    b.library = handle.ok();
                    cx.notify();
                })
                .ok();
            crate::peek::popovers_over_peek(&browser, cx);
        });
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

fn row(item: Item, title: String, bookmark: Option<Id>) -> Entry {
    Entry::Row(Row { item, title, named: bookmark.map(Named::Bookmark), detail: None })
}

fn bookmark_row(bm: &Bookmark) -> Entry {
    let item = match &bm.url {
        Some(url) => Item::Url(url.clone()),
        None => Item::Folder(bm.id),
    };
    row(item, title_or_host(&bm.title, bm.url.as_deref().unwrap_or_default()), Some(bm.id))
}

fn entries(b: &Browser, page: Page, query: &str, history: &[&HistoryEntry], downloads: &[Download], snapshots: &[Snapshot], sources: &[Source]) -> Vec<Entry> {
    let matches = |title: &str, url: &str| library::matches(query, title, url);
    let store = b.store(b.model.private());
    let bookmarks = &b.bookmarks;
    let found = |limit| library::bookmarks(bookmarks, query).take(limit).map(bookmark_row);
    let visits = history;
    let history = |limit| visits.iter().take(limit).map(|h| row(Item::Url(h.url.clone()), title_or_host(&h.title, &h.url), None));
    let downloads = |limit| {
        let now = now();
        library::downloads(downloads, query).take(limit).map(move |d| download_row(store, d, now))
    };
    let mut out = Vec::new();
    match page {
        Page::Home => {
            let tabs = library::open_tabs(&b.model.tabs, query);
            if query.is_empty() || !tabs.is_empty() {
                out.push(Entry::Header("Open Tabs", Some(tabs.len())));
            }
            if query.is_empty() && tabs.is_empty() {
                out.push(Entry::Empty("No open tabs".into()));
            }
            out.extend(tabs.into_iter().take(MAX_ROWS).map(|t| row(Item::Tab(b.model.key(t)), title_or_host(&t.title, &t.url), None)));
            if !query.is_empty() {
                let sections: [(&str, Vec<Entry>); 3] =
                    [("Bookmarks", found(MAX_ROWS).collect()), ("History", history(MAX_ROWS).collect()), ("Downloads", downloads(MAX_ROWS).collect())];
                for (name, rows) in sections.into_iter().filter(|(_, rows)| !rows.is_empty()) {
                    out.push(Entry::Header(name, None));
                    out.extend(rows);
                }
                if out.is_empty() {
                    out.push(Entry::Empty(empty_text(page, query)));
                }
            }
            out.push(Entry::Header("Library", None));
            out.push(row(Item::Page(Page::History), "History".into(), None));
            out.push(row(Item::Page(Page::Bookmarks(None)), "Bookmarks".into(), None));
            if let Some(folder) = b.reading_list() {
                out.push(row(Item::Page(Page::Bookmarks(Some(folder))), bookmarks::READING_LIST.into(), None));
            }
            out.push(row(Item::Page(Page::Downloads), "Downloads".into(), None));
            if !b.model.private() {
                out.push(row(Item::Page(Page::Snapshots), "Snapshots".into(), None));
            }
            out.push(row(Item::Page(Page::Memory), "Memory".into(), None));
            if query.is_empty() && data::available() {
                out.push(row(Item::Page(Page::Import), "Import Browser Data".into(), None));
            }
        }
        Page::History => {
            out.push(Entry::Back("History".into()));
            let now = now();
            let starts = library::day_starts(store, now);
            let mut last = None;
            for h in visits.iter().take(MAX_LIST) {
                let day = library::history_day(h.visited_at, starts);
                if last.replace(day) != Some(day) {
                    out.push(Entry::Header(day, None));
                }
                out.push(visit_row(store, h, now));
            }
        }
        Page::Bookmarks(folder) => {
            let name = folder.and_then(|id| bookmarks.iter().find(|bm| bm.id == id)).map_or("Bookmarks".into(), |bm| bounded_title(&bm.title));
            out.push(Entry::Back(name));
            if query.is_empty() {
                out.extend(bookmarks.iter().filter(|bm| bm.parent_id == folder).map(bookmark_row));
            } else {
                out.extend(found(MAX_LIST));
            }
        }
        Page::Downloads => {
            out.push(Entry::Back("Downloads".into()));
            out.extend(downloads(MAX_LIST));
            if out.len() > 1 && query.is_empty() {
                out.push(Entry::Divider);
                out.push(row(Item::ClearDownloads, "Clear List".into(), None));
            }
        }
        Page::Snapshots => {
            out.push(Entry::Back("Snapshots".into()));
            if snapshots.is_empty() {
                out.push(Entry::Empty("Save a space with “Save Space as Snapshot”".into()));
            }
            let now = now();
            out.extend(snapshots.iter().filter(|s| matches(&s.name, "")).map(|s| {
                let detail = Some(ago(store, s.created_at, now));
                Entry::Row(Row { item: Item::Snapshot(s.clone()), title: bounded_title(&s.name), named: Some(Named::Snapshot(s.id)), detail })
            }));
        }
        Page::Memory => {
            out.push(Entry::Back("Memory".into()));
            let costs = b.tab_costs();
            let mut tabs: Vec<(TabKey, &Tab)> = b.model.tabs.iter().filter(|t| matches(&t.title, &t.url)).map(|t| (b.model.key(t), t)).collect();
            tabs.sort_by_key(|(key, t)| (Reverse(costs.get(key).map_or(0, |c| c.footprint)), Reverse(t.last_active)));
            out.extend(tabs.into_iter().map(|(key, t)| {
                let detail = crate::memory::detail(costs.get(&key).copied(), b.sleeping(key));
                Entry::Row(Row { item: Item::Tab(key), title: title_or_host(&t.title, &t.url), named: None, detail: Some(detail) })
            }));
            if out.len() > 1 && query.is_empty() {
                out.push(Entry::Divider);
                out.push(row(Item::SleepInactive, "Sleep All Inactive Tabs".into(), None));
            }
        }
        Page::Import => {
            out.push(Entry::Back("Import Browser Data".into()));
            if sources.is_empty() {
                out.push(Entry::Empty("No supported browser found".into()));
            }
            out.extend(
                sources.iter().enumerate().filter(|(_, s)| matches(s.browser.name(), &s.label)).map(|(ix, s)| {
                    Entry::Row(Row { item: Item::Source(ix), title: s.browser.name().into(), named: None, detail: Some(bounded_title(&s.label)) })
                }),
            );
        }
    }
    if out.len() == 1 {
        out.push(Entry::Empty(empty_text(page, query)));
    }
    out
}

fn tally(n: usize) -> Option<String> {
    (n > 0).then(|| thousands(n as u64))
}

/// Rows listed on a page; on a page cut at `MAX_LIST` the cap reads "200+", never as the total.
fn listed_tally(n: usize, capped: bool) -> Option<String> {
    if capped && n >= MAX_LIST { Some(format!("{MAX_LIST}+")) } else { tally(n) }
}

fn empty_text(page: Page, query: &str) -> String {
    if !query.is_empty() {
        return format!("No results for “{}”", bounded_title(query));
    }
    match page {
        Page::History => "No history yet",
        Page::Bookmarks(None) => "No bookmarks yet",
        Page::Bookmarks(Some(_)) => "This folder is empty",
        Page::Downloads => "No downloads yet",
        _ => "Nothing here",
    }
    .into()
}

fn entry_height(entry: &Entry) -> f32 {
    match entry {
        Entry::Header(..) => HEADER_ROW,
        Entry::Back(_) => BACK_ROW,
        Entry::Empty(_) => EMPTY_ROW,
        Entry::Divider => DIVIDER,
        Entry::Row(r) if r.detail.is_some() => DETAIL_ROW,
        Entry::Row(_) => ROW,
    }
}

/// The tallest list up to `max_list` that ends below a row, so a cut list never shows a heading or
/// divider without the row under it, or half a row; `max_list` when the whole list fits.
fn row_boundary(entries: &[Entry], max_list: f32) -> f32 {
    let mut end = LIST_PAD;
    let mut fit = None;
    for (ix, entry) in entries.iter().enumerate() {
        end += entry_height(entry) + if ix > 0 { 1. } else { 0. };
        if end > max_list {
            return fit.unwrap_or(max_list);
        }
        if !matches!(entry, Entry::Header(..) | Entry::Divider) {
            fit = Some(end);
        }
    }
    max_list
}

/// The card's height for `entries`, with the list capped at `max_list`.
fn card_height(entries: &[Entry], max_list: f32) -> f32 {
    let list: f32 = entries.iter().map(entry_height).sum::<f32>() + entries.len().saturating_sub(1) as f32 + LIST_PAD;
    SEARCH_HEIGHT + 2. + list.min(max_list)
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

    fn begin_rename(&mut self, named: Named, window: &mut Window, cx: &mut Context<Self>) {
        let title = match named {
            Named::Bookmark(id) => self.browser.upgrade().and_then(|b| b.read(cx).bookmarks.iter().find(|bm| bm.id == id).map(|bm| bm.title.clone())),
            Named::Snapshot(id) => self.snapshots.iter().find(|s| s.id == id).map(|s| s.name.clone()),
        };
        let Some(title) = title else { return };
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Name", cx);
            input.set_text(title, cx);
            input.select_all_from_start(cx);
            input
        });
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            TextInputEvent::Submit => this.end_rename(true, window, cx),
            TextInputEvent::Cancel => this.end_rename(false, window, cx),
            TextInputEvent::MenuClosed if !window.is_window_active() && !this.holding => this.close(false, window, cx),
            _ => {}
        });
        let focus = input.focus_handle(cx);
        let blur = cx.on_blur(&focus, window, |this, window, cx| this.end_rename(true, window, cx));
        window.focus(&focus);
        self.renaming = Some((named, input, [events, blur]));
        cx.notify();
    }

    fn end_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((named, input, _)) = self.renaming.take() else { return };
        let title = input.read(cx).text().trim().to_owned();
        if commit && !title.is_empty() {
            match named {
                Named::Bookmark(id) => drop(self.browser.update(cx, |b, cx| b.rename_bookmark(id, &title, cx))),
                Named::Snapshot(id) => {
                    if let Some(saved) = self.snapshots.iter().find(|s| s.id == id).cloned() {
                        self.browser.update(cx, |b, _| b.rename_snapshot(&saved, &title)).ok();
                        self.reload_snapshots(cx);
                    }
                }
            }
        }
        window.focus(&self.query.focus_handle(cx));
        cx.notify();
    }

    fn new_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = match self.page {
            Page::Bookmarks(folder) => folder,
            _ => None,
        };
        let Some(id) = self.browser.update(cx, |b, cx| b.new_bookmark_folder(parent, cx)).ok().flatten() else { return };
        self.go(Page::Bookmarks(parent), window, cx);
        self.select(Named::Bookmark(id), cx);
        self.begin_rename(Named::Bookmark(id), window, cx);
    }

    fn delete(&mut self, named: Named, window: &mut Window, cx: &mut Context<Self>) {
        let id = match named {
            Named::Bookmark(id) => id,
            Named::Snapshot(id) => {
                let Some(saved) = self.snapshots.iter().find(|s| s.id == id).cloned() else { return };
                let message = format!("Delete the snapshot “{}”?", bounded_title(&saved.name));
                return self.confirm(&message, None, "Delete", window, cx, move |this, cx| {
                    this.browser.update(cx, |b, _| b.delete_snapshot(&saved)).ok();
                    this.reload_snapshots(cx);
                });
            }
        };
        let Some(browser) = self.browser.upgrade() else { return };
        let b = browser.read(cx);
        let Some(bm) = b.bookmarks.iter().find(|bm| bm.id == id) else { return };
        let message = if bookmarks::subtree(&b.bookmarks, id).len() == 1 {
            format!("Delete the bookmark “{}”?", bounded_title(&bm.title))
        } else {
            format!("Delete “{}” and everything in it?", bounded_title(&bm.title))
        };
        self.confirm(&message, None, "Delete", window, cx, move |this, cx| {
            this.browser.update(cx, |b, cx| b.delete_bookmark(id, cx)).ok();
        });
    }

    fn remove_history(&mut self, url: String, cx: &mut Context<Self>) {
        self.browser.update(cx, |b, cx| b.remove_history_url(&url, cx)).ok();
        self.history.retain(|h| h.url != url);
        *self.history_hits.borrow_mut() = None;
        self.shown = self.rows(cx);
        cx.notify();
    }

    /// Delete, F2 and Shift+F10 act on the selected row; the delete keys only while the search
    /// field is empty, so they still edit its text.
    fn on_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = &event.keystroke;
        let Some((_, r)) = self.shown.get(self.selected).cloned() else { return };
        if self.renaming.is_some() || self.holding || event.is_held {
            return;
        }
        match key.key.as_str() {
            "f2" if !key.modifiers.modified() => match r.named {
                Some(named) => self.begin_rename(named, window, cx),
                None => return,
            },
            "f10" if key.modifiers.shift => self.row_menu(r.item.clone(), r.named, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Deletes the selected row for Delete or ⌘⌫ while the search field is empty, before the
    /// field, which holds the focus, takes the key.
    fn delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, r)) = self.shown.get(self.selected).cloned() else { return cx.propagate() };
        if self.renaming.is_some() || self.holding || !self.query.read(cx).text().is_empty() {
            return cx.propagate();
        }
        match (r.named, visit(&r)) {
            (Some(named), _) => self.delete(named, window, cx),
            (None, Some(url)) => self.remove_history(url.to_owned(), cx),
            _ => return cx.propagate(),
        }
        cx.stop_propagation();
    }

    fn confirm(
        &mut self,
        message: &str,
        detail: Option<&str>,
        action: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Context<Self>) + 'static,
    ) {
        let answer = self.ask(PromptLevel::Warning, message, detail, &[action, "Cancel"], window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let yes = answer.await == Ok(0);
            this.update_in(cx, |this, window, cx| {
                this.answered(window, cx);
                if yes {
                    then(this, cx);
                }
                window.focus(&this.query.focus_handle(cx));
            })
            .ok();
        })
        .detach();
    }

    /// A dialog draws inside the popover, so the popover takes its full height while one is up.
    fn ask(
        &mut self,
        level: PromptLevel,
        message: &str,
        detail: Option<&str>,
        answers: &[&str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> oneshot::Receiver<usize> {
        self.holding = true;
        if let Some(fit) = &self.fit {
            self.place(SEARCH_HEIGHT + 2. + fit.max_list, window, cx);
        }
        crate::dialog::ask(level, message, detail, answers, window, cx)
    }

    fn answered(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.holding = false;
        self.refit(window, cx);
    }

    fn alert(&mut self, message: &str, detail: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let answer = self.ask(PromptLevel::Info, message, detail, &["OK"], window, cx);
        cx.spawn_in(window, async move |this, cx| {
            answer.await.ok();
            this.update_in(cx, |this, window, cx| {
                this.answered(window, cx);
                window.focus(&this.query.focus_handle(cx));
            })
            .ok();
        })
        .detach();
    }

    /// Asks which data to bring from a source into the current space's profile, then reads it in
    /// the background (where macOS may show its keychain prompt) and writes it.
    fn start_import(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(source), Some(browser)) = (self.sources.get(ix).cloned(), self.browser.upgrade()) else { return };
        let b = browser.read(cx);
        let Some(space) = b.model.current_space() else { return };
        let (private, profile_id, core) = (space.private, space.space.profile_id, b.core.clone());
        if core.importing.get() {
            return self.alert("An import is already running.", None, window, cx);
        }
        if core.import_target(private, profile_id).is_err() {
            let detail = "Private and Lockdown profiles keep website data in memory only, so imported sign-ins would not last.";
            return self.alert("Switch to a space of a normal profile to import into it.", Some(detail), window, cx);
        }
        let name = source.browser.name();
        let profile = core.disk.profile(profile_id).map(|p| p.name).unwrap_or_default();
        let data::Confirm { message, detail, cookies } = data::confirm(&source, &profile, ui::controls::THIS_DEVICE);
        let sealed = !cookies;
        let buttons: &[&str] = if sealed { &["Import", "Cancel"] } else { &["Import All", "History and Bookmarks Only", "Cancel"] };
        let answer = self.ask(PromptLevel::Info, &message, Some(&detail), buttons, window, cx);
        let (weak_browser, main) = (self.browser.clone(), self.main);
        cx.spawn_in(window, async move |this, cx| {
            let cookies = match answer.await {
                Ok(0) => !sealed,
                Ok(1) if !sealed => false,
                _ => return drop(this.update_in(cx, |this, window, cx| this.answered(window, cx))),
            };
            if core.importing.replace(true) {
                this.update_in(cx, |this, window, cx| {
                    this.answered(window, cx);
                    this.alert("An import is already running.", None, window, cx);
                })
                .ok();
                return;
            }
            let kinds = Kinds { cookies, history: true, bookmarks: true };
            let browser_kind = source.browser;
            let read = cx.background_executor().spawn(async move { data::read(&source, kinds) }).await;
            let (tx, rx) = oneshot::channel();
            let summary = match core.import(profile_id, browser_kind, read, move |r| drop(tx.send(r))) {
                // The engine answers each cookie write; a store closed meanwhile may never answer.
                Ok(()) => match futures::future::select(rx, cx.background_executor().timer(IMPORT_TIMEOUT)).await {
                    futures::future::Either::Left((report, _)) => data::summary(name, report.ok().as_ref()),
                    futures::future::Either::Right(_) => data::summary(name, None),
                },
                Err(_) => "Nothing was imported: this profile can no longer take imported data.".into(),
            };
            core.importing.set(false);
            let same_profile = weak_browser
                .update(cx, |b, cx| {
                    b.reload_bookmarks(cx);
                    b.model.current_space().is_some_and(|s| !s.private && s.space.profile_id == profile_id)
                })
                .unwrap_or(false);
            this.update_in(cx, |this, window, cx| {
                this.answered(window, cx);
                if same_profile {
                    this.history = core.disk.history(profile_id, HISTORY_LOAD).unwrap_or_default();
                    this.history_hits.take();
                }
                if !window.is_window_active() {
                    this.close(false, window, cx);
                }
            })
            .ok();
            main.update(cx, |_, window, cx| drop(window.prompt(PromptLevel::Info, &format!("Import from {name}"), Some(&summary), &["OK"], cx))).ok();
        })
        .detach();
    }

    fn run(&mut self, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            Cmd::Open(item) => self.open(item, window, cx),
            Cmd::OpenInNewTab(url) => {
                if let Ok(url) = url::Url::parse(&url) {
                    self.main
                        .update(cx, |b, window, cx| {
                            let before = b.model.active;
                            b.new_tab(&url, window, cx);
                            if b.model.active != before && b.is_bookmarked(url.as_str()) {
                                b.mark_typed(&url);
                            }
                        })
                        .ok();
                }
                self.close(true, window, cx);
            }
            Cmd::Rename(named) => self.begin_rename(named, window, cx),
            Cmd::Move(id, parent) => {
                self.browser.update(cx, |b, cx| b.move_bookmark(id, parent, cx)).ok();
            }
            Cmd::Delete(named) => self.delete(named, window, cx),
            Cmd::Restore(saved, new_space) => self.restore(saved, new_space, window, cx),
            Cmd::NewFolder => self.new_folder(window, cx),
            Cmd::ToggleBookmark(key) => {
                self.browser
                    .update(cx, |b, cx| match b.tab_url(key).is_some_and(|url| b.is_bookmarked(&url)) {
                        true => b.unbookmark_tab(Some(key), cx),
                        false => drop(b.bookmark_tab(Some(key), cx)),
                    })
                    .ok();
            }
            Cmd::CopyLink(url) => cx.write_to_clipboard(ClipboardItem::new_string(url)),
            Cmd::RemoveDownload(id) => self.remove_download(Some(id), cx),
            Cmd::RemoveHistory(url) => self.remove_history(url, cx),
        }
    }

    fn row_menu(&mut self, item: Item, named: Option<Named>, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(browser) = self.browser.upgrade() else { return };
        let b = browser.read(cx);
        let Some(host) = b.host.clone() else { return };
        let url = match &item {
            Item::Url(url) => Some(url.clone()),
            Item::Tab(key) => b.tab_url(*key),
            _ => None,
        };
        let item_key = item.clone();
        let history = match &item {
            Item::Url(url) if named.is_none() => Some(url.clone()),
            _ => None,
        };
        let mut items = match &item {
            Item::Download(d) => {
                let mut items = vec![MenuItem::action(ui::controls::REVEAL, Cmd::Open(item.clone()))];
                items.extend((!d.url.is_empty()).then(|| MenuItem::action("Copy Download Link", Cmd::CopyLink(d.url.clone()))));
                items.extend([MenuItem::Separator, MenuItem::action("Remove from List", Cmd::RemoveDownload(d.id))]);
                items
            }
            Item::Snapshot(saved) => vec![
                MenuItem::action("Restore in New Space", Cmd::Restore(saved.clone(), true)),
                MenuItem::action("Restore in This Space", Cmd::Restore(saved.clone(), false)),
                MenuItem::Separator,
                MenuItem::action("Rename", Cmd::Rename(Named::Snapshot(saved.id))),
                MenuItem::action("Delete…", Cmd::Delete(Named::Snapshot(saved.id))),
            ],
            _ => vec![MenuItem::action("Open", Cmd::Open(item))],
        };
        let bookmarked = url.as_deref().is_some_and(|url| b.is_bookmarked(url));
        if let Some(url) = url {
            items.extend([MenuItem::action("Open in New Tab", Cmd::OpenInNewTab(url.clone())), MenuItem::action("Copy Link", Cmd::CopyLink(url))]);
        }
        if let Item::Tab(key) = item_key {
            items.push(MenuItem::action(if bookmarked { "Remove Bookmark" } else { "Bookmark Tab" }, Cmd::ToggleBookmark(key)));
        }
        if let Some(url) = history {
            items.extend([MenuItem::Separator, MenuItem::action("Remove from History", Cmd::RemoveHistory(url))]);
        }
        if let Some(Named::Bookmark(id)) = named {
            let parent = b.bookmarks.iter().find(|bm| bm.id == id).and_then(|bm| bm.parent_id);
            let mut folders = vec![MenuItem::action("Bookmarks", Cmd::Move(id, None)).checked(parent.is_none()).enabled(parent.is_some())];
            folders.extend(
                bookmarks::move_targets(&b.bookmarks, id)
                    .into_iter()
                    .map(|f| MenuItem::action(bounded_title(&f.title), Cmd::Move(id, Some(f.id))).checked(parent == Some(f.id)).enabled(parent != Some(f.id))),
            );
            items.extend([
                MenuItem::Separator,
                MenuItem::action("Rename", Cmd::Rename(Named::Bookmark(id))),
                MenuItem::Submenu("Move to Folder".into(), folders),
                MenuItem::action("New Folder", Cmd::NewFolder),
                MenuItem::Separator,
                MenuItem::action("Delete", Cmd::Delete(Named::Bookmark(id))),
            ]);
        }
        self.holding = true;
        cx.spawn_in(window, async move |this, cx| {
            let (tx, rx) = oneshot::channel();
            menu::show(&host, crate::menu_glyphs::with_glyphs(items), None, move |cmd| {
                tx.send(cmd).ok();
            });
            let cmd = rx.await.ok();
            this.update_in(cx, |this, window, cx| {
                this.holding = false;
                match cmd {
                    Some(cmd) => this.run(cmd, window, cx),
                    None if !window.is_window_active() => this.close(false, window, cx),
                    None => {}
                }
            })
            .ok();
        })
        .detach();
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
        let (aside, detail) = match (&r.detail, aside) {
            (Some(detail), Some(host)) if matches!(r.item, Item::Url(_)) => (None, Some(format!("{detail} · {host}"))),
            (detail, aside) => (aside, detail.clone()),
        };
        let title: AnyElement = match (editing, detail) {
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
        };
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
                    Entry::Row(r) => Some(match &r.item {
                        Item::Tab(key) => {
                            b.model.tab(*key).map_or_else(|| icon(Icon::Search, 16., Theme::muted()).into_any_element(), |t| site_icon(b, &t.url, 16.))
                        }
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
                    }),
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
                Entry::Back(title) => div()
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
                    .children(
                        listed_tally(listed, capped)
                            .map(|n| div().flex_none().px_1().text_size(px(TEXT_SMALL)).font_weight(FontWeight::NORMAL).text_color(Theme::muted()).child(n)),
                    )
                    .when(on_bookmarks, |d| {
                        d.child(icon_button("library-new-folder", Icon::FolderPlus, true).tooltip(tooltip("New Folder", None)).on_click(cx.listener(
                            |this, _, window, cx| {
                                cx.stop_propagation();
                                this.new_folder(window, cx);
                            },
                        )))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.back(window, cx)))
                    .into_any_element(),
                Entry::Divider => div().mx_2().my_1p5().h(px(1.)).flex_none().bg(Theme::hairline()).into_any_element(),
                Entry::Empty(text) => empty(text).h(px(EMPTY_ROW)).px_2().into_any_element(),
                Entry::Row(r) => {
                    ix += 1;
                    self.render_row(ix - 1, r, leading.unwrap_or_else(|| div().into_any_element()), cx)
                }
            });
        }
        let empty_query = self.query.read(cx).text().is_empty();
        let search = div()
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
            .when(empty_query, |d| d.child(key_hint(crate::shortcuts::shown(&crate::ToggleLibrary, cx))));
        let list = match (self.turn, self.backward) {
            (0, _) => list.into_any_element(),
            (turn, backward) => {
                let shift = if backward { -PAGE_SHIFT } else { PAGE_SHIFT };
                animate(list, ("library-page", turn), POP, reduce, move |el, t| el.opacity(t).relative().left(px((1. - t) * shift)))
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
