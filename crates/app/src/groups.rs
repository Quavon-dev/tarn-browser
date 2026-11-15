use std::time::Instant;

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
        let colors = PROFILE_COLORS.iter().map(|(name, _)| Item::action(capitalized(name), cmd(GroupCmd::Color(id, name))).checked(current == Some(*name)));
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
        let Some(group) = self.model.groups.iter().find(|g| g.id == id).map(|g| (g.name.clone(), g.color.clone())) else { return };
        let keys = self.model.tabs.iter().filter(|t| t.group_id == Some(id)).map(|t| self.model.key(t)).collect();
        // Closing may switch space (leaving a burner), so the group's store is chosen first.
        let private = self.model.private();
        self.close_batch(keys, Some(group), window, cx);
        self.store(private).delete_group(id).ok();
        self.reload_space();
        cx.notify();
    }

    pub fn ungroup(&mut self, id: Id, cx: &mut Context<Self>) {
        self.store(self.model.private()).ungroup(id).ok();
        self.reload_space();
        cx.notify();
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
        if let Some(peek) = self.peek.window.filter(|_| !self.peek.is_window(window)) {
            let browser = cx.entity();
            cx.defer(move |cx| {
                peek.update(cx, |_, window, cx| browser.update(cx, |b, cx| b.begin_rename(target, window, cx))).ok();
            });
            return;
        }
        let name = match target {
            RenameTarget::Folder(id) => self.model.folders.iter().find(|f| f.id == id).map(|f| f.name.clone()),
            RenameTarget::Group(id) => self.model.groups.iter().find(|g| g.id == id).map(|g| g.name.clone()),
            RenameTarget::Space(id) => self.model.current_space().filter(|s| s.space.id == id).map(|s| s.space.name.clone()),
            RenameTarget::Tab(id) => self.model.tabs.iter().find(|t| t.id == id).map(crate::tabs::display_title),
        };
        let Some(name) = name else { return };
        self.commit_rename(cx);
        let input = cx.new(|cx| {
            let mut input = TextInput::new("Name", cx);
            input.set_text(name, cx);
            input
        });
        let events = cx.subscribe_in(&input, window, |b, _, event, window, cx| match event {
            TextInputEvent::Submit => b.end_rename(true, window, cx),
            TextInputEvent::Cancel => b.end_rename(false, window, cx),
            _ => {}
        });
        let focus = input.focus_handle(cx);
        let blur = cx.on_blur(&focus, window, |b, window, cx| b.end_rename(true, window, cx));
        let deactivate = cx.observe_window_activation(window, |b, window, cx| {
            if !window.is_window_active() {
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
        if !self.sidebar_open && !self.peek.is_window(window) {
            self.sidebar_open = true;
        }
        self.renaming = Some(Rename { target, input, _subscriptions: [events, blur, deactivate] });
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
        if commit && !name.is_empty() {
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
        self.renaming.as_ref().filter(|r| r.target == target).map(|r| r.input.clone())
    }
}
