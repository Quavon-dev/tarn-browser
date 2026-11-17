use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

use browser_core::import::{self as data, Error as ImportError, Kinds, Outcome, Report, Source};
use browser_core::snapshot::{self, LoadError, SpaceSnapshot};
use engine::menu::{self, Item as MenuItem};
use futures::channel::oneshot;
use gpui::{
    AnyElement, App, AppContext, Bounds, ClipboardItem, Context, Entity, Focusable, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    PromptLevel, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, WindowHandle, actions, div, point,
    prelude::FluentBuilder, px, size,
};
use store::{Bookmark, Download, HistoryEntry, Id, Snapshot, Tab};
use ui::command::fuzzy_score;
use ui::controls::{button, caption, empty, icon_button, key_hint, surface};
use ui::icon::{ICON_SMALL, Icon, icon};
use ui::motion::{EXIT, POP, animate};
use ui::text::bounded_title;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{TEXT, TEXT_SMALL, TOOLBAR_HEIGHT, Theme};

use crate::ToggleLibrary;
use crate::browser::{Browser, TabKey, now, tab_host};
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
const MAX_ROWS: usize = 8;
const MAX_LIST: usize = 200;
const READING_LIST: &str = "Reading List";
const IMPORT_TIMEOUT: Duration = Duration::from_secs(120);
const MEMORY_REFRESH: Duration = Duration::from_secs(2);

actions!(library, [SaveSnapshot, RestoreSnapshot]);

pub(crate) use browser_core::shield_card::date;

/// "5 min ago", "3 h ago", "2 d ago", then the date.
fn ago(ms: i64, now: i64) -> String {
    match (now - ms).max(0) / 60_000 {
        0 => "Just now".into(),
        min @ 1..60 => format!("{min} min ago"),
        min @ 60..1440 => format!("{} h ago", min / 60),
        min @ 1440..10_080 => format!("{} d ago", min / 1440),
        _ => date(ms),
    }
}

fn file_name(d: &Download) -> String {
    bounded_title(&std::path::Path::new(&d.path).file_name().map(|n| n.to_string_lossy()).unwrap_or_default())
}

fn download_row(d: &Download, now: i64) -> Entry {
    let state = match d.state.as_str() {
        "failed" => "Failed",
        s if s.starts_with("finished") => "Finished",
        _ => "In Progress",
    };
    let detail = tab_host(&d.url).into_iter().chain([state.to_owned(), ago(d.started_at, now)]).collect::<Vec<_>>().join(" · ");
    Entry::Row(Row { item: Item::Download(d.clone()), title: file_name(d), named: None, detail: Some(detail) })
}

fn title_or_host(title: &str, url: &str) -> String {
    let title = bounded_title(title);
    if title.is_empty() { tab_host(url).unwrap_or_else(|| bounded_title(url)) } else { title }
}

fn detect_sources() -> Vec<Source> {
    std::env::home_dir().map(|home| data::sources(&home)).unwrap_or_default()
}

fn import_line(kind: &str, browser: &str, result: &Option<Result<Outcome, ImportError>>) -> Option<String> {
    Some(match result.as_ref()? {
        Ok(o) if o.skipped + o.failed == 0 => format!("{kind}: {} imported.", thousands(o.imported as u64)),
        Ok(o) => format!("{kind}: {} imported, {} skipped.", thousands(o.imported as u64), thousands((o.skipped + o.failed) as u64)),
        Err(ImportError::KeychainDenied) => format!("{kind}: macOS did not give Tarn {browser}’s Safe Storage key, so none were imported."),
        Err(ImportError::DiskAccess) if cfg!(windows) => format!("{kind}: Windows did not let Tarn read {browser}’s files. Quit {browser} and try again."),
        Err(ImportError::DiskAccess) => format!(
            "{kind}: macOS did not let Tarn read {browser}’s files. Allow Tarn in System Settings › Privacy & Security › Full Disk Access, then try again."
        ),
        Err(ImportError::NotFound) => format!("{kind}: none found in this profile."),
        Err(e) => format!("{kind}: could not be imported ({e})."),
    })
}

/// The result alert's text, one line per kind that was asked for.
fn import_summary(browser: &str, report: Option<&Report>) -> String {
    let Some(r) = report else { return "The import did not finish. History and bookmarks may have been added; cookies may be incomplete.".into() };
    [("Cookies", &r.cookies), ("History", &r.history), ("Bookmarks", &r.bookmarks)]
        .into_iter()
        .filter_map(|(kind, result)| import_line(kind, browser, result))
        .collect::<Vec<_>>()
        .join("\n")
}

fn snapshot_error(e: LoadError) -> &'static str {
    match e {
        LoadError::Missing => "This snapshot no longer exists.",
        LoadError::Unreadable => "This snapshot could not be read.",
        LoadError::NewerVersion => "This snapshot was saved by a newer version of Tarn.",
    }
}

fn find_bookmark<'a>(bookmarks: &'a [Bookmark], url: &str) -> Option<&'a Bookmark> {
    bookmarks.iter().find(|b| b.url.as_deref() == Some(url))
}

/// `id` and everything nested in it.
fn subtree(bookmarks: &[Bookmark], id: Id) -> HashSet<Id> {
    let mut inside = HashSet::from([id]);
    loop {
        let more: Vec<Id> = bookmarks.iter().filter(|b| !inside.contains(&b.id) && b.parent_id.is_some_and(|p| inside.contains(&p))).map(|b| b.id).collect();
        if more.is_empty() {
            return inside;
        }
        inside.extend(more);
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
        find_bookmark(&self.bookmarks, url).is_some()
    }

    fn bookmark_target(&self, key: Option<TabKey>) -> Option<(String, String)> {
        let tab = key.or(self.model.active).and_then(|k| self.model.tab(k))?;
        let url = self.tab_url(self.model.key(tab)).unwrap_or_else(|| tab.url.clone());
        (!url.is_empty()).then(|| (title_or_host(&tab.title, &url), url))
    }

    /// The top-level folder Save for Later files pages into, if it exists.
    pub(crate) fn reading_list(&self) -> Option<Id> {
        self.bookmarks.iter().find(|b| b.url.is_none() && b.parent_id.is_none() && b.title == READING_LIST).map(|b| b.id)
    }

    /// Files the active page into the Reading List bookmark folder, made on first use, so it is
    /// kept, exported and deleted like any bookmark.
    pub fn save_for_later(&mut self, cx: &mut Context<Self>) {
        let (Some((title, url)), Some(space)) = (self.bookmark_target(None), self.model.current_space()) else { return };
        let (private, profile_id) = (space.private, space.space.profile_id);
        self.reload_bookmarks(cx);
        let folder = match self.reading_list() {
            Some(id) => id,
            None => match self.store(private).add_bookmark(profile_id, None, None, READING_LIST, now()) {
                Ok(folder) => folder.id,
                Err(_) => {
                    let (tone, text) = browser_core::toast::reading_list(false);
                    return self.show_toast_as(tone, text, None, cx);
                }
            },
        };
        let saved = self.bookmarks.iter().any(|b| b.parent_id == Some(folder) && b.url.as_deref() == Some(url.as_str()))
            || self.store(private).add_bookmark(profile_id, Some(folder), Some(&url), &title, now()).is_ok();
        self.reload_bookmarks(cx);
        let (tone, text) = browser_core::toast::reading_list(saved);
        self.show_toast_as(tone, text, None, cx);
    }

    /// Bookmarks the tab, or returns its bookmark when it already has one.
    pub fn bookmark_tab(&mut self, key: Option<TabKey>, cx: &mut Context<Self>) -> Option<Id> {
        let (title, url) = self.bookmark_target(key)?;
        if let Some(existing) = find_bookmark(&self.bookmarks, &url) {
            return Some(existing.id);
        }
        let space = self.model.current_space()?;
        let id = self.store(space.private).add_bookmark(space.space.profile_id, None, Some(&url), &title, now()).ok()?.id;
        self.reload_bookmarks(cx);
        Some(id)
    }

    pub fn unbookmark_tab(&mut self, key: Option<TabKey>, cx: &mut Context<Self>) {
        let Some((_, url)) = self.bookmark_target(key) else { return };
        let store = self.store(self.model.private());
        for bm in self.bookmarks.iter().filter(|b| b.url.as_deref() == Some(url.as_str())) {
            store.delete_bookmark(bm.id).ok();
        }
        self.reload_bookmarks(cx);
    }

    /// Saves every tab into a new dated folder and opens the library on its name field.
    pub fn bookmark_all_tabs(&mut self, cx: &mut Context<Self>) {
        let Some(space) = self.model.current_space().filter(|_| !self.model.tabs.is_empty()) else { return };
        let (store, profile_id) = (self.store(space.private), space.space.profile_id);
        let Ok(folder) = store.add_bookmark(profile_id, None, None, &format!("Tabs {}", date(now())), now()) else { return };
        for tab in self.model.tabs.iter().filter(|t| !t.url.is_empty()) {
            let url = self.tab_url(self.model.key(tab)).unwrap_or_else(|| tab.url.clone());
            store.add_bookmark(profile_id, Some(folder.id), Some(&url), &title_or_host(&tab.title, &url), now()).ok();
        }
        self.reload_bookmarks(cx);
        if let Some(main) = self.peek.main {
            cx.defer(move |cx| {
                main.update(cx, |b, window, cx| b.show_library(Some(folder.id), window, cx)).ok();
            });
        }
    }

    pub fn new_bookmark_folder(&mut self, parent: Option<Id>, cx: &mut Context<Self>) -> Option<Id> {
        let space = self.model.current_space()?;
        let id = self.store(space.private).add_bookmark(space.space.profile_id, parent, None, "New Folder", now()).ok()?.id;
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

    pub fn toggle_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.library.take() {
            Some(handle) => {
                cx.defer(move |cx| {
                    handle.update(cx, |library, window, cx| library.close(true, window, cx)).ok();
                });
                cx.notify();
            }
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
            cx.defer(move |cx| drop(handle.update(cx, |library, _, cx| library.go(page, cx))));
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>().or(self.peek.main) else { return };
        let inset = SIDEBAR_MARGIN;
        let anchor = window.bounds();
        let room = f32::from(anchor.size.height) - 2. * inset - SPACE_ROW - TOOLBAR_HEIGHT - SEARCH_HEIGHT - 2.;
        let downloads = self.downloads();
        let snapshots = self.snapshots();
        // GPUI cannot move a window and resizes it from the top, so the popover keeps the height of
        // the page it opened on; other pages scroll inside it.
        let sources = if page == Page::Import { detect_sources() } else { Vec::new() };
        let history = self.model.current_space().and_then(|s| self.store(s.private).history(s.space.profile_id, 2000).ok()).unwrap_or_default();
        let rows: Vec<&HistoryEntry> = history.iter().collect();
        let height = px(card_height(&entries(self, page, "", &rows, &downloads, &snapshots, &sources), room.max(ROW * 3.)));
        let bounds = Bounds::new(
            point(anchor.origin.x + px(inset + 4.), anchor.origin.y + anchor.size.height - px(inset + SPACE_ROW) - height),
            size(px(CARD_WIDTH), height),
        );
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
    let matches = |title: &str, url: &str| query.is_empty() || fuzzy_score(query, &format!("{title} {url}")).is_some();
    let bookmarks = &b.bookmarks;
    let found = |limit| bookmarks.iter().filter(|bm| bm.url.as_deref().is_some_and(|u| matches(&bm.title, u))).take(limit).map(bookmark_row);
    let history = |limit| history.iter().take(limit).map(|h| row(Item::Url(h.url.clone()), title_or_host(&h.title, &h.url), None));
    let downloads = |limit| {
        let now = now();
        downloads.iter().filter(move |d| query.is_empty() || fuzzy_score(query, &file_name(d)).is_some()).take(limit).map(move |d| download_row(d, now))
    };
    let mut out = Vec::new();
    match page {
        Page::Home => {
            let mut tabs: Vec<&Tab> = b.model.tabs.iter().filter(|t| matches(&t.title, &t.url)).collect();
            tabs.sort_by_key(|t| Reverse(t.last_active));
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
                out.push(row(Item::Page(Page::Bookmarks(Some(folder))), READING_LIST.into(), None));
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
            out.extend(history(MAX_LIST));
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
                let detail = Some(ago(s.created_at, now));
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
    history: Vec<HistoryEntry>,
    /// The query and the indices of the history entries it matches, kept until either changes.
    history_hits: RefCell<Option<(String, Rc<[usize]>)>>,
    /// The selectable rows the last render showed, with their child index in the list.
    shown: Vec<(usize, Row)>,
    downloads: Vec<Download>,
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
    _subscriptions: Vec<Subscription>,
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
        let mut seen = HashSet::new();
        let history = history.into_iter().filter(|h| seen.insert(h.url.clone())).collect();
        let mut library = LibraryWindow {
            browser,
            main,
            query,
            page: Page::Home,
            selected: 0,
            history,
            history_hits: RefCell::new(None),
            shown: Vec::new(),
            downloads,
            downloads_seen: None,
            snapshots: Vec::new(),
            sources: Vec::new(),
            renaming: None,
            scroll: ScrollHandle::new(),
            turn: 0,
            backward: false,
            closing: false,
            holding: false,
            _subscriptions: subscriptions,
        };
        library.reload_snapshots(cx);
        let found = match edit {
            Some(Named::Bookmark(id)) => library
                .browser
                .upgrade()
                .and_then(|b| b.read(cx).bookmarks.iter().find(|b| b.id == id).map(|bm| (Page::Bookmarks(bm.parent_id), bm.title.clone()))),
            Some(Named::Snapshot(id)) => library.snapshots.iter().find(|s| s.id == id).map(|s| (Page::Snapshots, s.name.clone())),
            None => None,
        };
        if let (Some((page, title)), Some(named)) = (found, edit) {
            library.page = page;
            library.select(named, cx);
            library.begin_rename(named, title, window, cx);
        }
        library
    }

    fn entries(&self, b: &Browser, cx: &App) -> Vec<Entry> {
        let query = self.query.read(cx).text().trim();
        let lists_history = self.page == Page::History || self.page == Page::Home && !query.is_empty();
        let hits = if lists_history { self.history_hits(query) } else { Rc::from([]) };
        let history: Vec<&HistoryEntry> = hits.iter().take(MAX_LIST).map(|&ix| &self.history[ix]).collect();
        entries(b, self.page, query, &history, &self.downloads, &self.snapshots, &self.sources)
    }

    fn history_hits(&self, query: &str) -> Rc<[usize]> {
        let mut memo = self.history_hits.borrow_mut();
        if let Some((_, hits)) = memo.as_ref().filter(|(q, _)| q == query) {
            return hits.clone();
        }
        let matches = |h: &HistoryEntry| query.is_empty() || fuzzy_score(query, &format!("{} {}", h.title, h.url)).is_some();
        let hits: Rc<[usize]> = self.history.iter().enumerate().filter(|(_, h)| matches(h)).map(|(ix, _)| ix).collect();
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

    fn go(&mut self, page: Page, cx: &mut Context<Self>) {
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
        cx.notify();
    }

    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = |id| self.browser.upgrade().and_then(|b| b.read(cx).bookmarks.iter().find(|bm| bm.id == id).and_then(|bm| bm.parent_id));
        match self.page {
            Page::Home => return self.close(true, window, cx),
            Page::History | Page::Bookmarks(None) | Page::Downloads | Page::Snapshots | Page::Import | Page::Memory => self.go(Page::Home, cx),
            Page::Bookmarks(Some(id)) => self.go(Page::Bookmarks(parent(id)), cx),
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
            Item::Folder(id) => self.go(Page::Bookmarks(Some(id)), cx),
            Item::Page(page) => self.go(page, cx),
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
        cx.notify();
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

    fn begin_rename(&mut self, named: Named, title: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Name", cx);
            input.set_text(title, cx);
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
        self.go(Page::Bookmarks(parent), cx);
        self.select(Named::Bookmark(id), cx);
        self.begin_rename(Named::Bookmark(id), "New Folder".into(), window, cx);
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
        if subtree(&b.bookmarks, id).len() == 1 {
            browser.update(cx, |b, cx| b.delete_bookmark(id, cx));
            return;
        }
        let message = format!("Delete “{}” and everything in it?", bounded_title(&bm.title));
        self.confirm(&message, None, "Delete", window, cx, move |this, cx| {
            this.browser.update(cx, |b, cx| b.delete_bookmark(id, cx)).ok();
        });
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
        let answer = window.prompt(PromptLevel::Warning, message, detail, &[action, "Cancel"], cx);
        self.holding = true;
        cx.spawn_in(window, async move |this, cx| {
            let yes = answer.await == Ok(0);
            this.update_in(cx, |this, window, cx| {
                this.holding = false;
                if yes {
                    then(this, cx);
                }
                window.focus(&this.query.focus_handle(cx));
            })
            .ok();
        })
        .detach();
    }

    fn alert(&mut self, message: &str, detail: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(PromptLevel::Info, message, detail, &["OK"], cx);
        self.holding = true;
        cx.spawn_in(window, async move |this, cx| {
            answer.await.ok();
            this.update_in(cx, |this, window, cx| {
                this.holding = false;
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
        let message = format!("Import from {name} ({}) into “{}”?", bounded_title(&source.label), bounded_title(&profile));
        let sealed = cfg!(windows) && source.browser != data::Browser::Firefox;
        let keychain = match source.browser {
            _ if sealed => format!(" Tarn cannot read {name}’s cookies on Windows, so only history and bookmarks are copied."),
            data::Browser::Firefox => String::new(),
            _ => format!(" To read the cookies, macOS asks once whether Tarn may use {name}’s Safe Storage key."),
        };
        let detail = format!(
            "Cookies keep you signed in to sites; history and bookmarks join the library. Everything is copied on {} and sent nowhere.{keychain}",
            ui::controls::THIS_DEVICE
        );
        let buttons: &[&str] = if sealed { &["Import", "Cancel"] } else { &["Import All", "History and Bookmarks Only", "Cancel"] };
        let answer = window.prompt(PromptLevel::Info, &message, Some(&detail), buttons, cx);
        self.holding = true;
        let (weak_browser, main) = (self.browser.clone(), self.main);
        cx.spawn_in(window, async move |this, cx| {
            let cookies = match answer.await {
                Ok(0) => !sealed,
                Ok(1) if !sealed => false,
                _ => return drop(this.update(cx, |this, _| this.holding = false)),
            };
            if core.importing.replace(true) {
                this.update_in(cx, |this, window, cx| {
                    this.holding = false;
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
                    futures::future::Either::Left((report, _)) => import_summary(name, report.ok().as_ref()),
                    futures::future::Either::Right(_) => import_summary(name, None),
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
                this.holding = false;
                if same_profile {
                    this.history = core.disk.history(profile_id, 2000).unwrap_or_default();
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
            Cmd::Rename(named) => {
                let title = match named {
                    Named::Bookmark(id) => self.browser.upgrade().and_then(|b| b.read(cx).bookmarks.iter().find(|bm| bm.id == id).map(|bm| bm.title.clone())),
                    Named::Snapshot(id) => self.snapshots.iter().find(|s| s.id == id).map(|s| s.name.clone()),
                };
                if let Some(title) = title {
                    self.begin_rename(named, title, window, cx);
                }
            }
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
        if let Some(Named::Bookmark(id)) = named {
            let parent = b.bookmarks.iter().find(|bm| bm.id == id).and_then(|bm| bm.parent_id);
            let inside = subtree(&b.bookmarks, id);
            let mut folders = vec![MenuItem::action("Bookmarks", Cmd::Move(id, None)).checked(parent.is_none()).enabled(parent.is_some())];
            folders.extend(
                b.bookmarks
                    .iter()
                    .filter(|f| f.url.is_none() && !inside.contains(&f.id))
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
        let title: AnyElement = match (editing, &r.detail) {
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
                .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(SharedString::from(r.title.clone())))
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(TEXT_SMALL))
                        .text_color(Theme::muted())
                        .child(SharedString::from(detail.clone())),
                )
                .into_any_element(),
            (None, None) => div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(SharedString::from(r.title.clone())).into_any_element(),
        };
        let item = r.item.clone();
        let menu_item = r.item.clone();
        let named = r.named;
        let browser = self.browser.upgrade();
        let b = browser.as_ref().map(|b| b.read(cx));
        let sleepable = match r.item {
            Item::Tab(key) if self.page == Page::Memory => {
                b.is_some_and(|b| b.model.active != Some(key) && !b.sleeping(key) && !b.in_split(key)).then_some(key)
            }
            _ => None,
        };
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
        div()
            .id(("library-row", ix))
            .group("library-row")
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
            .child(leading)
            .child(title)
            .when_some(aside, |d, aside| {
                d.child(div().flex_none().whitespace_nowrap().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child(SharedString::from(aside)))
            })
            .when_some(r.named, |d, named| {
                let title = r.title.clone();
                let id = match named {
                    Named::Bookmark(id) | Named::Snapshot(id) => id as u64,
                };
                d.child(
                    div()
                        .invisible()
                        .group_hover("library-row", |s| s.visible())
                        .child(icon_button(("rename", id), Icon::Pencil, true).on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.begin_rename(named, title.clone(), window, cx);
                        })))
                        .child(icon_button(("delete", id), Icon::Trash, true).on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.delete(named, window, cx);
                        }))),
                )
            })
            .when_some(sleepable, |d, key| {
                d.child(button(("library-sleep", ix), "Sleep", false).invisible().group_hover("library-row", |s| s.visible()).on_click(cx.listener(
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.sleep(vec![key], cx);
                    },
                )))
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
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().child(SharedString::from(title)))
                    .children(
                        listed_tally(listed, capped)
                            .map(|n| div().flex_none().px_1().text_size(px(TEXT_SMALL)).font_weight(FontWeight::NORMAL).text_color(Theme::muted()).child(n)),
                    )
                    .when(on_bookmarks, |d| {
                        d.child(icon_button("library-new-folder", Icon::FolderPlus, true).on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.new_folder(window, cx);
                        })))
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.back(window, cx)))
                    .into_any_element(),
                Entry::Divider => div().mx_2().my_1p5().h(px(1.)).flex_none().bg(Theme::hairline()).into_any_element(),
                Entry::Empty(text) => empty(text).h(px(EMPTY_ROW)).px_2().overflow_hidden().whitespace_nowrap().text_ellipsis().into_any_element(),
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
            .on_action(cx.listener(|this, _: &ToggleLibrary, window, cx| this.close(true, window, cx)))
            .on_action(cx.listener(|this, _: &crate::menubar::ShowHistory, _, cx| this.go(Page::History, cx)))
            .on_action(cx.listener(|this, _: &crate::menubar::ManageBookmarks, _, cx| this.go(Page::Bookmarks(None), cx)))
            .on_action(cx.listener(|this, _: &crate::menubar::ShowMemory, _, cx| this.go(Page::Memory, cx)))
            .size_full()
            .child(card)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bookmark(id: Id, parent_id: Option<Id>, url: Option<&str>) -> Bookmark {
        Bookmark { id, profile_id: 1, parent_id, url: url.map(String::from), title: String::new(), position: 0, created_at: 0 }
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
        let Entry::Row(r) = download_row(&d, 3 * 3_600_000) else { panic!() };
        assert_eq!((r.title.as_str(), r.detail.as_deref()), ("a.zip", Some("files.test · Finished · 3 h ago")));
        let blob = Download { url: String::new(), state: "failed".into(), ..d };
        let Entry::Row(r) = download_row(&blob, 0) else { panic!() };
        assert_eq!(r.detail.as_deref(), Some("Failed · Just now"));
    }

    #[test]
    fn ages_turn_into_dates_after_a_week() {
        let min = 60_000;
        assert_eq!(ago(0, 59_999), "Just now");
        assert_eq!(ago(0, 5 * min), "5 min ago");
        assert_eq!(ago(0, 2 * 1440 * min), "2 d ago");
        assert_eq!(ago(0, 7 * 1440 * min), "1970-01-01");
        assert_eq!(ago(10 * min, 0), "Just now");
    }

    #[test]
    fn an_already_bookmarked_url_is_found_instead_of_added_again() {
        let marks = [bookmark(1, None, None), bookmark(2, Some(1), Some("https://a.test/")), bookmark(3, None, Some("https://b.test/"))];
        assert_eq!(find_bookmark(&marks, "https://a.test/").map(|b| b.id), Some(2));
        assert!(find_bookmark(&marks, "https://c.test/").is_none());
    }

    #[test]
    fn the_import_summary_names_each_kind_and_why_one_failed() {
        let report = Report {
            cookies: Some(Err(ImportError::KeychainDenied)),
            history: Some(Ok(Outcome { imported: 1234, skipped: 0, failed: 0 })),
            bookmarks: Some(Ok(Outcome { imported: 5, skipped: 2, failed: 1 })),
        };
        assert_eq!(
            import_summary("Arc", Some(&report)),
            "Cookies: macOS did not give Tarn Arc’s Safe Storage key, so none were imported.\nHistory: 1,234 imported.\nBookmarks: 5 imported, 3 skipped."
        );
        let disk = Report { history: Some(Err(ImportError::DiskAccess)), ..Report::default() };
        assert!(import_summary("Firefox", Some(&disk)).contains(if cfg!(windows) { "Quit Firefox" } else { "Full Disk Access" }));
        assert!(import_summary("Arc", None).starts_with("The import did not finish."));
    }

    #[test]
    fn a_folder_cannot_move_into_itself_or_its_children() {
        let marks = [bookmark(1, None, None), bookmark(2, Some(1), None), bookmark(3, Some(2), Some("https://a.test/")), bookmark(4, None, None)];
        assert_eq!(subtree(&marks, 1), HashSet::from([1, 2, 3]));
        assert_eq!(subtree(&marks, 4), HashSet::from([4]));
    }
}
