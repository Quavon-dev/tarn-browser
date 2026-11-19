#![forbid(unsafe_code)]

mod address;
mod blocking;
mod browser;
mod capture;
mod content;
mod devtools;
mod dialog;
mod downloads;
use browser_core::ext_package;
mod ext_sources;
mod extensions;
mod filters;
mod find_tabs;
mod gestures;
mod groups;
mod hover_card;
mod library;
mod link_peek;
mod link_status;
mod memory;
mod menu_glyphs;
mod menubar;
mod peek;
mod prefs;
mod profiles;
mod qr;
mod reader;
mod settings;
mod share;
mod shared;
mod shield_card;
mod shortcuts;
mod sidebar;
mod site_settings;
mod spaces;
mod split;
mod tabs;
mod tarn_ext;
mod tear_off;
mod themes;
mod toast;
mod toolbar;
#[cfg(feature = "translation")]
mod translate;
mod zaps;

use browser::{Browser, CommandMode};
use gpui::{
    App, Application, Context, Focusable, InteractiveElement, IntoElement, KeyBinding, Menu, MenuItem, MouseButton, NavigationDirection, OsAction,
    ParentElement, Render, Styled, SystemMenuType, Window, actions, canvas, div, prelude::FluentBuilder, px,
};
use ui::text_input;
use ui::theme::Theme;

actions!(
    tarn,
    [
        Quit,
        NewTab,
        OpenLocation,
        OpenFile,
        CloseTab,
        Reload,
        Back,
        Forward,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        TogglePin,
        NewSpace,
        PrivateSpace,
        Find,
        FocusNextRegion,
        FocusPreviousRegion,
        FindNext,
        FindPrevious,
        Print,
        Space1,
        Space2,
        Space3,
        Space4,
        Space5,
        Space6,
        Space7,
        Space8,
        Space9,
        ToggleSidebar,
        Settings,
        CloseOverlay,
    ]
);

actions!(tarn, [ReopenClosedTab, CloseAllTabs, DuplicateTab, CopyLink, CopyCleanLink, ToggleCompactToolbar, OpenAppMenu]);
actions!(tarn, [AboutTarn, CheckForUpdates, HideTarn, HideOthers, ShowAll, ToggleFullScreen, NewChromiumWindow, NewWindow]);
actions!(tarn, [NewGroup, ToggleLibrary, BookmarkTab, BookmarkAllTabs, ToggleAutoHideSidebar, ToggleDownloads]);
actions!(tarn, [Tab1, Tab2, Tab3, Tab4, Tab5, Tab6, Tab7, Tab8, LastTab, NextTab, PreviousTab, BurnerTab]);
actions!(tarn, [CloseWindow, StopLoading, ReloadFromOrigin, NewPrivateWindow, NewBurnerWindow]);

/// Actions the command bar offers: label, search keywords, action.
pub(crate) const COMMANDS: &[(&str, &str, &dyn gpui::Action)] = &[
    ("Settings", "preferences options", &Settings),
    ("Open File…", "local html pdf image text document view", &OpenFile),
    ("New Tab Group", "group", &NewGroup),
    ("Bookmark Tab…", "favorite save", &BookmarkTab),
    ("Bookmark All Tabs", "favorite save", &BookmarkAllTabs),
    ("New Space", "workspace profile", &NewSpace),
    ("Edit Space", "workspace rename color icon", &spaces::EditSpace),
    ("Private Space", "incognito", &PrivateSpace),
    ("Burner Tab", "throwaway incognito temporary private", &BurnerTab),
    ("New Profile", "account", &profiles::NewProfile),
    ("Edit Profile", "account", &profiles::EditProfile),
    ("Reopen Closed Tab", "undo restore", &ReopenClosedTab),
    ("Notification Action", "toast undo reveal restart", &toast::ToastAction),
    ("Toggle Sidebar", "show hide", &ToggleSidebar),
    ("Auto-Hide Sidebar", "floating", &ToggleAutoHideSidebar),
    ("Compact Toolbar", "toggle address", &ToggleCompactToolbar),
    ("Close All Tabs", "clear", &CloseAllTabs),
    ("Show Library", "bookmarks history", &ToggleLibrary),
    ("Save Space as Snapshot", "session tabs save", &library::SaveSnapshot),
    ("Restore Snapshot…", "session tabs open", &library::RestoreSnapshot),
    ("Show Downloads", "files", &ToggleDownloads),
    ("Duplicate Tab", "copy", &DuplicateTab),
    ("Reload in Chromium", "engine chrome switch", &tabs::ReloadInChromium),
    ("Reload in WebKit", "engine safari switch", &tabs::ReloadInWebKit),
    ("Copy Link", "url address", &CopyLink),
    ("Copy Clean Link", "url tracking parameters utm strip", &CopyCleanLink),
    ("Pin or Unpin Tab", "", &TogglePin),
    ("Mute or Unmute Site", "sound audio silence", &menubar::ToggleMute),
    ("Show History", "visited", &menubar::ShowHistory),
    ("Rename Tab", "title name", &tabs::RenameTab),
    ("Reset Tab Title", "rename name", &tabs::ResetTabTitle),
    ("Return to Pinned URL", "home back pinned", &tabs::ReturnToPinnedUrl),
    ("Set Pinned URL to Current Page", "pinned home", &tabs::SetPinnedUrl),
    ("Sleep Other Tabs", "hibernate unload memory", &tabs::SleepOtherTabs),
    ("Show Memory Use", "ram cpu energy process task manager sleep", &menubar::ShowMemory),
    ("Close Duplicate Tabs", "same page dedupe clean up", &tabs::CloseDuplicateTabs),
    ("Site Settings", "shield permissions zoom mute clear data", &site_settings::SiteSettings),
    ("Zap Element", "hide remove element block", &zaps::ZapElement),
    ("Open in Split View", "open or close side by side tabs grid four", &split::ToggleSplitView),
    ("Focus Next Pane", "split view switch", &split::FocusNextPane),
    ("Focus Previous Pane", "split view switch", &split::FocusPreviousPane),
    ("Swap Split Panes", "split view exchange move", &split::SwapPanes),
    ("Rotate Split Layout", "split view stacked side by side rows columns", &split::RotateSplitLayout),
    ("Remove Tab from Split View", "split pane leave", &split::RemoveFromSplitView),
    ("Close Split View", "split panes end separate", &split::CloseSplitView),
    ("Reader", "article reading view text", &reader::ToggleReader),
    ("Save for Later", "reading list read later bookmark", &reader::SaveForLater),
    ("Picture in Picture", "video pip float", &capture::PictureInPicture),
    ("Take Screenshot", "capture image png", &capture::TakeScreenshot),
    ("Save Page as PDF", "export print", &capture::SavePdf),
    ("Copy Link as Markdown", "url markdown", &capture::CopyMarkdownLink),
    ("Copy Title", "page name", &capture::CopyTitle),
    ("Share", "send airdrop mail messages link", &qr::ShareLink),
    ("Show QR Code", "phone scan link", &qr::ShowQrCode),
    ("Find in Tabs", "search text all pages", &find_tabs::FindInTabs),
    ("Keyboard Shortcuts", "help keys cheat sheet", &shortcuts::ToggleShortcuts),
];

impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        engine::startup::mark("first window render");
        self.follow_theme();
        ui::motion::set_reduced(self.pref(prefs::Pref::ReduceMotion));
        // The menu bar is the app's; only the window in front decides what it lists.
        if window.is_window_active() {
            self.refresh_menus(cx);
        }
        self.keep_spinning(window, cx);
        // A field that is dropped or no longer drawn keeps no focus path, and with it no action reaches the browser.
        let stranded = window.focused(cx).is_none();
        if self.hidden_field_focused(window, cx) || (std::mem::take(&mut self.refocus) || stranded) && !self.overlay_open() && self.active_prompt().is_none() {
            window.focus(&self.focus_handle(cx));
        }
        let [back, forward, close_tab, bookmark, print, stop, reload, reload_fresh] =
            [&Back as &dyn gpui::Action, &Forward, &CloseTab, &BookmarkTab, &Print, &StopLoading, &Reload, &ReloadFromOrigin].map(|a| !self.idle(a));
        let mut root = div()
            .key_context("Browser")
            .track_focus(&self.focus_handle(cx))
            .size_full()
            .flex()
            .flex_row()
            .bg(Theme::backdrop())
            .text_color(Theme::text())
            .font_family(Theme::font())
            .text_size(px(ui::theme::TEXT))
            .on_action(cx.listener(|b, _: &ToggleSidebar, _, cx| b.toggle_sidebar(cx)))
            .on_action(cx.listener(|b, _: &Settings, w, cx| b.open_settings(w, cx)))
            .on_action(cx.listener(|b, _: &AboutTarn, w, cx| b.open_settings_pane(settings::SettingsPane::General, w, cx)))
            .on_action(cx.listener(|b, _: &ToggleFullScreen, w, _| {
                // Out of a page's full screen, the shell's toolbar and origin come back with the window.
                if b.page_fullscreen.is_on() { b.leave_page_fullscreen(w) } else { w.toggle_fullscreen() }
            }))
            .on_action(cx.listener(|b, _: &CloseOverlay, w, cx| {
                if b.overlay_open() {
                    b.dismiss_overlay(w, cx);
                } else if b.active_prompt().is_some() {
                    b.dismiss_prompt(w, cx);
                } else if b.find_open {
                    b.close_find(w, cx);
                }
            }))
            .on_key_down(cx.listener(|b, e: &gpui::KeyDownEvent, w, cx| b.prompt_key(e, w, cx)))
            .on_key_down(|e: &gpui::KeyDownEvent, w, cx| {
                let m = e.keystroke.modifiers;
                let in_field = w.context_stack().iter().any(|c| c.contains("TextInput"));
                if e.keystroke.key == "tab" && !(m.control || m.platform || m.alt || m.function) && !in_field {
                    if m.shift {
                        w.focus_prev()
                    } else {
                        w.focus_next()
                    }
                    cx.stop_propagation();
                }
            })
            .on_drag_move(cx.listener(|b, e: &gpui::DragMoveEvent<sidebar::DraggedTab>, w, cx| {
                let dragged = e.drag(cx).clone();
                b.tab_drag = Some(dragged.key);
                b.follow_tab_drag(dragged.key, dragged.title, e.event.position, w, cx);
            }))
            .capture_any_mouse_up(cx.listener(Browser::end_tab_drag))
            .on_mouse_up_out(MouseButton::Right, cx.listener(Browser::end_tab_drag))
            .on_mouse_up_out(MouseButton::Middle, cx.listener(Browser::end_tab_drag))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|b, e: &gpui::MouseUpEvent, w, cx| {
                    // Hitboxes that block the mouse leave the root unhovered inside the window too.
                    let inside = gpui::Bounds::new(gpui::Point::default(), w.viewport_size()).contains(&e.position);
                    b.drop_ghost(cx);
                    if let Some(key) = b.tab_drag.take().filter(|_| cx.has_active_drag() && !inside) {
                        b.tear_off(key, e.position, w, cx);
                    }
                }),
            )
            .on_action(cx.listener(|b, _: &NewTab, w, cx| b.new_tab_page(w, cx)))
            .on_action(cx.listener(|b, _: &NewChromiumWindow, w, cx| b.new_chromium_window(w, cx)))
            .on_action(cx.listener(|b, _: &profiles::NewProfile, w, cx| b.open_profile_editor(None, w, cx)))
            .on_action(cx.listener(|b, _: &profiles::EditProfile, w, cx| b.edit_current_profile(w, cx)))
            .on_action(cx.listener(|b, _: &OpenLocation, w, cx| b.open_command(CommandMode::Current, w, cx)))
            .on_action(cx.listener(|b, _: &OpenFile, w, cx| b.choose_file(w, cx)))
            .when(close_tab, |d| {
                d.on_action(cx.listener(|b, _: &CloseTab, w, cx| {
                    if b.card_open() {
                        b.dismiss_overlay(w, cx);
                    } else if let Some(key) = b.model.active {
                        b.close_tab(key, w, cx);
                    }
                }))
            })
            .on_action(cx.listener(|b, _: &CloseWindow, w, cx| {
                b.closing(cx);
                w.remove_window();
            }))
            .when(reload, |d| d.on_action(cx.listener(|b, _: &Reload, _, cx| b.reload(cx))))
            .when(reload_fresh, |d| d.on_action(cx.listener(|b, _: &ReloadFromOrigin, _, cx| b.reload_page(true, cx))))
            .when(stop, |d| d.on_action(cx.listener(|b, _: &StopLoading, _, cx| b.stop_loading(cx))))
            .when(back, |d| d.on_action(cx.listener(|b, _: &Back, _, _| b.back())))
            .when(forward, |d| d.on_action(cx.listener(|b, _: &Forward, _, _| b.forward())))
            .when(!self.idle(&ZoomIn), |d| {
                d.on_action(cx.listener(|b, _: &ZoomIn, _, cx| b.zoom(Some(1.1), cx)))
                    .on_action(cx.listener(|b, _: &ZoomOut, _, cx| b.zoom(Some(1.0 / 1.1), cx)))
                    .on_action(cx.listener(|b, _: &ZoomReset, _, cx| b.zoom(None, cx)))
            })
            .when(!self.idle(&TogglePin), |d| d.on_action(cx.listener(|b, _: &TogglePin, _, cx| b.toggle_pin(cx))))
            .when(!self.idle(&Find), |d| d.on_action(cx.listener(|b, _: &Find, w, cx| b.open_find(w, cx))))
            .on_action(cx.listener(|b, _: &FindNext, _, cx| b.find(false, cx)))
            .on_action(cx.listener(|b, _: &FindPrevious, _, cx| b.find(true, cx)))
            .when(print, |d| d.on_action(cx.listener(|b, _: &Print, _, _| b.print())))
            .on_action(cx.listener(|b, _: &NewSpace, w, cx| b.new_space(w, cx)))
            .on_action(cx.listener(|b, _: &spaces::EditSpace, w, cx| b.open_space_card(Some(b.model.space_ix), w, cx)))
            .on_action(cx.listener(|b, _: &PrivateSpace, w, cx| b.open_private_space(w, cx)))
            .on_action(cx.listener(|b, _: &BurnerTab, w, cx| b.open_burner(w, cx)))
            .on_action(cx.listener(|b, _: &NewGroup, w, cx| b.new_group(b.model.active, w, cx)))
            .on_action(cx.listener(|b, _: &ToggleLibrary, w, cx| b.toggle_library(w, cx)))
            .on_action(cx.listener(|b, _: &ToggleDownloads, w, cx| b.toggle_downloads(w, cx)))
            .on_action(cx.listener(|b, _: &site_settings::SiteSettings, w, cx| b.toggle_site_settings(w, cx)))
            .on_action(cx.listener(|b, _: &zaps::ZapElement, _, _| b.zap_element()))
            .when(bookmark, |d| {
                d.on_action(cx.listener(|b, _: &BookmarkTab, w, cx| {
                    if let Some(id) = b.bookmark_tab(None, cx) {
                        b.show_library(Some(id), w, cx);
                    }
                }))
            })
            .on_action(cx.listener(|b, _: &BookmarkAllTabs, _, cx| b.bookmark_all_tabs(cx)))
            .on_action(cx.listener(|b, _: &library::SaveSnapshot, w, cx| b.save_snapshot(w, cx)))
            .on_action(cx.listener(|b, _: &library::RestoreSnapshot, w, cx| b.open_library(library::Page::Snapshots, None, w, cx)))
            .on_action(cx.listener(|b, _: &ToggleAutoHideSidebar, _, cx| b.set_pref(prefs::Pref::AutoHideSidebar, !b.pref(prefs::Pref::AutoHideSidebar), cx)));
        macro_rules! indexed_actions {
            ($method:ident: $($action:ident => $ix:expr),*) => {
                $(root = root.on_action(cx.listener(|b, _: &$action, w, cx| b.$method($ix, w, cx)));)*
            };
        }
        indexed_actions!(select_tab: Tab1 => 1, Tab2 => 2, Tab3 => 3, Tab4 => 4, Tab5 => 5, Tab6 => 6, Tab7 => 7, Tab8 => 8, LastTab => 9);
        indexed_actions!(cycle_tab: NextTab => true, PreviousTab => false);
        root = menubar::listen(root, self, cx);
        root = tabs::listen(root, cx);
        let page = self.model.active.is_some();
        root = toast::listen(root.when(page, |r| qr::listen(reader::listen(r, cx), cx)), cx);
        root = find_tabs::listen(shortcuts::listen(root, cx).when(page, |r| capture::listen(split::listen(r, cx), cx)), cx);
        #[cfg(windows)]
        {
            use engine::chromium::EditCommand as E;
            root = root
                .on_action(cx.listener(|b, _: &text_input::Undo, w, _| b.edit_page(E::Undo, w)))
                .on_action(cx.listener(|b, _: &text_input::Redo, w, _| b.edit_page(E::Redo, w)))
                .on_action(cx.listener(|b, _: &text_input::Cut, w, _| b.edit_page(E::Cut, w)))
                .on_action(cx.listener(|b, _: &text_input::Copy, w, _| b.edit_page(E::Copy, w)))
                .on_action(cx.listener(|b, _: &text_input::Paste, w, _| b.edit_page(E::Paste, w)))
                .on_action(cx.listener(|b, _: &text_input::SelectAll, w, _| b.edit_page(E::SelectAll, w)));
        }
        indexed_actions!(switch_space: Space1 => 0, Space2 => 1, Space3 => 2, Space4 => 3, Space5 => 4, Space6 => 5, Space7 => 6, Space8 => 7, Space9 => 8);
        root = root
            .when(!self.idle(&ReopenClosedTab), |d| d.on_action(cx.listener(|b, _: &ReopenClosedTab, w, cx| b.reopen_closed_tab(w, cx))))
            .when(!self.idle(&CloseAllTabs), |d| d.on_action(cx.listener(|b, _: &CloseAllTabs, w, cx| b.close_all_tabs(w, cx))))
            .when(!self.idle(&DuplicateTab), |d| {
                d.on_action(cx.listener(|b, _: &DuplicateTab, w, cx| {
                    if let Some(key) = b.model.active {
                        b.duplicate_tab(key, w, cx);
                    }
                }))
                .on_action(cx.listener(|b, _: &CopyLink, _, cx| {
                    if let Some(key) = b.model.active {
                        b.copy_link(key, false, cx);
                    }
                }))
                .on_action(cx.listener(|b, _: &CopyCleanLink, _, cx| {
                    if let Some(key) = b.model.active {
                        b.copy_link(key, true, cx);
                    }
                }))
            })
            .on_action(cx.listener(|b, _: &ToggleCompactToolbar, _, cx| b.set_pref(prefs::Pref::CompactToolbar, !b.pref(prefs::Pref::CompactToolbar), cx)))
            .on_action(cx.listener(|b, _: &OpenAppMenu, window, cx| b.open_app_menu(window, cx)))
            .on_action(cx.listener(|b, _: &FocusNextRegion, w, cx| b.cycle_focus(false, w, cx)))
            .on_action(cx.listener(|b, _: &FocusPreviousRegion, w, cx| b.cycle_focus(true, w, cx)))
            // The mouse's buttons never arrive here, `watch_navigation_buttons` takes them; GPUI reports swipe events as these.
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Back), cx.listener(|b, _, _, _| b.swipe_history(true)))
            .on_mouse_down(MouseButton::Navigate(NavigationDirection::Forward), cx.listener(|b, _, _, _| b.swipe_history(false)));
        let shown = self.sidebar_shown();
        // A list taken away under the pointer sends no hover-out.
        if shown < 1. {
            self.motion.pointer = None;
        }
        let sidebar = (shown > 0.).then(|| {
            div()
                .flex_none()
                .flex()
                .justify_end()
                .child(sidebar::render(self, cx))
                .when(shown < 1., |d| d.w(px((self.sidebar_width + sidebar::SIDEBAR_MARGIN) * shown)).overflow_hidden().opacity(shown))
        });
        if self.page_fullscreen.is_on() && !self.page_fullscreen_holds() {
            self.leave_page_fullscreen(window);
        }
        if let Some(view) = self.active_view().filter(|_| self.page_fullscreen.is_on()) {
            let page = canvas(
                move |bounds, _, _| {
                    view.set_frame(
                        f32::from(bounds.origin.x).into(),
                        f32::from(bounds.origin.y).into(),
                        f32::from(bounds.size.width).into(),
                        f32::from(bounds.size.height).into(),
                    )
                },
                |_, _, _, _| {},
            );
            return root.bg(gpui::black()).child(page.size_full()).into_any_element();
        }
        root.children(sidebar)
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().child(toolbar::render(self, cx)).child(content::render(self, window, cx)))
            .into_any_element()
    }
}

pub(crate) fn bind_keys(cx: &mut App, keymap: &[(String, &'static dyn gpui::Action)]) {
    text_input::bind_keys(cx);
    profiles::bind_keys(cx);
    // Windows opens an app's menu with F10; the toolbar's menu button stands in for the menu bar.
    if cfg!(windows) {
        cx.bind_keys([KeyBinding::new("f10", OpenAppMenu, Some("Browser"))]);
    }
    cx.bind_keys(keymap.iter().filter_map(|(keys, action)| shortcuts::binding(keys, *action)));
    // After the keymap: menus show an action's first keys, which stay the rebindable ones.
    cx.bind_keys([
        KeyBinding::new(&shortcuts::platform("cmd-+"), ZoomIn, None),
        KeyBinding::new(&shortcuts::platform("cmd-}"), NextTab, None),
        KeyBinding::new(&shortcuts::platform("cmd-{"), PreviousTab, None),
        KeyBinding::new(&shortcuts::platform("alt-cmd-right"), NextTab, None),
        KeyBinding::new(&shortcuts::platform("alt-cmd-left"), PreviousTab, None),
        KeyBinding::new("escape", CloseOverlay, Some("Browser")),
        KeyBinding::new("escape", CloseOverlay, Some("Settings")),
        KeyBinding::new("f6", FocusNextRegion, Some("Browser")),
        KeyBinding::new("shift-f6", FocusPreviousRegion, Some("Browser")),
        KeyBinding::new(&shortcuts::platform("cmd-q"), Quit, None),
        KeyBinding::new(&shortcuts::platform("cmd-z"), text_input::Undo, None),
        KeyBinding::new(&shortcuts::platform("cmd-shift-z"), text_input::Redo, None),
        KeyBinding::new(&shortcuts::platform("ctrl-1"), Space1, None),
        KeyBinding::new(&shortcuts::platform("ctrl-2"), Space2, None),
        KeyBinding::new(&shortcuts::platform("ctrl-3"), Space3, None),
        KeyBinding::new(&shortcuts::platform("ctrl-4"), Space4, None),
        KeyBinding::new(&shortcuts::platform("ctrl-5"), Space5, None),
        KeyBinding::new(&shortcuts::platform("ctrl-6"), Space6, None),
        KeyBinding::new(&shortcuts::platform("ctrl-7"), Space7, None),
        KeyBinding::new(&shortcuts::platform("ctrl-8"), Space8, None),
        KeyBinding::new(&shortcuts::platform("ctrl-9"), Space9, None),
        KeyBinding::new(&shortcuts::platform("cmd-1"), Tab1, None),
        KeyBinding::new(&shortcuts::platform("cmd-2"), Tab2, None),
        KeyBinding::new(&shortcuts::platform("cmd-3"), Tab3, None),
        KeyBinding::new(&shortcuts::platform("cmd-4"), Tab4, None),
        KeyBinding::new(&shortcuts::platform("cmd-5"), Tab5, None),
        KeyBinding::new(&shortcuts::platform("cmd-6"), Tab6, None),
        KeyBinding::new(&shortcuts::platform("cmd-7"), Tab7, None),
        KeyBinding::new(&shortcuts::platform("cmd-8"), Tab8, None),
    ]);
}

fn set_menus(cx: &mut App, dynamic: menubar::Dynamic) {
    let (all, marks) = menus(dynamic);
    cx.set_menus(all);
    for (menu, marks) in &marks {
        engine::menu::decorate(menu, marks);
    }
    engine::menu::list_windows();
}

/// The app's menus with one window's dynamic ones, and per menu the check marks and images GPUI
/// cannot set.
pub(crate) type MenuMarks = Vec<(&'static str, Vec<engine::menu::Mark>)>;

pub(crate) fn menus(dynamic: menubar::Dynamic) -> (Vec<Menu>, MenuMarks) {
    let all = vec![
        Menu {
            name: "Tarn".into(),
            items: vec![
                MenuItem::action("About Tarn", AboutTarn),
                MenuItem::action("Check for Updates…", CheckForUpdates),
                MenuItem::separator(),
                MenuItem::action("Settings…", Settings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Tarn", HideTarn),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Tarn", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: [MenuItem::action("New Tab", NewTab), MenuItem::action("New Window", NewWindow)]
                .into_iter()
                .chain(cfg!(feature = "chromium").then(|| MenuItem::action("New Chromium Window", NewChromiumWindow)))
                .chain([
                    MenuItem::action("New Burner Tab", BurnerTab),
                    MenuItem::action("Private Space", PrivateSpace),
                    MenuItem::separator(),
                    MenuItem::action("Open Location…", OpenLocation),
                    MenuItem::action("Open File…", OpenFile),
                    MenuItem::action("Reopen Closed Tab", ReopenClosedTab),
                    MenuItem::separator(),
                    MenuItem::action("New Tab Group", NewGroup),
                    MenuItem::action("New Space…", NewSpace),
                    MenuItem::action("Edit Space…", spaces::EditSpace),
                    MenuItem::action("New Profile…", profiles::NewProfile),
                    MenuItem::action("Edit Profile…", profiles::EditProfile),
                    MenuItem::separator(),
                    MenuItem::action("Print…", Print),
                    MenuItem::action("Save Page as PDF", capture::SavePdf),
                    MenuItem::action("Take Screenshot", capture::TakeScreenshot),
                    MenuItem::separator(),
                    MenuItem::action("Close Window", CloseWindow),
                    MenuItem::action(dynamic.close_tab, CloseTab),
                    MenuItem::action("Close All Tabs", CloseAllTabs),
                ])
                .collect(),
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", text_input::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", text_input::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", text_input::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", text_input::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", text_input::Paste, OsAction::Paste),
                MenuItem::os_action("Select All", text_input::SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find…", Find),
                MenuItem::action("Find Next", FindNext),
                MenuItem::action("Find Previous", FindPrevious),
                MenuItem::action("Find in Tabs…", find_tabs::FindInTabs),
            ],
        },
        Menu { name: "View".into(), items: dynamic.view },
        Menu { name: "Tabs".into(), items: dynamic.tabs },
        Menu { name: "Bookmarks".into(), items: dynamic.bookmarks },
        Menu { name: "History".into(), items: dynamic.history },
        Menu { name: "Window".into(), items: dynamic.window },
    ];
    (all, dynamic.marks)
}

/// Panics abort (release builds and GPUI's AppKit callbacks), and an app started from Finder has
/// no stderr, so the last panic is kept in the data folder (PRIVACY.md: crash reports stay local).
/// Release backtraces are unsymbolized; the message and location still name the cause. The folder
/// is never created here: one made before the container move would block that move for good.
/// Only a literal message is kept: a formatted one may carry a page address (S8).
fn record_panics(dir: std::path::PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map(|l| l.to_string()).unwrap_or_default();
        let message = info.payload().downcast_ref::<&'static str>().copied().unwrap_or("<formatted message left out>");
        let report = format!("{}\npanicked at {location}:\n{message}\n{}", browser::now(), std::backtrace::Backtrace::force_capture());
        std::fs::write(dir.join("last-panic.txt"), report).ok();
        previous(info);
    }));
}

/// Runs Tarn: a Chromium child process when CEF started this process as one, else the browser.
pub fn run() {
    #[cfg(windows)]
    if let Some(code) = engine::chromium::subprocess() {
        std::process::exit(code);
    }
    #[cfg(windows)]
    let data_dir = browser::data_dir().to_string_lossy().into_owned();
    #[cfg(windows)]
    if !engine::instance::claim(&data_dir, &browser::launch_urls(std::env::args().skip(1))) {
        return;
    }
    #[cfg(windows)]
    engine::chromium::prepare_browser_process();
    #[cfg(windows)]
    engine::refresh_browser_registration();
    engine::startup::mark("main");
    record_panics(browser::data_dir());
    #[cfg(feature = "chromium")]
    engine::chromium::register_early_malloc_zone();
    engine::menu::disable_saved_application_state();
    // macOS hands links from other apps to the default browser as an event, also at a cold launch.
    let (urls, mut opened) = futures::channel::mpsc::unbounded();
    std::thread::spawn(|| std::sync::LazyLock::force(&content::MARK_IMAGE));
    let app = Application::new().with_assets(ui::icon::Assets);
    #[cfg(windows)]
    let links = urls.clone();
    app.on_open_urls(move |list| drop(urls.unbounded_send(list)));
    app.on_reopen(shared::reopen);
    app.run(move |cx: &mut App| {
        #[cfg(windows)]
        engine::instance::listen(&data_dir, move |list| drop(links.unbounded_send(list)));
        cx.on_action(|_: &Quit, cx| cx.quit());
        #[cfg(target_os = "macos")]
        {
            cx.on_action(|_: &HideTarn, cx| cx.hide());
            cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
            cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        }
        if shared::start(cx) {
            cx.on_action(|_: &NewWindow, cx| shared::new_window(cx));
            cx.on_action(|_: &NewPrivateWindow, cx| shared::new_window_then(cx, |b, window, cx| b.open_private_space(window, cx)));
            cx.on_action(|_: &NewBurnerWindow, cx| shared::new_window_then(cx, |b, window, cx| b.open_burner(window, cx)));
            shortcuts::rebind(cx);
            cx.set_dock_menu(vec![
                MenuItem::action("New Window", NewWindow),
                MenuItem::action("New Private Window", NewPrivateWindow),
                MenuItem::action("New Burner Window", NewBurnerWindow),
            ]);
            if shared::core(cx).updater.is_some() {
                cx.on_action(|_: &CheckForUpdates, cx| {
                    if let Some(updater) = &shared::core(cx).updater {
                        updater.check_now();
                    }
                });
            }
            shared::open_windows(cx);
            // A jump-list entry that started Tarn arrives as its arguments, not from a running instance.
            #[cfg(windows)]
            if let [flag, index] = std::env::args().skip(1).collect::<Vec<_>>().as_slice()
                && flag == "--dock-action"
                && let Ok(index) = index.parse()
            {
                cx.perform_dock_menu_action(index);
            }
            cx.spawn(async move |cx| {
                while let Some(list) = futures::StreamExt::next(&mut opened).await {
                    // A Windows launch with no links asks the running Tarn to come to the front.
                    #[cfg(windows)]
                    if list.is_empty() {
                        if cx.update(shared::bring_forward).is_err() {
                            break;
                        }
                        continue;
                    }
                    #[cfg(windows)]
                    if let [flag, index] = list.as_slice()
                        && flag == "--dock-action"
                    {
                        let Ok(index) = index.parse() else { continue };
                        if cx.update(|cx| cx.perform_dock_menu_action(index)).is_err() {
                            break;
                        }
                        continue;
                    }
                    if cx.update(|cx| shared::open_urls(list, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
            cx.activate(true);
        } else {
            bind_keys(cx, &shortcuts::keymap(&Default::default()));
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn panics_are_kept_in_the_data_folder() {
        let dir = tempfile::tempdir().unwrap();
        super::record_panics(dir.path().to_owned());
        let caught = std::panic::catch_unwind(|| panic!("tab click went wrong"));
        drop(std::panic::take_hook());
        assert!(caught.is_err());
        let report = std::fs::read_to_string(dir.path().join("last-panic.txt")).unwrap();
        assert!(report.contains("tab click went wrong") && report.contains("lib.rs"), "{report}");
        let url = url::Url::parse("https://bank.test/a?x=1").unwrap();
        super::record_panics(dir.path().to_owned());
        for message in [format!("{url} failed"), format!("no tab for {url:?}")] {
            std::panic::catch_unwind(|| panic!("{message}")).ok();
            let report = std::fs::read_to_string(dir.path().join("last-panic.txt")).unwrap();
            assert!(!report.contains("bank.test") && report.contains("lib.rs"), "{report}");
        }
        drop(std::panic::take_hook());

        let missing = dir.path().join("missing");
        super::record_panics(missing.clone());
        std::panic::catch_unwind(|| panic!("before the data folder exists")).ok();
        drop(std::panic::take_hook());
        assert!(!missing.exists(), "the hook created the data folder");
    }
}
