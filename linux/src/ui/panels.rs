use nsh_core::element::IntoProp;
use nsh_core::prelude::*;
use shield::ShieldLevel;

use super::tools::{Command, line, note};
use super::{Panel, Ui, act, confirmed, read, show, site_tile};
use crate::browser::{Browser, ZOOM_STEPS};
use crate::controls::{body, card, group, icon_button, label, pill, segmented, small, title};
use crate::icons;
use crate::shortcuts::{self, Action};
use crate::theme::Theme;
use crate::theme::{SYSTEM_FONT, Secure};
use browser_core::shield_card::{motto, thousands};
use store::TabKind;
use tarn_nsh_shared::controls::{caps_label, glyph_row, glyph_switch};

const PANEL_WIDTH: f32 = 720.;
const SITE_WIDTH: f32 = 340.;

enum Side {
    Start,
    Centre,
    End,
}

pub(super) fn levels() -> Vec<(ShieldLevel, String)> {
    ShieldLevel::ALL.iter().map(|l| (*l, capitalised(l.as_str()))).collect()
}

pub(super) fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

pub(super) use tarn_nsh_shared::controls::section;

pub(super) fn counted(name: &str, count: usize) -> View {
    view().flex_row().items_baseline().gap_2().flex_1().child(title(name.to_owned())).child(small(count.to_string()).text_color(Theme::faint()))
}

pub(super) fn library_row<M>(
    ui: Ui,
    name: impl IntoProp<Option<SharedString>, M>,
    host: &str,
    label_text: String,
    press: impl Fn() + 'static,
) -> nsh_base::ButtonElement {
    crate::controls::row(name, press)
        .h(px(30.))
        .child(site_tile(ui, host, 16.))
        .child(label(label_text).flex_1().truncate())
        .child(small(host.trim_start_matches("www.").to_owned()).max_w(px(220.)).truncate().text_color(Theme::faint()))
}

pub(super) fn popover() -> View {
    card().gap_3().p_4()
}

/// A panel's first row: `content`, and the button that closes the panel.
pub(super) fn heading(ui: Ui, content: impl IntoElement) -> View {
    view().flex_row().items_center().gap_2().child(view().flex_row().items_center().flex_1().min_w(px(0.)).child(content)).child(icon_button(
        icons::CLOSE,
        "Close",
        move || show(ui, Panel::None),
    ))
}

fn shield_panel(ui: Ui) -> AnyElement {
    let (host, level, overridden, secure, zoom, total) = ui.snap.with(|s| (s.host.clone(), s.level.clone(), s.overridden, s.secure, s.zoom, s.site_total));
    let default = read(ui, |b| b.default_level()).unwrap_or_default();
    let set = move |level: Option<ShieldLevel>| act(ui, |b| b.set_site_level(level));
    let (glyph, color, connection) =
        if secure { (icons::LOCK, Secure::success(), "Connection is secure") } else { (icons::TRIANGLE_ALERT, Secure::warning(), "Not secure") };
    let header = view()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .child(site_tile(ui, &host, 28.))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.))
                .child(label(if host.is_empty() { "This page".to_owned() } else { host.clone() }).font_weight(FontWeight::BOLD).truncate())
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .text_color(color)
                        .child(icons::icon(glyph, 11.))
                        .child(small(connection).text_color(color).font_family(SYSTEM_FONT)),
                ),
        )
        .child(icon_button(icons::CLOSE, "Close", move || show(ui, Panel::None)));
    let body = popover().gap_3().child(header);
    if host.is_empty() {
        return body.into_element();
    }
    let summary = view()
        .flex_col()
        .px_3()
        .py_2()
        .rounded(px(Theme::radius_group()))
        .bg(Theme::panel_row())
        .border_1()
        .border_color(Theme::surface_edge())
        .child(text(thousands(total.max(0) as u64)).text_size(px(22.)).font_weight(FontWeight::BOLD).text_color(Theme::text()))
        .child(small("stopped on this site in this profile"));
    let mut choices = vec![(None, "Default".to_owned())];
    choices.extend(levels().into_iter().map(|(l, name)| (Some(l), name)));
    let effective = ShieldLevel::parse(&level).unwrap_or_default();
    let meaning = if overridden {
        motto(effective).1.to_owned()
    } else {
        let (name, words) = motto(default);
        format!("Follows the profile: {name} — {}", words.to_lowercase())
    };
    let chosen = overridden.then_some(effective);
    let zoom_button = |icon: &'static str, name: &str, step: i32, limit: f64| {
        icon_button(icon, name, move || act(ui, move |b| b.zoom(step))).disabled((zoom - limit).abs() < 0.001).disabled_style(|s| s.text_color(Theme::faint()))
    };
    let dark = read(ui, Browser::site_dark).unwrap_or_default();
    let list = group()
        .child(
            glyph_switch(icons::MOON, "Dark mode for this site", move || dark, move || act(ui, Browser::toggle_site_dark))
                .border_b_1()
                .border_color(Theme::separator()),
        )
        .child(
            glyph_row(icons::ZOOM_IN, "Zoom").child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .child(zoom_button(icons::CHEVRON_LEFT, "Zoom Out", -1, ZOOM_STEPS[0]))
                    .child(small(format!("{:.0} %", zoom * 100.0)).w(px(48.)).text_center())
                    .child(zoom_button(icons::CHEVRON_RIGHT, "Zoom In", 1, ZOOM_STEPS[ZOOM_STEPS.len() - 1]))
                    .child(pill("Reset", move || act(ui, |b| b.zoom(0)))),
            ),
        );
    let more = ui.site_more.get_untracked();
    let toggle = crate::controls::row(if more { "Fewer site settings" } else { "More site settings" }, move || {
        ui.site_more.update(|m| *m = !*m);
        act(ui, |_| {});
    })
    .px_1()
    .gap_1()
    .text_color(Theme::accent())
    .child(small(if more { "Fewer site settings" } else { "More site settings" }).text_color(Theme::accent()))
    .child(icons::icon(if more { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT }, 12.));
    body.child(summary)
        .child(
            view()
                .flex_col()
                .gap(px(6.))
                .child(caps_label("Shield for this site"))
                .child(view().flex_row().child(segmented(choices, chosen, set)))
                .child(small(meaning).px_1()),
        )
        .child(list)
        .child(view().flex_row().child(toggle))
        .children(more.then(|| site_extras(ui, &host)))
        .into_element()
}

fn site_extras(ui: Ui, host: &str) -> View {
    let Some((css, (spaces, routed), zaps, zappable, private)) =
        read(ui, |b| (b.site_css(), b.route_choices(host), b.zapped(Some(host)), b.zappable(), b.model.private()))
    else {
        return view();
    };
    if ui.css.with_untracked(|c| c.is_none()) || ui.css_host.with_untracked(|h| h != host) {
        ui.css.set(Some(css));
        ui.css_host.set(host.to_owned());
        ui.css_error.set(None);
    }
    let mut routes = vec![(None, "Any Space".to_owned())];
    routes.extend(spaces.into_iter().map(|(id, name)| (Some(id), name)));
    let route_host = host.to_owned();
    let editor = super::text_field_multiline(ui, "body { font-size: 18px }");
    let save = move || {
        let css = ui.css.get_untracked().unwrap_or_default();
        let mut result = Ok(());
        act(ui, |b| result = b.save_site_css(&css));
        ui.css_error.set(result.err());
    };
    let hidden: Vec<AnyElement> = zaps
        .into_iter()
        .map(|(h, selector)| {
            let shown = selector.clone();
            line(format!("Hidden: {shown}")).child(pill("Undo", move || act(ui, |b| b.unzap(&h, Some(&selector))))).into_element()
        })
        .collect();
    let mut out = view()
        .flex_col()
        .gap_2()
        .child(small(ui.css_error.get_untracked().unwrap_or_else(|| "Site CSS".to_owned())).text_color(if ui.css_error.with_untracked(Option::is_some) {
            Theme::warning()
        } else {
            Theme::muted()
        }))
        .child(editor)
        .child(view().flex_row().justify_end().child(pill("Save", save)));
    if !private {
        out = out.child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(label("Always open in").flex_1())
                .child(segmented(routes, routed, move |id| act(ui, |b| b.route_site(&route_host, id)))),
        );
    }
    if zappable {
        out = out.child(view().flex_row().items_center().child(label("Hidden elements").flex_1()).child(pill("Zap Element", move || {
            ui.panel.set(Panel::None);
            act(ui, Browser::zap_element);
        })));
    }
    out.children(hidden)
}

fn history_panel(ui: Ui) -> AnyElement {
    let entries = read(ui, |b| b.model.profile().map(|(p, id)| b.core.store(p).history(id, 300).unwrap_or_default()).unwrap_or_default()).unwrap_or_default();
    let count = entries.len();
    let rows: Vec<AnyElement> = entries
        .into_iter()
        .map(|h| {
            let url = h.url.clone();
            let host = url::Url::parse(&h.url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
            let name = if h.title.is_empty() { h.url.clone() } else { h.title.clone() };
            let link = url.clone();
            library_row(ui, h.url.clone(), &host, name, move || {
                ui.panel.set(Panel::None);
                let url = url.clone();
                act(ui, move |b| {
                    b.new_tab(&url);
                });
            })
            .child(copy_link(ui, link))
            .into_element()
        })
        .collect();
    let clear = (count > 0).then(|| confirmed(ui, "history".to_owned(), "Clear History", move || super::clear_history(ui)));
    let empty = (count == 0).then(|| note("No history yet"));
    popover()
        .flex_1()
        .child(heading(
            ui,
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .flex_1()
                .child(counted("History", count))
                .child(pill("Reading List", move || show(ui, Panel::ReadingList)))
                .children(clear),
        ))
        .children(empty)
        .child(scroll().flex_1().child(view().flex_col().gap(px(1.)).children(rows)))
        .into_element()
}

fn download_row(name: String, note: String, warning: bool) -> View {
    let glyph = if warning { icons::TRIANGLE_ALERT } else { icons::glyph(browser_core::glyphs::file_glyph(&name)) };
    let tint = move || if warning { Secure::warning() } else { Theme::muted() };
    view().flex_row().items_center().gap(px(10.)).min_h(px(44.)).px_2().child(tarn_nsh_shared::controls::glyph_tile(glyph, 28., Theme::control, tint)).child(
        view()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .child(label(name).truncate())
            .child(small(note).text_color(if warning { Secure::warning() } else { Theme::muted() }).truncate()),
    )
}

fn progress(done: u64, total: u64) -> View {
    let rest = total.saturating_sub(done) as f32;
    view()
        .flex_row()
        .h(px(3.))
        .ml(px(46.))
        .mr_2()
        .mb_1()
        .rounded_full()
        .overflow_hidden()
        .bg(Theme::separator())
        .child(view().h_full().flex_grow(done as f32).bg(Theme::accent()))
        .child(view().h_full().flex_grow(rest))
}

fn downloads_panel(ui: Ui) -> AnyElement {
    let (session, saved) = read(ui, |b| {
        let saved = b.model.profile().map(|(p, id)| b.core.store(p).downloads(id).unwrap_or_default()).unwrap_or_default();
        (b.downloads.clone(), saved)
    })
    .unwrap_or_default();
    let mut rows: Vec<AnyElement> = session
        .iter()
        .rev()
        .filter(|d| d.state != "finished")
        .map(|d| {
            let running = d.state == "started";
            let state = match (running, d.total) {
                (true, Some(total)) => format!("{} of {}", crate::memory::size(d.done), crate::memory::size(total)),
                (true, None) => crate::memory::size(d.done),
                _ => capitalised(&d.state),
            };
            let note = if d.message.is_empty() { state } else { format!("{state} · {}", d.message) };
            let id = d.id.clone();
            let row = download_row(d.name.clone(), note, !running)
                .children(running.then(|| icon_button(icons::CLOSE, "Cancel Download", move || act(ui, |b| b.cancel_download(&id)))));
            let bar = d.total.filter(|t| running && *t > 0).map(|total| progress(d.done, total));
            view().flex_col().child(row).children(bar).into_element()
        })
        .collect();
    let empty = rows.is_empty() && saved.is_empty();
    rows.extend(saved.into_iter().map(|d| {
        let path = d.path.clone();
        let file = std::path::Path::new(&d.path);
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let size = std::fs::metadata(file).map(|m| crate::memory::size(m.len())).ok();
        let host = url::Url::parse(&d.url).ok().and_then(|u| u.host_str().map(str::to_owned));
        let note = [size, host].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        download_row(name, note, false)
            .child(icon_button(icons::FOLDER_OPEN, "Show in Folder", move || {
                drop(reveal_path(std::path::PathBuf::from(&path)));
            }))
            .into_element()
    }));
    let clear = (!empty).then(|| confirmed(ui, "downloads".to_owned(), "Clear", move || act(ui, |b| b.clear_downloads())));
    let header = heading(ui, view().flex_row().items_center().flex_1().child(title("Downloads")).child(view().flex_1()).children(clear));
    let none = empty.then(|| view().h(px(44.)).px_2().justify_center().child(body("No downloads yet").text_color(Theme::faint())));
    popover().gap_2().child(header).child(view().flex_col().gap(px(2.)).children(rows).children(none)).into_element()
}

/// A name field and its button, for the space or folder `target` named `current`.
pub(super) fn rename_row(ui: Ui, target: String, current: &str, save: impl Fn(String) + Copy + 'static) -> View {
    if ui.rename_for.get_untracked() != target {
        ui.rename_for.set(target);
        ui.rename.set(current.to_owned());
    } else if ui.rename.with_untracked(String::is_empty) {
        ui.rename.set(current.to_owned());
    }
    let save = move || save(ui.rename.get_untracked());
    view().flex_row().items_center().gap_2().child(super::text_field(ui.rename, "Name", 260.).on_submit(move |_: &str| save())).child(pill("Rename", save))
}

/// Copies `url` from a library row.
pub(super) fn copy_link(ui: Ui, url: String) -> nsh_base::ButtonElement {
    pill("Copy Link", move || {
        set_clipboard(url.clone());
        act(ui, |b| b.show_toast_as(browser_core::toast::Tone::Success, "Link copied", None));
    })
}

fn folder_panel(ui: Ui, id: store::Id) -> AnyElement {
    let Some(name) = read(ui, |b| b.model.folders.iter().find(|f| f.id == id).map(|f| f.name.clone())).flatten() else {
        return view().into_element();
    };
    let new = menu_row("New Subfolder", None, move || {
        ui.panel.set(Panel::None);
        act(ui, |b| b.new_folder(Some(id)));
    });
    popover()
        .child(heading(ui, title(format!("Folder “{name}”")).truncate()))
        .child(rename_row(ui, format!("folder {id}"), &name, move |name| act(ui, |b| b.rename_folder(id, &name))))
        .child(group().child(new))
        .into_element()
}

pub(super) fn menu_row(name: &str, keys: Option<String>, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    crate::controls::row(name.to_owned(), press)
        .px_2()
        .gap(px(10.))
        .child(view().w(px(16.)).items_center().text_color(Theme::muted()).child(icons::icon(icons::glyph(browser_core::command::glyph(name)), 15.)))
        .child(label(name.to_owned()).flex_1())
        .children(keys.map(|k| tarn_nsh_shared::controls::key_caps(&k)))
}

fn pills(name: &str, buttons: Vec<AnyElement>) -> Option<View> {
    (!buttons.is_empty()).then(|| view().flex_col().child(section(name.to_owned())).child(view().flex_row().flex_wrap().gap_2().children(buttons)))
}

fn tab_panel(ui: Ui) -> AnyElement {
    let menu = ui.tab_menu.get_untracked();
    let Some((key, active, name, kind, targets, duplicates, keymap, split)) = read(ui, |b| {
        let key = menu.filter(|k| b.model.tab(*k).is_some()).or(b.model.active)?;
        let tab = b.model.tab(key)?;
        let keymap = shortcuts::keymap(&shortcuts::stored(&b.core.disk));
        Some((
            key,
            b.model.active == Some(key),
            crate::snapshot::tab_label(tab, b, key),
            tab.kind,
            b.model.move_targets(&b.core, key),
            !b.duplicate_tabs().is_empty(),
            keymap,
            b.splits.menu(b.model.active, key),
        ))
    })
    .flatten() else {
        return view().into_element();
    };
    let (folders, spaces, burner, article) = ui.snap.with(|s| (s.folders.clone(), s.spaces.clone(), s.burner, s.article));
    let keys = |action: Action| active.then(|| keymap.iter().find(|(_, a)| *a == action).map(|(k, _)| shortcuts::display(k))).flatten();
    let folder_buttons: Vec<AnyElement> =
        folders.into_iter().map(|(id, name)| pill(name, move || act(ui, |b| b.move_to_folder(key, id))).into_element()).collect();
    let space_buttons: Vec<AnyElement> = spaces
        .into_iter()
        .filter(|s| targets.contains(&s.ix))
        .map(|s| {
            pill(s.label, move || {
                ui.panel.set(Panel::None);
                act(ui, |b| b.move_to_space(key, s.ix));
            })
            .into_element()
        })
        .collect();
    let tool = |name: &'static str, command: Command, action: Option<Action>| {
        menu_row(name, action.and_then(keys), move || super::tools::run(ui, command)).into_element()
    };
    let tab_rows = (!burner).then(|| {
        let (pinned, tile) = (kind != TabKind::Ephemeral, kind == TabKind::Tile);
        let pin = menu_row(if pinned { "Unpin" } else { "Pin" }, keys(Action::TogglePin), move || act(ui, |b| b.pin_toggle(key)));
        let tiles = menu_row(if tile { "Remove from Tiles" } else { "Add to Tiles" }, None, move || act(ui, |b| b.tile_toggle(key)));
        group().child(pin).child(tiles)
    });
    let page_rows = active.then(|| {
        let mut rows = vec![
            tool("Copy Link", Command::CopyLink, Some(Action::CopyLink)),
            tool("Copy Clean Link", Command::CopyCleanLink, Some(Action::CopyCleanLink)),
            tool("Show QR Code", Command::ShowQrCode, None),
            tool("Save for Later", Command::SaveForLater, None),
        ];
        if article {
            rows.push(tool("Reader", Command::Reader, Some(Action::ToggleReader)));
        }
        rows.push(tool("Zap Element", Command::ZapElement, None));
        if duplicates {
            rows.push(tool("Close Duplicate Tabs", Command::CloseDuplicates, None));
        }
        rows.push(tool("New Burner Tab", Command::BurnerTab, Some(Action::BurnerTab)));
        group().children(rows)
    });
    let in_split = move |run: fn(&mut Browser, browser_core::tabs::TabKey)| {
        move || {
            ui.panel.set(Panel::None);
            act(ui, |b| run(b, key));
        }
    };
    let mut split_rows = Vec::new();
    if split.enabled {
        split_rows.push(menu_row(split.entry.label(), keys(Action::ToggleSplitView), in_split(Browser::split_with)));
    }
    if split.member {
        split_rows.push(menu_row("Remove from Split View", keys(Action::RemoveFromSplitView), in_split(Browser::leave_split)));
        split_rows.push(menu_row("Rotate Layout", keys(Action::RotateSplitLayout), in_split(Browser::rotate_split)));
        split_rows.push(menu_row("Close Split View", keys(Action::ToggleSplitView), in_split(Browser::close_split)));
    }
    let split_rows = (!split_rows.is_empty()).then(|| view().flex_col().child(section("Split View".to_owned())).child(group().children(split_rows)));
    let close = menu_row("Close Tab", keys(Action::CloseTab), move || {
        ui.panel.set(Panel::None);
        act(ui, |b| b.close_tab(key));
    });
    popover()
        .child(heading(ui, title(name).truncate()))
        .children(burner.then(|| note("Burner tab: nothing is kept after it closes")))
        .children(page_rows)
        .children(split_rows)
        .children(tab_rows)
        .children(pills("Move into a folder", folder_buttons))
        .children(pills("Move to another space of this profile", space_buttons))
        .child(group().child(close))
        .into_element()
}

pub fn panel_view(ui: Ui) -> AnyElement {
    dynamic(move || {
        ui.rev.get();
        let body = match ui.panel.get() {
            Panel::None => return None,
            Panel::Shield => shield_panel(ui),
            Panel::Settings => super::settings::settings_panel(ui),
            Panel::Space => super::library::space_panel(ui),
            Panel::Memory => super::library::memory_panel(ui),
            Panel::ReadingList => super::library::reading_list_panel(ui),
            Panel::Qr => super::tools::qr_panel(ui),
            Panel::Share => super::tools::share_panel(ui),
            Panel::History => history_panel(ui),
            Panel::Downloads => downloads_panel(ui),
            Panel::Tab => tab_panel(ui),
            Panel::Folder(id) => folder_panel(ui, id),
        };
        let panel = ui.panel.get_untracked();
        let (width, side) = match panel {
            Panel::Share => (320., Side::End),
            Panel::Shield => (SITE_WIDTH, Side::Centre),
            Panel::Downloads => (380., Side::Start),
            Panel::Settings => (PANEL_WIDTH + 200., Side::Centre),
            _ => (PANEL_WIDTH, Side::Centre),
        };
        let content =
            if panel == Panel::Settings { view().flex_col().flex_1().child(body).into_element() } else { scroll().flex_1().child(body).into_element() };
        let column = view().flex_col().flex_1().w(px(width)).child(content);
        let area = view().flex_col().flex_1().p_4();
        let area = match side {
            Side::Start => area.items_start(),
            Side::Centre => area.items_center(),
            Side::End => area.items_end(),
        };
        Some(area.child(tarn_nsh_shared::motion::popping(column)))
    })
    .into_element()
}
