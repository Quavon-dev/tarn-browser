use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use browser_core::sidebar::{self as bar, DropAt, Row, RowKind, Swipe};
use browser_core::tabs::TabKey;
use nsh_base::{PopoverEvent, button, input, popover};
use nsh_core::keymap::When;
use nsh_core::prelude::*;
use store::{Id, TabKind};
use tarn_nsh_shared::controls::{avatar, site_tile};
use tarn_nsh_shared::icons as glyphs;
use tarn_nsh_shared::motion::spinner;
use tarn_nsh_shared::sidebar::{
    BUTTON, CLOSE_GAP, CLOSE_WIDTH, FOOTER_HEIGHT, GLYPH, HEADER_HEIGHT, HEADER_INSET, INDENT, INNER_PAD, MARGIN, ROW_GAP, ROW_INSET, STATE_GLYPH, TAB_ROW,
    TILE_GAP, TILE_ICON, WIDTH, divider, glyph_button, group_color, menu_card, menu_heading, menu_item, menu_separator, new_tab_mark, progress_ring, tile_grid,
    tip,
};

use super::super::{AppShell, Screen, Ui, bars, panels};
use crate::theme::{Paint, TEXT, TEXT_SMALL, Theme, themed};

#[derive(Clone, Copy, PartialEq)]
enum Target {
    Tab(TabKey),
    Folder(Id),
    Group(Id),
    Space,
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

#[derive(Clone, Copy)]
struct Bar {
    ui: Ui,
    menu: Signal<Option<(Target, Point<Points>)>>,
    layout: Memo<bar::Sidebar>,
    renaming: Signal<Option<TabKey>>,
    pointer: Signal<bool>,
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

fn host(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()
}

fn tab_title(s: &AppShell, key: TabKey) -> String {
    bar::shown_title(&s.model, key, s.tab_title(key))
}

fn asleep(s: &AppShell, key: TabKey) -> bool {
    s.model.active != Some(key) && !s.model.lifecycle.live().any(|k| k == key)
}

fn tab_icon(ui: Ui, key: TabKey, size: f32, spin: f32) -> AnyElement {
    let state = computed(move || {
        ui.read(|s| {
            let url = s.tab_url(key);
            (s.pages.get(&key).is_some_and(|p| p.loading), s.site_icon(&url), host(&url), asleep(s, key))
        })
    });
    dynamic(move || {
        let (loading, icon, host, asleep) = state.get();
        if loading && !untracked(reduce_motion) {
            return view().text_color(Theme::space_label).child(spinner(spin, glyphs::icon(glyphs::RELOAD, spin))).into_element();
        }
        site_tile(icon, &host, size).opacity(if asleep { 0.4_f32 } else { 1. }).into_element()
    })
}

fn open_menu(bar: Bar, target: Target, at: Point<Points>) {
    bar.menu.set(Some((target, at)));
}

fn menu_anchor(bar: Bar, target: Target) -> AnyElement {
    dynamic(move || {
        let at = bar.menu.with(|m| m.filter(|(t, _)| *t == target).map(|(_, at)| at))?;
        let close = move |e: &PopoverEvent| {
            if let PopoverEvent::OpenChange(false, _) = e {
                bar.menu.set(None);
            }
        };
        Some(
            view().absolute().left(px(at.x.0)).top(px(at.y.0)).w(px(0.)).h(px(0.)).child(
                popover()
                    .open(move || Some(true))
                    .on_open_change(close)
                    .placement(Placement::Bottom)
                    .align(Align::Start)
                    .offset(Points(2.))
                    .trigger(view().w(px(0.)).h(px(0.)))
                    .child(menu_card().child(menu_items(bar, target))),
            ),
        )
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
    };
    view().flex_col().children(items)
}

fn tab_items(bar: Bar, key: TabKey) -> Vec<AnyElement> {
    let ui = bar.ui;
    let Some((kind, group, custom)) = ui.read(|s| s.model.tab(key).map(|t| (t.kind, t.group_id, !t.custom_title.is_empty()))) else { return Vec::new() };
    let url = ui.read(|s| s.tab_url(key));
    let (active, burner, split, in_split, spaces, groups, others, below) = ui.read(|s| {
        let groups: Vec<(Id, String)> = s.model.groups.iter().filter(|g| Some(g.id) != group).map(|g| (g.id, g.name.clone())).collect();
        (
            s.model.active == Some(key),
            s.model.burner_shown(),
            s.splits.menu(s.model.active, key),
            s.splits.contains(key),
            if s.in_burner() { Vec::new() } else { s.move_targets(key) },
            groups,
            !bar::other_unpinned(&s.model, key).is_empty(),
            !bar::tabs_below(&s.model, key).is_empty(),
        )
    });
    let ephemeral = kind == TabKind::Ephemeral;
    let mut items = vec![if active {
        item(bar, "Reload", true, |ui| ui.act(|s| s.active_command("reload", serde_json::Value::Null)))
    } else {
        item(bar, "Open", true, move |ui| activate(ui, key))
    }];
    let link = url.clone();
    items.extend([
        item(bar, "Duplicate", !url.is_empty(), move |ui| {
            act(ui, |s| {
                s.new_tab(Some(&url));
            });
            ui.show(Screen::Browse);
        }),
        menu_separator().into_element(),
        item(bar, "Copy Link", !link.is_empty(), move |ui| {
            set_clipboard(link.clone());
            act(ui, |s| s.toast_as(browser_core::toast::Tone::Success, "Link copied".to_owned()));
        }),
    ]);
    if ui.wide() {
        items.push(menu_separator().into_element());
        items.push(item(bar, split.entry.label(), split.enabled, move |ui| {
            act(ui, |s| s.split_with(key));
            ui.show(Screen::Browse);
        }));
        if in_split {
            items.push(item(bar, "Remove from Split View", true, move |ui| act(ui, |s| s.leave_split(key))));
        }
    }
    items.extend([
        menu_separator().into_element(),
        item(bar, if kind == TabKind::Pinned { "Unpin Tab" } else { "Pin Tab" }, !burner && kind != TabKind::Tile, move |ui| {
            act(ui, |s| {
                bar::toggle_pin(&mut s.model, &s.core, key);
            })
        }),
        item(bar, if kind == TabKind::Tile { "Remove from Tiles" } else { "Add to Tiles" }, !burner, move |ui| {
            act(ui, |s| {
                bar::toggle_tile(&mut s.model, &s.core, key);
            })
        }),
        item(bar, "Rename", kind != TabKind::Tile, move |_| bar.renaming.set(Some(key))),
    ]);
    if custom {
        items.push(item(bar, "Reset Title", true, move |ui| {
            act(ui, |s| {
                s.core.store(key.private).set_tab_custom_title(key.id, "").ok();
                s.model.reload_space(&s.core);
            })
        }));
    }
    if !spaces.is_empty() {
        items.push(menu_separator().into_element());
        items.push(menu_heading("Move to Space").into_element());
        items.extend(spaces.into_iter().map(|(ix, name)| {
            item(bar, name, true, move |ui| {
                act(ui, |s| {
                    s.move_tab(key, ix);
                })
            })
        }));
    }
    if ephemeral {
        items.push(menu_separator().into_element());
        items.push(menu_heading("Add to Group").into_element());
        items.extend(groups.into_iter().map(|(id, name)| {
            item(bar, name, true, move |ui| {
                act(ui, |s| {
                    bar::set_group(&mut s.model, &s.core, key, Some(id));
                })
            })
        }));
        items.push(item(bar, "New Group", true, move |ui| {
            act(ui, |s| {
                bar::new_group(&mut s.model, &s.core, Some(key));
            })
        }));
        if group.is_some() {
            items.push(item(bar, "Remove from Group", true, move |ui| {
                act(ui, |s| {
                    bar::set_group(&mut s.model, &s.core, key, None);
                })
            }));
        }
    }
    items.push(menu_separator().into_element());
    if !ephemeral {
        items.push(item(bar, "Unload Tab", true, move |ui| act(ui, |s| s.unload_tab(key))));
        return items;
    }
    items.push(item(bar, "Close Tab", true, move |ui| act(ui, |s| s.close_tab(key))));
    items.push(item(bar, "Close Other Tabs", others, move |ui| act(ui, |s| close(s, &bar::other_unpinned(&s.model, key)))));
    items.push(item(bar, "Close Tabs Below", below, move |ui| act(ui, |s| close(s, &bar::tabs_below(&s.model, key)))));
    items
}

fn folder_items(bar: Bar, id: Id) -> Vec<AnyElement> {
    vec![
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

fn space_items(bar: Bar) -> Vec<AnyElement> {
    let (ix, private) = bar.ui.read(|s| (s.model.space_ix, s.model.current_space().is_none_or(|c| c.private)));
    vec![item(bar, "Edit Space…", !private, move |ui| ui.show(Screen::SpaceEdit(ix))), item(bar, "New Space…", true, |ui| ui.show(Screen::Spaces))]
}

fn close(s: &mut AppShell, keys: &[TabKey]) {
    if keys.is_empty() {
        return;
    }
    if s.in_burner() {
        s.burn();
    } else {
        s.close_tabs(keys);
    }
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

fn with_menu(bar: Bar, target: Target, element: View) -> View {
    element
        .relative()
        .on_context_menu(move |e: &ContextMenuEvent| open_menu(bar, target, e.position))
        .on_long_press(move |e: &PressEvent| open_menu(bar, target, e.position))
        .child(menu_anchor(bar, target))
}

fn state_glyph(ui: Ui, key: TabKey, busy: impl Fn() -> bool + 'static) -> AnyElement {
    let state = computed(move || {
        ui.read(|s| match (asleep(s, key), s.model.tab(key).map(|t| t.kind)) {
            _ if busy() => None,
            (true, _) => Some((glyphs::MOON, "Asleep")),
            (false, Some(TabKind::Pinned)) => Some((glyphs::PIN, "Pinned")),
            _ => None,
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
    let shown = move || closable() && (hovered.get() || active());
    let title = move || SharedString::from(ui.read(|s| tab_title(s, key)));
    let split = move || ui.read(|s| (s.splits.contains(key) && s.model.active != Some(key)).then(|| s.shown_split().is_some_and(|p| p.panes().contains(&key))));
    let close = view()
        .flex_none()
        .overflow_hidden()
        .w(move || px(CLOSE_WIDTH * f32::from(u8::from(shown()))))
        .ml(move || px(-CLOSE_GAP * f32::from(u8::from(!shown()))))
        .opacity(move || f32::from(u8::from(shown())))
        .child(glyph_button(glyphs::CLOSE, "Close Tab", Placement::Top, move || act(ui, |s| s.close_tab(key))));
    let row = pressable_view()
        .label(title)
        .on_press(move |e: &PressEvent| {
            activate(ui, key);
            if e.clicks == 2 && renamable(ui, key) {
                bar.renaming.set(Some(key));
            }
        })
        .relative()
        .flex_row()
        .items_center()
        .h(px(TAB_ROW))
        .pl(px(ROW_INSET + depth as f32 * INDENT))
        .pr(px(2.))
        .gap(px(CLOSE_GAP))
        .rounded(px(Theme::radius_row()))
        .bg(move || match (active(), hovered.get()) {
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
        .child(view().w(px(GLYPH)).h(px(GLYPH)).flex_none().items_center().justify_center().child(tab_icon(ui, key, GLYPH, 14.)))
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
        .child(state_glyph(ui, key, shown))
        .child(when(closable, move || close.clone()));
    draggable(bar, key, with_menu(bar, Target::Tab(key), row))
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
}

fn renamable(ui: Ui, key: TabKey) -> bool {
    ui.read(|s| s.model.tab(key).is_some_and(|t| t.kind != TabKind::Tile))
}

fn spot(bar: Bar, at: Spot, element: View) -> View {
    let handle = element_ref();
    bar.spots.with(|m| m.insert(at, handle));
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
    let start = ui.read(|s| tab_title(s, key));
    let typed = stored(start.clone());
    let done = move |commit: bool| {
        if bar.renaming.get_untracked() != Some(key) {
            return;
        }
        let name = typed.get();
        bar.renaming.set(None);
        if commit {
            act(ui, |s| {
                let page = s.tab_title(key);
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
        .h(px(TAB_ROW - 8.))
        .px_1()
        .rounded(px(Theme::radius_small()))
        .bg(Theme::raised)
        .text_size(px(TEXT))
        .text_color(Theme::text)
        .into_element()
}

fn row(bar: Bar, r: &Row) -> View {
    match &r.kind {
        RowKind::Tab(key) => tab_row(bar, *key, r.depth),
        RowKind::Split(panes) => view()
            .flex_row()
            .gap(px(ROW_GAP))
            .pl(px(r.depth as f32 * INDENT))
            .children(panes.iter().map(|k| view().flex_1().min_w(px(0.)).child(tab_row(bar, *k, 0)))),
        RowKind::Folder(id) => folder_row(bar, *id, r.depth),
    }
}

fn rows(bar: Bar, list: impl Fn() -> Vec<Row> + 'static) -> AnyElement {
    each(list, |r: &Row| r.clone(), move |r, _| row(bar, &r.get_untracked()))
}

fn folder_row(bar: Bar, id: Id, depth: usize) -> View {
    let ui = bar.ui;
    let name = move || ui.read(|s| s.model.folders.iter().find(|f| f.id == id).map(|f| f.name.clone()).unwrap_or_default());
    let open = move || ui.read(|s| s.model.expanded.contains(&id));
    let header = pressable_view()
        .label(move || SharedString::from(name()))
        .on_press(move || act(ui, |s| s.model.toggle_folder(id)))
        .flex_row()
        .items_center()
        .h(px(TAB_ROW))
        .pl(px(ROW_INSET + depth as f32 * INDENT))
        .pr(px(ROW_INSET))
        .gap(px(CLOSE_GAP))
        .rounded(px(Theme::radius_row()))
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .child(view().w(px(GLYPH)).h(px(GLYPH)).items_center().justify_center().text_color(Theme::space_label).child(glyphs::icon(glyphs::FOLDER, 14.)))
        .child(text(name).text_size(px(TEXT)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text).truncate())
        .child(view().text_color(Theme::muted).child(dynamic(move || glyphs::icon(if open() { glyphs::CHEVRON_DOWN } else { glyphs::CHEVRON_RIGHT }, 12.))));
    spot(bar, Spot::Folder(id), with_menu(bar, Target::Folder(id), header))
}

fn pinned_block(bar: Bar, block: Vec<Row>) -> View {
    let ui = bar.ui;
    let list = view().flex_col().gap(px(ROW_GAP)).children(block.iter().map(|r| row(bar, r)));
    let Some(RowKind::Folder(id)) = block.first().map(|r| r.kind.clone()) else { return list };
    let open = move || ui.read(|s| s.model.expanded.contains(&id));
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
        .gap(px(ROW_GAP))
        .mt_1()
        .min_h(px(12.))
        .rounded(px(Theme::radius_row()))
        .child(each(move || bar.layout.with(|l| l.pinned.clone()), |b: &Vec<Row>| b.clone(), move |b, _| pinned_block(bar, b.get_untracked())))
        .child(when(
            move || ui.read(|s| !s.model.tabs.iter().any(|t| t.kind == TabKind::Pinned) && s.model.folders.is_empty()),
            || {
                view()
                    .h(px(TAB_ROW))
                    .px(px(ROW_INSET))
                    .justify_center()
                    .rounded(px(Theme::radius_row()))
                    .border_1()
                    .border_dashed()
                    .border_color(Theme::hairline)
                    .child(text("Drag tabs here to pin them").text_size(px(TEXT_SMALL)).text_color(Theme::faint))
            },
        ));
    spot(bar, Spot::Pinned, area)
}

fn tiles(bar: Bar) -> View {
    let ui = bar.ui;
    let (_, side) = tile_grid(WIDTH);
    let grid = view().flex_row().flex_wrap().gap(px(TILE_GAP)).child(each(
        move || bar.layout.with(|l| l.tiles.clone()),
        |k: &TabKey| *k,
        move |key, _| {
            let key = key.get_untracked();
            let active = move || ui.read(|s| s.model.active == Some(key));
            let split = move || ui.read(|s| (s.splits.contains(key), s.shown_split().is_some_and(|p| p.panes().contains(&key))));
            let tile = pressable_view()
                .label(move || SharedString::from(ui.read(|s| tab_title(s, key))))
                .on_press(move || activate(ui, key))
                .w(px(side))
                .h(px(side))
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_tile()))
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
                .child(tab_icon(ui, key, TILE_ICON, 18.));
            draggable(bar, key, with_menu(bar, Target::Tab(key), tile))
        },
    ));
    spot(bar, Spot::Tiles, grid)
}

fn new_tab_row(bar: Bar) -> View {
    let ui = bar.ui;
    let new = button()
        .accessibility_label("New Tab")
        .on_press(move || bars::open_new_tab(ui))
        .flex_1()
        .flex_row()
        .items_center()
        .h(px(TAB_ROW))
        .pl(px(ROW_INSET))
        .gap(px(CLOSE_GAP))
        .rounded(px(Theme::radius_row()))
        .text_color(Theme::muted)
        .hover(|s| s.bg(Theme::hover()).text_color(Theme::text()))
        .active(|s| s.bg(Theme::pressed()))
        .child(new_tab_mark())
        .child(text("New Tab").text_size(px(TEXT)).flex_1());
    view()
        .flex_row()
        .items_center()
        .pr(px(2.))
        .child(new)
        .child(glyph_button(glyphs::GROUP, "New Tab Group", Placement::Top, move || {
            act(ui, |s| {
                let active = s.model.active;
                bar::new_group(&mut s.model, &s.core, active);
            })
        }))
        .child(glyph_button(glyphs::FOLDER_PLUS, "New Folder", Placement::Top, move || {
            act(ui, |s| {
                bar::new_folder(&mut s.model, &s.core, None);
            })
        }))
        .child(when(
            move || ui.read(|s| bar::cleanable(&s.model)),
            move || {
                glyph_button(glyphs::BROOM_SPARKLES, "Clean Up: sorts tabs into groups by what each page is. Runs on this device.", Placement::Top, move || {
                    act(ui, |s| {
                        let private = s.model.private();
                        let urls: std::collections::HashMap<Id, String> =
                            s.model.tabs.iter().map(|t| (t.id, s.tab_url(TabKey { private, id: t.id }))).collect();
                        let sorted = bar::clean_up(&mut s.model, &s.core, |t| urls.get(&t.id).cloned().unwrap_or_else(|| t.url.clone()));
                        s.toast(bar::clean_up_text(sorted));
                    })
                })
                .accessibility_label("Clean Up")
            },
        ))
}

fn group_card(bar: Bar, id: Id) -> View {
    let ui = bar.ui;
    let group = move || ui.read(|s| s.model.groups.iter().find(|g| g.id == id).map(|g| (g.name.clone(), g.color.clone(), g.collapsed)).unwrap_or_default());
    let name = move || group().0;
    let header = pressable_view()
        .label(move || SharedString::from(name()))
        .on_press(move || {
            act(ui, |s| {
                s.model.toggle_group(&s.core, id);
            })
        })
        .flex_row()
        .items_center()
        .h(px(TAB_ROW - 2.))
        .pl(px(6.))
        .pr(px(2.))
        .gap_2()
        .rounded(px(Theme::radius_row()))
        .hover(|s| s.bg(Theme::hover()))
        .active(|s| s.bg(Theme::pressed()))
        .child(dynamic(move || {
            let (name, color, _) = group();
            let initial = name.trim().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            avatar(initial, group_color(&color, id), 18.).into_element()
        }))
        .child(text(name).text_size(px(TEXT)).font_weight(FontWeight::MEDIUM).text_color(Theme::text).truncate())
        .child(view().text_color(Theme::muted).child(dynamic(move || glyphs::icon(if group().2 { glyphs::CHEVRON_RIGHT } else { glyphs::CHEVRON_DOWN }, 12.))))
        .child(view().flex_1())
        .child(glyph_button(glyphs::CLOSE, "Close Group and Its Tabs", Placement::Top, move || close_group(ui, id)));
    let tabs = move || bar.layout.with(|l| l.groups.iter().find(|(g, _)| *g == id).map(|(_, rows)| rows.clone()).unwrap_or_default());
    let card = view()
        .flex_col()
        .gap(px(ROW_GAP))
        .my(px(2.))
        .p(px(INNER_PAD))
        .rounded(px(Theme::radius_tile()))
        .bg(Theme::group)
        .border_1()
        .border_color(Theme::hairline)
        .child(with_menu(bar, Target::Group(id), header))
        .child(when(
            move || !tabs().is_empty(),
            move || {
                view()
                    .flex_col()
                    .gap(px(ROW_GAP))
                    .p(px(INNER_PAD))
                    .overflow_hidden()
                    .rounded(px(Theme::radius_row() + INNER_PAD))
                    .bg(Theme::raised)
                    .border_1()
                    .border_color(Theme::hairline)
                    .child(rows(bar, tabs))
            },
        ));
    spot(bar, Spot::Group(id), card)
}

fn ephemeral(bar: Bar) -> View {
    let ui = bar.ui;
    let area = view()
        .flex_col()
        .gap(px(ROW_GAP))
        .min_h(px(120.))
        .pb(px(TAB_ROW * 2.))
        .child(new_tab_row(bar))
        .child(each(
            move || ui.read(|s| s.model.groups.iter().map(|g| g.id).collect::<Vec<_>>()),
            |id: &Id| *id,
            move |id, _| group_card(bar, id.get_untracked()),
        ))
        .child(rows(bar, move || bar.layout.with(|l| l.loose.clone())));
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
        .child(text(name).text_size(px(14.)).font_weight(FontWeight::BOLD).text_color(Theme::text).truncate())
        .child(view().flex_none().text_color(Theme::muted).child(glyphs::icon(glyphs::CHEVRON_DOWN, 14.)));
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

fn profile_avatar(ui: Ui) -> AnyElement {
    dynamic(move || {
        let (private, burner, name, color) = ui.read_data(|s| {
            let profile = s.model.current_space().filter(|c| !c.private).and_then(|c| s.core.disk.profile(c.space.profile_id).ok());
            (
                s.model.private(),
                s.model.burner_shown(),
                profile.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
                profile.map(|p| p.color).unwrap_or_default(),
            )
        });
        let tip_text = if private { "Private space".to_owned() } else { name.clone() };
        let face = if private {
            view()
                .w(px(24.))
                .h(px(24.))
                .items_center()
                .justify_center()
                .text_color(Theme::warning)
                .child(glyphs::icon(if burner { glyphs::FLAME } else { glyphs::HAND }, GLYPH))
        } else {
            let initial = name.trim().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            avatar(initial, Theme::profile_color(&color).unwrap_or(Theme::accent()), 24.)
        };
        button()
            .accessibility_label(tip_text.clone())
            .on_press(move || ui.show(Screen::Settings))
            .w(px(BUTTON))
            .h(px(BUTTON))
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_full()
            .hover(|s| s.bg(Theme::hover()))
            .active(|s| s.bg(Theme::pressed()))
            .child(view().size_full().items_center().justify_center().tooltip(tip(&tip_text, Placement::Bottom)).child(face))
    })
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
        .child(profile_avatar(bar.ui))
}

fn space_buttons(ui: Ui) -> AnyElement {
    each(
        move || {
            ui.read(|s| {
                s.spaces()
                    .into_iter()
                    .filter_map(|item| {
                        let space = s.model.spaces.get(item.ix)?;
                        let glyph = if item.burner { glyphs::FLAME } else { glyphs::space_icon(&space.space.icon) };
                        Some((item.ix, item.name, glyph, item.private, space.space.color.clone(), item.ix == s.model.space_ix))
                    })
                    .collect::<Vec<_>>()
            })
        },
        |s: &(usize, String, &'static str, bool, String, bool)| s.clone(),
        move |space, _| {
            let (ix, name, glyph, private, color, current) = space.get_untracked();
            let tint = space_tint(&color, private);
            let color = match (current, private, tint) {
                (true, true, _) => Theme::warning(),
                (true, false, tint) => tint.unwrap_or(Theme::text()),
                (false, _, Some(tint)) => Rgba { a: 0.45, ..tint },
                (false, _, None) => Theme::faint(),
            };
            button()
                .accessibility_label(name.clone())
                .on_press(move || act(ui, |s| s.choose_space(ix, browser_core::now())))
                .w(px(BUTTON))
                .h(px(BUTTON))
                .flex_none()
                .items_center()
                .justify_center()
                .rounded(px(Theme::radius_row()))
                .bg(if current { Theme::hover() } else { Theme::clear() })
                .hover(|s| s.bg(Theme::hover()))
                .active(|s| s.bg(Theme::pressed()))
                .text_color(color)
                .child(view().size_full().items_center().justify_center().tooltip(tip(&name, Placement::Top)).child(glyphs::icon(glyph, GLYPH)))
        },
    )
}

fn footer(ui: Ui) -> View {
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
        .child(scroll().horizontal().flex_1().min_w(px(0.)).child(view().flex_row().flex_grow(1.).justify_center().gap(px(2.)).child(space_buttons(ui))))
        .child(glyph_button(glyphs::PLUS, "New Space", Placement::Top, move || ui.show(Screen::Spaces)))
        .child(glyph_button(glyphs::GEAR, "Settings", Placement::Top, move || ui.toggle(Screen::Settings)))
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
        menu: signal(None),
        layout: computed(move || ui.read(|s| bar::sidebar(&s.model, &s.splits))),
        renaming: signal(None),
        pointer: signal(false),
        spots: stored(HashMap::new()),
    };
    let swipe = Rc::new(Cell::new(Swipe::default()));
    let list = view()
        .flex_col()
        .px(px(TILE_GAP))
        .pb_2()
        .text_size(px(TEXT))
        .child(when(move || ui.read(|s| s.model.tabs.iter().any(|t| t.kind == TabKind::Tile)), move || tiles(bar).pb_1()))
        .child(pinned(bar))
        .child(divider())
        .child(ephemeral(bar));
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
        .child(header(bar))
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
                        act(ui, |s| {
                            if let Some(ix) = bar::neighbour_space(&s.model, step) {
                                s.choose_space(ix, browser_core::now());
                            }
                        });
                    }
                })
                .child(scroll().flex_1().child(list)),
        )
        .child(footer(ui))
}
