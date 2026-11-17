use browser_core::shortcuts::{self, Action, tab_to_select};
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use serde_json::Value;

use super::super::{Screen, Ui, bars};

const ALWAYS: KeyOptions = KeyOptions { when: When::Always, repeat: false, pass_through: false };
const UNBOUND: [Action; 2] = [Action::TogglePin, Action::ToggleSidebar];

fn select_tab(ui: Ui, action: Action) {
    let target = ui.read(|s| {
        let keys: Vec<_> = s.model.tabs.iter().map(|t| s.model.key(t)).collect();
        let active = s.model.active.and_then(|a| keys.iter().position(|k| *k == a));
        tab_to_select(action, keys.len(), active).and_then(|ix| keys.get(ix).copied())
    });
    if let Some(key) = target {
        ui.act(|s| {
            s.activate(key);
            Vec::new()
        });
        ui.show(Screen::Browse);
    }
}

fn zoom(ui: Ui, delta: f64) {
    ui.act(|s| {
        let z = s.active_page().map_or(1.0, |p| p.zoom);
        s.zoom(if delta == 0.0 { 1.0 } else { z + delta })
    });
}

fn copy(link: Option<String>) {
    if let Some(url) = link {
        set_clipboard(url);
    }
}

fn run(ui: Ui, action: Action) {
    match action {
        Action::NewTab => bars::open_new_tab(ui),
        Action::OpenLocation | Action::CommandBar => {
            ui.show(Screen::Browse);
            ui.field.focus();
        }
        Action::CloseTab => ui.act(|s| {
            if let Some(key) = s.model.active {
                s.close_tab(key);
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
        Action::Back => ui.act_on_screen(|s| s.active_command("back", Value::Null)),
        Action::Forward => ui.act_on_screen(|s| s.active_command("forward", Value::Null)),
        Action::Find => ui.show(Screen::Find),
        Action::FindInTabs => ui.toggle(Screen::FindTabs),
        Action::ZoomIn => zoom(ui, 0.1),
        Action::ZoomOut => zoom(ui, -0.1),
        Action::ZoomReset => zoom(ui, 0.0),
        Action::CopyLink => copy(ui.read(|s| s.share_url())),
        Action::CopyCleanLink => {
            let mut link = None;
            ui.act(|s| {
                link = s.copy_clean_link();
                Vec::new()
            });
            copy(link);
        }
        Action::ToggleReader => ui.act(|s| if s.offers_reader() { s.open_reader() } else { Vec::new() }),
        Action::ShowHistory => ui.toggle(Screen::History),
        Action::ToggleDownloads => ui.toggle(Screen::Downloads),
        Action::NewSpace => ui.toggle(Screen::Spaces),
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
        Action::TogglePin | Action::ToggleSidebar => {}
    }
}

pub(super) fn bindings(ui: Ui) -> impl IntoElement {
    for (keys, action) in shortcuts::bindings(apple_keys()).into_iter().filter(|(_, a)| !UNBOUND.contains(a)) {
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
