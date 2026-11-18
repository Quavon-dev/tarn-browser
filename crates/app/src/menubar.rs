use std::hash::{DefaultHasher, Hash, Hasher};

use engine::menu::{Image, Mark};
use gpui::{Action, App, Context, Div, InteractiveElement, Menu, MenuItem, actions, prelude::FluentBuilder};
use store::{Bookmark, Id, TabKind};

use crate::browser::Browser;
use crate::library::Page;
use crate::settings::SettingsPane;
use crate::tabs::state_label;

/// Dynamic lists stay short, like other browsers' menus.
const MAX_ITEMS: usize = 12;
const MAX_TITLE: usize = 48;

actions!(tarn, [Minimize, ZoomWindow, BringAllToFront, ShowHistory, ManageBookmarks, ShowMemory, ImportBrowserData, ManageProfiles, ToggleMute, Heading]);

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = tarn, no_json)]
pub struct OpenUrl {
    pub url: String,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = tarn, no_json)]
pub struct ReopenClosed {
    pub serial: u64,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = tarn, no_json)]
pub struct SwitchProfile {
    pub id: Id,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = tarn, no_json)]
pub struct MoveToSpace {
    pub ix: usize,
}

#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = tarn, no_json)]
pub struct SwitchToSpace {
    pub ix: usize,
}

pub(crate) const SPACES: [&dyn Action; 9] =
    [&crate::Space1, &crate::Space2, &crate::Space3, &crate::Space4, &crate::Space5, &crate::Space6, &crate::Space7, &crate::Space8, &crate::Space9];

#[derive(Default)]
pub struct State {
    signature: u64,
    history: u64,
    bookmarks: u64,
    last_visit: String,
}

impl State {
    /// Reloads of the page visited last do not change the History menu.
    pub fn visited(&mut self, url: &str) {
        if self.last_visit != url {
            self.last_visit = url.to_owned();
            self.history += 1;
        }
    }

    pub fn history_cleared(&mut self) {
        self.last_visit.clear();
        self.history += 1;
    }

    /// Makes the next render rebuild the menu bar, which another window may have filled.
    pub fn reset(&mut self) {
        self.signature = 0;
    }
}

/// The dynamic menus and, per menu, the images and check marks GPUI cannot set.
pub struct Dynamic {
    pub bookmarks: Vec<MenuItem>,
    pub history: Vec<MenuItem>,
    pub tabs: Vec<MenuItem>,
    pub window: Vec<MenuItem>,
    pub view: Vec<MenuItem>,
    /// File › Close Tab, which unloads a pinned tab.
    pub close_tab: &'static str,
    pub marks: Vec<(&'static str, Vec<Mark>)>,
}

/// Collects a mark for every entry of one menu in the order shown, so a page or profile named
/// like a fixed entry cannot take that entry's mark.
#[derive(Default)]
struct Marks(Vec<Mark>);

impl Marks {
    fn action(&mut self, title: &str, action: impl Action) -> MenuItem {
        self.item(title, Box::new(action), None, false)
    }

    fn item(&mut self, title: &str, action: Box<dyn Action>, image: Option<Image>, checked: bool) -> MenuItem {
        let title = menu_title(title);
        self.0.push(Mark { title: title.clone(), image, checked });
        MenuItem::Action { name: title.into(), action, os_action: None }
    }

    fn submenu(&mut self, title: &str, image: Option<Image>, items: impl FnOnce(&mut Self) -> Vec<MenuItem>) -> MenuItem {
        let title = menu_title(title);
        self.0.push(Mark { title: title.clone(), image, checked: false });
        MenuItem::submenu(Menu { name: title.into(), items: items(self) })
    }
}

fn menu_title(raw: &str) -> String {
    let title = ui::text::bounded_title(raw);
    match title.char_indices().nth(MAX_TITLE) {
        Some((at, _)) => format!("{}…", &title[..at]),
        None => title,
    }
}

/// `b` leaves out the mute handler on a page without a host, which greys its menu entry.
pub fn listen(root: Div, b: &Browser, cx: &mut Context<Browser>) -> Div {
    let mute = !b.idle(&ToggleMute);
    root.on_action(cx.listener(|_, _: &Minimize, w, _| w.minimize_window()))
        .on_action(cx.listener(|_, _: &ZoomWindow, w, _| w.zoom_window()))
        .on_action(cx.listener(|_, _: &BringAllToFront, _, cx| cx.activate(true)))
        .on_action(cx.listener(|b, _: &ShowHistory, w, cx| b.open_library(Page::History, None, w, cx)))
        .on_action(cx.listener(|b, _: &ManageBookmarks, w, cx| b.open_library(Page::Bookmarks(None), None, w, cx)))
        .on_action(cx.listener(|b, _: &ShowMemory, w, cx| b.open_library(Page::Memory, None, w, cx)))
        .on_action(cx.listener(|b, _: &ImportBrowserData, w, cx| b.open_library(Page::Import, None, w, cx)))
        .on_action(cx.listener(|b, _: &ManageProfiles, w, cx| b.open_settings_pane(SettingsPane::Profiles, w, cx)))
        .when(mute, |d| {
            d.on_action(cx.listener(|b, _: &ToggleMute, _, cx| {
                if let Some(key) = b.model.active {
                    b.toggle_site_muted(key, cx);
                }
            }))
        })
        .on_action(cx.listener(|b, a: &OpenUrl, w, cx| b.open_url(&a.url, w, cx)))
        .on_action(cx.listener(|b, a: &ReopenClosed, w, cx| b.reopen_closed(Some(a.serial), w, cx)))
        .on_action(cx.listener(|b, a: &SwitchProfile, w, cx| {
            if let Some(ix) = b.model.spaces.iter().position(|s| !s.private && s.space.profile_id == a.id) {
                b.switch_space(ix, w, cx);
            }
        }))
        .on_action(cx.listener(|b, a: &SwitchToSpace, w, cx| b.switch_space(a.ix, w, cx)))
        .on_action(cx.listener(|b, a: &MoveToSpace, w, cx| {
            if let Some(key) = b.model.active {
                b.move_to_space(key, a.ix, w, cx);
            }
        }))
}

impl Browser {
    /// Whether `action` has nothing to act on now. Its menu entry is greyed: macOS greys an entry
    /// whose action has no handler, so the window leaves those out, and Windows' menu asks this.
    pub(crate) fn idle(&self, action: &dyn Action) -> bool {
        let is = |a: &dyn Action| action.partial_eq(a);
        let page = self.model.active.and_then(|k| self.pages.get(&k));
        let no_tab = self.model.active.is_none();
        is(&crate::Back) && !page.is_some_and(|p| p.can_back)
            || is(&crate::Forward) && !page.is_some_and(|p| p.can_forward)
            || is(&crate::StopLoading) && !page.is_some_and(|p| p.loading)
            || is(&crate::ReopenClosedTab) && self.model.closed_tabs.is_empty()
            || is(&crate::CloseAllTabs) && self.model.ephemeral_keys().is_empty()
            || is(&ToggleMute) && self.model.active.and_then(|k| self.tab_url(k)).and_then(|u| crate::browser::tab_host(&u)).is_none()
            || is(&crate::CloseTab) && no_tab && !self.overlay_open()
            || no_tab && (is(&crate::BookmarkTab) || is(&crate::Print) || is(&crate::Reload) || is(&crate::ReloadFromOrigin))
    }

    /// Rebuilds the menu bar when what it lists has changed since the last render. GPUI keeps every
    /// action of every menu bar it was given, so the signature leaves out what the menus do not
    /// show; each newly visited page still costs one rebuild.
    pub(crate) fn refresh_menus(&mut self, cx: &mut App) {
        let mut hash = DefaultHasher::new();
        (self.model.space_ix, self.menu.history, self.menu.bookmarks, self.profile_editor.is_some()).hash(&mut hash);
        for s in &self.model.spaces {
            (s.private, s.space.id, s.space.profile_id, &s.space.name).hash(&mut hash);
        }
        if let Some(s) = self.model.current_space() {
            self.model.closed_tabs.signature(s.private, s.space.profile_id).hash(&mut hash);
        }
        self.model.active_tab().map(|t| t.kind == TabKind::Ephemeral).hash(&mut hash);
        self.model.active.map(|k| self.tab_muted(k)).hash(&mut hash);
        self.model.active_tab().map(|t| t.custom_title.is_empty()).hash(&mut hash);
        self.model.active.and_then(|k| self.pinned_drift(k)).map(|u| u.host_str().map(str::to_owned)).hash(&mut hash);
        (self.model.active.is_some_and(|a| self.splits.contains(a)), self.site().is_some(), self.reader_shown()).hash(&mut hash);
        (self.sidebar_open, self.pref(crate::prefs::Pref::CompactToolbar), self.pref(crate::prefs::Pref::AutoHideSidebar)).hash(&mut hash);
        let signature = hash.finish();
        if self.menu.signature != signature {
            self.menu.signature = signature;
            let menus = self.dynamic_menus();
            cx.defer(move |cx| crate::set_menus(cx, menus));
        }
    }

    pub(crate) fn set_bookmarks(&mut self, bookmarks: Vec<Bookmark>) {
        if self.bookmarks != bookmarks {
            self.bookmarks = bookmarks;
            self.menu.bookmarks += 1;
        }
    }

    fn site_image(&self, url: &str) -> Option<Image> {
        Some(self.favicon(url).map_or(Image::Symbol("globe"), |icon| Image::Png(icon.bytes().to_vec())))
    }

    pub(crate) fn dynamic_menus(&self) -> Dynamic {
        let space = self.model.current_space().map(|s| (s.private, s.space.profile_id));
        let (mut bookmarks, mut history, mut tabs, mut window) = (Marks::default(), Marks::default(), Marks::default(), Marks::default());

        let recent = browser_core::bookmarks::recent(&self.bookmarks, MAX_ITEMS);
        let mut bookmark_items = vec![
            bookmarks.action("Bookmark Tab…", crate::BookmarkTab),
            bookmarks.action("Bookmark All Tabs", crate::BookmarkAllTabs),
            bookmarks.action("Manage Bookmarks", ManageBookmarks),
        ];
        if browser_core::import::available() {
            bookmark_items.push(bookmarks.action("Import Browser Data…", ImportBrowserData));
        }
        if !recent.is_empty() {
            bookmark_items.extend([MenuItem::separator(), bookmarks.action("Recent Bookmarks", Heading)]);
            bookmark_items.extend(recent.into_iter().map(|b| self.bookmark_item(&mut bookmarks, b)));
        }
        let folders: Vec<&Bookmark> = self.bookmarks.iter().filter(|b| b.url.is_none() && b.parent_id.is_none()).take(MAX_ITEMS).collect();
        if !folders.is_empty() {
            bookmark_items.push(MenuItem::separator());
            bookmark_items.extend(folders.into_iter().map(|f| self.folder_item(&mut bookmarks, f)));
        }

        let mut history_items = Vec::new();
        if let Some((private, profile_id)) = space {
            let visited = self.store(private).history(profile_id, MAX_ITEMS).unwrap_or_default();
            if !visited.is_empty() {
                history_items.push(history.action("Recently Visited", Heading));
                history_items.extend(visited.iter().map(|h| {
                    let title = if h.title.is_empty() { &h.url } else { &h.title };
                    history.item(title, Box::new(OpenUrl { url: h.url.clone() }), self.site_image(&h.url), false)
                }));
                history_items.push(MenuItem::separator());
            }
            history_items.extend([history.action("Recently Closed", Heading), history.action("Reopen Closed Tab", crate::ReopenClosedTab)]);
            history_items.extend(
                self.model
                    .closed_tabs
                    .recent(private, profile_id)
                    .into_iter()
                    .take(MAX_ITEMS)
                    .map(|(serial, title, url)| history.item(&title, Box::new(ReopenClosed { serial }), self.site_image(&url), false)),
            );
            history_items.push(MenuItem::separator());
        }
        history_items.push(history.action("Show History", ShowHistory));

        let pinned = self.model.active_tab().is_some_and(|t| t.kind != TabKind::Ephemeral);
        let muted = self.model.active.is_some_and(|k| self.tab_muted(k));
        let split = self.model.active.is_some_and(|a| self.splits.contains(a));
        let mut tab_items = vec![
            tabs.action("Back", crate::Back),
            tabs.action("Forward", crate::Forward),
            MenuItem::separator(),
            tabs.action("Next Tab", crate::NextTab),
            tabs.action("Previous Tab", crate::PreviousTab),
            MenuItem::separator(),
            tabs.action("Show Library", crate::ToggleLibrary),
            tabs.action(state_label(&crate::TogglePin, pinned).unwrap_or_default(), crate::TogglePin),
            tabs.action("Rename Tab", crate::tabs::RenameTab),
        ];
        if self.model.active_tab().is_some_and(|t| !t.custom_title.is_empty()) {
            tab_items.push(tabs.action("Reset Title", crate::tabs::ResetTabTitle));
        }
        if let Some(host) = self.model.active.and_then(|k| self.pinned_drift(k)).and_then(|u| u.host_str().map(str::to_owned)) {
            tab_items.push(tabs.action(&format!("Return to {host}"), crate::tabs::ReturnToPinnedUrl));
            tab_items.push(tabs.action("Set Pinned URL to Current Page", crate::tabs::SetPinnedUrl));
        }
        tab_items.extend([
            MenuItem::separator(),
            tabs.action("Duplicate Tab", crate::DuplicateTab),
            tabs.action("Copy Link", crate::CopyLink),
            tabs.action("Copy Clean Link", crate::CopyCleanLink),
            tabs.action("Copy Link as Markdown", crate::capture::CopyMarkdownLink),
            tabs.action("Copy Title", crate::capture::CopyTitle),
            tabs.action("Share…", crate::qr::ShareLink),
            tabs.action("Show QR Code", crate::qr::ShowQrCode),
            MenuItem::separator(),
            tabs.action(state_label(&crate::split::ToggleSplitView, split).unwrap_or_default(), crate::split::ToggleSplitView),
        ]);
        if split {
            tab_items.extend([
                tabs.action("Focus Next Pane", crate::split::FocusNextPane),
                tabs.action("Focus Previous Pane", crate::split::FocusPreviousPane),
                tabs.action("Swap Panes", crate::split::SwapPanes),
                tabs.action("Rotate Split Layout", crate::split::RotateSplitLayout),
                tabs.action("Remove Tab from Split View", crate::split::RemoveFromSplitView),
            ]);
        }
        if let Some(key) = self.model.active {
            let others: Vec<(usize, &str)> =
                self.model.move_targets(&self.core, key).into_iter().take(MAX_ITEMS).map(|ix| (ix, self.model.spaces[ix].space.name.as_str())).collect();
            if !others.is_empty() {
                tab_items.push(tabs.submenu("Move to Space", None, |tabs| {
                    others.into_iter().map(|(ix, name)| tabs.item(name, Box::new(MoveToSpace { ix }), None, false)).collect()
                }));
            }
        }
        tab_items.extend([MenuItem::separator(), tabs.action(state_label(&ToggleMute, muted).unwrap_or_default(), ToggleMute)]);
        if self.site().is_some() {
            tab_items.push(tabs.action("Site Settings…", crate::site_settings::SiteSettings));
        }
        tab_items.extend([tabs.action("Sleep Other Tabs", crate::tabs::SleepOtherTabs), MenuItem::separator()]);
        tab_items.push(tabs.submenu("Spaces", None, |tabs| {
            self.model
                .spaces
                .iter()
                .enumerate()
                .map(|(ix, s)| {
                    let action = SPACES.get(ix).map_or_else(|| Box::new(SwitchToSpace { ix }) as Box<dyn Action>, |a| a.boxed_clone());
                    tabs.item(&s.space.name, action, None, ix == self.model.space_ix)
                })
                .collect()
        }));

        let mut window_items = vec![
            window.action("Minimize", Minimize),
            window.action("Zoom", ZoomWindow),
            MenuItem::separator(),
            window.action("Bring All to Front", BringAllToFront),
            MenuItem::separator(),
        ];
        let profiles = self.core.disk.profiles().unwrap_or_default();
        let current = space.filter(|(private, _)| !private).and_then(|(_, id)| profiles.iter().find(|p| p.id == id));
        let dot = |color: &str| {
            let rgb = ui::theme::PROFILE_COLORS.iter().find(|(name, _)| *name == color).unwrap_or(&ui::theme::PROFILE_COLORS[0]).1;
            Some(Image::Dot(rgb))
        };
        let title = current.map_or("Profiles".to_owned(), |p| p.name.clone());
        window_items.push(window.submenu(&title, current.and_then(|p| dot(&p.color)), |window| {
            let mut items: Vec<MenuItem> = profiles
                .iter()
                .take(MAX_ITEMS)
                .map(|p| window.item(&p.name, Box::new(SwitchProfile { id: p.id }), dot(&p.color), Some(p.id) == current.map(|c| c.id)))
                .collect();
            items.extend([
                MenuItem::separator(),
                window.action("New Profile…", crate::profiles::NewProfile),
                window.action("Manage Profiles…", ManageProfiles),
            ]);
            items
        }));
        window_items.extend([MenuItem::separator(), window.action("Downloads", crate::ToggleDownloads)]);

        let mut view = Marks::default();
        let compact = self.pref(crate::prefs::Pref::CompactToolbar);
        let auto_hide = self.pref(crate::prefs::Pref::AutoHideSidebar);
        let mut view_items = vec![
            view.action(if self.sidebar_open { "Hide Sidebar" } else { "Show Sidebar" }, crate::ToggleSidebar),
            view.item("Compact Toolbar", Box::new(crate::ToggleCompactToolbar), None, compact),
            view.item("Auto-Hide Sidebar", Box::new(crate::ToggleAutoHideSidebar), None, auto_hide),
            MenuItem::separator(),
            view.action("Reload", crate::Reload),
            view.action("Reload Without Cache", crate::ReloadFromOrigin),
            view.action("Stop Loading", crate::StopLoading),
            MenuItem::separator(),
            view.action("Zoom In", crate::ZoomIn),
            view.action("Zoom Out", crate::ZoomOut),
            view.action("Actual Size", crate::ZoomReset),
        ];
        // AppKit adds Enter Full Screen to a menu named View on its own.
        if !cfg!(target_os = "macos") {
            view_items.push(view.action("Full Screen", crate::ToggleFullScreen));
        }
        view_items.extend([
            MenuItem::separator(),
            view.action(if self.reader_shown() { "Hide Reader" } else { "Show Reader" }, crate::reader::ToggleReader),
            view.action("Picture in Picture", crate::capture::PictureInPicture),
            view.action("Keyboard Shortcuts", crate::shortcuts::ToggleShortcuts),
        ]);

        Dynamic {
            bookmarks: bookmark_items,
            history: history_items,
            tabs: tab_items,
            window: window_items,
            view: view_items,
            close_tab: if pinned { "Unload Tab" } else { "Close Tab" },
            marks: vec![("View", view.0), ("Bookmarks", bookmarks.0), ("History", history.0), ("Tabs", tabs.0), ("Window", window.0)],
        }
    }

    fn bookmark_item(&self, marks: &mut Marks, b: &Bookmark) -> MenuItem {
        let url = b.url.clone().unwrap_or_default();
        let title = if b.title.is_empty() { url.clone() } else { b.title.clone() };
        marks.item(&title, Box::new(OpenUrl { url: url.clone() }), self.site_image(&url), false)
    }

    fn folder_item(&self, marks: &mut Marks, folder: &Bookmark) -> MenuItem {
        marks.submenu(&folder.title, Some(Image::Symbol("folder")), |marks| {
            let children: Vec<&Bookmark> = self.bookmarks.iter().filter(|b| b.parent_id == Some(folder.id)).take(MAX_ITEMS).collect();
            if children.is_empty() {
                return vec![marks.action("Empty", Heading)];
            }
            children.into_iter().map(|b| if b.url.is_some() { self.bookmark_item(marks, b) } else { self.folder_item(marks, b) }).collect()
        })
    }
}
