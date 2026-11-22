use engine::menu::{self, Item};
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{ClipboardItem, Context, Pixels, Point, PromptLevel, Window, px};
use store::{Counter, Id, SavedWindow, Tab, WebEngine};
use url::Url;

use browser_core::closed::{Closed, ClosedGroup, ClosedTab};
use browser_core::tab_menu;

use super::{Browser, TabKey, now};
use crate::groups::RenameTarget;
use crate::prefs::Pref;
use crate::settings::SettingsPane;

#[derive(Clone)]
pub enum Cmd {
    Open(TabKey),
    Reload(TabKey),
    Duplicate(TabKey),
    CopyLink(TabKey, bool),
    TogglePin(TabKey),
    ToggleTile(TabKey),
    MoveToSpace(TabKey, usize),
    Close(TabKey),
    CloseOthers(TabKey),
    CloseBelow(TabKey),
    CloseDuplicates,
    NewTab,
    ReopenClosed,
    NewFolder(Option<Id>),
    ToggleSidebar,
    AutoHideSidebar,
    CloseAll,
    DeleteFolder(Id),
    NewSpace,
    DeleteSpace(usize),
    CompactToolbar(bool),
    Settings,
    Rename(RenameTarget),
    ChangeIcon(RenameTarget),
    EditSpace(usize),
    NewGroup(Option<TabKey>),
    SetGroup(TabKey, Option<Id>),
    NewTabInGroup(Id),
    Ungroup(Id),
    CloseGroup(Id),
    Group(crate::groups::GroupCmd),
    Bookmark(TabKey),
    BookmarkAll,
    NewProfile,
    EditProfile(Id),
    ToggleMute(TabKey),
    ResetTitle(TabKey),
    ReturnToPinned(TabKey),
    SetPinnedUrl(TabKey),
    Sleep(Vec<TabKey>),
    SiteSettings,
    PasteAndGo,
    Split(TabKey),
    LeaveSplit(TabKey),
    RotateSplit(TabKey),
    CloseSplit(TabKey),
    SwitchWebEngine(TabKey, WebEngine),
    ProfileShield(shield::ShieldLevel),
}

impl Browser {
    pub fn tab_menu(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_hover_card(cx);
        let Some(tab) = self.model.tab(key) else { return };
        let active = self.model.active == Some(key);
        let rest = self.model.other_tabs(key);
        let state = tab_menu::Tab {
            kind: tab.kind,
            active,
            has_link: self.tab_url(key).is_some(),
            engine: (cfg!(feature = "chromium") && !key.private).then(|| (self.tab_web_engine(key), self.web_engine_switch(key).is_some())),
            split: self.splits.menu(self.model.active, key),
            split_view: true,
            burner: self.model.burner_shown(),
            custom_title: !tab.custom_title.is_empty(),
            drift: self.pinned_drift(key).and_then(|u| u.host_str().map(ui::text::bounded_title)),
            grouped: tab.group_id.is_some(),
            muted: self.tab_url(key).and_then(|u| super::tab_host(&u)).map(|_| self.tab_muted(key)),
            sleepable: !active && !self.sleeping(key) && !self.in_split(key),
            sleep_others: rest.iter().any(|k| !self.sleeping(*k) && self.model.active != Some(*k) && !self.in_split(*k)),
            others: self.model.ephemeral_keys().iter().any(|k| *k != key),
            below: !browser_core::sidebar::tabs_below(&self.model, key).is_empty(),
            spaces: self.model.move_targets(&self.core, key).into_iter().map(|ix| (ix, ui::text::bounded_title(&self.model.spaces[ix].space.name))).collect(),
            groups: self.model.groups.iter().filter(|g| Some(g.id) != tab.group_id).map(|g| (g.id, ui::text::bounded_title(&g.name))).collect(),
        };
        let items = self.tab_menu_items(key, tab_menu::items(&state, |_| true), &rest, cx);
        self.show_menu(items, window, cx);
    }

    fn tab_menu_items(&self, key: TabKey, items: Vec<tab_menu::Item>, rest: &[TabKey], cx: &gpui::App) -> Vec<Item<Cmd>> {
        use tab_menu::Command as C;
        items
            .into_iter()
            .map(|item| match item {
                tab_menu::Item::Separator => Item::Separator,
                tab_menu::Item::Submenu(name, inner) => Item::Submenu(name.into(), self.tab_menu_items(key, inner, rest, cx)),
                tab_menu::Item::Command(command, label, enabled, shortcut) => {
                    let cmd = match command {
                        C::Reload => Cmd::Reload(key),
                        C::Open => Cmd::Open(key),
                        C::Duplicate => Cmd::Duplicate(key),
                        C::SwitchEngine => Cmd::SwitchWebEngine(key, self.web_engine_switch(key).unwrap_or(WebEngine::Chromium)),
                        C::CopyLink => Cmd::CopyLink(key, false),
                        C::CopyCleanLink => Cmd::CopyLink(key, true),
                        C::Bookmark => Cmd::Bookmark(key),
                        C::Split => Cmd::Split(key),
                        C::LeaveSplit => Cmd::LeaveSplit(key),
                        C::RotateSplit => Cmd::RotateSplit(key),
                        C::CloseSplit => Cmd::CloseSplit(key),
                        C::TogglePin => Cmd::TogglePin(key),
                        C::ToggleTile => Cmd::ToggleTile(key),
                        C::Rename => Cmd::Rename(RenameTarget::Tab(key.id)),
                        C::ResetTitle => Cmd::ResetTitle(key),
                        C::ReturnToPinned => Cmd::ReturnToPinned(key),
                        C::SetPinnedUrl => Cmd::SetPinnedUrl(key),
                        C::MoveToSpace(ix) => Cmd::MoveToSpace(key, ix),
                        C::AddToGroup(id) => Cmd::SetGroup(key, Some(id)),
                        C::NewGroup => Cmd::NewGroup(Some(key)),
                        C::RemoveFromGroup => Cmd::SetGroup(key, None),
                        C::ToggleMute => Cmd::ToggleMute(key),
                        C::Sleep => Cmd::Sleep(vec![key]),
                        C::SleepOthers => Cmd::Sleep(rest.to_vec()),
                        C::Unload | C::Close => Cmd::Close(key),
                        C::CloseOthers => Cmd::CloseOthers(key),
                        C::CloseBelow => Cmd::CloseBelow(key),
                    };
                    let dot = match command {
                        C::AddToGroup(id) => self.model.groups.iter().find(|g| g.id == id).map(|g| engine::menu::Image::Dot(crate::sidebar::group_rgb(g))),
                        _ => None,
                    };
                    let keys = shortcut.and_then(crate::shortcuts::gpui_action).map_or(String::new(), |a| crate::shortcuts::keys(a, cx));
                    let item = Item::action(label, cmd).enabled(enabled).key(keys);
                    if dot.is_some() { item.image(dot) } else { item }
                }
            })
            .collect()
    }

    pub fn folder_menu(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let items = vec![
            Item::action("Rename", Cmd::Rename(RenameTarget::Folder(id))),
            Item::action("Change Icon…", Cmd::ChangeIcon(RenameTarget::Folder(id))),
            Item::action("New Subfolder", Cmd::NewFolder(Some(id))),
            Item::Separator,
            Item::action("Delete Folder…", Cmd::DeleteFolder(id)),
        ];
        self.show_menu(items, window, cx);
    }

    pub fn group_menu(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let items = vec![
            Item::action("Rename", Cmd::Rename(RenameTarget::Group(id))),
            Item::action("Change Icon…", Cmd::ChangeIcon(RenameTarget::Group(id))),
            Item::action("New Tab in Group", Cmd::NewTabInGroup(id)),
            self.group_color_menu(id, Cmd::Group),
            Item::Separator,
            Item::action("Ungroup", Cmd::Ungroup(id)),
            Item::Separator,
            Item::action("Close Group", Cmd::CloseGroup(id)),
        ];
        self.show_menu(items, window, cx);
    }

    pub fn sidebar_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.model.current_space().filter(|s| !s.private).map(|s| s.space.profile_id);
        let profiles = self
            .core
            .disk
            .profiles()
            .unwrap_or_default()
            .into_iter()
            .map(|p| {
                let dot = ui::theme::PROFILE_COLORS.iter().find(|(name, _)| *name == p.color).map(|(_, rgb)| engine::menu::Image::Dot(*rgb));
                Item::action(ui::text::bounded_title(&p.name), Cmd::EditProfile(p.id)).checked(current == Some(p.id)).image(dot)
            })
            .collect();
        let items = vec![
            Item::action("New Tab", Cmd::NewTab).key(crate::shortcuts::keys(&crate::NewTab, cx)),
            Item::action("Reopen Closed Tab", Cmd::ReopenClosed)
                .key(crate::shortcuts::keys(&crate::ReopenClosedTab, cx))
                .enabled(!self.model.closed_tabs.is_empty()),
            Item::Separator,
            Item::action("New Folder", Cmd::NewFolder(None)),
            Item::action("New Group", Cmd::NewGroup(self.model.active)).key(crate::shortcuts::keys(&crate::NewGroup, cx)),
            Item::action("Bookmark All Tabs", Cmd::BookmarkAll).enabled(!self.model.tabs.is_empty()),
            Item::Separator,
            Item::action("Show Tabs in Sidebar", Cmd::ToggleSidebar).key(crate::shortcuts::keys(&crate::ToggleSidebar, cx)).checked(self.sidebar_open),
            Item::action("Auto-Hide Tabs", Cmd::AutoHideSidebar).checked(self.pref(Pref::AutoHideSidebar)),
            Item::Separator,
            Item::action("New Profile…", Cmd::NewProfile),
            Item::Submenu("Edit Profile".into(), profiles),
            Item::Separator,
            Item::action("Close All Tabs", Cmd::CloseAll)
                .key(crate::shortcuts::keys(&crate::CloseAllTabs, cx))
                .enabled(!self.model.ephemeral_keys().is_empty()),
            Item::action("Close Duplicate Tabs", Cmd::CloseDuplicates).enabled(!self.duplicate_tabs().is_empty()),
        ];
        self.show_menu(items, window, cx);
    }

    pub fn space_menu(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let mut items = Vec::new();
        // The space name is edited inline in the sidebar header, which shows the current space only.
        if ix == self.model.space_ix {
            items.push(Item::action("Rename", Cmd::Rename(RenameTarget::Space(self.model.spaces[ix].space.id))));
        }
        items.extend([
            Item::action("Edit Space…", Cmd::EditSpace(ix)),
            Item::action("New Space…", Cmd::NewSpace).key(crate::shortcuts::keys(&crate::NewSpace, cx)),
            Item::Separator,
            Item::action("Delete Space…", Cmd::DeleteSpace(ix)).enabled(self.model.can_delete_space(ix)),
        ]);
        self.show_menu(items, window, cx);
    }

    pub fn shield_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_loaded().map(|rt| rt.profile.shield_level).unwrap_or_default();
        let items = shield::ShieldLevel::ALL
            .into_iter()
            .map(|level| Item::action(crate::toolbar::shield_badge(level), Cmd::ProfileShield(level)).checked(level == current))
            .collect();
        self.show_menu(items, window, cx);
    }

    pub fn toolbar_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let compact = self.pref(Pref::CompactToolbar);
        let mut items = vec![
            Item::action("Classic Toolbar", Cmd::CompactToolbar(false)).checked(!compact),
            Item::action("Compact Toolbar", Cmd::CompactToolbar(true)).checked(compact),
            Item::Separator,
            Item::action("Customize in Settings…", Cmd::Settings).key(crate::shortcuts::keys(&crate::Settings, cx)),
        ];
        if let Some(key) = self.model.active {
            items.push(Item::Separator);
            items.extend(self.site().map(|_| Item::action("Site Settings…", Cmd::SiteSettings)));
            items.push(self.mute_item(key));
        }
        self.show_menu(items, window, cx);
    }

    /// The address at rest: copy it, or go to the address or search on the clipboard.
    pub fn address_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let clipboard = cx.read_from_clipboard().and_then(|item| item.text()).is_some_and(|t| !t.trim().is_empty());
        let mut items = vec![
            Item::action("Copy Link", Cmd::CopyLink(key, false)).key(crate::shortcuts::keys(&crate::CopyLink, cx)),
            Item::action("Copy Clean Link", Cmd::CopyLink(key, true)).key(crate::shortcuts::keys(&crate::CopyCleanLink, cx)),
            Item::action("Paste and Go", Cmd::PasteAndGo).enabled(clipboard),
        ];
        if self.site().is_some() {
            items.extend([Item::Separator, Item::action("Site Settings…", Cmd::SiteSettings)]);
        }
        self.show_menu(items, window, cx);
    }

    fn mute_item(&self, key: TabKey) -> Item<Cmd> {
        let host = self.tab_url(key).and_then(|u| super::tab_host(&u));
        Item::action(crate::tabs::state_label(&crate::menubar::ToggleMute, self.tab_muted(key)).unwrap_or_default(), Cmd::ToggleMute(key))
            .enabled(host.is_some())
    }

    /// The menu runs AppKit's modal tracking loop, so it opens from a task, outside this update.
    /// Commands run in the main window, also when the menu opened over the floating sidebar.
    /// A start card pill's `tab_stop` blurs on press; while the command field is typing, give it the
    /// keyboard back so typing continues after the menu.
    pub(crate) fn keep_command_focus(&mut self, typing: bool, window: &mut Window, cx: &mut Context<Self>) {
        if typing {
            window.focus(&gpui::Focusable::focus_handle(self.command.read(cx), cx));
        }
    }

    fn show_menu(&mut self, items: Vec<Item<Cmd>>, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let at = self.menu_at.take().and_then(below);
        let Some(host) = self.host.clone() else { return };
        let main = self.peek.main.filter(|_| self.peek.is_window(window));
        self.menu_open = true;
        cx.spawn_in(window, async move |this, cx| {
            let (tx, rx) = oneshot::channel();
            menu::show(&host, crate::menu_glyphs::with_glyphs(items), at, move |cmd| {
                tx.send(cmd).ok();
            });
            this.update(cx, |b, _| b.menu_open = false).ok();
            if let Ok(cmd) = rx.await {
                match main {
                    Some(main) => main.update(cx, |b, window, cx| b.run(cmd, window, cx)).ok(),
                    None => this.update_in(cx, |b, window, cx| b.run(cmd, window, cx)).ok(),
                };
            }
        })
        .detach();
    }

    /// The Edit menu on the page when no field of the shell has the keyboard; on macOS the
    /// menu's own commands reach the page through the responder chain.
    #[cfg(windows)]
    pub(crate) fn edit_page(&mut self, command: engine::chromium::EditCommand, window: &Window) {
        if let Some(view) = self.active_view().filter(|_| self.focus.is_focused(window)) {
            view.edit(command);
        }
    }

    /// Windows has no menu bar, so the toolbar's menu button shows the app's menus as one native
    /// menu and runs the chosen command in this window.
    pub(crate) fn open_app_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let Some(host) = self.host.clone() else { return };
        // This window's menus, not the app-wide ones another window may have set last.
        let (menus, mut marks) = crate::menus(self.dynamic_menus());
        let idle = |a: &dyn gpui::Action| self.idle(a);
        let items = menus
            .into_iter()
            .map(|m| {
                let marked: Vec<Marked> = marks
                    .iter_mut()
                    .find(|(name, _)| m.name == *name)
                    .map(|(_, marks)| marks.drain(..).map(|k| (k.title, k.checked, k.image)).collect())
                    .unwrap_or_default();
                app_menu(m.owned(), &mut marked.into_iter().peekable(), &idle, cx)
            })
            .collect();
        let button = self.app_menu_anchor.get();
        let at = below(button);
        self.menu_open = true;
        cx.spawn_in(window, async move |this, cx| {
            let (tx, rx) = oneshot::channel();
            menu::show(&host, items, at, move |action: Box<dyn gpui::Action>| {
                tx.send(action).ok();
            });
            this.update(cx, |b, _| b.menu_open = false).ok();
            if let Ok(action) = rx.await {
                this.update_in(cx, |_, window, cx| window.dispatch_action(action, cx)).ok();
            }
        })
        .detach();
    }

    fn run(&mut self, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        match cmd {
            Cmd::Open(key) => self.switch_to(key, window, cx),
            Cmd::Reload(key) => {
                self.activate(key, window, cx);
                self.reload(cx);
            }
            Cmd::Duplicate(key) => self.duplicate_tab(key, window, cx),
            Cmd::CopyLink(key, clean) => self.copy_link(key, clean, cx),
            Cmd::TogglePin(key) => self.toggle_pin_tab(key, cx),
            Cmd::ToggleTile(key) => {
                if let Some((kind, end)) = self.model.tile_toggle(key) {
                    self.move_tab(key, kind, None, end, cx);
                }
            }
            Cmd::MoveToSpace(key, ix) => self.move_to_space(key, ix, window, cx),
            Cmd::Close(key) => self.close_tab(key, window, cx),
            Cmd::CloseOthers(key) => {
                let keys = self.model.ephemeral_keys().into_iter().filter(|k| *k != key).collect();
                self.close_tabs(keys, window, cx);
            }
            Cmd::CloseBelow(key) => {
                let keys = browser_core::sidebar::tabs_below(&self.model, key);
                self.close_tabs(keys, window, cx);
            }
            Cmd::CloseDuplicates => {
                let keys = self.duplicate_tabs();
                self.close_tabs(keys, window, cx);
            }
            Cmd::NewTab => self.new_tab_page(window, cx),
            Cmd::ReopenClosed => self.reopen_closed_tab(window, cx),
            Cmd::NewFolder(parent) => self.new_folder(parent, window, cx),
            Cmd::ToggleSidebar => self.toggle_sidebar(cx),
            Cmd::AutoHideSidebar => self.set_pref(Pref::AutoHideSidebar, !self.pref(Pref::AutoHideSidebar), cx),
            Cmd::CloseAll => self.close_all_tabs(window, cx),
            Cmd::DeleteFolder(id) => self.confirm_delete_folder(id, window, cx),
            Cmd::NewSpace => self.new_space(window, cx),
            Cmd::DeleteSpace(ix) => self.confirm_delete_space(ix, window, cx),
            Cmd::CompactToolbar(on) => self.set_pref(Pref::CompactToolbar, on, cx),
            Cmd::Settings => self.open_settings_pane(SettingsPane::Appearance, window, cx),
            Cmd::Rename(target) => self.begin_rename(target, window, cx),
            Cmd::ChangeIcon(target) => self.begin_icon(target, window, cx),
            Cmd::EditSpace(ix) => self.open_space_card(Some(ix), window, cx),
            Cmd::NewGroup(tab) => self.new_group(tab, window, cx),
            Cmd::SetGroup(key, group) => self.set_tab_group(key, group, cx),
            Cmd::NewTabInGroup(id) => self.new_tab_in_group(id, window, cx),
            Cmd::Ungroup(id) => self.ungroup(id, cx),
            Cmd::CloseGroup(id) => self.close_group(id, window, cx),
            Cmd::Group(cmd) => self.run_group(cmd, cx),
            Cmd::Bookmark(key) => {
                if let Some(id) = self.bookmark_tab(Some(key), cx) {
                    self.show_library(Some(id), window, cx);
                }
            }
            Cmd::BookmarkAll => self.bookmark_all_tabs(cx),
            Cmd::NewProfile => self.open_profile_editor(None, window, cx),
            Cmd::EditProfile(id) => self.open_profile_editor(Some(id), window, cx),
            Cmd::ToggleMute(key) => self.toggle_site_muted(key, cx),
            Cmd::ResetTitle(key) => self.reset_tab_title(key, cx),
            Cmd::ReturnToPinned(key) => self.return_to_pinned(key, window, cx),
            Cmd::SetPinnedUrl(key) => self.set_pinned_url(key, cx),
            Cmd::Sleep(keys) => self.sleep_tabs(keys, cx),
            Cmd::SiteSettings => self.toggle_site_settings(window, cx),
            Cmd::ProfileShield(level) => {
                if self.current_loaded().is_some_and(|rt| rt.profile.shield_level != level) {
                    self.set_profile_shield(level, window, cx);
                }
            }
            Cmd::PasteAndGo => {
                self.open_command(super::CommandMode::Current, window, cx);
                self.command.update(cx, |input, cx| input.paste_and_go(window, cx));
            }
            Cmd::Split(key) => self.split_with(key, window, cx),
            Cmd::LeaveSplit(key) => self.leave_split(key, window, cx),
            Cmd::RotateSplit(key) => self.rotate_split(key, cx),
            Cmd::CloseSplit(key) => self.close_split(key, cx),
            Cmd::SwitchWebEngine(key, engine) => self.switch_web_engine(key, engine, window, cx),
        }
    }

    pub(crate) fn tab_url(&self, key: TabKey) -> Option<String> {
        self.views.get(&key).and_then(|v| v.url()).map(String::from).or_else(|| self.model.tab(key).map(|t| t.url.clone()))
    }

    /// What reopening a tab needs; called before an unpinned tab is deleted.
    pub(super) fn closed_tab(&self, tab: &Tab, key: TabKey) -> ClosedTab {
        let url = self.tab_url(key).unwrap_or_else(|| tab.url.clone());
        let restorable = self.level_for_tab(key, &url).policy().save_interaction_state;
        let live = self.views.get(&key).and_then(|v| v.interaction_state());
        let state = live.or_else(|| self.store(key.private).interaction_state(tab.id).ok().flatten()).filter(|_| restorable);
        let group = tab.group_id.and_then(|id| Some((id, self.model.groups.iter().find(|g| g.id == id)?.name.clone())));
        ClosedTab { url, title: tab.title.clone(), state, position: tab.position, group, web_engine: tab.web_engine }
    }

    pub fn reopen_closed_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reopen_closed(None, window, cx);
    }

    /// Reopens the entry with `serial`, or the newest one.
    pub fn reopen_closed(&mut self, serial: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let entry = match serial {
            Some(serial) => self.model.closed_tabs.take(serial, &self.model.spaces),
            None => self.model.closed_tabs.pop(&self.model.spaces),
        };
        let Some((ix, closed)) = entry else { return };
        if ix != self.model.space_ix {
            self.switch_space(ix, window, cx);
        }
        let private = closed.private;
        let last = closed.restore(&self.core);
        self.reload_space();
        match last {
            Some(id) => self.activate(TabKey { private, id }, window, cx),
            None => cx.notify(),
        }
    }

    pub fn duplicate_tab(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.tab_url(key).and_then(|u| Url::parse(&u).ok()) {
            let engine = self.model.tab(key).and_then(|t| t.web_engine);
            // A copy of a file the user opened shows that file too.
            if self.page(key).opened_file.as_ref() == Some(&url) && !self.model.burner_shown() {
                self.new_file_tab(&url, engine, window, cx);
            } else {
                self.new_tab_in(&url, engine, window, cx);
            }
            if let Some(copy) = self.model.active.filter(|k| *k != key) {
                self.model.place_copy(&self.core, copy, key);
                self.reload_space();
                cx.notify();
            }
        }
    }

    /// Copies the tab's URL, with `clean` without its tracking parameters, which count on the tab's Shield card.
    pub fn copy_link(&mut self, key: TabKey, clean: bool, cx: &mut Context<Self>) {
        let Some(url) = self.tab_url(key) else { return };
        let (url, removed) = if clean { shield::url_clean::clean_link(&url) } else { (url, 0) };
        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
        if let Some(profile_id) = self.profile_of(key).filter(|_| removed > 0) {
            let host = Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned));
            self.count_shield(key.private, profile_id, Counter::TrackingParams, removed, host.as_deref(), cx);
        }
        let text = match removed {
            0 => "Link copied".to_owned(),
            1 => "Link copied, 1 tracking parameter removed".to_owned(),
            n => format!("Link copied, {n} tracking parameters removed"),
        };
        self.show_toast_as(browser_core::toast::Tone::Success, text, None, cx);
    }

    pub fn toggle_pin_tab(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if let Some((kind, end)) = self.model.pin_toggle(key) {
            self.move_tab(key, kind, None, end, cx);
        }
    }

    pub(crate) fn close_tabs(&mut self, keys: Vec<TabKey>, window: &mut Window, cx: &mut Context<Self>) {
        if self.peek.is_window(window) {
            self.in_main(window, cx, move |b, window, cx| b.close_tabs(keys, window, cx));
            return;
        }
        self.close_batch(keys, None, window, cx);
    }

    /// Closes the tabs as one entry of the reopen stack and offers to undo it. The space reloads
    /// once, and the next active tab is the nearest one that stays open.
    pub(crate) fn close_batch(&mut self, keys: Vec<TabKey>, group: Option<ClosedGroup>, window: &mut Window, cx: &mut Context<Self>) {
        let Some((private, profile_id, space_id)) = self.model.current_space().map(|s| (s.private, s.space.profile_id, s.space.id)) else { return };
        // Closing the burner's tab leaves the burner, so whether it was shown is read first.
        let burner = self.model.burner_shown();
        let active = self.model.active.filter(|a| keys.contains(a));
        // A split that keeps two panes keeps the focus too.
        let next =
            active.and_then(|a| self.splits.heir(a, &keys).or_else(|| self.model.close_neighbour(a, &keys, self.pref(crate::prefs::Pref::ClosePrevious))));
        self.hide_hover_card(cx);
        let mut tabs: Vec<ClosedTab> = keys.iter().filter_map(|key| self.close_one(*key, cx)).collect();
        self.reload_space();
        if burner && !tabs.is_empty() {
            self.leave_burner(window, cx);
        } else if active.is_some() && self.model.active.is_none() {
            // An unloaded pinned tab stays listed and must not be picked straight back.
            let next = next.filter(|k| self.model.tab(*k).is_some() && !crate::shared::live_elsewhere(*k, cx));
            match next.or_else(|| self.model.pick(|k| keys.contains(&k) || crate::shared::live_elsewhere(k, cx))) {
                Some(next) => self.activate(next, window, cx),
                None => self.model.active = None,
            }
        }
        // SQLite reuses a deleted tab's id, so no other window may keep a view under it.
        Self::sync_other_windows(cx);
        cx.notify();
        tabs.sort_by_key(|t| t.position);
        // A burner tab never goes on the reopen stack, nor does a group closed in the burner.
        if burner || (tabs.is_empty() && group.is_none()) {
            return;
        }
        let text = browser_core::toast::closed(group.is_some(), tabs.len());
        let serial = self.model.closed_tabs.push(Closed { private, profile_id, space_id, tabs, group });
        self.show_toast(text, Some(crate::toast::Action::Undo(serial)), cx);
    }

    /// Archives the tabs unused for longer than Settings › Tabs allows; the number archived.
    pub(crate) fn archive_unused_tabs(&mut self, cx: &mut Context<Self>) -> usize {
        let Some(max_age) = crate::prefs::archive_after(&self.core.disk) else { return 0 };
        let (core, now) = (self.core.clone(), now());
        let archives = self.model.archive_unused(&core, max_age, now);
        let changed = !archives.is_empty();
        let mut archived = 0;
        for archive in archives {
            archived += archive.stale().len();
            let private = archive.private();
            let urls = archive.stale().iter().map(|t| self.tab_url(TabKey { private, id: t.id }).unwrap_or_else(|| t.url.clone())).collect();
            let emptied = archive.run(&core, now, urls, |key| {
                self.discard_tab(key, cx);
            });
            self.forget_groups(private, &emptied, cx);
        }
        if changed {
            self.reload_space();
            self.reload_bookmarks(cx);
            Self::sync_other_windows(cx);
        }
        archived
    }

    /// Sorts the space's loose tabs into groups by what each page is and archives tabs unused for
    /// longer than Settings › Tabs allows, all on this device.
    pub fn clean_up_tabs(&mut self, cx: &mut Context<Self>) {
        if self.model.burner_shown() {
            return;
        }
        let archived = self.archive_unused_tabs(cx);
        let Some((private, space_id)) = self.model.current_space().map(|s| (s.private, s.space.id)) else { return };
        let store = self.store(private);
        let (tabs, groups) = match (store.tabs(space_id), store.groups(space_id)) {
            (Ok(tabs), Ok(groups)) => (tabs, groups),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("tarn: Clean Up could not read the space: {e}");
                return self.show_toast_as(browser_core::toast::Tone::Warning, "Could not sort the tabs", None, cx);
            }
        };
        let sort = browser_core::cleanup::plan(&tabs, &groups, |t| self.tab_url(TabKey { private, id: t.id }).unwrap_or_else(|| t.url.clone()));
        let applied = if sort.changed { browser_core::cleanup::apply(self.store(private), space_id, &sort).map(Some) } else { Ok(None) };
        if let Err(e) = &applied {
            eprintln!("tarn: Clean Up could not sort the tabs: {e}");
        }
        let text = sort.summary(applied.is_ok(), archived, crate::prefs::archive_after(&self.core.disk).is_some());
        let tone = if applied.is_ok() { browser_core::toast::Tone::Info } else { browser_core::toast::Tone::Warning };
        let undo = applied.ok().flatten();
        if sort.changed {
            self.reload_space();
            Self::sync_other_windows(cx);
        }
        self.show_toast_as(tone, text, undo.map(|undo| crate::toast::Action::UndoSort(Box::new((private, undo)))), cx);
        cx.notify();
    }

    /// Puts back what Clean Up sorted, in whichever space it sorted.
    pub(crate) fn undo_sort(&mut self, (private, undo): (bool, browser_core::cleanup::Undo), cx: &mut Context<Self>) {
        match browser_core::cleanup::undo(self.store(private), &undo) {
            Ok(dropped) => self.forget_groups(private, &dropped, cx),
            Err(e) => {
                eprintln!("tarn: Clean Up could not undo the sort: {e}");
                self.show_toast_as(browser_core::toast::Tone::Warning, "Could not undo the sort", None, cx);
            }
        }
        self.reload_space();
        Self::sync_other_windows(cx);
    }

    pub fn close_all_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keys = self.model.ephemeral_keys();
        self.close_tabs(keys, window, cx);
    }

    /// Drops a tab's live state and its row without remembering it as closed.
    pub(crate) fn discard_tab(&mut self, key: TabKey, cx: &mut Context<Self>) -> bool {
        let was_active = self.model.active == Some(key);
        self.splits.leave(key);
        self.hibernate(key, cx);
        self.pages.remove(&key);
        was_active
    }

    pub fn move_to_space(&mut self, key: TabKey, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.model.tab(key).cloned().filter(|_| self.model.same_profile(&self.core, key, ix)) else { return };
        let url = self.tab_url(key).unwrap_or(tab.url);
        let was_active = self.discard_tab(key, cx);
        self.model.move_to_space(&self.core, key, ix, &url);
        self.reload_space();
        if was_active {
            self.pick_active(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn tear_off(&mut self, key: TabKey, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.model.tab(key).cloned() else { return };
        let url = self.tab_url(key).unwrap_or(tab.url);
        let Some(torn) = self.model.tear_off(&self.core, key, &url, self.pref(Pref::TearOffTemporary)) else { return };
        self.reload_space();
        if torn.confined.is_some() {
            Self::sync_other_windows(cx);
        }
        let [x, y, width, height] = crate::shared::window_frame(window);
        let (space_id, active_tab_id) = torn.place(&self.core);
        let row = SavedWindow {
            web_engine: self.saved.web_engine,
            x: x + f64::from(at.x) - 120.,
            y: y + f64::from(at.y) - 20.,
            width,
            height,
            space_id,
            active_tab_id,
            ..Default::default()
        };
        let (from, display) = (window.window_handle().downcast::<Browser>(), window.display(cx).map(|d| d.id()));
        cx.defer(move |cx| crate::shared::open_torn(torn, row, display, from, cx));
        cx.notify();
    }

    fn confirm(&mut self, message: &str, window: &mut Window, cx: &mut Context<Self>, then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        let answer = window.prompt(PromptLevel::Warning, message, None, &["Delete", "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |b, window, cx| then(b, window, cx)).ok();
            }
        })
        .detach();
    }

    fn confirm_delete_folder(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self.model.folders.iter().find(|f| f.id == id) else { return };
        let count = self.model.folder_contents(id).0.len();
        if count == 0 {
            return self.delete_folder(id, window, cx);
        }
        let message = format!("Delete “{}” and the {count} tabs in it?", ui::text::bounded_title(&folder.name));
        self.confirm(&message, window, cx, move |b, window, cx| b.delete_folder(id, window, cx));
    }

    pub fn delete_folder(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let Some((private, _)) = self.model.profile() else { return };
        let (keys, tree) = self.model.folder_contents(id);
        let mut was_active = false;
        for key in keys {
            was_active |= self.discard_tab(key, cx);
        }
        self.store(private).delete_folder(id).ok();
        Self::sync_other_windows(cx);
        self.model.expanded.retain(|f| !tree.contains(f));
        self.reload_space();
        if was_active {
            self.pick_active(window, cx);
        }
        cx.notify();
    }

    fn confirm_delete_space(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(space) = self.model.spaces.get(ix).filter(|_| self.model.can_delete_space(ix)) else { return };
        let (private, id) = (space.private, space.space.id);
        let message = format!("Delete the space “{}” and all its tabs?", ui::text::bounded_title(&space.space.name));
        self.confirm(&message, window, cx, move |b, window, cx| b.delete_space(private, id, window, cx));
    }

    pub fn delete_space(&mut self, private: bool, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let Some((ix, keys)) = self.model.space_to_delete(&self.core, private, id) else { return };
        let current = self.model.current_space().map(|s| (s.private, s.space.id));
        for key in keys {
            self.discard_tab(key, cx);
        }
        if self.store(private).delete_space(id).is_err() {
            return;
        }
        Self::sync_other_windows(cx);
        self.reload_spaces();
        self.model.closed_tabs.retain(&self.model.spaces);
        let next = self.model.spaces.iter().position(|s| Some((s.private, s.space.id)) == current).unwrap_or(ix.saturating_sub(1));
        self.switch_space(next, window, cx);
    }

    pub(crate) fn apply_view_prefs(&self) {
        self.views.values().for_each(|view| self.view_prefs(view));
        if let Some(peek) = &self.link_peek.view {
            peek.set_passkeys(self.pref(Pref::Passkeys));
        }
    }

    pub(crate) fn view_prefs(&self, view: &engine::WebView) {
        view.set_gestures(self.pref(Pref::Gestures));
        view.set_passkeys(self.pref(Pref::Passkeys));
    }

    /// The mouse's back and forward buttons, which web views ignore.
    pub(super) fn watch_navigation_buttons(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (tx, mut rx) = mpsc::unbounded();
        self.nav_buttons = self.host.as_ref().and_then(|h| h.on_navigation_buttons(move |back| drop(tx.unbounded_send(back))));
        self._tasks.push(cx.spawn_in(window, async move |this, cx| {
            while let Some(back) = rx.next().await {
                if this.update(cx, |b, _| if back { b.back() } else { b.forward() }).is_err() {
                    break;
                }
            }
        }));
        let (tx, mut rx) = mpsc::unbounded();
        self.page_clicks = self.host.as_ref().and_then(|h| h.on_page_mouse_down(move || drop(tx.unbounded_send(()))));
        self._tasks.push(cx.spawn_in(window, async move |this, cx| {
            while rx.next().await.is_some() {
                let saved = this.update(cx, |b, cx| {
                    b.commit_rename(cx);
                    cx.notify();
                });
                if saved.is_err() {
                    break;
                }
            }
        }));
    }
}

/// The lower left corner of a painted button, where a menu it opens drops from.
fn below(button: gpui::Bounds<gpui::Pixels>) -> Option<(f64, f64)> {
    (button.size.width > px(0.)).then(|| (f32::from(button.origin.x).into(), f32::from(button.bottom()).into()))
}

/// A menu entry's title, check mark and image, as `crate::menus` gives them.
type Marked = (String, bool, Option<engine::menu::Image>);

/// One of the app's menus as a native submenu with its shortcuts and the check marks the menu
/// bar would show, matched in order by title (`menubar::Dynamic::marks`); headings and commands
/// with nothing to do here (`idle`) are greyed, macOS-only commands and system menus left out, and so are
/// separators that would end up doubled or at an edge.
fn app_menu(
    menu: gpui::OwnedMenu,
    marks: &mut std::iter::Peekable<std::vec::IntoIter<Marked>>,
    idle: &dyn Fn(&dyn gpui::Action) -> bool,
    cx: &gpui::App,
) -> Item<Box<dyn gpui::Action>> {
    let mut items: Vec<Item<Box<dyn gpui::Action>>> = Vec::new();
    for item in menu.items {
        let next = match item {
            gpui::OwnedMenuItem::Separator => Item::Separator,
            gpui::OwnedMenuItem::Submenu(submenu) => {
                marks.next_if(|(title, ..)| *title == submenu.name.as_ref());
                app_menu(submenu, marks, idle, cx)
            }
            gpui::OwnedMenuItem::SystemMenu(_) => continue,
            gpui::OwnedMenuItem::Action { name, action, .. } if crate::shortcuts::macos_only(&*action) => {
                marks.next_if(|(title, ..)| *title == name);
                continue;
            }
            gpui::OwnedMenuItem::Action { name, action, .. } => {
                let (checked, image) = marks.next_if(|(title, ..)| *title == name).map_or((false, None), |(_, on, image)| (on, image));
                let idle = idle(&*action)
                    || action.partial_eq(&crate::menubar::Heading)
                    || (action.partial_eq(&crate::CheckForUpdates) && crate::shared::core(cx).updater.is_none())
                    || (action.partial_eq(&crate::RestartToUpdate)
                        && crate::shared::core(cx).updater.as_ref().and_then(platform::updater::Updater::ready_version).is_none());
                let keys = crate::shortcuts::keys(&*action, cx);
                Item::action(name, action).key(keys).checked(checked).enabled(!idle).image(image)
            }
        };
        if matches!(next, Item::Separator) && items.last().is_none_or(|last| matches!(last, Item::Separator)) {
            continue;
        }
        items.push(next);
    }
    if matches!(items.last(), Some(Item::Separator)) {
        items.pop();
    }
    Item::Submenu(menu.name.to_string(), items)
}
