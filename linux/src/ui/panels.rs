use nsh_core::element::IntoProp;
use nsh_core::prelude::*;
use shield::ShieldLevel;

use super::tools::{line, note};
use super::{Panel, Ui, act, read, show, site_tile};
use crate::browser::Browser;
use crate::controls::{body, card, icon_button, label, pill, segmented, small, switch, title};
use crate::icons;
use crate::theme::Theme;

const PANEL_WIDTH: f32 = 720.;

pub(super) fn levels() -> Vec<(ShieldLevel, String)> {
    ShieldLevel::ALL.iter().map(|l| (*l, capitalised(l.as_str()))).collect()
}

pub(super) fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

pub(super) fn section(name: &str) -> Text {
    label(name.to_owned()).font_weight(FontWeight::SEMIBOLD).pt_5().pb_2().px_1()
}

pub(super) fn counted(name: &str, count: usize) -> View {
    view().flex_row().items_baseline().gap_2().flex_1().child(title(name.to_owned())).child(small(count.to_string()).text_color(Theme::faint()))
}

pub(super) fn library_row<M>(
    name: impl IntoProp<Option<SharedString>, M>,
    host: &str,
    label_text: String,
    press: impl Fn() + 'static,
) -> nsh_base::ButtonElement {
    crate::controls::row(name, press)
        .h(px(30.))
        .child(site_tile(host, 16.))
        .child(label(label_text).flex_1().truncate())
        .child(small(host.trim_start_matches("www.").to_owned()).max_w(px(220.)).truncate().text_color(Theme::faint()))
}

pub(super) fn popover() -> View {
    card().gap_3().p_4().shadow(Theme::elevation(2., 5.))
}

fn shield_panel(ui: Ui) -> AnyElement {
    let (host, level, overridden, secure, zoom) = ui.snap.with(|s| (s.host.clone(), s.level.clone(), s.overridden, s.secure, s.zoom));
    let default = read(ui, |b| b.default_level()).unwrap_or_default();
    let set = move |level: Option<ShieldLevel>| act(ui, |b| b.set_site_level(level));
    let (glyph, color, connection) = if secure {
        (icons::SHIELD_CHECK, Theme::success(), "Connection is secure")
    } else {
        (icons::SHIELD, Theme::warning(), "Not Secure — this page is not encrypted")
    };
    let header = view()
        .flex_col()
        .gap(px(2.))
        .pb_3()
        .border_b_1()
        .border_color(Theme::hairline())
        .child(label(if host.is_empty() { "This page".to_owned() } else { host.clone() }).font_weight(FontWeight::SEMIBOLD).truncate())
        .child(view().flex_row().items_center().gap(px(6.)).text_color(color).child(icons::icon(glyph, 12.)).child(small(connection).text_color(color)));
    let mut body = popover().child(header);
    if !host.is_empty() {
        let mut choices = vec![(None, "Default".to_owned())];
        choices.extend(levels().into_iter().map(|(l, name)| (Some(l), name)));
        let chosen = overridden.then(|| ShieldLevel::parse(&level).unwrap_or_default());
        let zoom_button = |icon: &'static str, name: &str, step: i32| icon_button(icon, name, move || act(ui, move |b| b.zoom(step)));
        body = body
            .child(
                view()
                    .flex_col()
                    .gap(px(6.))
                    .child(small(format!("Shield for this site · profile default {}", capitalised(default.as_str()))))
                    .child(view().flex_row().child(segmented(choices, chosen, set))),
            )
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(label("Zoom").flex_1())
                    .child(zoom_button(icons::CHEVRON_LEFT, "Zoom Out", -1))
                    .child(small(format!("{:.0}%", zoom * 100.0)).w(px(44.)).text_center())
                    .child(zoom_button(icons::CHEVRON_RIGHT, "Zoom In", 1))
                    .child(pill("Reset", move || act(ui, |b| b.zoom(0)))),
            )
            .child(site_extras(ui, &host));
    }
    body.into_element()
}

fn site_extras(ui: Ui, host: &str) -> View {
    let Some((dark, css, (spaces, routed), zaps, zappable, private)) =
        read(ui, |b| (b.site_dark(), b.site_css(), b.route_choices(host), b.zapped(Some(host)), b.zappable(), b.model.private()))
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
        .child(switch(dark, "Dark mode for this site", move || act(ui, Browser::toggle_site_dark)).px_0())
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
            library_row(h.url.clone(), &host, name, move || {
                ui.panel.set(Panel::None);
                let url = url.clone();
                act(ui, move |b| {
                    b.new_tab(&url);
                });
            })
            .into_element()
        })
        .collect();
    let clear = (count > 0).then(|| pill("Clear History", move || act(ui, |b| b.clear_history())));
    let empty = (count == 0).then(|| note("No history yet"));
    popover()
        .flex_1()
        .child(view().flex_row().items_center().child(counted("History", count)).children(clear))
        .children(empty)
        .child(scroll().flex_1().child(view().flex_col().gap(px(1.)).children(rows)))
        .into_element()
}

fn download_row(name: String, note: String, icon: &'static str, tint: Rgba) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .h(px(44.))
        .px_2()
        .rounded(px(Theme::radius_row()))
        .text_color(Theme::muted())
        .child(icons::icon(icon, 20.).text_color(tint))
        .child(view().flex_col().flex_1().child(label(name).truncate()).child(small(note).truncate()))
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
                _ => d.state.clone(),
            };
            let note = if d.message.is_empty() { state } else { format!("{state} — {}", d.message) };
            let id = d.id.clone();
            download_row(d.name.clone(), note, icons::ARROW_DOWN, Theme::accent())
                .children(running.then(|| icon_button(icons::CLOSE, "Cancel Download", move || act(ui, |b| b.cancel_download(&id)))))
                .into_element()
        })
        .collect();
    let empty = rows.is_empty() && saved.is_empty();
    let count = rows.len() + saved.len();
    rows.extend(saved.into_iter().map(|d| {
        let path = d.path.clone();
        let file = std::path::Path::new(&d.path);
        let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let kind = file.extension().map_or_else(|| "File".to_owned(), |e| e.to_string_lossy().to_lowercase());
        download_row(name, kind, icons::FILE_PLAIN, Theme::muted())
            .child(icon_button(icons::SEARCH, "Show in folder", move || {
                drop(reveal_path(std::path::PathBuf::from(&path)));
            }))
            .into_element()
    }));
    let clear = (!empty).then(|| pill("Clear", move || act(ui, |b| b.clear_downloads())));
    let header = view().flex_row().items_center().justify_between().child(counted("Downloads", count)).children(clear);
    let none = empty.then(|| view().h(px(44.)).px_2().justify_center().child(body("No downloads yet").text_color(Theme::faint())));
    popover().child(header).child(view().flex_col().children(rows).children(none)).into_element()
}

fn tab_panel(ui: Ui) -> AnyElement {
    let Some(key) = read(ui, |b| b.model.active).flatten() else { return view().into_element() };
    let (pinned, tile, folders, targets) = ui.snap.with(|s| (s.pinned, s.tile, s.folders.clone(), s.move_targets.clone()));
    let folder_buttons: Vec<AnyElement> =
        folders.into_iter().map(|(id, name)| pill(name, move || act(ui, |b| b.move_to_folder(key, id))).into_element()).collect();
    let space_buttons: Vec<AnyElement> = targets
        .into_iter()
        .map(|(ix, name)| {
            pill(name, move || {
                ui.panel.set(Panel::None);
                act(ui, |b| b.move_to_space(key, ix));
            })
            .into_element()
        })
        .collect();
    let (duplicates, burner, article) =
        (read(ui, |b| !b.duplicate_tabs().is_empty()).unwrap_or(false), ui.snap.with(|s| s.burner), ui.snap.with(|s| s.article));
    let tool = |name: &'static str, command: super::tools::Command| pill(name, move || super::tools::run(ui, command)).into_element();
    let mut tools: Vec<AnyElement> = vec![
        tool("Copy Link", super::tools::Command::CopyLink),
        tool("Copy Clean Link", super::tools::Command::CopyCleanLink),
        tool("Show QR Code", super::tools::Command::ShowQrCode),
        tool("Save for Later", super::tools::Command::SaveForLater),
    ];
    if article {
        tools.push(tool("Reader", super::tools::Command::Reader));
    }
    tools.push(tool("Zap Element", super::tools::Command::ZapElement));
    if duplicates {
        tools.push(tool("Close Duplicate Tabs", super::tools::Command::CloseDuplicates));
    }
    let pin = (!burner).then(|| pill(if pinned { "Unpin" } else { "Pin" }, move || act(ui, |b| b.pin_toggle(key))));
    let tiles = (!burner).then(|| pill(if tile { "Remove from Tiles" } else { "Add to Tiles" }, move || act(ui, |b| b.tile_toggle(key))));
    popover()
        .child(title("This Tab"))
        .child(
            view()
                .flex_row()
                .gap_2()
                .children(pin)
                .children(tiles)
                .child(pill("Close Tab", move || {
                    ui.panel.set(Panel::None);
                    act(ui, |b| b.close_tab(key));
                }))
                .child(pill("New Burner Tab", move || super::tools::run(ui, super::tools::Command::BurnerTab))),
        )
        .children(burner.then(|| note("Burner tab: nothing is kept after it closes")))
        .child(view().flex_row().flex_wrap().gap_2().children(tools))
        .child(section("Move into a folder"))
        .child(view().flex_row().flex_wrap().gap_2().children(folder_buttons))
        .child(section("Move to another space of this profile"))
        .child(view().flex_row().flex_wrap().gap_2().children(space_buttons))
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
            Panel::History => history_panel(ui),
            Panel::Downloads => downloads_panel(ui),
            Panel::Tab => tab_panel(ui),
        };
        let wide = ui.panel.get_untracked() == Panel::Settings;
        let close = view().flex_row().justify_end().child(icon_button(icons::CLOSE, "Close", move || show(ui, Panel::None)));
        Some(view().flex_col().flex_1().items_center().p_4().child(
            view().flex_col().flex_1().w(px(if wide { PANEL_WIDTH + 200. } else { PANEL_WIDTH })).gap_2().child(close).child(if wide {
                view().flex_col().flex_1().child(body).into_element()
            } else {
                scroll().flex_1().child(body).into_element()
            }),
        ))
    })
    .into_element()
}
