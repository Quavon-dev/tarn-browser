use std::path::PathBuf;

use nsh_core::prelude::*;
use store::{Id, Store};
use tarn_nsh_shared::appearance::{Host, Layout, appearance};

use super::{Ui, read};
use crate::controls::{self, label, segmented, small};
use browser_core::prefs::Switch;

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
    fn layout(self) -> Option<Layout> {
        Some(layout(self.0))
    }
}

pub(super) fn pane(ui: Ui) -> AnyElement {
    appearance(Looks(ui))
}

fn choice(title: &str, about: impl Into<String>, control: View) -> View {
    view().flex_col().gap_2().px_3().py_2().child(label(title.to_owned())).child(small(about.into())).child(control)
}

fn toggle(switch: Switch, name: &str, about: &str) -> View {
    let on = switch.on(&super::core().disk);
    view().flex_col().pb_2().child(controls::switch(on, name, move || super::set_pref(switch, !on))).child(view().px_3().child(small(about.to_owned())))
}

fn layout(ui: Ui) -> Layout {
    use browser_core::prefs::{
        COMPACT_TOOLBAR, DIM_SLEEPING, HOVER_CARDS, PAGE_PREVIEWS, SIDEBAR_CARDS, SIDEBAR_CHATS, SIDEBAR_TOOLBAR, SMALL_TILES, UNREAD_DOTS,
    };
    let (auto_hide, top) = read(ui, |b| (b.auto_hide_sidebar(), b.tabs_on_top())).unwrap_or_default();
    let on = move |s: Switch| read(ui, |b| b.pref(s)).unwrap_or(s.default);
    let toolbar = if on(SIDEBAR_TOOLBAR) { 0 } else { 1 + on(COMPACT_TOOLBAR) as usize };
    let previews = if !on(HOVER_CARDS) {
        2
    } else if on(PAGE_PREVIEWS) {
        0
    } else {
        1
    };
    let set = |switch| move |value: bool| super::set_pref(switch, value);
    let options = |names: &[&str]| names.iter().enumerate().map(|(ix, n)| (ix, (*n).to_owned())).collect::<Vec<_>>();
    let shortcut = super::shortcut_of(ui, crate::shortcuts::Action::ToggleSidebar).map(|k| format!(" ({k})")).unwrap_or_default();
    Layout {
        lead: vec![
            choice(
                "Design",
                "Where Back, Forward and the address field sit: at the top of the sidebar, in a bar above the page, or in a slimmer bar. The sidebar below looks the same in all three; with tabs on top, or the sidebar hidden, Sidebar shows as Compact.",
                segmented(options(&["Sidebar", "Classic", "Compact"]), toolbar, move |ix| {
                    super::set_pref(SIDEBAR_TOOLBAR, ix == 0);
                    if ix > 0 {
                        super::set_pref(COMPACT_TOOLBAR, ix == 2);
                    }
                }),
            ),
            choice(
                "Tabs",
                "In the sidebar, or in a strip across the top with the sidebar hidden.",
                segmented([(false, "Sidebar".to_owned()), (true, "Top".to_owned())].to_vec(), top, super::set_tabs_on_top),
            ),
        ],
        sidebar: vec![
            choice(
                "Pinned sites",
                "The tiles at the top of the sidebar: large, or a compact row that fits more.",
                segmented([(false, "Large".to_owned()), (true, "Compact".to_owned())].to_vec(), on(SMALL_TILES), set(SMALL_TILES)),
            ),
            choice(
                "Groups and folders",
                "Cards show a folder icon and, when closed, the first sites and how many tabs; Classic keeps the coloured badges.",
                segmented([(true, "Cards".to_owned()), (false, "Classic".to_owned())].to_vec(), on(SIDEBAR_CARDS), set(SIDEBAR_CARDS)),
            ),
            toggle(SIDEBAR_CHATS, "Chats", "Your latest assistant chats under the pinned sites, while the profile's AI is on."),
            toggle(UNREAD_DOTS, "Mark changed tabs", "A dot on a tab whose page changed its title while you looked elsewhere, such as a new message count."),
            toggle(DIM_SLEEPING, "Dim sleeping tabs", "Grey out the icons of tabs that sleep to save memory, with a moon beside them."),
            choice(
                "Tab previews",
                "While the pointer rests on a tab: a snapshot of its page with its title, the title alone, or nothing. Snapshots stay on this computer and are dropped when the pointer leaves.",
                segmented(options(&["Page", "Title", "Off"]), previews, move |ix| {
                    super::set_pref(HOVER_CARDS, ix < 2);
                    super::set_pref(PAGE_PREVIEWS, ix == 0);
                    super::peek::hide(ui);
                }),
            ),
        ],
        tail: vec![
            view().flex_col().pb_2().child(controls::switch(auto_hide, "Hide sidebar automatically", move || super::set_auto_hide_sidebar(ui))).child(
                view()
                    .px_3()
                    .child(small(format!("With the sidebar hidden{shortcut}, it slides in over the page while the pointer rests at the window's left edge."))),
            ),
        ],
    }
}
