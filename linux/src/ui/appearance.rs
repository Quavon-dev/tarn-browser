use std::path::PathBuf;

use nsh_core::prelude::*;
use store::{Id, Store};
use tarn_nsh_shared::appearance::{Host, appearance};

use super::panels::section;
use super::tools::note;
use super::{Ui, read};
use crate::controls::{group, group_row, label, segmented, switch};
use browser_core::prefs::{Style, Switch};

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
    use browser_core::prefs::{
        COMPACT_TOOLBAR, DIM_SLEEPING, HOVER_CARDS, PAGE_PREVIEWS, SIDEBAR_CARDS, SIDEBAR_CHATS, SIDEBAR_TOOLBAR, SMALL_TILES, UNREAD_DOTS,
    };
    let Some((style, auto_hide, top)) = read(ui, |b| (b.style(), b.auto_hide_sidebar(), b.tabs_on_top())) else { return view() };
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
    let tabs = [(false, "Sidebar".to_owned()), (true, "Top".to_owned())].to_vec();
    let shortcut = super::shortcut_of(ui, crate::shortcuts::Action::ToggleSidebar).map(|k| format!(" ({k})")).unwrap_or_default();
    let style_note = match style {
        Style::Custom => "Custom: you changed single options under Layout and Sidebar. Pick one to set them all again.",
        _ => {
            "Modern puts the toolbar in the sidebar with cards for groups and folders; Classic is Tarn's earlier layout. Single options are under Layout and Sidebar."
        }
    };
    let styles = [(Style::Modern, "Modern".to_owned()), (Style::Classic, "Classic".to_owned())].to_vec();
    view()
        .flex_col()
        .child(section("Style"))
        .child(group().child(group_row().child(label("Layout").flex_1()).child(segmented(styles, style, move |s| {
            super::act_everywhere(move |b| b.set_style(s == Style::Modern));
            super::peek::hide(ui);
        }))))
        .child(note(style_note))
        .child(section("Layout"))
        .child(
            group()
                .child(group_row().child(label("Tabs").flex_1()).child(segmented(tabs, top, super::set_tabs_on_top)))
                .child(group_row().child(label("Toolbar").flex_1()).child(segmented(options(&["Sidebar", "Classic", "Compact"]), toolbar, move |ix| {
                    super::set_pref(SIDEBAR_TOOLBAR, ix == 0);
                    if ix > 0 {
                        super::set_pref(COMPACT_TOOLBAR, ix == 2);
                    }
                })))
                .child(switch(auto_hide, "Hide sidebar automatically", move || super::set_auto_hide_sidebar(ui)))
                .child(group_row().child(label("Tab previews").flex_1()).child(segmented(options(&["Page", "Title", "Off"]), previews, move |ix| {
                    super::set_pref(HOVER_CARDS, ix < 2);
                    super::set_pref(PAGE_PREVIEWS, ix == 0);
                    super::peek::hide(ui);
                }))),
        )
        .child(note("Tabs on top list them in a strip across the window, with the sidebar hidden."))
        .child(note("Sidebar puts Back, Forward and the address field above the tabs; with tabs on top, or the sidebar hidden, it shows as Compact. Compact puts the address field into a slimmer bar."))
        .child(note(format!("With the sidebar hidden{shortcut}, it slides in over the page while the pointer rests at the window's left edge.")))
        .child(note("Tab previews: while the pointer rests on a tab, a snapshot of its page with its title, the title alone, or nothing. Snapshots stay on this computer and are dropped when the pointer leaves."))
        .child(section("Sidebar"))
        .child(note("These apply while the toolbar sits in the sidebar (Layout › Toolbar › Sidebar)."))
        .child(
            group()
                .child(group_row().child(label("Pinned sites").flex_1()).child(segmented(
                    [(false, "Large".to_owned()), (true, "Compact".to_owned())].to_vec(),
                    on(SMALL_TILES),
                    set(SMALL_TILES),
                )))
                .child(group_row().child(label("Groups and folders").flex_1()).child(segmented(
                    [(true, "Cards".to_owned()), (false, "Classic".to_owned())].to_vec(),
                    on(SIDEBAR_CARDS),
                    set(SIDEBAR_CARDS),
                )))
                .child(switch(on(SIDEBAR_CHATS), "Chats", move || super::set_pref(SIDEBAR_CHATS, !on(SIDEBAR_CHATS))))
                .child(switch(on(UNREAD_DOTS), "Mark changed tabs", move || super::set_pref(UNREAD_DOTS, !on(UNREAD_DOTS))))
                .child(switch(on(DIM_SLEEPING), "Dim sleeping tabs", move || super::set_pref(DIM_SLEEPING, !on(DIM_SLEEPING)))),
        )
        .child(note("Pinned sites: large tiles, or a compact row that fits more. Cards show a folder icon and, when closed, the first sites and how many tabs; Classic keeps the coloured badges."))
        .child(note("Chats lists your latest assistant chats under the pinned sites, while the profile's AI is on. Mark changed tabs puts a dot on a tab whose page changed its title while you looked elsewhere. Dim sleeping tabs greys out the icons of tabs that sleep to save memory, with a moon beside them."))
}
