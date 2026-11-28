use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use browser_core::prefs::{DIM_SLEEPING, SIDEBAR_CARDS, SIDEBAR_CHATS, SMALL_TILES, UNREAD_DOTS};
use browser_core::sidebar::{self as bar, DropAt, Row, RowKind, Swipe};
use browser_core::tab_menu;
use browser_core::tabs::TabKey;
use nsh_base::{button, input};
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use store::{Id, TabKind};
use tarn_nsh_shared::controls::{site_tile, tip};
use tarn_nsh_shared::icons as glyphs;
use tarn_nsh_shared::motion::{popping, spinner};
use tarn_nsh_shared::panes::Pane;
use tarn_nsh_shared::popover::{self as pop, Anchor};
use tarn_nsh_shared::sidebar::{
    BUTTON, CLOSE_GAP, CLOSE_WIDTH, FOOTER_HEIGHT, GLYPH, HEADER_HEIGHT, HEADER_INSET, INDENT, INNER_PAD, MARGIN, MENU_WIDTH, ROW_INSET, STATE_GLYPH, TILE_GAP,
    TILE_ICON, WIDTH, chosen_icon, divider, glyph_button, group_avatar, group_color, menu_check, menu_heading, menu_item, menu_separator, new_tab_mark,
    progress_ring, tile_grid,
};

use super::super::{AppShell, Screen, Ui, bars, panels};
use crate::theme::{Paint, TEXT, TEXT_SMALL, Theme, themed};

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Tab(TabKey),
    Folder(Id),
    Group(Id),
    Space,
    Organize,
    FolderIcon(Id),
    GroupIcon(Id),
    Chats,
    Profile,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Spot {
    Tab(TabKey),
    Folder(Id),
    Group(Id),
    Tiles,
    Pinned,
    Loose,
}

/// What Settings › Appearance › Style and Sidebar pick; all off but `dim` in the Classic sidebar.
#[derive(Clone, Copy, PartialEq, Default)]
struct Look {
    chrome: bool,
    cards: bool,
    chats: bool,
    small: bool,
    dots: bool,
    dim: bool,
}

fn look(ui: Ui) -> Memo<Look> {
    computed(move || {
        ui.read_data(|s| {
            let chrome = s.sidebar_chrome();
            let on = |switch| chrome && s.switch_on(switch);
            Look {
                chrome,
                cards: on(SIDEBAR_CARDS),
                chats: on(SIDEBAR_CHATS),
                small: on(SMALL_TILES),
                dots: on(UNREAD_DOTS),
                dim: !chrome || s.switch_on(DIM_SLEEPING),
            }
        })
    })
}

#[derive(Clone, Copy)]
struct Bar {
    ui: Ui,
    look: Memo<Look>,
    chats_open: Signal<bool>,
    menu: Signal<Option<(Target, Point<Points>)>>,
    layout: Memo<bar::Sidebar>,
    renaming: Signal<Option<TabKey>>,
    pointer: Signal<bool>,
    slid: Signal<Option<TabKey>>,
    spots: Stored<HashMap<Spot, ElementRef>>,
}

fn act(ui: Ui, f: impl FnOnce(&mut AppShell)) {
    ui.act(|s| {
        f(s);
        Vec::new()
    });
}

fn activate(ui: Ui, key: TabKey) {
    act(ui, |s| s.activate(key));
    ui.show(Screen::Browse);
}

pub(super) fn host(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()
}

fn asleep(s: &AppShell, key: TabKey) -> bool {
    s.model.active != Some(key) && !s.model.lifecycle.live().any(|k| k == key)
}

fn tab_icon(ui: Ui, key: TabKey, size: f32, spin: f32, dim: bool) -> AnyElement {
    let loading = computed(move || ui.read(|s| s.pages.get(&key).is_some_and(|p| p.loading)));
    let state = computed(move || {
        ui.read(|s| {
            let url = s.tab_url(key);
            (s.site_icon(&url), host(&url), asleep(s, key))
        })
    });
    dynamic(move || {
        if loading.get() && !untracked(reduce_motion) {
            return popping(view().text_color(Theme::space_label).child(spinner(spin, glyphs::icon(glyphs::RELOAD, spin)))).into_element();
        }
        popping(view().child(dynamic(move || {
            let (icon, host, asleep) = state.get();
            site_tile(icon, &host, size).opacity(if asleep && dim { 0.4_f32 } else { 1. })
        })))
        .into_element()
    })
}

fn open_menu(bar: Bar, target: Target, at: Point<Points>) {
    bar.menu.set(Some((target, at)));
}

fn menu_anchor(bar: Bar, target: Target) -> AnyElement {
    dynamic(move || {
        let at = bar.menu.with(|m| m.filter(|(t, _)| *t == target).map(|(_, at)| at))?;
        let point = element_ref();
        let menu = view().p_1().child(untracked(|| menu_items(bar, target)));
        // The footer's menus open upwards.
        let anchor = match target {
            Target::Profile => Anchor::Above(point, Align::End),
            Target::Space if bar.look.get_untracked().chrome => Anchor::Above(point, Align::Start),
            _ => Anchor::Below(point, Align::Start),
        };
        Some(view().absolute().left(px(at.x.0)).top(px(at.y.0)).w(px(0.)).h(px(0.)).element_ref(&point).child(pop::popover(
            anchor,
            MENU_WIDTH,
            move || bar.menu.set(None),
            menu,
        )))
    })
}

fn item(bar: Bar, name: impl Into<String>, enabled: bool, run: impl Fn(Ui) + 'static) -> AnyElement {
    menu_item(name, enabled, move || {
        bar.menu.set(None);
        run(bar.ui);
    })
    .into_element()
}

fn menu_items(bar: Bar, target: Target) -> View {
    let items = match target {
        Target::Tab(key) => tab_items(bar, key),
        Target::Folder(id) => folder_items(bar, id),
        Target::Group(id) => group_items(bar, id),
        Target::Space => space_items(bar),
        Target::Organize => organize_items(bar),
        Target::Chats => vec![
            item(bar, "Hide Chats in Sidebar", true, |ui| act(ui, |s| s.set_switch(SIDEBAR_CHATS, false))),
            menu_separator().into_element(),
            item(bar, "Customize in Settings…", true, |ui| open_settings(ui, Pane::Appearance)),
        ],
        Target::Profile => profile_items(bar),
        Target::FolderIcon(_) | Target::GroupIcon(_) => return icon_picker(bar, target),
    };
    view().flex_col().children(items)
}

fn tab_items(bar: Bar, key: TabKey) -> Vec<AnyElement> {
    let ui = bar.ui;
    let Some(tab) = ui.read(|s| {
        let t = s.model.tab(key)?;
        Some(tab_menu::Tab {
            kind: t.kind,
            active: s.model.active == Some(key),
            has_link: s.web_link_of(key).is_some(),
            engine: None,
            split: s.splits.menu(s.model.active, key),
            split_view: ui.wide(),
            burner: s.model.burner_shown(),
            custom_title: !t.custom_title.is_empty(),
            drift: None,
            grouped: t.group_id.is_some(),
            muted: None,
            sleepable: false,
            sleep_others: false,
            others: !bar::other_unpinned(&s.model, key).is_empty(),
            below: !bar::tabs_below(&s.model, key).is_empty(),
            spaces: if s.in_burner() { Vec::new() } else { s.move_targets(key) },
            groups: s.model.groups.iter().filter(|g| Some(g.id) != t.group_id).map(|g| (g.id, g.name.clone())).collect(),
        })
    }) else {
        return Vec::new();
    };
    tab_entries(bar, key, tab_menu::items(&tab, offered))
}

fn offered(command: tab_menu::Command) -> bool {
    use tab_menu::Command as C;
    !matches!(
        command,
        C::SwitchEngine | C::Bookmark | C::RotateSplit | C::CloseSplit | C::ReturnToPinned | C::SetPinnedUrl | C::ToggleMute | C::Sleep | C::SleepOthers
    )
}

fn tab_entries(bar: Bar, key: TabKey, items: Vec<tab_menu::Item>) -> Vec<AnyElement> {
    items
        .into_iter()
        .flat_map(|entry| match entry {
            tab_menu::Item::Command(command, label, enabled, _) => vec![item(bar, label, enabled, move |ui| run_tab(bar, ui, key, command))],
            tab_menu::Item::Submenu(name, inner) => std::iter::once(menu_heading(name).into_element()).chain(tab_entries(bar, key, inner)).collect(),
            tab_menu::Item::Separator => vec![menu_separator().into_element()],
        })
        .collect()
}

fn run_tab(bar: Bar, ui: Ui, key: TabKey, command: tab_menu::Command) {
    use tab_menu::Command as C;
    match command {
        C::Reload => ui.act(|s| s.active_command("reload", serde_json::Value::Null)),
        C::Open => activate(ui, key),
        C::Duplicate => {
            act(ui, |s| {
                let url = s.tab_url(key);
                s.new_tab(Some(&url));
            });
            ui.show(Screen::Browse);
        }
        C::CopyLink => ui.copy_link(Some(key), false),
        C::CopyCleanLink => ui.copy_link(Some(key), true),
        C::Split => {
            act(ui, |s| s.split_with(key));
            ui.show(Screen::Browse);
        }
        C::LeaveSplit => act(ui, |s| s.leave_split(key)),
        C::TogglePin => act(ui, |s| s.toggle_pin(key)),
        C::ToggleTile => act(ui, |s| {
            bar::toggle_tile(&mut s.model, &s.core, key);
        }),
        C::Rename => bar.renaming.set(Some(key)),
        C::ResetTitle => act(ui, |s| {
            s.core.store(key.private).set_tab_custom_title(key.id, "").ok();
            s.model.reload_space(&s.core);
        }),
        C::MoveToSpace(ix) => act(ui, |s| {
            s.move_tab(key, ix);
        }),
        C::AddToGroup(id) => act(ui, |s| {
            bar::set_group(&mut s.model, &s.core, key, Some(id));
        }),
        C::NewGroup => act(ui, |s| {
            bar::new_group(&mut s.model, &s.core, Some(key));
        }),
        C::RemoveFromGroup => act(ui, |s| {
            bar::set_group(&mut s.model, &s.core, key, None);
        }),
        C::Unload => act(ui, |s| s.unload_tab(key)),
        C::Close => act(ui, |s| s.close_or_unload(key)),
        C::CloseOthers => act(ui, |s| s.close_many(&bar::other_unpinned(&s.model, key))),
        C::CloseBelow => act(ui, |s| s.close_many(&bar::tabs_below(&s.model, key))),
        C::SwitchEngine | C::Bookmark | C::RotateSplit | C::CloseSplit | C::ReturnToPinned | C::SetPinnedUrl | C::ToggleMute | C::Sleep | C::SleepOthers => {}
    }
}

fn change_icon(bar: Bar, target: Target) -> AnyElement {
    item(bar, "Change Icon…", true, move |_| open_menu(bar, target, Point::new(Points(0.), Points(Theme::tab_row()))))
}

fn stored_icon(ui: Ui, target: Target) -> String {
    ui.read(|s| match target {
        Target::FolderIcon(id) => s.model.folders.iter().find(|f| f.id == id).map(|f| f.icon.clone()),
        Target::GroupIcon(id) => s.model.groups.iter().find(|g| g.id == id).map(|g| g.icon.clone()),
        _ => None,
    })
    .unwrap_or_default()
}

fn set_icon(bar: Bar, target: Target, icon: &str) {
    bar.menu.set(None);
    act(bar.ui, |s| {
        let store = s.core.store(s.model.private());
        let saved = match target {
            Target::FolderIcon(id) => browser_core::icon::set_folder(store, id, icon),
            Target::GroupIcon(id) => browser_core::icon::set_group(store, id, icon),
            _ => return,
        };
        match saved {
            Ok(()) => {
                s.model.reload_space(&s.core);
            }
            Err(_) => s.toast_as(browser_core::toast::Tone::Warning, "Use one emoji or a listed symbol".to_owned()),
        }
    });
}

fn icon_picker(bar: Bar, target: Target) -> View {
    let current = stored_icon(bar.ui, target);
    let symbols = browser_core::icon::SYMBOLS.iter().filter_map(|name| {
        let glyph = glyphs::named(name)?;
        let chosen = current == *name;
        Some(
            button()
                .accessibility_label(*name)
                .selected(chosen)
                .on_press(move || set_icon(bar, target, name))
                .w(px(BUTTON))
                .h(px(BUTTON))
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_row()))
                .text_color(Theme::muted)
                .bg(if chosen { Theme::selected() } else { Theme::clear() })
                .hover(|s| s.bg(Theme::hover()))
                .active(|s| s.bg(Theme::pressed()))
                .child(glyphs::icon(glyph, 14.))
                .into_element(),
        )
    });
    let emoji = signal(browser_core::icon::emoji(&current).to_owned());
    let field = input()
        .label("Emoji")
        .placeholder("Emoji")
        .auto_correct(false)
        .value(move || emoji.get())
        .on_change(move |v: &str| emoji.set(v.to_owned()))
        .on_submit(move |v: &str| set_icon(bar, target, v.trim()))
        .flex_1()
        .min_w(px(0.))
        .h(px(BUTTON))
        .px_2()
        .rounded(px(Theme::radius_row()))
        .bg(Theme::control)
        .text_size(px(TEXT));
    view().flex_col().gap_1().child(view().flex_row().flex_wrap().children(symbols)).child(
        view()
            .flex_row()
            .items_center()
            .gap_1()
            .child(field)
            .child((!current.is_empty()).then(|| menu_item("Remove", true, move || set_icon(bar, target, "")).flex_none())),
    )
}

fn folder_items(bar: Bar, id: Id) -> Vec<AnyElement> {
    vec![
        change_icon(bar, Target::FolderIcon(id)),
        item(bar, "New Subfolder", true, move |ui| {
            act(ui, |s| {
                bar::new_folder(&mut s.model, &s.core, Some(id));
            })
        }),
        menu_separator().into_element(),
        item(bar, "Delete Folder…", true, move |ui| {
            let delete = move || act(ui, |s| delete_folder(s, id));
            if ui.read(|s| s.model.folder_contents(id).0.is_empty()) {
                delete();
            } else {
                panels::confirm("Delete this folder?", "Its tabs and subfolders are deleted too.", "Delete", delete);
            }
        }),
    ]
}

fn group_items(bar: Bar, id: Id) -> Vec<AnyElement> {
    vec![
        change_icon(bar, Target::GroupIcon(id)),
        item(bar, "New Tab in Group", true, move |ui| {
            act(ui, |s| {
                if let Some(key) = s.new_tab(None) {
                    bar::set_group(&mut s.model, &s.core, key, Some(id));
                }
            });
            ui.show(Screen::Browse);
        }),
        menu_separator().into_element(),
        item(bar, "Ungroup", true, move |ui| {
            act(ui, |s| {
                bar::remove_group(&mut s.model, &s.core, id, true);
            })
        }),
        menu_separator().into_element(),
        item(bar, "Close Group", true, move |ui| close_group(ui, id)),
    ]
}

fn organize_items(bar: Bar) -> Vec<AnyElement> {
    let (cleanable, assistant) = bar.ui.read(|s| (bar::cleanable(&s.model), s.model.assistant_offered(&s.core, None)));
    bar::organize(cleanable, assistant)
        .into_iter()
        .map(|(label, o)| {
            item(bar, label, true, move |ui| match o {
                bar::Organize::NewGroup => act(ui, |s| {
                    let active = s.model.active;
                    bar::new_group(&mut s.model, &s.core, active);
                }),
                bar::Organize::NewFolder => act(ui, |s| {
                    bar::new_folder(&mut s.model, &s.core, None);
                }),
                bar::Organize::CleanUp => act(ui, |s| s.clean_up()),
                bar::Organize::GroupWithAssistant => super::super::assistant::group_tabs(ui),
            })
        })
        .collect()
}

fn space_items(bar: Bar) -> Vec<AnyElement> {
    let (ix, private) = bar.ui.read(|s| (s.model.space_ix, s.model.current_space().is_none_or(|c| c.private)));
    // The switcher at the sidebar's bottom edge lists the spaces to go to first.
    let spaces = if bar.look.get_untracked().chrome { space_list(bar.ui, false) } else { Vec::new() };
    let listed = !spaces.is_empty();
    spaces
        .into_iter()
        .map(|(to, name, .., current, _)| {
            menu_check(name, current, move || {
                bar.menu.set(None);
                act(bar.ui, |s| s.choose_space(to, browser_core::now()));
            })
            .into_element()
        })
        .chain(listed.then(|| menu_separator().into_element()))
        .chain([
            item(bar, "Edit Space…", !private, move |ui| ui.show(Screen::SpaceEdit(ix))),
            item(bar, "New Space…", bar.ui.read(|s| s.can_add_space()), |ui| ui.show(Screen::SpaceNew)),
        ])
        .collect()
}

fn open_settings(ui: Ui, pane: Pane) {
    ui.settings_pane.set(pane);
    ui.show(Screen::Settings);
}

/// A private space's profile lives in the private store, so it is not edited from here.
fn profile_items(bar: Bar) -> Vec<AnyElement> {
    let private = bar.ui.read(|s| s.model.current_space().is_none_or(|c| c.private));
    let mut items = vec![
        item(bar, "Settings…", true, |ui| open_settings(ui, Pane::Appearance)),
        item(bar, "Show Library", true, |ui| ui.show(Screen::History)),
        menu_separator().into_element(),
    ];
    if !private {
        items.push(item(bar, "Edit Profile…", true, |ui| open_settings(ui, Pane::Profile)));
    }
    items.push(item(bar, "New Space…", bar.ui.read(|s| s.can_add_space()), |ui| ui.show(Screen::SpaceNew)));
    items
}

fn delete_folder(s: &mut AppShell, id: Id) {
    let (keys, _) = s.model.folder_contents(id);
    let active = s.model.active.is_some_and(|a| keys.contains(&a));
    if !bar::delete_folder(&mut s.model, &s.core, id) {
        return;
    }
    for key in &keys {
        s.forget(*key);
    }
    if active {
        s.model.active = None;
        match s.model.pick(|k| keys.contains(&k)) {
            Some(next) => s.activate(next),
            None => {
                s.new_tab(None);
            }
        }
    }
}

fn close_group(ui: Ui, id: Id) {
    act(ui, |s| s.close_group(id));
}

fn pressable_view() -> View {
    view().accessible(true).role(AccessibilityRole::Button)
}

fn press(bar: Bar, target: Target) {
    let ui = bar.ui;
    match target {
        Target::Tab(key) => activate(ui, key),
        Target::Folder(id) => act(ui, |s| s.model.toggle_folder(id)),
        Target::Group(id) => act(ui, |s| {
            s.model.toggle_group(&s.core, id);
        }),
        Target::Space | Target::Organize | Target::FolderIcon(_) | Target::GroupIcon(_) | Target::Chats | Target::Profile => {}
    }
}

fn with_menu(bar: Bar, target: Target, element: View) -> View {
    let pointer = signal(false);
    element
        .relative()
        .on_mouse_enter(move |_: &MouseEvent| pointer.set(true))
        .on_mouse_leave(move |_: &MouseEvent| pointer.set(false))
        .on_context_menu(move |e: &ContextMenuEvent| open_menu(bar, target, e.position))
        .on_long_press(move |e: &PressEvent| if pointer.get_untracked() { press(bar, target) } else { open_menu(bar, target, e.position) })
        .child(menu_anchor(bar, target))
}

fn state_glyph(bar: Bar, key: TabKey, busy: impl Fn() -> bool + 'static) -> AnyElement {
    let ui = bar.ui;
    let look = bar.look.get_untracked();
    let state = computed(move || {
        ui.read(|s| {
            let asleep = asleep(s, key);
            let playing = look.chrome && !asleep && s.player.shown(&[]).is_some_and(|(k, m)| k == key && m.playing);
            match (asleep && look.dim, playing, s.model.tab(key).map(|t| t.kind)) {
                _ if busy() => None,
                (true, ..) => Some((glyphs::MOON, "Asleep")),
                (false, true, _) => Some((glyphs::VOLUME_2, "Playing")),
                (false, false, Some(TabKind::Pinned)) if !look.cards => Some((glyphs::PIN, "Pinned")),
                _ => None,
            }
        })
    });
    dynamic(move || {
        state.get().map(|(glyph, name)| {
            view().w(px(CLOSE_WIDTH)).flex_none().items_center().text_color(Theme::faint).accessible(true).label(name).child(glyphs::icon(glyph, STATE_GLYPH))
        })
    })
}

fn tab_row(bar: Bar, key: TabKey, depth: usize) -> View {
    let ui = bar.ui;
    let hovered = signal(false);
    let active = move || ui.read(|s| s.model.active == Some(key));
    let closable = move || ui.read(|s| s.model.tab(key).is_some_and(|t| t.kind == TabKind::Ephemeral));
    let under = move || hovered.get() || bar.slid.get() == Some(key);
    let shown = move || closable() && (under() || active());
    let title = move || SharedString::from(ui.read(|s| s.tab_title(key)));
    let split = move || ui.read(|s| (s.splits.contains(key) && s.model.active != Some(key)).then(|| s.shown_split().is_some_and(|p| p.panes().contains(&key))));
    let close = view()
        .flex_none()
        .overflow_hidden()
        .w(move || px(CLOSE_WIDTH * f32::from(u8::from(shown()))))
        .ml(move || px(-CLOSE_GAP * f32::from(u8::from(!shown()))))
        .opacity(move || f32::from(u8::from(shown())))
        .child(glyph_button(glyphs::CLOSE, "Close Tab", Placement::Top, move || {
            bar.slid.set(if hovered.get_untracked() || bar.slid.get_untracked() == Some(key) {
                bar.layout.with_untracked(|l| bar::slides_under(l, key))
            } else {
                None
            });
            act(ui, |s| s.close_or_unload(key))
        }));
    let row = pressable_view()
        .test_id("tab")
        .label(title)
        .on_press(move |e: &PressEvent| {
            press(bar, Target::Tab(key));
            if e.clicks == 2 && renamable(ui, key) {
                bar.renaming.set(Some(key));
            }
        })
        .relative()
        .flex_row()
        .items_center()
        .h(move || px(Theme::tab_row()))
        .pl(px(ROW_INSET + indent(bar, depth)))
        .pr(px(2.))
        .gap(px(CLOSE_GAP))
        .rounded(px(Theme::radius_row()))
        .bg(move || match (active(), under()) {
            (true, _) => Theme::selected(),
            (false, true) => Theme::hover(),
            (false, false) => Theme::clear(),
        })
        .active(|s| s.bg(Theme::pressed()))
        .border_1()
        .border_color(move || if active() { Theme::surface_edge() } else { Theme::clear() })
        .child(dynamic(move || {
            split().map(|on| {
                view().absolute().left(px(0.)).top(px(9.)).bottom(px(9.)).w(px(3.)).rounded_full().bg(if on { Theme::accent() } else { Theme::faint() })
            })
        }))
        .child(view().w(px(GLYPH)).h(px(GLYPH)).flex_none().items_center().justify_center().child(tab_icon(ui, key, GLYPH, 14., bar.look.get_untracked().dim)))
        .child(dynamic(move || {
            ui.read(|s| s.model.burner_shown()).then(|| {
                view()
                    .flex_none()
                    .text_color(Theme::warning)
                    .tooltip(tip("Burner tab: nothing is kept after it closes", Placement::Top))
                    .child(glyphs::icon(glyphs::FLAME, 14.))
            })
        }))
        .child(dynamic(move || {
            if bar.renaming.get() == Some(key) {
                return rename_field(bar, key);
            }
            text(title)
                .text_size(px(TEXT))
                .text_color(move || if ui.read(|s| asleep(s, key)) && !active() { Theme::asleep() } else { Theme::text() })
                .style_when(active, |s| s.font_weight(FontWeight::MEDIUM))
                .flex_1()
                .truncate()
                .into_element()
        }))
        .child(dynamic(move || {
            (bar.look.get_untracked().dots && bar.renaming.get() != Some(key) && ui.read(|s| s.model.unread(key)))
                .then(|| view().w(px(6.)).h(px(6.)).flex_none().rounded_full().bg(Theme::muted).accessible(true).label("Changed since you last looked"))
        }))
        .child(state_glyph(bar, key, shown))
        .child(when(closable, move || close.clone()));
    draggable(bar, key, with_menu(bar, Target::Tab(key), row))
        .on_mouse_enter(move |_: &MouseEvent| {
            bar.slid.set(None);
            hovered.set(true);
        })
        .on_mouse_leave(move |_: &MouseEvent| {
            if bar.slid.get_untracked() == Some(key) {
                bar.slid.set(None);
            }
            hovered.set(false);
        })
}

fn renamable(ui: Ui, key: TabKey) -> bool {
    ui.read(|s| s.model.tab(key).is_some_and(|t| t.kind != TabKind::Tile))
}

fn spot(bar: Bar, at: Spot, element: View) -> View {
    let handle = element_ref();
    bar.spots.with(|m| m.insert(at, handle));
    cleanup(move || {
        bar.spots.try_with(|m| {
            if m.get(&at) == Some(&handle) {
                m.remove(&at);
            }
        });
    });
    element.element_ref(&handle)
}

fn drop_at(s: &AppShell, at: Spot) -> Option<DropAt> {
    match at {
        Spot::Tab(key) => DropAt::before(&s.model, key),
        Spot::Folder(id) => Some(DropAt::zone(TabKind::Pinned, Some(id), None)),
        Spot::Group(id) => Some(DropAt::zone(TabKind::Ephemeral, None, Some(id))),
        Spot::Tiles => Some(DropAt::zone(TabKind::Tile, None, None)),
        Spot::Pinned => Some(DropAt::zone(TabKind::Pinned, None, None)),
        Spot::Loose => Some(DropAt::zone(TabKind::Ephemeral, None, None)),
    }
}

fn land(bar: Bar, key: TabKey, by: Point<Points>) {
    let ui = bar.ui;
    let Some(row) = bar.spots.with(|m| m.get(&Spot::Tab(key)).copied()) else { return };
    let spots: Vec<(Spot, ElementRef)> = bar.spots.with(|m| m.iter().map(|(k, v)| (*k, *v)).collect());
    drop(spawn(async move {
        let Some(row) = row.measure().await else { return };
        let at = Point::new(row.page.x + row.frame.size.width / 2. + by.x, row.page.y + row.frame.size.height / 2. + by.y);
        let mut best: Option<(f32, Spot)> = None;
        for (spot, handle) in spots {
            let Some(m) = handle.measure().await else { continue };
            let (w, h) = (m.frame.size.width, m.frame.size.height);
            let inside = at.x >= m.page.x && at.x <= m.page.x + w && at.y >= m.page.y && at.y <= m.page.y + h;
            if inside && best.is_none_or(|(area, _)| w.0 * h.0 < area) {
                best = Some((w.0 * h.0, spot));
            }
        }
        if let Some((_, spot)) = best {
            act(ui, |s| {
                if let Some(at) = drop_at(s, spot) {
                    bar::drop_tab(&mut s.model, &s.core, key, at);
                }
            });
        }
    }));
}

fn draggable(bar: Bar, key: TabKey, element: View) -> View {
    let lift = animated_value(0.);
    spot(bar, Spot::Tab(key), element)
        .pan_values(None, Some(&lift))
        .bind_translate_y(&lift)
        .pan_enabled(move || bar.pointer.get() && bar.renaming.get() != Some(key))
        .on_pan_end(move |e: &PanEvent| {
            lift.set(0.);
            if !e.cancelled {
                land(bar, key, e.translation);
            }
        })
        .on_mouse_enter(move |_: &MouseEvent| bar.pointer.set(true))
}

fn rename_field(bar: Bar, key: TabKey) -> AnyElement {
    let ui = bar.ui;
    let start = ui.read(|s| s.tab_title(key));
    let typed = stored(start.clone());
    let start_name = stored(start.clone());
    let done = move |commit: bool| {
        if bar.renaming.get_untracked() != Some(key) {
            return;
        }
        let name = typed.get();
        let changed = name != start_name.get();
        bar.renaming.set(None);
        if commit && changed {
            act(ui, |s| {
                let page = s.page_shown_title(key);
                bar::rename_tab(&mut s.model, &s.core, key, &name, &page);
            });
        }
    };
    on_key_with("escape", KeyOptions { when: When::Always, ..KeyOptions::default() }, move |_| done(false));
    let all = start.encode_utf16().count() as u32;
    input()
        .default_value(start)
        .selected_range((0, all))
        .auto_focus(true)
        .label("Name")
        .on_change(move |v: &str| {
            typed.replace(v.to_owned());
        })
        .on_submit(move |v: &str| {
            typed.replace(v.to_owned());
            done(true);
        })
        .on_blur(move || done(true))
        .flex_1()
        .min_w(px(0.))
        .h(move || px(Theme::tab_row() - 8.))
        .px_1()
        .rounded(px(Theme::radius_small()))
        .bg(Theme::raised)
        .text_size(px(TEXT))
        .text_color(Theme::text)
        .into_element()
}

/// In a card a top folder's rows sit flush, so each level steps in one less.
fn indent(bar: Bar, depth: usize) -> f32 {
    (if bar.look.get_untracked().cards { depth.saturating_sub(1) } else { depth }) as f32 * INDENT
}

fn summary_row(ui: Ui, (urls, count): (Vec<String>, usize)) -> View {
    let icons = ui.read(|s| urls.iter().map(|u| (s.site_icon(u), host(u))).collect::<Vec<_>>());
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .children(icons.into_iter().map(|(icon, host)| site_tile(icon, &host, 14.).into_element()))
        .child((count > 0).then(|| text(count.to_string()).text_size(px(TEXT_SMALL)).text_color(Theme::muted).pl_1().pr(px(6.))))
}

fn row(bar: Bar, r: &Row) -> View {
    match &r.kind {
        RowKind::Tab(key) => tab_row(bar, *key, r.depth),
        RowKind::Split(panes) => view()
            .flex_row()
            .gap(move || px(Theme::row_gap()))
            .pl(px(indent(bar, r.depth)))
            .children(panes.iter().map(|k| view().flex_1().min_w(px(0.)).child(tab_row(bar, *k, 0)))),
        RowKind::Folder(id) => folder_row(bar, *id, r.depth),
    }
}

fn rows(bar: Bar, list: impl Fn() -> Vec<Row> + 'static) -> AnyElement {
    each(list, |r: &Row| r.clone(), move |r, _| row(bar, &r.get_untracked()))
}

fn folder_row(bar: Bar, id: Id, depth: usize) -> View {
    let ui = bar.ui;
    let card = bar.look.get_untracked().cards && depth == 0;
    let icon = computed(move || ui.read(|s| s.model.folders.iter().find(|f| f.id == id).map(|f| f.icon.clone()).unwrap_or_default()));
    let name = move || ui.read(|s| s.model.folders.iter().find(|f| f.id == id).map(|f| f.name.clone()).unwrap_or_default());
    let open = move || ui.read(|s| s.model.expanded.contains(&id));
    let header = pressable_view()
        .label(move || SharedString::from(name()))
        .on_press(move || press(bar, Target::Folder(id)))
        .flex_row()
        .items_center()
        .h(move || px(Theme::tab_row() - if card { 2. } else { 0. }))
        .pl(px(if card { 6. } else { ROW_INSET + indent(bar, depth) }))
        .pr(px(if card { 2. } else { ROW_INSET }))
        .gap(px(if card { 8. } else { CLOSE_GAP }))
        .rounded(px(Theme::radius_row()))
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .child(view().w(px(GLYPH)).h(px(GLYPH)).items_center().justify_center().text_color(if card { Theme::muted } else { Theme::space_label }).child(
            dynamic(move || {
                let plain = if card && open() { glyphs::FOLDER_OPEN } else { glyphs::FOLDER };
                chosen_icon(&icon.get(), 14.).unwrap_or_else(|| glyphs::icon(plain, 14.).into_element())
            }),
        ))
        .child(text(name).text_size(px(TEXT)).font_weight(if card { FontWeight::MEDIUM } else { FontWeight::SEMIBOLD }).text_color(Theme::text).truncate())
        .child(view().text_color(Theme::muted).child(dynamic(move || glyphs::icon(if open() { glyphs::CHEVRON_DOWN } else { glyphs::CHEVRON_RIGHT }, 12.))))
        .child(view().flex_1())
        .child(dynamic(move || {
            (card && !open()).then(|| summary_row(ui, ui.read(|s| bar::folder_summary(&s.model, &browser_core::tabs::folder_tree(&s.model.folders, id)))))
        }));
    spot(bar, Spot::Folder(id), with_menu(bar, Target::Folder(id), header).child(menu_anchor(bar, Target::FolderIcon(id))))
}

fn pinned_block(bar: Bar, block: Vec<Row>) -> View {
    let ui = bar.ui;
    let list = view().flex_col().gap(move || px(Theme::row_gap())).children(block.iter().map(|r| row(bar, r)));
    let Some(RowKind::Folder(id)) = block.first().map(|r| r.kind.clone()) else { return list };
    let open = move || ui.read(|s| s.model.expanded.contains(&id));
    if bar.look.get_untracked().cards {
        let empty =
            move || open() && ui.read(|s| !s.model.tabs.iter().any(|t| t.folder_id == Some(id)) && !s.model.folders.iter().any(|f| f.parent_id == Some(id)));
        let hint = move || {
            view()
                .h(move || px(Theme::tab_row()))
                .pl(px(ROW_INSET))
                .justify_center()
                .child(text("Drag tabs here").text_size(px(TEXT_SMALL)).text_color(Theme::faint))
        };
        return view()
            .my(px(2.))
            .p(px(INNER_PAD))
            .rounded(px(Theme::radius_tile()))
            .bg(Theme::group)
            .border_1()
            .border_color(Theme::hairline)
            .child(list)
            .child(when(move || empty() && bar.pointer.get(), hint));
    }
    view()
        .relative()
        .my(move || px(if open() { 4. } else { 0. }))
        .p(move || px(if open() { INNER_PAD } else { 0. }))
        .child(when(open, || view().absolute().inset_0().rounded(px(Theme::radius_tile())).bg(Theme::group)))
        .child(list)
}

fn pinned(bar: Bar) -> View {
    let ui = bar.ui;
    let area = view()
        .flex_col()
        .gap(move || px(Theme::row_gap()))
        .mt_1()
        .min_h(px(12.))
        .rounded(px(Theme::radius_row()))
        .child(each(move || bar.layout.with(|l| l.pinned.clone()), |b: &Vec<Row>| b.clone(), move |b, _| pinned_block(bar, b.get_untracked())))
        .child(when(
            move || !bar.look.get_untracked().chrome && ui.read(|s| !s.model.tabs.iter().any(|t| t.kind == TabKind::Pinned) && s.model.folders.is_empty()),
            move || {
                view()
                    .h(move || px(Theme::tab_row()))
                    .px(px(ROW_INSET))
                    .justify_center()
                    .rounded(px(Theme::radius_row()))
                    .border_1()
                    .border_dashed()
                    .border_color(Theme::hairline)
                    .child(text(move || pin_hint(bar.pointer.get())).text_size(px(TEXT_SMALL)).text_color(Theme::faint))
            },
        ));
    spot(bar, Spot::Pinned, area)
}

fn pin_hint(pointer: bool) -> &'static str {
    if pointer { "Drag tabs here to pin them" } else { "Pin tabs from their menu" }
}

fn tiles(bar: Bar) -> View {
    let ui = bar.ui;
    let look = bar.look.get_untracked();
    let (_, side) = tile_grid(WIDTH);
    let (side, glyph, min, radius) = match (look.chrome, look.small) {
        (true, true) => (34., 16., browser_core::tabs::SMALL_TILE_MIN_WIDTH, Theme::radius_row()),
        (true, false) => ((side * 0.72).round(), 20., browser_core::tabs::TILE_MIN_WIDTH, Theme::radius_tile()),
        (false, _) => (side, TILE_ICON, browser_core::tabs::TILE_MIN_WIDTH, Theme::radius_tile()),
    };
    let count = computed(move || bar.layout.with(|l| l.tiles.len()));
    let width = move || browser_core::tabs::tile_row_sized(WIDTH - 2. - 2. * TILE_GAP, TILE_GAP, count.get(), min).1;
    let grid = view().flex_row().flex_wrap().gap(px(TILE_GAP)).child(each(
        move || bar.layout.with(|l| l.tiles.clone()),
        |k: &TabKey| *k,
        move |key, _| {
            let key = key.get_untracked();
            let active = move || ui.read(|s| s.model.active == Some(key));
            let split = move || ui.read(|s| (s.splits.contains(key), s.shown_split().is_some_and(|p| p.panes().contains(&key))));
            let tile = pressable_view()
                .label(move || SharedString::from(ui.read(|s| s.tab_title(key))))
                .on_press(move || press(bar, Target::Tab(key)))
                .w(move || px(width()))
                .h(px(side))
                .items_center()
                .justify_center()
                .rounded(px(radius))
                .bg(move || if active() { Theme::selected() } else { Theme::raised() })
                .hover(move |s| if active() { s } else { s.bg(Theme::hover()) })
                .active(|s| s.bg(Theme::pressed()))
                .border_1()
                .border_color(move || match (active(), split()) {
                    (true, _) => Theme::sidebar_edge(),
                    (false, (_, true)) => Theme::accent(),
                    (false, (true, false)) => Theme::faint(),
                    (false, (false, false)) => Theme::hairline(),
                })
                .child(tab_icon(ui, key, glyph, 18., look.dim));
            draggable(bar, key, with_menu(bar, Target::Tab(key), tile))
        },
    ));
    spot(bar, Spot::Tiles, grid)
}

fn new_tab_row(bar: Bar) -> View {
    let ui = bar.ui;
    let chrome = bar.look.get_untracked().chrome;
    let new = button()
        .accessibility_label("New Tab")
        .on_press(move || bars::open_new_tab(ui))
        .flex_1()
        .flex_row()
        .items_center()
        .h(move || px(Theme::tab_row()))
        .pl(px(ROW_INSET))
        .gap(px(CLOSE_GAP))
        .rounded(px(Theme::radius_row()))
        .text_color(Theme::muted)
        .hover(|s| s.bg(Theme::hover()).text_color(Theme::text()))
        .active(|s| s.bg(Theme::pressed()))
        .child(if chrome {
            view().w(px(GLYPH)).h(px(GLYPH)).flex_none().items_center().justify_center().child(glyphs::icon(glyphs::PLUS, 14.))
        } else {
            new_tab_mark()
        })
        .child(text(if chrome { "New tab" } else { "New Tab" }).text_size(px(TEXT)).flex_1());
    if chrome {
        return view().flex_row().items_center().pr(px(2.)).child(new);
    }
    view().flex_row().items_center().pr(px(2.)).child(new).child(
        view()
            .relative()
            .child(glyph_button(glyphs::ELLIPSIS, bar::ORGANIZE, Placement::Top, move || {
                open_menu(bar, Target::Organize, Point::new(Points(0.), Points(BUTTON)))
            }))
            .child(menu_anchor(bar, Target::Organize)),
    )
}

fn open_chat(ui: Ui, id: Option<Id>) {
    let host = super::super::assistant::Host(ui);
    match id {
        Some(id) if !ui.chat.restore(host, id) => return,
        Some(_) => {}
        None => {
            ui.chat.finish(host);
            ui.chat.close();
        }
    }
    super::super::assistant::open(ui, false);
}

/// The profile's latest assistant chats under a header that folds them and starts a new one.
fn chats(bar: Bar) -> AnyElement {
    let ui = bar.ui;
    // Read again each minute, so a time from yesterday turns into its date soon after midnight.
    let key = computed(move || ui.read(|s| (s.chat_profile(), s.core.disk.chat_changes(), browser_core::now() / 60_000)));
    let listed = computed(move || {
        let (profile, ..) = key.get();
        profile.map(|p| untracked(|| ui.read(|s| bar::chat_rows(&s.core.disk, p, browser_core::now()))))
    });
    dynamic(move || {
        let listed = listed.get()?;
        let open = bar.chats_open.get();
        let shown = (ui.screen.get() == Screen::Assistant).then(|| ui.chat.saved()).flatten();
        let header = pressable_view()
            .label("Chats")
            .on_press(move || bar.chats_open.update(|o| *o = !*o))
            .flex_row()
            .items_center()
            .gap_1()
            .h(move || px(Theme::tab_row()))
            .pl(px(ROW_INSET))
            .pr(px(2.))
            .rounded(px(Theme::radius_row()))
            .hover(|s| s.bg(Theme::hover()))
            .active(|s| s.bg(Theme::pressed()))
            .child(text("Chats").text_size(px(TEXT)).text_color(Theme::muted))
            .child(view().text_color(Theme::faint).child(glyphs::icon(if open { glyphs::CHEVRON_DOWN } else { glyphs::CHEVRON_RIGHT }, 12.)))
            .child(view().flex_1())
            .child(glyph_button(glyphs::SQUARE_PEN, "New Chat", Placement::Top, move || open_chat(ui, None)));
        let rows = listed.into_iter().filter(|_| open).map(move |row| {
            let id = row.id;
            pressable_view()
                .label(row.title.clone())
                .on_press(move || open_chat(ui, Some(id)))
                .flex_row()
                .items_center()
                .gap(px(CLOSE_GAP))
                .h(move || px(Theme::tab_row()))
                .pl(px(ROW_INSET))
                .pr(px(ROW_INSET))
                .rounded(px(Theme::radius_row()))
                .bg(if shown == Some(id) { Theme::selected() } else { Theme::clear() })
                .hover(|s| s.bg(Theme::hover()))
                .active(|s| s.bg(Theme::pressed()))
                .child(
                    view()
                        .w(px(GLYPH))
                        .h(px(GLYPH))
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .text_color(Theme::muted)
                        .child(glyphs::icon(glyphs::MESSAGE_CIRCLE, 14.)),
                )
                .child(text(row.title).text_size(px(TEXT)).text_color(Theme::text).flex_1().truncate())
                .child(text(row.when).text_size(px(TEXT_SMALL)).text_color(Theme::faint))
                .into_element()
        });
        Some(view().flex_col().gap(move || px(Theme::row_gap())).mt_1().child(with_menu(bar, Target::Chats, header)).children(rows.collect::<Vec<_>>()))
    })
}

/// A rule across the list, with Tidy, which sorts the loose tabs into groups; its menu has the other ways to organise.
fn tidy(bar: Bar) -> View {
    let ui = bar.ui;
    let tint = move || if ui.read(|s| bar::cleanable(&s.model)) { Theme::muted() } else { Theme::faint() };
    let button = pressable_view()
        .label("Tidy")
        .on_press(move || {
            if ui.read(|s| bar::cleanable(&s.model)) {
                act(ui, |s| s.clean_up());
            }
        })
        .flex_row()
        .items_center()
        .gap_1()
        .h(px(24.))
        .px(px(6.))
        .rounded(px(Theme::radius_row()))
        .text_color(tint)
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .child(glyphs::icon(glyphs::BROOM_SPARKLES, 13.))
        .child(text("Tidy").text_size(px(TEXT_SMALL)).text_color(tint));
    view().flex_row().items_center().gap_2().mx_2().my_1().child(view().flex_1().h(px(1.)).bg(Theme::hairline)).child(with_menu(bar, Target::Organize, button))
}

fn group_card(bar: Bar, id: Id) -> View {
    let ui = bar.ui;
    let group = move || {
        ui.read(|s| s.model.groups.iter().find(|g| g.id == id).map(|g| (g.name.clone(), g.color.clone(), g.collapsed, g.icon.clone())).unwrap_or_default())
    };
    let name = move || group().0;
    let cards = bar.look.get_untracked().cards;
    let color = move || group_color(&group().1, id);
    let header = pressable_view()
        .label(move || SharedString::from(name()))
        .on_press(move || press(bar, Target::Group(id)))
        .flex_row()
        .items_center()
        .h(move || px(Theme::tab_row() - 2.))
        .pl(px(6.))
        .pr(px(2.))
        .gap_2()
        .rounded(px(Theme::radius_row()))
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .child(dynamic(move || {
            let (name, tint, collapsed, icon) = group();
            if !cards {
                return group_avatar(&name, &icon, group_color(&tint, id), 18.).into_element();
            }
            let plain = || glyphs::icon(if collapsed { glyphs::FOLDER } else { glyphs::FOLDER_OPEN }, 14.).into_element();
            view()
                .w(px(18.))
                .h(px(18.))
                .flex_none()
                .items_center()
                .justify_center()
                .text_color(color())
                .child(chosen_icon(&icon, 14.).unwrap_or_else(plain))
                .into_element()
        }))
        .child(text(name).text_size(px(TEXT)).font_weight(FontWeight::MEDIUM).text_color(Theme::text).truncate())
        .child(view().text_color(Theme::muted).child(dynamic(move || glyphs::icon(if group().2 { glyphs::CHEVRON_RIGHT } else { glyphs::CHEVRON_DOWN }, 12.))))
        .child(view().flex_1())
        .child(dynamic(move || {
            if cards && group().2 {
                return summary_row(ui, ui.read(|s| bar::group_summary(&s.model, id))).into_element();
            }
            glyph_button(glyphs::CLOSE, "Close Group and Its Tabs", Placement::Top, move || close_group(ui, id)).into_element()
        }));
    let tabs = move || bar.layout.with(|l| l.groups.iter().find(|(g, _)| *g == id).map(|(_, rows)| rows.clone()).unwrap_or_default());
    let card = view()
        .flex_col()
        .gap(move || px(Theme::row_gap()))
        .my(px(2.))
        .p(px(INNER_PAD))
        .rounded(px(Theme::radius_tile()))
        .bg(move || if cards && group().2 { Rgba { a: 0.2, ..color() } } else { Theme::group() })
        .border_1()
        .border_color(Theme::hairline)
        .child(with_menu(bar, Target::Group(id), header).child(menu_anchor(bar, Target::GroupIcon(id))))
        .child(when(
            move || !tabs().is_empty(),
            move || {
                let inner = view().flex_col().gap(move || px(Theme::row_gap())).p(px(INNER_PAD)).overflow_hidden().rounded(px(Theme::radius_row() + INNER_PAD));
                let inner = if cards { inner } else { inner.bg(Theme::raised).border_1().border_color(Theme::hairline) };
                inner.child(rows(bar, tabs))
            },
        ));
    spot(bar, Spot::Group(id), card)
}

fn ephemeral(bar: Bar) -> View {
    let ui = bar.ui;
    let area = view().flex_col().gap(move || px(Theme::row_gap())).min_h(px(120.)).pb(move || px(Theme::tab_row() * 2.)).child(new_tab_row(bar));
    let groups =
        each(move || ui.read(|s| s.model.groups.iter().map(|g| g.id).collect::<Vec<_>>()), |id: &Id| *id, move |id, _| group_card(bar, id.get_untracked()));
    let loose = rows(bar, move || bar.layout.with(|l| l.loose.clone()));
    let area = if bar.look.get_untracked().chrome { area.child(loose).child(groups) } else { area.child(groups).child(loose) };
    spot(bar, Spot::Loose, area)
}

fn space_tint(color: &str, private: bool) -> Option<Rgba> {
    (!private).then(|| Theme::profile_color(color)).flatten()
}

fn switcher(bar: Bar) -> View {
    let ui = bar.ui;
    let space = move || {
        ui.read(|s| {
            s.model.current_space().map(|c| {
                let glyph = if c.private && s.model.burner_shown() { glyphs::FLAME } else { glyphs::space_icon(&c.space.icon) };
                (c.space.name.clone(), glyph, c.private, c.space.color.clone())
            })
        })
    };
    let name = move || space().map(|s| s.0).unwrap_or_default();
    let open = button()
        .accessibility_label(move || SharedString::from(format!("Space {}", name())))
        .on_press(move || open_menu(bar, Target::Space, Point::new(Points(0.), Points(BUTTON))))
        .flex_1()
        .min_w(px(0.))
        .h(px(BUTTON))
        .pr_1()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(Theme::radius_row()))
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .child(dynamic(move || {
            space().map(|(_, glyph, private, color)| {
                let tint = if private { Theme::warning() } else { space_tint(&color, false).unwrap_or(Theme::space_label()) };
                view()
                    .w(px(BUTTON))
                    .h(px(BUTTON))
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(Theme::radius_row()))
                    .bg(Theme::selected)
                    .text_color(tint)
                    .child(glyphs::icon(glyph, GLYPH))
            })
        }))
        .child(text(name).text_size(px(14.)).font_weight(FontWeight::BOLD).text_color(Theme::text).truncate());
    view().flex_1().min_w(px(0.)).relative().child(open).child(menu_anchor(bar, Target::Space))
}

fn downloads_button(ui: Ui) -> View {
    let saving = computed(move || {
        ui.read(|s| {
            let saving: Vec<_> = s.transfers.iter().filter(|t| t.state == crate::scripts::TransferState::Saving).map(|t| (t.done, t.total)).collect();
            (!saving.is_empty()).then(|| bar::saved_share(saving))
        })
    });
    view()
        .relative()
        .child(
            glyph_button(glyphs::CIRCLE_ARROW_DOWN, "Downloads", Placement::Bottom, move || ui.toggle(Screen::Downloads))
                .element_ref(&ui.popovers.downloads)
                .bg(move || if ui.screen.get() == Screen::Downloads { Theme::selected() } else { Theme::clear() }),
        )
        .child(dynamic(move || {
            saving.get().map(|share| match share {
                Some(share) => view()
                    .absolute()
                    .inset_0()
                    .items_center()
                    .justify_center()
                    .text_color(Theme::accent)
                    .accessible(true)
                    .label(format!("{}% saved", (share * 100.).round()))
                    .child(progress_ring(share))
                    .into_element(),
                None => view().absolute().top(px(5.)).right(px(5.)).w(px(6.)).h(px(6.)).rounded_full().bg(Theme::accent).into_element(),
            })
        }))
}

fn header(bar: Bar) -> View {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .h(px(HEADER_HEIGHT))
        .flex_none()
        .pl(px(HEADER_INSET))
        .pr(px(TILE_GAP))
        .child(switcher(bar))
        .child(downloads_button(bar.ui))
        .child(glyph_button(glyphs::GEAR, "Settings", Placement::Bottom, move || bar.ui.toggle(Screen::Settings)))
}

const SPACE_BAR: bar::SpaceBar = bar::SpaceBar { width: WIDTH - 22. - 2. * BUTTON, icon: BUTTON, gap: 2. };

type SpaceButton = (usize, String, &'static str, bool, String, bool, bool);

fn space_strip(ui: Ui) -> Memo<bar::SpaceStrip> {
    computed(move || {
        ui.read_data(|s| {
            let items: Vec<_> = s.spaces().into_iter().filter(|i| s.model.space_shown(i.ix)).collect();
            SPACE_BAR.strip(items.len(), items.iter().position(|i| i.ix == s.model.space_ix).unwrap_or(0))
        })
    })
}

fn space_list(ui: Ui, compact: bool) -> Vec<SpaceButton> {
    ui.read(|s| {
        s.spaces()
            .into_iter()
            .filter(|item| s.model.space_shown(item.ix))
            .filter_map(|item| {
                let space = s.model.spaces.get(item.ix)?;
                let glyph = if item.burner { glyphs::FLAME } else { glyphs::space_icon(&space.space.icon) };
                let current = item.ix == s.model.space_ix;
                Some((item.ix, item.name, glyph, item.private, space.space.color.clone(), current, compact && !current))
            })
            .collect()
    })
}

fn space_buttons(ui: Ui, strip: Memo<bar::SpaceStrip>) -> AnyElement {
    each(
        move || space_list(ui, strip.get().compact),
        |s: &SpaceButton| s.clone(),
        move |space, _| {
            let (ix, name, glyph, private, color, current, dot) = space.get_untracked();
            let tint = space_tint(&color, private);
            let color = match (current, private, tint) {
                (true, true, _) => Theme::warning(),
                (true, false, tint) => tint.unwrap_or(Theme::text()),
                (false, _, Some(tint)) => Rgba { a: 0.45, ..tint },
                (false, _, None) => Theme::faint(),
            };
            let face = if dot { view().w(px(6.)).h(px(6.)).rounded(px(3.)).bg(color).into_element() } else { glyphs::icon(glyph, GLYPH).into_element() };
            button()
                .accessibility_label(name.clone())
                .on_press(move || {
                    tarn_nsh_shared::motion::space_switch();
                    act(ui, |s| s.choose_space(ix, browser_core::now()))
                })
                .w(px(if dot { bar::SPACE_DOT } else { BUTTON }))
                .h(px(BUTTON))
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_row()))
                .bg(if current { Theme::hover() } else { Theme::clear() })
                .hover(|s| s.bg(Theme::hover()))
                .active(|s| s.bg(Theme::pressed()))
                .text_color(color)
                .child(view().size_full().items_center().justify_center().tooltip(tip(&name, Placement::Top)).child(face))
        },
    )
}

fn footer(ui: Ui) -> View {
    let strip = space_strip(ui);
    let strip_ref = element_ref();
    let shown = computed(move || (strip.get(), ui.read_data(|s| s.model.space_ix)));
    let reveal = move |animated| {
        if let Some(x) = shown.get().0.scroll {
            strip_ref.scroll_to(Point::new(Points(x), Points(0.)), animated);
        }
    };
    effect(move || reveal(true));
    view()
        .flex_row()
        .items_center()
        .gap(px(2.))
        .h(px(FOOTER_HEIGHT))
        .flex_none()
        .px_2()
        .child(
            glyph_button(glyphs::LIBRARY, "Library", Placement::Top, move || ui.toggle(Screen::History))
                .bg(move || if ui.screen.get() == Screen::History { Theme::selected() } else { Theme::clear() }),
        )
        .child(
            scroll().horizontal().element_ref(&strip_ref).flex_1().min_w(px(0.)).child(
                view()
                    .flex_row()
                    .flex_grow(1.)
                    .items_center()
                    .justify_center()
                    .gap(px(2.))
                    .on_layout(move |_: &Bounds<Points>| reveal(false))
                    .child(space_buttons(ui, strip)),
            ),
        )
        .child(view().w(px(BUTTON)).h(px(BUTTON)).flex_none().child(when(
            move || ui.read_data(|s| s.can_add_space()),
            move || {
                glyph_button(glyphs::PLUS, "New Space", Placement::Top, move || ui.toggle(Screen::SpaceNew))
                    .element_ref(&ui.popovers.new_space)
                    .text_color(Theme::faint())
                    .bg(move || if ui.screen.get() == Screen::SpaceNew { Theme::selected() } else { Theme::clear() })
            },
        )))
}

/// Downloads, the space switcher and the profile, along the bottom edge of the Modern sidebar.
fn chrome_footer(bar: Bar) -> View {
    let ui = bar.ui;
    let space = button()
        .accessibility_label(move || {
            SharedString::from(format!("Space {}", ui.read(|s| s.model.current_space().map(|c| c.space.name.clone()).unwrap_or_default())))
        })
        .on_press(move || open_menu(bar, Target::Space, Point::new(Points(0.), Points(0.))))
        .element_ref(&ui.popovers.new_space)
        .min_w(px(0.))
        .h(px(BUTTON))
        .px_2()
        .flex_row()
        .items_center()
        .gap_1()
        .rounded(px(Theme::radius_row()))
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .child(dynamic(move || {
            ui.read(|s| {
                s.model.current_space().filter(|c| c.private).map(|c| if s.model.burner_shown() { glyphs::FLAME } else { glyphs::space_icon(&c.space.icon) })
            })
            .map(|glyph| view().text_color(Theme::warning).child(glyphs::icon(glyph, 14.)))
        }))
        .child(
            text(move || SharedString::from(ui.read(|s| s.model.current_space().map(|c| c.space.name.clone()).unwrap_or_default())))
                .text_size(px(14.))
                .font_weight(FontWeight::BOLD)
                .text_color(Theme::text)
                .truncate(),
        )
        .child(view().text_color(Theme::muted).child(glyphs::icon(glyphs::CHEVRON_UP, 12.)));
    let profile = dynamic(move || {
        let (name, color) = ui
            .read_data(|s| {
                let c = s.model.current_space()?;
                let p = s.core.store(c.private).profile(c.space.profile_id).ok()?;
                Some((p.name, p.color))
            })
            .unwrap_or_else(|| ("Profile".to_owned(), String::new()));
        let fill = Theme::profile_color(&color).unwrap_or(Theme::faint());
        let initial = name.trim().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        Some(
            button()
                .accessibility_label(format!("Profile {name}"))
                .on_press(move || open_menu(bar, Target::Profile, Point::new(Points(BUTTON), Points(0.))))
                .w(px(BUTTON))
                .h(px(BUTTON))
                .items_center()
                .justify_center()
                .rounded_full()
                .hover(|s| s.bg(Theme::hover()))
                .active(|s| s.bg(Theme::pressed()))
                .child(
                    view()
                        .w(px(22.))
                        .h(px(22.))
                        .rounded_full()
                        .items_center()
                        .justify_center()
                        .bg(fill)
                        .child(text(initial).text_size(px(10.)).font_weight(FontWeight::BOLD).text_color(Theme::on_fill(fill))),
                ),
        )
    });
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .h(px(44.))
        .flex_none()
        .px_2()
        .child(view().w(px(BUTTON)).flex_none().child(downloads_button(ui)))
        .child(
            view()
                .flex_1()
                .min_w(px(0.))
                .flex_row()
                .justify_center()
                .child(view().relative().min_w(px(0.)).child(space).child(menu_anchor(bar, Target::Space))),
        )
        .child(view().w(px(BUTTON)).flex_none().relative().child(profile).child(menu_anchor(bar, Target::Profile)))
}

fn background(ui: Ui) -> AnyElement {
    themed(move || {
        let tint = ui.read(|s| s.model.current_space().and_then(|c| space_tint(&c.space.color, c.private)));
        view().absolute().inset_0().rounded(px(Theme::radius_tile())).paint(tint.map_or_else(Theme::sidebar, Theme::sidebar_tinted))
    })
}

pub(super) fn sidebar(ui: Ui) -> impl IntoElement {
    let bar = Bar {
        ui,
        look: look(ui),
        chats_open: signal(true),
        menu: signal(None),
        layout: computed(move || ui.read(|s| bar::sidebar(&s.model, &s.splits))),
        renaming: signal(None),
        pointer: signal(false),
        slid: signal(None),
        spots: stored(HashMap::new()),
    };
    let swipe = Rc::new(Cell::new(Swipe::default()));
    // A change of style or sidebar option builds the list again.
    let list = dynamic(move || {
        let look = bar.look.get();
        Some(
            view()
                .flex_col()
                .px(px(TILE_GAP))
                .pb_2()
                .text_size(px(TEXT))
                .child(when(move || ui.read(|s| s.model.tabs.iter().any(|t| t.kind == TabKind::Tile)), move || tiles(bar).pb_1()))
                .child(look.chats.then(|| chats(bar)))
                .child(pinned(bar))
                .child(if look.chrome { tidy(bar) } else { divider() })
                .child(ephemeral(bar)),
        )
    });
    view()
        .flex_col()
        .w(px(WIDTH))
        .flex_none()
        .m(px(MARGIN))
        .mr(px(0.))
        .rounded(px(Theme::radius_tile()))
        .border_1()
        .border_color(Theme::sidebar_edge)
        .shadow(Theme::elevation(6., 24.))
        .child(background(ui))
        .child(dynamic(move || Some(if bar.look.get().chrome { view().h(px(TILE_GAP)).flex_none() } else { header(bar) })))
        .child(
            view()
                .flex_1()
                .min_h(px(0.))
                .on_wheel(move |e: &WheelEvent| {
                    let WheelDelta::Points(p) = e.delta else { return };
                    let mut state = swipe.get();
                    let step = state.feed(p.x.0, p.y.0, Instant::now());
                    swipe.set(state);
                    if let Some(step) = step {
                        tarn_nsh_shared::motion::space_switch();
                        act(ui, |s| {
                            if let Some(ix) = s.neighbour_space(step) {
                                s.choose_space(ix, browser_core::now());
                            }
                        });
                    }
                })
                .child(scroll().flex_1().child(list)),
        )
        .child(super::super::media::bar(ui, true))
        .child(dynamic(move || Some(if bar.look.get().chrome { chrome_footer(bar) } else { footer(ui) })))
}

/// Tabs on top: the pinned tabs as icons, then the others, in a strip under the toolbar.
pub(super) fn strip(ui: Ui) -> impl IntoElement {
    let strip = computed(move || ui.read(|s| bar::strip(&s.model)));
    let chips = move || strip.with(|t| t.pinned.iter().map(|k| (*k, true)).chain(t.tabs.iter().map(|k| (*k, false))).collect::<Vec<_>>());
    view()
        .flex_row()
        .items_center()
        .gap(px(4.))
        .px(px(MARGIN))
        .pb(px(MARGIN))
        .accessible(true)
        .label("Tabs")
        .child(each(chips, |(k, _)| *k, move |item, _| strip_chip(ui, item.get_untracked())))
        .child(glyph_button(glyphs::PLUS, "New Tab", Placement::Bottom, move || bars::open_new_tab(ui)))
}

fn strip_chip(ui: Ui, (key, pinned): (TabKey, bool)) -> AnyElement {
    let active = move || ui.read(|s| s.model.active == Some(key));
    let title = move || SharedString::from(ui.read(|s| s.tab_title(key)));
    let chip = pressable_view()
        .test_id("strip-tab")
        .label(title)
        .on_press(move |_: &PressEvent| activate(ui, key))
        .flex_row()
        .items_center()
        .h(move || px(Theme::tab_row()))
        .gap(px(8.))
        .rounded(px(Theme::radius_row()))
        .bg(move || if active() { Theme::selected() } else { Theme::clear() })
        .active(|s| s.bg(Theme::pressed()))
        .border_1()
        .border_color(move || if active() { Theme::surface_edge() } else { Theme::clear() })
        .child(view().w(px(GLYPH)).h(px(GLYPH)).flex_none().items_center().justify_center().child(tab_icon(ui, key, GLYPH, 14., true)));
    if pinned {
        return chip.w(move || px(Theme::tab_row() + 6.)).justify_center().into_element();
    }
    chip.flex_1()
        .min_w(px(0.))
        .max_w(px(200.))
        .pl(px(ROW_INSET))
        .pr(px(2.))
        .child(
            text(title)
                .text_size(px(TEXT))
                .text_color(move || if ui.read(|s| asleep(s, key)) && !active() { Theme::asleep() } else { Theme::text() })
                .style_when(active, |s| s.font_weight(FontWeight::MEDIUM))
                .flex_1()
                .truncate(),
        )
        .child(glyph_button(glyphs::CLOSE, "Close Tab", Placement::Bottom, move || act(ui, |s| s.close_or_unload(key))))
        .into_element()
}
