use nsh_core::element::IntoProp;
use nsh_core::prelude::*;
use shield::ShieldLevel;

use super::tools::{Command, line, note};
use super::{Panel, Ui, act, confirmed, read, show, site_tile};
use crate::browser::library::Link;
use crate::browser::{Browser, ZOOM_STEPS};
use crate::controls::{body, card, group, icon_button, label, pill, segmented, small, title};
use crate::icons;
use crate::shortcuts::{self, Action};
use crate::theme::Theme;
use crate::theme::{SYSTEM_FONT, Secure};
use browser_core::library::LIST_ROWS;
use browser_core::shield_card::{motto, thousands};
use browser_core::tab_menu;
use store::TabKind;
use tarn_nsh_shared::controls::{caps_label, glyph_row, glyph_switch};

const PANEL_WIDTH: f32 = 720.;
const SITE_WIDTH: f32 = 340.;
const SPACE_CARD_WIDTH: f32 = 420.;

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
    if host.is_empty() {
        let profile = heading(ui, label("Shield level").font_weight(FontWeight::BOLD));
        let levels = segmented(levels(), default, move |l| super::pick_level(ui, l));
        return popover().gap_3().child(profile).child(levels).child(super::lockdown_confirm(ui)).child(small(motto(default).1)).into_element();
    }
    let (glyph, color, connection) =
        if secure { (icons::LOCK, Secure::success(), "Connection is secure") } else { (icons::TRIANGLE_ALERT, Secure::warning(), "Not secure") };
    let header = view()
        .flex_row()
        .items_center()
        .gap(px(10.))
        .child(site_tile(ui, &host, 28.))
        .child(
            view().flex_col().flex_1().min_w(px(0.)).child(label(host.clone()).font_weight(FontWeight::BOLD).truncate()).child(
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
    let muted = read(ui, |b| b.model.active.map(|k| b.tab_muted(k))).flatten().unwrap_or_default();
    let (paused, popups, permissions) = read(ui, |b| (b.site_paused(&host), b.site_popups(), b.site_permissions())).unwrap_or_default();
    let pause_host = host.clone();
    let popup_choices = browser_core::site::POPUP_RULES.iter().map(|(rule, name)| (*rule, (*name).to_owned())).collect();
    let permission_rows: Vec<AnyElement> = permissions.into_iter().map(|row| permission_row(ui, row)).collect();
    let list = group()
        .child(
            glyph_switch(
                icons::SHIELD,
                "Block ads, trackers and cookie banners",
                move || !paused,
                move || {
                    let host = pause_host.clone();
                    act(ui, move |b| b.set_site_paused(&host, !paused));
                },
            )
            .border_b_1()
            .border_color(Theme::separator()),
        )
        .child(
            glyph_row(icons::APP_WINDOW, "Pop-ups")
                .child(segmented(popup_choices, popups, move |rule| act(ui, move |b| b.set_site_popups(rule))))
                .border_b_1()
                .border_color(Theme::separator()),
        )
        .child(
            glyph_switch(icons::MOON, "Dark mode for this site", move || dark, move || act(ui, Browser::toggle_site_dark))
                .border_b_1()
                .border_color(Theme::separator()),
        )
        .child(
            glyph_switch(
                icons::VOLUME_X,
                "Mute this site",
                move || muted,
                move || act(ui, |b| b.model.active.into_iter().for_each(|k| b.toggle_site_muted(k))),
            )
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
        )
        .children(permission_rows);
    let more = ui.site_more.get_untracked();
    let toggle = crate::controls::row(if more { "Fewer site settings" } else { "More site settings" }, move || {
        ui.site_more.update(|m| *m = !*m);
        act(ui, |_| {});
    })
    .px_1()
    .gap_1()
    .text_color(Theme::accent_text())
    .child(small(if more { "Fewer site settings" } else { "More site settings" }).text_color(Theme::accent_text()))
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
        .child(view().flex_row().items_center().child(toggle).child(view().flex_1()).child(clear_site(ui, &host)))
        .children(more.then(|| site_extras(ui, &host)))
        .into_element()
}

fn permission_row(ui: Ui, row: browser_core::permissions::PermissionRow) -> AnyElement {
    let glyph = icons::named(browser_core::glyphs::glyph(&row.kind)).unwrap_or(icons::INFO);
    let choices = browser_core::permissions::DECISIONS.iter().map(|(d, name)| (*d, (*name).to_owned())).collect();
    let (origin, kind) = (row.origin.clone(), row.kind.clone());
    let (forget_origin, forget_kind) = (row.origin.clone(), row.kind.clone());
    let decision = browser_core::permissions::DECISIONS.iter().map(|(d, _)| *d).find(|d| *d == row.decision).unwrap_or("ask");
    glyph_row(glyph, row.label)
        .border_t_1()
        .border_color(Theme::separator())
        .child(segmented(choices, decision, move |d| {
            let (origin, kind) = (origin.clone(), kind.clone());
            act(ui, move |b| b.set_site_permission(&origin, &kind, Some(d)));
        }))
        .child(icon_button(icons::CLOSE, "Remove", move || {
            let (origin, kind) = (forget_origin.clone(), forget_kind.clone());
            act(ui, move |b| b.set_site_permission(&origin, &kind, None));
        }))
        .into_element()
}

/// "Clear site data…" asks before it removes anything.
fn clear_site(ui: Ui, host: &str) -> AnyElement {
    let host = host.to_owned();
    let question = format!("Clear the data of {}?", browser_core::text::bounded_origin(&host));
    crate::controls::row("Clear site data…", move || {
        let host = host.clone();
        tarn_nsh_shared::controls::confirm(question.clone(), "Its cookies and stored data are deleted.", "Clear", move || {
            let mut store = None;
            act(ui, |b| store = b.clear_site(&host));
            let Some(store) = store else { return };
            let (cleared, host) = (host.clone(), host.clone());
            crate::webview::session::clear_site(&store, &host, move |count| act(ui, move |b| b.site_cleared(&cleared, count)));
        });
    })
    .px_1()
    .child(small("Clear site data…").text_color(Theme::danger()))
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
            line(format!("Hidden: {shown}")).child(pill("Show Again", move || act(ui, |b| b.unzap(&h, Some(&selector))))).into_element()
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

/// A page's row; with a `detail` line, the host follows it there, so the title keeps the row's width.
pub(super) fn link_row(ui: Ui, link: Link, detail: Option<String>) -> nsh_base::ButtonElement {
    let host = url::Url::parse(&link.url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
    let (open, new) = (link.url.clone(), link.url.clone());
    let press = move || {
        ui.panel.set(Panel::None);
        let url = open.clone();
        act(ui, move |b| b.open_url(&url));
    };
    let row = match detail {
        None => library_row(ui, link.url.clone(), &host, link.name, press),
        Some(detail) => {
            let detail = [detail.as_str(), host.trim_start_matches("www.")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
            crate::controls::row(link.url.clone(), press)
                .min_h(px(40.))
                .child(site_tile(ui, &host, 16.))
                .child(view().flex_col().flex_1().min_w(px(0.)).child(label(link.name).truncate()).child(small(detail).truncate().text_color(Theme::faint())))
        }
    };
    row.child(pill("Open in New Tab", move || {
        ui.panel.set(Panel::None);
        let url = new.clone();
        act(ui, move |b| {
            b.new_tab(&url);
        });
    }))
    .child(copy_link(ui, "Copy Link", link.url))
}

pub(super) fn search_field(ui: Ui) -> nsh_base::Input {
    super::text_field(ui.library_query, "Search", 220.)
}

pub(super) fn no_results(query: &str) -> Text {
    note(format!("No results for “{}”", browser_core::text::bounded_title(query)))
}

fn history_panel(ui: Ui) -> AnyElement {
    super::library::bind_row_keys(ui, ui.library_query);
    let history = std::rc::Rc::new(read(ui, Browser::library_history).unwrap_or_default());
    let count = history.len();
    let starts = read(ui, Browser::history_day_starts).unwrap_or_default();
    let rows = dynamic(move || {
        let query = ui.library_query.get();
        let query = query.trim();
        let (now, mut day) = (browser_core::now(), None);
        let mut list = view().flex_col().gap(px(1.));
        let hits: Vec<usize> = browser_core::library::history_hits(&history, query).into_iter().take(LIST_ROWS).collect();
        if hits.is_empty() {
            list = list.child(if query.is_empty() { note("No history yet") } else { no_results(query) });
        }
        for ix in hits {
            let visit = &history[ix];
            let named = browser_core::library::history_day(visit.visited_at, starts);
            if day.replace(named) != Some(named) {
                list = list.child(section(named.to_owned()));
            }
            let url = visit.url.clone();
            let ago = read(ui, |b| browser_core::library::ago(b.core.store(b.model.private()), visit.visited_at, now)).unwrap_or_default();
            list = list.child(
                super::library::focus_row(ui, link_row(ui, Link::new(&visit.title, &visit.url), Some(ago)), super::library::Focused::Visit(url.clone()))
                    .child(icon_button(icons::CLOSE, "Remove from History", move || super::remove_history(ui, url.clone()))),
            );
        }
        Some(list)
    });
    let closed: Vec<AnyElement> = read(ui, Browser::recently_closed)
        .unwrap_or_default()
        .into_iter()
        .map(|(serial, name, url)| {
            let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
            library_row(ui, name.clone(), &host, name, move || {
                ui.panel.set(Panel::None);
                act(ui, |b| b.reopen_closed(serial));
            })
            .into_element()
        })
        .collect();
    let closed = (!closed.is_empty())
        .then(|| view().flex_col().gap(px(1.)).child(section("Recently Closed".to_owned())).children(closed).child(section("History".to_owned())));
    let clear = (count > 0).then(|| super::clear_history_choice(ui, "Clear History…"));
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
                .child(pill("Bookmarks", move || show(ui, Panel::Bookmarks(None))))
                .child(pill("Reading List", move || show(ui, Panel::ReadingList)))
                .child(pill("Archived Tabs", move || show(ui, Panel::Archived)))
                .children(browser_core::import::available().then(|| pill("Import", move || super::import::open(ui))))
                .children(clear),
        ))
        .child(search_field(ui))
        .child(scroll().flex_1().child(view().flex_col().gap(px(1.)).children(closed).child(rows)))
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

pub(super) fn saved_download(ui: Ui, d: store::Download) -> AnyElement {
    let path = d.path.clone();
    let file = std::path::Path::new(&d.path);
    let name = browser_core::library::download_name(&d);
    let size = std::fs::metadata(file).map(|m| crate::memory::size(m.len())).ok();
    let host = url::Url::parse(&d.url).ok().and_then(|u| u.host_str().map(str::to_owned));
    let ended = matches!(d.state.as_str(), "failed" | "cancelled").then(|| capitalised(&d.state));
    let missing = browser_core::library::download_missing(&d);
    let note = if missing { browser_core::library::MISSING.to_owned() } else { [ended, size, host].into_iter().flatten().collect::<Vec<_>>().join(" · ") };
    let id = d.id;
    download_row(name, note, missing)
        .children((!missing).then(|| {
            icon_button(icons::FOLDER_OPEN, "Show in Folder", move || {
                drop(reveal_path(std::path::PathBuf::from(&path)));
            })
        }))
        .children((!d.url.is_empty()).then(|| copy_link(ui, "Copy Download Link", d.url)))
        .child(icon_button(icons::TRASH, "Remove from List", move || act(ui, |b| b.remove_download(id))))
        .into_element()
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
        .filter(|d| d.state != "finished" && !saved.iter().any(|s| !d.path.is_empty() && s.path == d.path))
        .map(|d| {
            let running = d.state == "started";
            let state = match (running, d.total) {
                (true, Some(total)) => format!("{} of {}", crate::memory::size(d.done), crate::memory::size(total)),
                (true, None) => crate::memory::size(d.done),
                _ => capitalised(&d.state),
            };
            let flagged = d.state == "flagged";
            let note = match (flagged, d.message.is_empty()) {
                (true, _) => d.message.clone(),
                (false, true) => state,
                (false, false) => format!("{state} · {}", d.message),
            };
            let id = d.id.clone();
            let (keep, delete) = (id.clone(), id.clone());
            let row = download_row(d.name.clone(), note, !running)
                .children(running.then(|| icon_button(icons::CLOSE, "Cancel Download", move || act(ui, |b| b.cancel_download(&id)))))
                .children(flagged.then(|| pill("Keep", move || act(ui, |b| b.keep_download(&keep, true)))))
                .children(flagged.then(|| {
                    pill("Delete", move || {
                        act(ui, |b| b.keep_download(&delete, false));
                        for other in super::WINDOWS.with_borrow(Vec::clone).into_iter().filter(|o| o.window != ui.window) {
                            act(other, |b| b.forget_download(&delete));
                        }
                    })
                }));
            let bar = d.total.filter(|t| running && *t > 0).map(|total| progress(d.done, total));
            view().flex_col().child(row).children(bar).into_element()
        })
        .collect();
    let empty = rows.is_empty() && saved.is_empty();
    rows.extend(saved.into_iter().map(|d| saved_download(ui, d)));
    let clear = (!empty).then(|| pill("Clear", move || act(ui, |b| b.clear_downloads())));
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
pub(super) fn copy_link(ui: Ui, name: &'static str, url: String) -> nsh_base::ButtonElement {
    pill(name, move || {
        set_clipboard(url.clone());
        act(ui, |b| b.show_toast_as(browser_core::toast::Tone::Success, "Link copied", None));
    })
}

fn folder_panel(ui: Ui, id: store::Id) -> AnyElement {
    let Some((name, count, current)) =
        read(ui, |b| b.model.folders.iter().find(|f| f.id == id).map(|f| (f.name.clone(), b.model.folder_contents(id).0.len(), f.icon.clone()))).flatten()
    else {
        return view().into_element();
    };
    let delete = move || {
        ui.panel.set(Panel::None);
        act(ui, |b| b.delete_folder(id));
    };
    let delete = if count == 0 {
        group().child(menu_row("Delete Folder…", None, delete)).into_element()
    } else {
        view()
            .flex_row()
            .items_center()
            .gap_2()
            .child(small(format!("Deletes the {count} tabs in it too")).flex_1())
            .child(confirmed(ui, format!("folder {id} delete"), "Delete Folder", delete))
            .into_element()
    };
    let new = menu_row("New Subfolder", None, move || {
        ui.panel.set(Panel::None);
        act(ui, |b| b.new_folder(Some(id)));
    });
    popover()
        .child(heading(ui, title(format!("Folder “{name}”")).truncate()))
        .child(rename_row(ui, format!("folder {id}"), &name, move |name| act(ui, |b| b.rename_folder(id, &name))))
        .child(group().child(new))
        .child(icon_choice(ui, format!("folder {id}"), current, move |icon| {
            ui.emoji.set(browser_core::icon::emoji(&icon).to_owned());
            act(ui, |b| b.set_folder_icon(id, &icon));
        }))
        .child(delete)
        .into_element()
}

/// The symbols to pick from, a field for one emoji and, while one is set, Remove.
pub(super) fn icon_choice(ui: Ui, target: String, current: String, set: impl Fn(String) + Copy + 'static) -> View {
    if ui.emoji_for.get_untracked() != target {
        ui.emoji_for.set(target);
        ui.emoji.set(browser_core::icon::emoji(&current).to_owned());
    }
    let emoji = ui.emoji;
    let symbols = browser_core::icon::SYMBOLS
        .iter()
        .filter_map(|name| Some(icon_button(icons::named(name)?, name, move || set((*name).to_owned())).opacity(if current == *name { 1. } else { 0.7 })));
    let save = move || Some(emoji.get_untracked()).filter(|e| !e.trim().is_empty()).into_iter().for_each(set);
    let field = view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(super::text_field(emoji, "Emoji", 120.).on_submit(move |_: &str| save()))
        .child(pill("Set Emoji", save))
        .children((!current.is_empty()).then(|| pill("Remove", move || set(String::new()))));
    view().flex_col().gap_2().child(section("Icon".to_owned())).child(view().flex_row().flex_wrap().children(symbols)).child(field)
}

pub(super) fn menu_row(name: &str, keys: Option<String>, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    named_row(browser_core::command::glyph(name), name, keys, press)
}

pub(super) fn named_row(glyph: &str, name: &str, keys: Option<String>, press: impl Fn() + 'static) -> nsh_base::ButtonElement {
    crate::controls::row(name.to_owned(), press)
        .px_2()
        .gap(px(10.))
        .child(view().w(px(16.)).items_center().text_color(Theme::muted()).child(icons::icon(icons::glyph(glyph), 15.)))
        .child(label(name.to_owned()).flex_1())
        .children(keys.map(|k| tarn_nsh_shared::controls::key_caps(&k)))
}

fn pills(name: &str, buttons: Vec<AnyElement>) -> Option<View> {
    (!buttons.is_empty()).then(|| view().flex_col().child(section(name.to_owned())).child(view().flex_row().flex_wrap().gap_2().children(buttons)))
}

fn offered(command: tab_menu::Command) -> bool {
    !matches!(command, tab_menu::Command::SwitchEngine | tab_menu::Command::Rename)
}

fn tab_state(b: &Browser, key: browser_core::tabs::TabKey, spaces: &[(usize, String)]) -> Option<tab_menu::Tab> {
    let tab = b.model.tab(key)?;
    let active = b.model.active == Some(key);
    let targets = b.model.move_targets(&b.core, key);
    Some(tab_menu::Tab {
        kind: tab.kind,
        active,
        has_link: b.tab_url(key).is_some(),
        engine: None,
        split: b.splits.menu(b.model.active, key),
        split_view: true,
        burner: b.model.burner_shown(),
        custom_title: !tab.custom_title.is_empty(),
        drift: b.pinned_drift(key).and_then(|u| u.host_str().map(browser_core::text::bounded_title)),
        grouped: tab.group_id.is_some(),
        muted: b.can_mute(key).then(|| b.tab_muted(key)),
        sleepable: !active && !b.sleeping(key) && !b.in_split(key),
        sleep_others: b.can_sleep_others(key),
        others: b.model.ephemeral_keys().iter().any(|k| *k != key),
        below: !b.model.tabs_below(key).is_empty(),
        spaces: spaces.iter().filter(|(ix, _)| targets.contains(ix)).cloned().collect(),
        groups: b.model.groups.iter().filter(|g| Some(g.id) != tab.group_id).map(|g| (g.id, g.name.clone())).collect(),
    })
}

fn run_tab(ui: Ui, key: browser_core::tabs::TabKey, command: tab_menu::Command) {
    use tab_menu::Command as C;
    if matches!(
        command,
        C::Open
            | C::Duplicate
            | C::Split
            | C::LeaveSplit
            | C::RotateSplit
            | C::CloseSplit
            | C::ReturnToPinned
            | C::MoveToSpace(_)
            | C::Unload
            | C::Close
            | C::CloseOthers
            | C::CloseBelow
    ) {
        ui.panel.set(Panel::None);
    }
    match command {
        C::Reload => super::reload(ui),
        C::Open => act(ui, |b| b.activate(key)),
        C::Duplicate => act(ui, |b| b.duplicate_tab(key)),
        C::CopyLink => super::tools::copy_link(ui, false),
        C::CopyCleanLink => super::tools::copy_link(ui, true),
        C::Bookmark => super::library::bookmark_tab(ui, Some(key)),
        C::Split => act(ui, |b| b.split_with(key)),
        C::LeaveSplit => act(ui, |b| b.leave_split(key)),
        C::RotateSplit => act(ui, |b| b.rotate_split(key)),
        C::CloseSplit => act(ui, |b| b.close_split(key)),
        C::TogglePin => act(ui, |b| b.pin_toggle(key)),
        C::ToggleTile => act(ui, |b| b.tile_toggle(key)),
        C::ResetTitle => act(ui, |b| b.reset_tab_title(key)),
        C::ReturnToPinned => act(ui, |b| b.return_to_pinned(key)),
        C::SetPinnedUrl => act(ui, |b| b.set_pinned_url(key)),
        C::MoveToSpace(ix) => act(ui, |b| b.move_to_space(key, ix)),
        C::AddToGroup(id) => act(ui, |b| b.set_tab_group(key, Some(id))),
        C::NewGroup => super::groups::new_group(ui, Some(key)),
        C::RemoveFromGroup => act(ui, |b| b.set_tab_group(key, None)),
        C::ToggleMute => act(ui, |b| b.toggle_site_muted(key)),
        C::Sleep => act(ui, |b| b.sleep_tabs(vec![key])),
        C::SleepOthers => act(ui, |b| b.sleep_tabs(b.model.other_tabs(key))),
        C::Unload | C::Close => act(ui, |b| b.close_tab(key)),
        C::CloseOthers => act(ui, |b| b.close_others(key)),
        C::CloseBelow => act(ui, |b| b.close_below(key)),
        C::SwitchEngine | C::Rename => {}
    }
}

fn tab_rows(ui: Ui, key: browser_core::tabs::TabKey, items: Vec<tab_menu::Item>, keys: &dyn Fn(Action) -> Option<String>) -> Vec<View> {
    let mut out = Vec::new();
    let mut rows = Vec::new();
    for item in items {
        match item {
            tab_menu::Item::Command(command, name, enabled, shortcut) => rows.push(
                menu_row(&name, shortcut.and_then(keys), move || run_tab(ui, key, command))
                    .disabled(move || !enabled)
                    .disabled_style(|s| s.text_color(Theme::faint()))
                    .into_element(),
            ),
            tab_menu::Item::Separator => out.extend((!rows.is_empty()).then(|| group().children(std::mem::take(&mut rows)))),
            tab_menu::Item::Submenu(name, inner) => {
                out.extend((!rows.is_empty()).then(|| group().children(std::mem::take(&mut rows))));
                let inner = inner.into_iter().filter(|i| *i != tab_menu::Item::Separator).collect();
                out.push(view().flex_col().child(section(name.to_owned())).children(tab_rows(ui, key, inner, keys)));
            }
        }
    }
    out.extend((!rows.is_empty()).then(|| group().children(rows)));
    out
}

fn tab_panel(ui: Ui) -> AnyElement {
    let menu = ui.tab_menu.get_untracked();
    let (folders, spaces, burner, article) = ui.snap.with(|s| (s.folders.clone(), s.spaces.clone(), s.burner, s.article));
    let labels: Vec<(usize, String)> = spaces.iter().map(|s| (s.ix, s.label.clone())).collect();
    let Some((key, active, name, kind, duplicates, keymap, state)) = read(ui, |b| {
        let key = menu.filter(|k| b.model.tab(*k).is_some()).or(b.model.active)?;
        let tab = b.model.tab(key)?;
        Some((
            key,
            b.model.active == Some(key),
            crate::snapshot::tab_label(tab, b, key),
            tab.kind,
            !b.duplicate_tabs().is_empty(),
            shortcuts::keymap(&shortcuts::stored(&b.core.disk)),
            tab_state(b, key, &labels)?,
        ))
    })
    .flatten() else {
        return view().into_element();
    };
    let keys = move |action: Action| keymap.iter().find(|(_, a)| *a == action).map(|(k, _)| shortcuts::display(k));
    let folder_buttons: Vec<AnyElement> =
        folders.into_iter().map(|(id, name)| pill(name, move || act(ui, |b| b.move_to_folder(key, id))).into_element()).collect();
    let tool = |name: &'static str, command: Command, action: Option<Action>| {
        menu_row(name, action.and_then(&keys), move || super::tools::run_from_menu(ui, command.clone())).into_element()
    };
    let rename =
        (kind != TabKind::Tile).then(|| rename_row(ui, format!("tab {}-{}", key.private, key.id), &name, move |name| act(ui, |b| b.rename_tab(key, &name))));
    let page_rows = active.then(|| {
        let mut rows = vec![tool("Show QR Code", Command::ShowQrCode, None), tool("Save for Later", Command::SaveForLater, None)];
        if article {
            let shown = ui.snap.with_untracked(|s| s.reader.is_some());
            rows.push(tool(if shown { "Hide Reader" } else { "Show Reader" }, Command::Reader, Some(Action::ToggleReader)));
        }
        rows.push(tool("Zap Element", Command::ZapElement, None));
        rows.push(tool("Take Screenshot", Command::Key(Action::TakeScreenshot), Some(Action::TakeScreenshot)));
        rows.push(tool("Capture Area", Command::CaptureArea, None));
        rows.push(tool("Capture Element", Command::CaptureElement, None));
        rows.push(tool("Capture Full Page", Command::CaptureFullPage, None));
        rows.push(tool("Insert Snippet\u{2026}", Command::InsertSnippet, None));
        if duplicates {
            rows.push(tool("Close Duplicate Tabs", Command::CloseDuplicates, None));
        }
        rows.push(tool("New Burner Tab", Command::BurnerTab, Some(Action::BurnerTab)));
        group().children(rows)
    });
    popover()
        .child(heading(ui, title(name).truncate()))
        .children(rename)
        .children(burner.then(|| note("Burner tab: nothing is kept after it closes")))
        .children(tab_rows(
            ui,
            key,
            tab_menu::items(&state, |c| offered(c) && (state.active || !matches!(c, tab_menu::Command::CopyLink | tab_menu::Command::CopyCleanLink))),
            &keys,
        ))
        .children(page_rows)
        .children(pills("Move into a folder", folder_buttons))
        .into_element()
}

fn organize_panel(ui: Ui) -> AnyElement {
    use browser_core::sidebar::{ORGANIZE, Organize, organize};
    let Some((cleanable, assistant, keymap)) = read(ui, |b| {
        (browser_core::sidebar::cleanable(&b.model), b.model.assistant_offered(&b.core, None), shortcuts::keymap(&shortcuts::stored(&b.core.disk)))
    }) else {
        return view().into_element();
    };
    let group_keys = keymap.iter().find(|(_, a)| *a == Action::NewGroup).map(|(k, _)| shortcuts::display(k));
    let rows = organize(cleanable, assistant).into_iter().map(move |(name, o)| {
        let keys = if o == Organize::NewGroup { group_keys.clone() } else { None };
        menu_row(name, keys, move || {
            ui.panel.set(Panel::None);
            match o {
                Organize::NewGroup => super::groups::new_group(ui, None),
                Organize::NewFolder => act(ui, |b| b.new_folder(None)),
                Organize::CleanUp => act(ui, Browser::clean_up_tabs),
                Organize::GroupWithAssistant => super::assistant::group_tabs(ui),
            }
        })
    });
    popover().child(heading(ui, title(ORGANIZE))).child(group().children(rows)).into_element()
}

fn sidebar_panel(ui: Ui) -> AnyElement {
    let Some((reopenable, tabs, unpinned, duplicates, keymap)) = read(ui, |b| {
        (
            b.model.closed_tabs.newest().is_some(),
            !b.model.tabs.is_empty(),
            !b.model.ephemeral_keys().is_empty(),
            !b.duplicate_tabs().is_empty(),
            shortcuts::keymap(&shortcuts::stored(&b.core.disk)),
        )
    }) else {
        return view().into_element();
    };
    let keys = |action: Action| keymap.iter().find(|(_, a)| *a == action).map(|(k, _)| shortcuts::display(k));
    let closing = |name: &str, keys: Option<String>, run: fn(&mut Browser)| {
        menu_row(name, keys, move || {
            ui.panel.set(Panel::None);
            act(ui, run);
        })
    };
    let tab_rows = group()
        .child(menu_row("New Tab", keys(Action::NewTab), move || super::run_action(ui, Action::NewTab)))
        .children(reopenable.then(|| closing("Reopen Closed Tab", keys(Action::ReopenClosedTab), Browser::reopen)));
    let list_rows = group()
        .child(closing("New Folder", None, |b| b.new_folder(None)))
        .children(tabs.then(|| {
            menu_row("Bookmark All Tabs", None, move || {
                ui.panel.set(Panel::None);
                super::library::bookmark_all_tabs(ui);
            })
        }))
        .child(menu_row("Hide Tabs", keys(Action::ToggleSidebar), move || {
            ui.panel.set(Panel::None);
            super::run_action(ui, Action::ToggleSidebar);
        }));
    let profile_rows = group()
        .child(menu_row("New Profile…", None, move || {
            ui.panel.set(Panel::None);
            super::profiles::open_profile_card(ui, None);
        }))
        .child(menu_row("Edit Profile", None, move || super::open_settings(ui, tarn_nsh_shared::panes::Pane::Profile)));
    let close_rows = (unpinned || duplicates).then(|| {
        group()
            .children(unpinned.then(|| closing("Close All Tabs", keys(Action::CloseAllTabs), Browser::close_all_tabs)))
            .children(duplicates.then(|| closing("Close Duplicate Tabs", None, Browser::close_duplicates)))
    });
    popover().child(heading(ui, title("Tabs"))).child(tab_rows).child(list_rows).child(profile_rows).children(close_rows).into_element()
}

fn address_panel(ui: Ui) -> AnyElement {
    let site = ui.snap.with(|s| !s.host.is_empty());
    let tool = |name: &'static str, command: Command, action: Action| {
        menu_row(name, super::shortcut_of(ui, action), move || super::tools::run_from_menu(ui, command.clone()))
    };
    let paste =
        menu_row("Paste and Go", None, move || super::paste_and_go(ui)).disabled(move || !ui.pasteable.get()).disabled_style(|s| s.text_color(Theme::faint()));
    popover()
        .child(
            group()
                .child(tool("Copy Link", Command::CopyLink, Action::CopyLink))
                .child(tool("Copy Clean Link", Command::CopyCleanLink, Action::CopyCleanLink))
                .child(paste),
        )
        .children(site.then(|| group().child(menu_row("Site Settings…", None, move || show(ui, Panel::Shield)))))
        .into_element()
}

fn shortcuts_panel(ui: Ui) -> AnyElement {
    let map = read(ui, |b| shortcuts::keymap(&shortcuts::stored(&b.core.disk))).unwrap_or_default();
    let columns = shortcuts::sections().map(|(name, rows)| {
        let bound = rows.into_iter().map(|(row_label, action)| {
            let keys = map.iter().find(|(_, a)| *a == action).map(|(k, _)| shortcuts::display(k)).unwrap_or_default();
            line(row_label).child(tarn_nsh_shared::controls::key_caps(&keys)).into_element()
        });
        let fixed =
            shortcuts::FIXED.iter().filter(|(section, ..)| *section == name).map(|(_, row_label, keys)| line(*row_label).child(small(*keys)).into_element());
        view().flex_col().w(px(250.)).gap_1().child(caps_label(name)).children(bound.chain(fixed).collect::<Vec<_>>()).into_element()
    });
    let sheet = element_ref();
    super::peek::later(ui, std::time::Duration::ZERO, move || sheet.focus());
    popover()
        .child(heading(ui, title("Keyboard Shortcuts")))
        .child(scroll().flex_1().focusable(true).element_ref(&sheet).child(view().flex_row().flex_wrap().gap_6().children(columns.collect::<Vec<_>>())))
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
            Panel::NewSpace => super::library::new_space_panel(ui),
            Panel::Memory => super::library::memory_panel(ui),
            Panel::ReadingList => super::library::reading_list_panel(ui),
            Panel::Archived => super::library::archived_panel(ui),
            Panel::Qr => super::tools::qr_panel(ui),
            Panel::Snippets => super::tools::snippets_panel(ui),
            Panel::Share => super::tools::share_panel(ui),
            Panel::Library => super::library::library_panel(ui),
            Panel::Address => address_panel(ui),
            Panel::Shortcuts => shortcuts_panel(ui),
            Panel::Import => super::import::panel(ui),
            Panel::History => history_panel(ui),
            Panel::Downloads => downloads_panel(ui),
            Panel::Tab => tab_panel(ui),
            Panel::Folder(id) => folder_panel(ui, id),
            Panel::Bookmarks(folder) => super::library::bookmarks_panel(ui, folder),
            Panel::Group(id) => super::groups::group_panel(ui, id),
            Panel::Sidebar => sidebar_panel(ui),
            Panel::Organize => organize_panel(ui),
            Panel::Card(card) => super::profiles::card_panel(ui, card),
            Panel::SpaceMenu(key) => super::profiles::space_menu(ui, key),
        };
        let panel = ui.panel.get_untracked();
        let (width, side) = match panel {
            Panel::Share => (320., Side::End),
            Panel::Address => (320., Side::Centre),
            Panel::Shortcuts => (PANEL_WIDTH + 200., Side::Centre),
            Panel::Shield => (SITE_WIDTH, Side::Centre),
            Panel::NewSpace => (SPACE_CARD_WIDTH, Side::Centre),
            Panel::Downloads => (380., Side::Start),
            Panel::Sidebar | Panel::Organize => (320., Side::Start),
            Panel::Settings => (PANEL_WIDTH + 200., Side::Centre),
            _ => (PANEL_WIDTH, Side::Centre),
        };
        let content =
            if panel == Panel::Settings { view().flex_col().flex_1().child(body).into_element() } else { scroll().flex_1().child(body).into_element() };
        let column = view().flex_col().flex_1().w(px(width)).child(content);
        let outside = matches!(panel, Panel::NewSpace | Panel::Space)
            .then(|| nsh_base::button().accessibility_label("Close").on_press(move || show(ui, Panel::None)).absolute().inset_0());
        let area = view().flex_col().flex_1().relative().p_4().children(outside);
        let area = match side {
            Side::Start => area.items_start(),
            Side::Centre => area.items_center(),
            Side::End => area.items_end(),
        };
        Some(area.child(tarn_nsh_shared::motion::popping(column)))
    })
    .into_element()
}
