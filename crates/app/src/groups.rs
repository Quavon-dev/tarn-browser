use std::time::Instant;

use browser_core::closed::ClosedGroup;
use engine::menu::Item;
use gpui::{AppContext, Context, Entity, Focusable, Subscription, Window};
use store::Id;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::PROFILE_COLORS;

use crate::browser::{Browser, CommandMode, TabKey};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenameTarget {
    Folder(Id),
    Group(Id),
    Space(Id),
    Tab(Id),
}

#[derive(Clone, Copy)]
pub enum GroupCmd {
    Color(Id, &'static str),
}

pub(crate) fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

pub struct Rename {
    pub target: RenameTarget,
    /// Edits the icon of a group or folder, shown in its picker, instead of the name.
    pub icon: bool,
    pub input: Entity<TextInput>,
    _subscriptions: [Subscription; 3],
}

impl Browser {
    pub fn new_group(&mut self, tab: Option<TabKey>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group) = self.model.new_group(&self.core, tab) else { return };
        self.reload_space();
        self.begin_rename(RenameTarget::Group(group.id), window, cx);
    }

    pub fn toggle_group(&mut self, id: Id, cx: &mut Context<Self>) {
        if !self.model.groups.iter().any(|g| g.id == id) {
            return;
        }
        if self.model.toggle_group(&self.core, id) {
            self.motion.toggled_group = Some((id, Instant::now()));
        }
        cx.notify();
    }

    /// The "Color" submenu of a group's context menu; `cmd` wraps each choice in the menu's command type.
    pub(crate) fn group_color_menu<T>(&self, id: Id, cmd: impl Fn(GroupCmd) -> T) -> Item<T> {
        let current = self.model.groups.iter().find(|g| g.id == id).map(|g| g.color.as_str());
        let colors = PROFILE_COLORS.iter().map(|(name, rgb)| {
            Item::action(capitalized(name), cmd(GroupCmd::Color(id, name))).checked(current == Some(*name)).image(Some(engine::menu::Image::Dot(*rgb)))
        });
        Item::Submenu("Color".into(), colors.collect())
    }

    pub(crate) fn run_group(&mut self, cmd: GroupCmd, cx: &mut Context<Self>) {
        match cmd {
            GroupCmd::Color(id, color) => {
                self.store(self.model.private()).set_group_color(id, color).ok();
            }
        }
        self.reload_space();
        cx.notify();
    }

    pub fn move_group(&mut self, id: Id, index: usize, cx: &mut Context<Self>) {
        let private = self.model.private();
        if self.store(private).move_group(id, index).is_ok() {
            self.reload_space();
            cx.notify();
        }
    }

    pub fn close_group(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.close_group(id, window, cx)) {
            return;
        }
        let Some(group) =
            self.model.groups.iter().find(|g| g.id == id).map(|g| ClosedGroup { name: g.name.clone(), color: g.color.clone(), icon: g.icon.clone() })
        else {
            return;
        };
        let keys = self.model.tabs.iter().filter(|t| t.group_id == Some(id)).map(|t| self.model.key(t)).collect();
        // Closing may switch space (leaving a burner), so the group's store is chosen first.
        let private = self.model.private();
        self.close_batch(keys, Some(group), window, cx);
        self.store(private).delete_group(id).ok();
        self.forget_groups(private, &[id], cx);
        self.reload_space();
        cx.notify();
    }

    pub fn ungroup(&mut self, id: Id, cx: &mut Context<Self>) {
        let private = self.model.private();
        self.store(private).ungroup(id).ok();
        self.forget_groups(private, &[id], cx);
        self.reload_space();
        cx.notify();
    }

    /// Deleted groups leave every window's reopen stack, since a new group can take their ids.
    pub(crate) fn forget_groups(&mut self, private: bool, ids: &[Id], cx: &mut Context<Self>) {
        for id in ids {
            self.model.closed_tabs.forget_group(private, *id);
        }
        let ids = ids.to_vec();
        crate::shared::others(cx, move |b, _, _| ids.iter().for_each(|id| b.model.closed_tabs.forget_group(private, *id)));
    }

    pub fn new_tab_in_group(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        self.open_command(CommandMode::NewTab, window, cx);
        self.new_tab_group = Some(id);
    }

    pub fn set_tab_group(&mut self, key: TabKey, group: Option<Id>, cx: &mut Context<Self>) {
        if self.store(key.private).set_tab_group(key.id, group).is_ok() {
            self.reload_space();
            cx.notify();
        }
    }

    /// Shows an inline name field with the current name selected; Enter or blur saves, Esc keeps the name.
    pub fn begin_rename(&mut self, target: RenameTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit(target, false, window, cx);
    }

    /// Shows the icon picker of a group or folder: symbols to click and a field for one emoji.
    pub fn begin_icon(&mut self, target: RenameTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_edit(target, true, window, cx);
    }

    fn begin_edit(&mut self, target: RenameTarget, icon: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(peek) = self.peek.window.filter(|_| !self.peek.is_window(window)) {
            let browser = cx.entity();
            cx.defer(move |cx| {
                peek.update(cx, |_, window, cx| browser.update(cx, |b, cx| b.begin_edit(target, icon, window, cx))).ok();
            });
            return;
        }
        let emoji = |stored: &str| browser_core::icon::emoji(stored).to_owned();
        let text = match (target, icon) {
            (RenameTarget::Folder(id), true) => self.model.folders.iter().find(|f| f.id == id).map(|f| emoji(&f.icon)),
            (RenameTarget::Group(id), true) => self.model.groups.iter().find(|g| g.id == id).map(|g| emoji(&g.icon)),
            (_, true) => None,
            (RenameTarget::Folder(id), false) => self.model.folders.iter().find(|f| f.id == id).map(|f| f.name.clone()),
            (RenameTarget::Group(id), false) => self.model.groups.iter().find(|g| g.id == id).map(|g| g.name.clone()),
            (RenameTarget::Space(id), false) => self.model.current_space().filter(|s| s.space.id == id).map(|s| s.space.name.clone()),
            (RenameTarget::Tab(id), false) => self.model.tabs.iter().find(|t| t.id == id).map(crate::tabs::display_title),
        };
        let Some(name) = text else { return };
        self.commit_rename(cx);
        let input = cx.new(|cx| {
            let mut input = TextInput::new(if icon { "Emoji" } else { "Name" }, cx);
            input.set_text(name, cx);
            input
        });
        let events = cx.subscribe_in(&input, window, |b, _, event, window, cx| match event {
            TextInputEvent::Submit => b.end_rename(true, window, cx),
            TextInputEvent::Cancel => b.end_rename(false, window, cx),
            TextInputEvent::MenuClosed if !window.is_window_active() => b.end_rename(true, window, cx),
            _ => {}
        });
        let focus = input.focus_handle(cx);
        let blur = cx.on_blur(&focus, window, move |b, window, cx| {
            if !icon || window.focused(cx).is_some() {
                b.end_rename(true, window, cx);
            }
        });
        let menu_input = input.clone();
        let deactivate = cx.observe_window_activation(window, move |b, window, cx| {
            if !window.is_window_active() && !menu_input.read(cx).menu_open() {
                b.end_rename(true, window, cx);
            }
        });
        if let Some(host) = &self.host {
            host.focus();
        }
        if self.peek.is_window(window) {
            window.activate_window();
        }
        window.focus(&focus);
        if !self.peek.is_window(window) {
            self.slide_sidebar(true);
        }
        self.renaming = Some(Rename { target, icon, input, _subscriptions: [events, blur, deactivate] });
        cx.notify();
    }

    /// Saves a rename in progress; called before anything that would change which store it belongs to.
    pub(crate) fn commit_rename(&mut self, cx: &mut Context<Self>) {
        self.finish_rename(true, cx);
    }

    pub(crate) fn end_rename(&mut self, commit: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_rename(commit, cx);
        window.focus(&self.focus_handle(cx));
        if let Some(main) = self.peek.main.filter(|_| self.peek.is_window(window)) {
            cx.defer(move |cx| {
                main.update(cx, |_, window, _| window.activate_window()).ok();
            });
        }
        cx.notify();
    }

    fn finish_rename(&mut self, commit: bool, cx: &mut Context<Self>) {
        let Some(rename) = self.renaming.take() else { return };
        let name = rename.input.read(cx).text().trim().to_owned();
        if rename.icon {
            if commit && !name.is_empty() {
                self.set_icon(rename.target, &name, cx);
            }
        } else if commit && (!name.is_empty() || matches!(rename.target, RenameTarget::Tab(_))) {
            let store = self.store(self.model.private());
            match rename.target {
                RenameTarget::Folder(id) => store.rename_folder(id, &name).ok(),
                RenameTarget::Group(id) => {
                    let collapsed = self.model.groups.iter().find(|g| g.id == id).is_some_and(|g| g.collapsed);
                    store.update_group(id, &name, collapsed).ok()
                }
                RenameTarget::Space(id) => store.rename_space(id, &name).ok(),
                RenameTarget::Tab(id) => self.save_tab_title(id, &name),
            };
            self.reload_spaces();
            self.reload_space();
        }
    }

    pub fn rename_input(&self, target: RenameTarget) -> Option<Entity<TextInput>> {
        self.renaming.as_ref().filter(|r| r.target == target && !r.icon).map(|r| r.input.clone())
    }

    pub fn icon_input(&self, target: RenameTarget) -> Option<Entity<TextInput>> {
        self.renaming.as_ref().filter(|r| r.target == target && r.icon).map(|r| r.input.clone())
    }

    /// Sets the icon of a group or folder from a symbol name or an emoji; empty clears it, and
    /// text that is neither changes nothing.
    pub fn set_icon(&mut self, target: RenameTarget, text: &str, cx: &mut Context<Self>) {
        let store = self.store(self.model.private());
        let saved = match target {
            RenameTarget::Folder(id) => browser_core::icon::set_folder(store, id, text),
            RenameTarget::Group(id) => browser_core::icon::set_group(store, id, text),
            RenameTarget::Space(_) | RenameTarget::Tab(_) => return,
        };
        match saved {
            Ok(()) => self.reload_space(),
            Err(_) => self.show_toast_as(browser_core::toast::Tone::Warning, "Use one emoji or a listed symbol", None, cx),
        }
        cx.notify();
    }
}
