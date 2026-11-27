use browser_core::shortcuts::{self, Action, Keyboard};
use browser_core::switcher::{COMMIT_DELAY, Switcher};
use browser_core::tabs::TabKey;
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
    activate(ui, target);
}

fn activate(ui: Ui, target: Option<TabKey>) {
    if let Some(key) = target {
        ui.act(|s| {
            s.activate(key);
            Vec::new()
        });
        ui.show(Screen::Browse);
    }
}

fn recent_tab(ui: Ui, forward: bool) {
    let mut open = ui.switcher.get_untracked();
    ui.read(|s| Switcher::press(&mut open, &s.model, forward));
    let Some(presses) = open.as_ref().map(|s| s.presses) else { return };
    if ui.switcher.with_untracked(Option::is_none) {
        ui.show(Screen::Browse);
    }
    ui.switcher.set(open);
    ui.root.run(|| {
        drop(set_timeout(COMMIT_DELAY, move || {
            if ui.switcher.with_untracked(|s| s.as_ref().map(|s| s.presses)) == Some(presses) {
                commit(ui);
            }
        }))
    });
}

fn commit(ui: Ui) {
    let open = ui.switcher.get_untracked();
    ui.switcher.set(None);
    activate(ui, open.map(|s| s.selected()));
}

pub(super) fn switcher(ui: Ui) -> impl IntoElement {
    dynamic(move || {
        let open = ui.switcher.get()?;
        let cards = untracked(|| {
            ui.read(|s| {
                let card = |key: &TabKey| {
                    let url = s.tab_url(*key);
                    tarn_nsh_shared::switcher::Card { icon: s.site_icon(&url), host: super::sidebar::host(&url), title: s.tab_title(*key) }
                };
                open.order.iter().map(card).collect()
            })
        });
        let pick = move |ix| {
            ui.switcher.update(|s| s.iter_mut().for_each(|s| s.index = ix));
            commit(ui);
        };
        Some(tarn_nsh_shared::switcher::strip(cards, open.index, pick, move || commit(ui), move || ui.switcher.set(None)))
    })
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
        Action::RecentTab | Action::RecentTabBack => recent_tab(ui, action == Action::RecentTab),
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
        Action::TakeScreenshot => ui.act(|s| s.take_screenshot()),
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
            tarn_nsh_shared::motion::space_switch();
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
        Action::ToggleTabsOnTop => ui.act(|s| {
            s.toggle_tabs_on_top();
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
    on_key_with("mod+z", KeyOptions { when: When::NotEditing, ..KeyOptions::default() }, move |_| {
        ui.act(|s| {
            s.undo_last();
            Vec::new()
        })
    });
    view()
}
