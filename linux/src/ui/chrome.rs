//! The sidebar while it holds the toolbar (`Browser::sidebar_chrome`): navigation and the address
//! pill at its top, chats and Tidy in the list, downloads, the space switcher and the profile at
//! its bottom edge.

use nsh_core::prelude::*;
use store::Id;

use super::panels::{heading, menu_row, popover};
use super::{Panel, Ui, act, assistant, extensions, show, site_tile};
use crate::browser::Browser;
use crate::controls::{self, body, group, icon_button, icon_button_at, small, title};
use crate::icons;
use crate::theme::{SYSTEM_FONT, Secure, TOOLBAR_HEIGHT, Theme, themed};

const PILL: f32 = 36.;

/// Back, Forward and Reload, their tooltips on `side`, away from the page.
pub(super) fn history(ui: Ui, side: Placement) -> View {
    let button = |icon: &str, name: &str, press: fn(Ui)| icon_button_at(icon, name, side, move || press(ui));
    view()
        .flex_row()
        .items_center()
        .gap(px(2.))
        .child(
            button(icons::BACK, "Back", |ui| act(ui, |b| b.back()))
                .disabled(move || !ui.snap.with(|s| s.can_back))
                .disabled_style(|s| s.text_color(Theme::faint())),
        )
        .child(
            button(icons::FORWARD, "Forward", |ui| act(ui, |b| b.forward()))
                .disabled(move || !ui.snap.with(|s| s.can_forward))
                .disabled_style(|s| s.text_color(Theme::faint())),
        )
        .child(super::reload_button(ui))
}

pub(super) fn header(ui: Ui) -> View {
    let top = view()
        .flex_row()
        .items_center()
        .h(px(TOOLBAR_HEIGHT - 8.))
        .pl_1()
        .child(icon_button_at(icons::SIDEBAR, "Hide Sidebar", Placement::Right, move || super::toggle_sidebar(ui)))
        .child(view().flex_1())
        .child(history(ui, Placement::Bottom));
    view().flex_col().pb(px(6.)).child(themed(move || top.clone())).child(address(ui))
}

/// The address field as a round pill, with the extension buttons beside it.
fn address(ui: Ui) -> View {
    let hovered = signal(false);
    let host =
        move || ui.snap.with(|s| if s.url == crate::browser::START_URL || s.url.is_empty() { String::new() } else { crate::input::display_host(&s.url) });
    let shown =
        controls::pressable("Address", move || super::open_palette(ui)).flex_row().flex_1().min_w(px(0.)).h_full().items_center().child(dynamic(move || {
            let host = host();
            if host.is_empty() {
                return body("Search or type a URL").text_color(Theme::faint()).flex_1().min_w(px(0.)).truncate();
            }
            body(host).text_color(if ui.snap.with(|s| s.secure) { Secure::text() } else { Secure::warning() }).flex_1().min_w(px(0.)).truncate()
        }));
    let pill = view()
        .flex_row()
        .flex_1()
        .min_w(px(0.))
        .items_center()
        .gap_2()
        .h(px(PILL))
        .pl(px(4.))
        .pr(px(4.))
        .rounded(px(PILL / 2.))
        .bg(Theme::raised)
        .border_1()
        .border_color(move || if ui.editing.get() { Theme::accent_text() } else { Theme::hairline() })
        .font_family(SYSTEM_FONT)
        .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
        .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
        .on_context_menu(move |_: &ContextMenuEvent| super::address_menu(ui))
        .child(themed(move || {
            if ui.snap.with(|s| s.host.is_empty()) {
                view().w(px(28.)).items_center().text_color(Theme::muted()).child(icons::icon(icons::SEARCH, 16.)).into_element()
            } else {
                super::shield_chip(ui).rounded_full().into_element()
            }
        }))
        .child(shown)
        .child(super::reader_chip(ui))
        .child(super::share_chip(ui, hovered));
    view().flex_row().items_center().gap_1().px(px(2.)).child(pill).child(themed(move || extensions::buttons(ui)))
}

/// The profile's latest assistant chats under a header that folds them and starts a new one.
pub(super) fn chats(ui: Ui) -> AnyElement {
    themed(move || {
        let rows = ui.snap.with(|s| s.chats.clone())?;
        let open = ui.chats_open.get();
        let header = controls::row("Chats", move || ui.chats_open.update(|o| *o = !*o))
            .h(move || px(Theme::tab_row()))
            .pl(px(10.))
            .pr(px(2.))
            .gap_1()
            .text_color(Theme::muted())
            .child(body("Chats").text_color(Theme::muted()))
            .child(view().text_color(Theme::faint()).child(icons::icon(if open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }, 12.)))
            .child(view().flex_1())
            .child(icon_button(icons::SQUARE_PEN, "New Chat", move || new_chat(ui)));
        let shown = ui.assistant.get().then(|| ui.chat.saved()).flatten();
        let rows = rows.into_iter().filter(|_| open).map(|row| {
            let id = row.id;
            controls::row(row.title.clone(), move || open_chat(ui, id))
                .h(move || px(Theme::tab_row()))
                .pl(px(10.))
                .gap(px(10.))
                .bg(if shown == Some(id) { Theme::selected() } else { Theme::clear() })
                .child(view().w(px(16.)).items_center().text_color(Theme::muted()).child(icons::icon(icons::MESSAGE_CIRCLE, 14.)))
                .child(body(browser_core::text::bounded_title(&row.title)).flex_1().truncate())
                .child(small(row.when).text_color(Theme::faint()))
                .into_element()
        });
        Some(
            view()
                .flex_col()
                .gap(px(2.))
                .mt_1()
                .child(view().on_context_menu(move |_: &ContextMenuEvent| show(ui, Panel::Chats)).child(header))
                .children(rows.collect::<Vec<_>>()),
        )
    })
}

fn new_chat(ui: Ui) {
    ui.chat.finish(assistant::Host(ui));
    ui.chat.close();
    assistant::open(ui, false);
}

fn open_chat(ui: Ui, id: Id) {
    ui.chat.restore(assistant::Host(ui), id);
    assistant::open(ui, false);
    // Redraws the list, which marks the chat now open.
    act(ui, |_| {});
}

pub(super) fn chats_menu(ui: Ui) -> AnyElement {
    popover()
        .child(heading(ui, title("Chats")))
        .child(
            group()
                .child(menu_row("Hide Chats in Sidebar", None, move || {
                    ui.panel.set(Panel::None);
                    super::set_pref(browser_core::prefs::SIDEBAR_CHATS, false);
                }))
                .child(menu_row("Customize in Settings…", None, move || super::open_settings(ui, tarn_nsh_shared::panes::Pane::Appearance))),
        )
        .into_element()
}

/// A rule across the list, with Tidy, which sorts the loose tabs into groups.
pub(super) fn tidy(ui: Ui) -> View {
    let enabled = move || ui.snap.with(|s| s.cleanable);
    let color = move || if enabled() { Theme::muted() } else { Theme::faint() };
    let button = controls::row("Tidy: sort loose tabs into groups", move || {
        if enabled() {
            act(ui, Browser::clean_up_tabs);
        }
    })
    .h(px(22.))
    .px(px(6.))
    .gap_1()
    .text_color(color)
    .child(icons::icon(icons::BROOM_SPARKLES, 13.))
    .child(small("Tidy").text_color(color));
    let button = super::keyboard_row(ui, "tidy".to_owned(), button);
    view().flex_row().items_center().gap_2().mx_2().my_1().h(px(24.)).child(view().flex_1().h(px(1.)).bg(Theme::hairline())).child(
        view()
            .tooltip(controls::tip("Tidy: sort loose tabs into groups — right-click or Shift+F10 for more", Placement::Top))
            .on_context_menu(move |_: &ContextMenuEvent| show(ui, Panel::Organize))
            .child(button),
    )
}

/// A closed card's first site icons and how many tabs it holds.
pub(super) fn summary(ui: Ui, (hosts, count): (Vec<String>, usize)) -> View {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .children(hosts.iter().map(|h| site_tile(ui, h, 14.).into_element()).collect::<Vec<_>>())
        .children((count > 0).then(|| small(count.to_string()).pl_1().pr(px(6.))))
}

/// What waits on the user outside the page: an agent at work in this window, or its question.
pub(super) fn pills(ui: Ui) -> AnyElement {
    themed(move || {
        let (text, asking) = ui.snap.with(|s| s.agent.as_ref().filter(|_| s.chrome).map(|a| (a.text.clone(), a.asking)))?;
        let tint = if asking { Theme::warning() } else { Theme::muted() };
        Some(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .mx(px(6.))
                .mb_1()
                .h(px(40.))
                .px_3()
                .rounded(px(20.))
                .bg(Theme::raised())
                .border_1()
                .border_color(Theme::hairline())
                .accessible(true)
                .label(browser_core::text::bounded_title(&text))
                .tooltip(controls::tip(&browser_core::text::bounded_title(&text), Placement::Top))
                .child(view().text_color(tint).child(icons::icon(icons::SPARKLES, 14.)))
                .child(
                    body(if asking { "Agent needs you" } else { "Agent at work" })
                        .text_color(if asking { Theme::warning() } else { Theme::text() })
                        .flex_1()
                        .truncate(),
                ),
        )
    })
}

/// Downloads, the space switcher and the profile, under the list.
pub(super) fn footer(ui: Ui) -> View {
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .h(px(44.))
        .px_1()
        .child(view().min_w(px(30.)).child(super::downloads_button(ui)))
        .child(view().flex_row().flex_1().min_w(px(0.)).justify_center().child(themed(move || space_switcher(ui))))
        .child(view().w(px(30.)).child(avatar(ui)))
}

fn space_switcher(ui: Ui) -> nsh_base::ButtonElement {
    let name = move || ui.snap.with(|s| s.space_name.clone());
    let open = move || {
        if let Some(key) = ui.snap.with_untracked(|s| s.space) {
            show(ui, Panel::SpaceMenu(key));
        }
    };
    controls::row(move || Some(SharedString::from(format!("Space {}", name()))), open)
        .min_w(px(0.))
        .h(px(28.))
        .px_2()
        .gap_1()
        .child(dynamic(move || {
            let (private, burner) = ui.snap.with(|s| (s.private, s.burner));
            private.then(|| view().text_color(Theme::warning()).child(icons::icon(if burner { icons::FLAME } else { icons::HAND }, 14.)))
        }))
        .child(text(name).text_size(px(14.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).truncate())
        .child(view().text_color(Theme::muted()).child(icons::icon(icons::CHEVRON_UP, 12.)))
}

fn avatar(ui: Ui) -> AnyElement {
    themed(move || {
        let (id, name, color) = ui.snap.with(|s| s.avatar.clone())?;
        let fill = Theme::profile_color(&color).unwrap_or(Theme::faint());
        let initial: String = name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        let name = browser_core::text::bounded_title(&name);
        Some(
            view().tooltip(controls::tip(&name, Placement::Top)).child(
                controls::pressable(format!("Profile {name}"), move || show(ui, Panel::Profile(id)))
                    .w(px(30.))
                    .h(px(30.))
                    .rounded_full()
                    .items_center()
                    .justify_center()
                    .child(
                        view()
                            .w(px(22.))
                            .h(px(22.))
                            .rounded_full()
                            .bg(fill)
                            .items_center()
                            .justify_center()
                            .child(text(initial).text_size(px(11.)).font_weight(FontWeight::BOLD).text_color(Theme::on_fill(fill))),
                    ),
            ),
        )
    })
}

/// `id` is `None` for a private space's profile, which is not edited from here.
pub(super) fn profile_menu(ui: Ui, id: Option<Id>) -> AnyElement {
    let keys = |action| super::shortcut_of(ui, action);
    let close_then = move |f: fn(Ui)| {
        move || {
            ui.panel.set(Panel::None);
            f(ui);
        }
    };
    let edit = id.map(|_| menu_row("Edit Profile…", None, move || super::open_settings(ui, tarn_nsh_shared::panes::Pane::Profile)));
    popover()
        .child(heading(ui, title("Profile")))
        .child(
            group()
                .child(menu_row("Settings…", None, move || show(ui, Panel::Settings)))
                .child(menu_row("Show Library", None, move || show(ui, Panel::Library))),
        )
        .child(group().children(edit).child(menu_row("New Profile…", None, close_then(|ui| super::profiles::open_profile_card(ui, None)))).child(menu_row(
            "New Space…",
            keys(crate::shortcuts::Action::NewSpace),
            move || show(ui, Panel::NewSpace),
        )))
        .into_element()
}

/// The spaces to switch to, listed first in the space menu at the sidebar's bottom edge.
pub(super) fn space_rows(ui: Ui) -> View {
    let spaces = ui.snap.with_untracked(|s| s.spaces.iter().filter(|x| x.shown).cloned().collect::<Vec<_>>());
    group().children(spaces.into_iter().map(|space| {
        let ix = space.ix;
        let name = if space.private { space.label.clone() } else { space.label.rsplit(" · ").next().unwrap_or_default().to_owned() };
        controls::row(name.clone(), move || {
            ui.panel.set(Panel::None);
            tarn_nsh_shared::motion::space_switch();
            act(ui, |b| b.switch_space_by_user(ix));
        })
        .px_2()
        .gap(px(10.))
        .child(view().w(px(16.)).items_center().text_color(Theme::accent_text()).children(space.current.then(|| icons::icon(icons::CHECK, 14.))))
        .child(body(browser_core::text::bounded_title(&name)).flex_1().truncate())
        .into_element()
    }))
}

/// The page toolbar while the sidebar holds it: only the developer buttons stay above the page.
pub(super) fn folded_toolbar(ui: Ui) -> AnyElement {
    dynamic(move || {
        ui.snap.with(|s| s.devtools).then(|| {
            view()
                .flex_row()
                .items_center()
                .justify_end()
                .gap(px(2.))
                .h(px(TOOLBAR_HEIGHT))
                .child(super::devtools::responsive_button(ui))
                .child(super::devtools::measure_button(ui))
        })
    })
}

/// A tab dropped on an empty folder card moves into the folder.
pub(super) fn drop_into_folder(ui: Ui, id: Id) {
    if let Some(dragged) = ui.dragging.get_untracked() {
        super::split::release(ui);
        act(ui, |b: &mut Browser| b.move_to_folder(dragged, id));
    }
}
