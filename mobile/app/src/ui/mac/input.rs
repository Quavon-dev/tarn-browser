use browser_core::shortcuts::{self, Action, Keyboard};
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use serde_json::Value;

use super::super::{Screen, Ui, bars};

const ALWAYS: KeyOptions = KeyOptions { when: When::Always, repeat: false, pass_through: false };

fn select_tab(ui: Ui, action: Action) {
    let target = ui.read(|s| match action {
        Action::SelectTab(n) => s.model.slot(usize::from(n)),
        Action::LastTab => s.model.slot(9),
        Action::NextTab => s.model.cycled(true),
        Action::PreviousTab => s.model.cycled(false),
        _ => None,
    });
    if let Some(key) = target {
        ui.act(|s| {
            s.activate(key);
            Vec::new()
        });
        ui.show(Screen::Browse);
    }
}

pub(super) fn keyboard() -> Keyboard {
    if apple_keys() { Keyboard::Apple } else { Keyboard::Other }
}

pub(super) fn offered(action: Action) -> bool {
    !matches!(
        action,
        Action::CloseAllTabs
            | Action::NewGroup
            | Action::ToggleSplitView
            | Action::FocusNextPane
            | Action::FocusPreviousPane
            | Action::SwapPanes
            | Action::RotateSplitLayout
            | Action::RemoveFromSplitView
            | Action::StopLoading
            | Action::FindNext
            | Action::FindPrevious
            | Action::CopyMarkdownLink
            | Action::TakeScreenshot
            | Action::PictureInPicture
            | Action::Print
            | Action::BookmarkTab
            | Action::ToggleLibrary
            | Action::ManageBookmarks
            | Action::NewWindow
            | Action::CloseWindow
            | Action::Minimize
            | Action::ToggleFullScreen
            | Action::HideTarn
            | Action::HideOthers
            | Action::ToggleShortcuts
    )
}

fn zoom(ui: Ui, delta: f64) {
    ui.act(|s| s.zoom_by(delta));
}

fn run(ui: Ui, action: Action) {
    match action {
        Action::NewTab => bars::open_new_tab(ui),
        Action::OpenFile => super::super::panels::open_file(ui),
        Action::OpenLocation | Action::CommandBar => {
            ui.show(Screen::Browse);
            ui.edit_address();
        }
        Action::CloseTab => ui.act(|s| {
            if let Some(key) = s.model.active {
                s.close_or_unload(key);
            }
            Vec::new()
        }),
        Action::ReopenClosedTab => ui.act(|s| {
            if !s.reopen_closed() {
                s.notice = Some("No closed tab to reopen.".into());
            }
            Vec::new()
        }),
        Action::NextTab | Action::PreviousTab | Action::LastTab | Action::SelectTab(_) => select_tab(ui, action),
        Action::Reload => ui.act(|s| s.active_command("reload", Value::Null)),
        Action::ReloadFromOrigin => ui.act(|s| s.active_command("reloadFromOrigin", Value::Null)),
        Action::Back => ui.act_on_screen(|s| s.active_command("back", Value::Null)),
        Action::Forward => ui.act_on_screen(|s| s.active_command("forward", Value::Null)),
        Action::Find => ui.show(Screen::Find),
        Action::FindInTabs => ui.toggle(Screen::FindTabs),
        Action::ZoomIn => zoom(ui, 0.1),
        Action::ZoomOut => zoom(ui, -0.1),
        Action::ZoomReset => zoom(ui, 0.0),
        Action::CopyLink => ui.copy_link(None, false),
        Action::CopyCleanLink => ui.copy_link(None, true),
        Action::ToggleReader => ui.act(|s| if s.offers_reader() { s.open_reader() } else { Vec::new() }),
        Action::ShowHistory => ui.toggle(Screen::History),
        Action::ToggleDownloads => ui.toggle(Screen::Downloads),
        Action::NewSpace if ui.read(|s| s.can_add_space()) => ui.toggle(Screen::SpaceNew),
        Action::NewSpace => {}
        Action::PrivateSpace => {
            ui.act(|s| {
                if s.model.private() {
                    s.close_private()
                } else {
                    s.enter_private()
                }
                Vec::new()
            });
            ui.show(Screen::Browse);
        }
        Action::BurnerTab => {
            ui.act(|s| {
                if !s.in_burner() && !<crate::data::Data as crate::data::Platform>::private_process() {
                    s.open_burner();
                }
                Vec::new()
            });
            ui.show(Screen::Browse);
        }
        Action::Settings => ui.toggle(Screen::Settings),
        Action::SwitchSpace(n) => {
            let Some(ix) = ui.read(|s| s.spaces().get(usize::from(n) - 1).map(|i| i.ix)) else { return };
            ui.act(|s| {
                s.choose_space(ix, browser_core::now());
                Vec::new()
            });
            ui.show(Screen::Browse);
        }
        Action::TogglePin => ui.act(|s| {
            if let Some(key) = s.model.active {
                s.toggle_pin(key);
            }
            Vec::new()
        }),
        Action::ToggleSidebar => ui.act(|s| {
            s.toggle_sidebar();
            Vec::new()
        }),
        Action::CloseAllTabs
        | Action::NewGroup
        | Action::ToggleSplitView
        | Action::FocusNextPane
        | Action::FocusPreviousPane
        | Action::SwapPanes
        | Action::RotateSplitLayout
        | Action::RemoveFromSplitView
        | Action::StopLoading
        | Action::FindNext
        | Action::FindPrevious
        | Action::CopyMarkdownLink
        | Action::TakeScreenshot
        | Action::PictureInPicture
        | Action::Print
        | Action::BookmarkTab
        | Action::ToggleLibrary
        | Action::ManageBookmarks
        | Action::NewWindow
        | Action::CloseWindow
        | Action::Minimize
        | Action::ToggleFullScreen
        | Action::HideTarn
        | Action::HideOthers
        | Action::ToggleShortcuts => {}
    }
}

pub(super) fn bindings(ui: Ui) -> impl IntoElement {
    for (keys, action) in shortcuts::bindings(keyboard(), offered) {
        on_key_with(&keys, ALWAYS, move |_| run(ui, action));
    }
    view().child(when(
        move || ui.editing.get(),
        move || {
            on_key_with("escape", ALWAYS, move |_| bars::cancel(ui));
            view()
        },
    ))
}
