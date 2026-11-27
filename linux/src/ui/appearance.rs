use std::path::PathBuf;

use nsh_core::prelude::*;
use store::{Id, Store};
use tarn_nsh_shared::appearance::{Host, appearance};

use super::panels::section;
use super::tools::note;
use super::{Ui, act, read};
use crate::browser::COMPACT_TOOLBAR;
use crate::controls::{group, group_row, label, segmented, switch};

#[derive(Clone, Copy)]
struct Looks(Ui);

impl Host for Looks {
    type Transport = crate::fetch::Soup;
    fn transport() -> crate::fetch::Soup {
        crate::fetch::Soup::new()
    }
    fn catalog_dir(self) -> PathBuf {
        crate::browser::themes::catalog_dir()
    }
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R {
        f(&super::core().disk)
    }
    fn changed(self) {
        super::themes_changed();
    }
    fn profile(self) -> Option<(Id, String)> {
        read(self.0, |b| {
            let space = b.model.current_space().filter(|s| !s.private)?;
            Some((space.space.profile_id, b.core.disk.profile(space.space.profile_id).ok()?.name))
        })
        .flatten()
    }
    fn safe(self) -> bool {
        read(self.0, |b| b.themes.safe).unwrap_or_default()
    }
    fn notice(self) -> Option<String> {
        read(self.0, |b| b.themes.active.notice.clone()).flatten()
    }
    fn system_light(self) -> bool {
        !dark()
    }
    fn themes_dir(self) -> PathBuf {
        crate::data::root().join("themes")
    }
    fn desktop(self) -> bool {
        true
    }
    fn accent_and_motion(self) -> bool {
        true
    }
    fn backing(self) -> browser_core::theme::Backing {
        crate::browser::themes::desktop_backing()
    }
}

pub(super) fn pane(ui: Ui) -> AnyElement {
    view().flex_col().child(appearance(Looks(ui))).child(layout(ui)).into_element()
}

fn layout(ui: Ui) -> View {
    let (compact, auto_hide, top) = read(ui, |b| (b.compact_toolbar(), b.auto_hide_sidebar(), b.tabs_on_top())).unwrap_or_default();
    let toolbar = [(false, "Classic".to_owned()), (true, "Compact".to_owned())].to_vec();
    let tabs = [(false, "Sidebar".to_owned()), (true, "Top".to_owned())].to_vec();
    let shortcut = super::shortcut_of(ui, crate::shortcuts::Action::ToggleSidebar).map(|k| format!(" ({k})")).unwrap_or_default();
    view()
        .flex_col()
        .child(section("Layout"))
        .child(
            group()
                .child(group_row().child(label("Tabs").flex_1()).child(segmented(tabs, top, super::set_tabs_on_top)))
                .child(group_row().child(label("Toolbar").flex_1()).child(segmented(toolbar, compact, move |on| {
                    act(ui, move |b| {
                        if b.compact_toolbar() != on {
                            b.toggle_off_setting(COMPACT_TOOLBAR)
                        }
                    })
                })))
                .child(switch(auto_hide, "Hide sidebar automatically", move || super::set_auto_hide_sidebar(ui))),
        )
        .child(note("Tabs on top list them in a strip across the window, with the sidebar hidden."))
        .child(note("Compact puts the address field into a slimmer bar."))
        .child(note(format!("With the sidebar hidden{shortcut}, it slides in over the page while the pointer rests at the window's left edge.")))
}
