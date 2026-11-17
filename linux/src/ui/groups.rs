use browser_core::tabs::TabKey;
use nsh_core::prelude::*;
use store::Id;
use tarn_nsh_shared::theme::PROFILE_COLORS;

use super::panels::{capitalised, heading, menu_row, popover, rename_row};
use super::{Panel, TAB_ROW, Ui, act, list_row, read, show, split};
use crate::browser::Browser;
use crate::controls::{self, group, icon_button, label, small, title};
use crate::icons;
use crate::snapshot::Row;
use crate::theme::Theme;

const INNER_PAD: f32 = 3.;
const SWATCH: f32 = 22.;

/// Makes a group, holding `tab` when it is unpinned, and opens its panel to name it.
pub(super) fn new_group(ui: Ui, tab: Option<TabKey>) {
    let mut made = None;
    act(ui, |b| made = b.new_group(tab.or(b.model.active)));
    if let Some(id) = made {
        show(ui, Panel::Group(id));
    }
}

/// Opens a new tab in group `id`, its address field focused as for any new tab.
fn new_tab_in(ui: Ui, id: Id) {
    let before = read(ui, |b| b.model.active).flatten();
    super::new_tab(ui);
    act(ui, |b| {
        if let Some(key) = b.model.active.filter(|k| Some(*k) != before) {
            b.set_tab_group(key, Some(id));
        }
    });
}

/// The group's chosen symbol or emoji on its colour, else its initial, as on macOS.
fn badge(icon: &str, label: &str, fill: Rgba) -> View {
    let inside = match browser_core::icon::parse(icon) {
        Some(browser_core::icon::Icon::Symbol(name)) => {
            view().text_color(Theme::on_fill(fill)).child(icons::icon(icons::named(name).unwrap_or(icons::FOLDER), 11.)).into_element()
        }
        Some(browser_core::icon::Icon::Emoji(emoji)) => text(emoji.to_owned()).text_size(px(10.)).into_element(),
        None => text(initial(label)).text_size(px(9.)).font_weight(FontWeight::BOLD).text_color(Theme::on_fill(fill)).into_element(),
    };
    view().w(px(18.)).h(px(18.)).flex_none().rounded_full().items_center().justify_center().bg(fill).child(inside)
}

fn initial(name: &str) -> String {
    name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// Drops the dragged tab on a group's header: the tab joins the group.
fn drop_into(ui: Ui, id: Id) {
    if let Some(dragged) = ui.dragging.get_untracked() {
        split::release(ui);
        act(ui, |b| b.move_to_group(dragged, id));
    }
}

pub(super) fn group_card(ui: Ui, id: Id, item: ReadSignal<Row>) -> AnyElement {
    let name = move || item.with(|r| r.label.clone());
    let target = signal(false);
    let header = controls::row(move || Some(SharedString::from(name())), move || act(ui, |b| b.toggle_group(id)))
        .h(px(TAB_ROW - 2.))
        .pl(px(6.))
        .pr(px(2.))
        .child(dynamic(move || {
            let (label, color, icon) = item.with(|r| (r.label.clone(), r.color, r.icon.clone()));
            badge(&icon, &label, rgb(color))
        }))
        .child(label(name).font_weight(FontWeight::MEDIUM).truncate())
        .child(
            view()
                .text_color(Theme::muted())
                .child(dynamic(move || icons::icon(if item.with(|r| r.open) { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }, 12.))),
        )
        .child(view().flex_1())
        .child(icon_button(icons::CLOSE, "Close Group and Its Tabs", move || act(ui, |b| b.close_group(id))));
    let tabs = view()
        .flex_col()
        .gap(px(2.))
        .p(px(INNER_PAD))
        .rounded(px(Theme::radius_row() + INNER_PAD))
        .bg(Theme::raised())
        .border_1()
        .border_color(Theme::hairline())
        .child(each(move || item.with(|r| r.panes.clone()), |r| r.id(), move |pane, _| list_row(ui, false, pane)));
    view()
        .flex_col()
        .gap(px(2.))
        .my(px(2.))
        .p(px(INNER_PAD))
        .rounded(px(Theme::radius_tile()))
        .bg(Theme::group())
        .border_1()
        .border_color(move || if target.get() && ui.dragging.with(Option::is_some) { Theme::accent() } else { Theme::hairline() })
        .on_mouse_enter(move |_: &MouseEvent| target.set(true))
        .on_mouse_leave(move |_: &MouseEvent| target.set(false))
        .on_context_menu(move |_: &ContextMenuEvent| show(ui, Panel::Group(id)))
        .child(view().on_mouse_up(move |_: &MouseEvent| drop_into(ui, id)).child(header))
        .child(when(move || item.with(|r| !r.panes.is_empty()), move || tabs.clone()))
        .into_element()
}

fn swatch(ui: Ui, id: Id, name: &'static str, rgb_hex: u32, current: bool) -> AnyElement {
    let button = controls::pressable(capitalised(name), move || act(ui, |b| b.set_group_color(id, name)))
        .w(px(SWATCH))
        .h(px(SWATCH))
        .rounded_full()
        .bg(rgb(rgb_hex))
        .border_2()
        .border_color(if current { Theme::text() } else { Theme::clear() });
    view().tooltip(controls::tip(&capitalised(name), Placement::Top)).child(button).into_element()
}

pub(super) fn group_panel(ui: Ui, id: Id) -> AnyElement {
    let Some((name, color, current)) =
        read(ui, |b| b.model.groups.iter().find(|g| g.id == id).map(|g| (g.name.clone(), g.color.clone(), g.icon.clone()))).flatten()
    else {
        return view().into_element();
    };
    let swatches: Vec<AnyElement> = PROFILE_COLORS.iter().map(|(n, hex)| swatch(ui, id, n, *hex, color == *n)).collect();
    let close_panel = move |run: fn(&mut Browser, Id)| {
        move || {
            ui.panel.set(Panel::None);
            act(ui, |b| run(b, id));
        }
    };
    let rows = group()
        .child(menu_row("New Tab in Group", None, move || new_tab_in(ui, id)))
        .child(menu_row("Ungroup", None, close_panel(Browser::ungroup)))
        .child(menu_row("Close Group", None, close_panel(Browser::close_group)));
    popover()
        .child(heading(ui, title(format!("Group “{name}”")).truncate()))
        .child(rename_row(ui, format!("group {id}"), &name, move |name| act(ui, |b| b.rename_group(id, &name))))
        .child(view().flex_col().gap_2().child(small("Color".to_owned())).child(view().flex_row().flex_wrap().gap_2().children(swatches)))
        .child(super::panels::icon_choice(ui, format!("group {id}"), current, move |icon| {
            ui.emoji.set(browser_core::icon::emoji(&icon).to_owned());
            act(ui, |b| b.set_group_icon(id, &icon));
        }))
        .child(rows)
        .into_element()
}
