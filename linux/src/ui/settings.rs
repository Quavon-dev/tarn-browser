use browser_core::shield_card::{Card, H, SHORT, W, back, day_totals, days_text, face, front, front_svg, thousands};
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;
use shield::{Category, ShieldLevel};
use tarn_nsh_shared::panes::{self, GROUPS, Pane};

use super::panels::{capitalised, levels, section};
use super::tools::{line, note};
use super::{Panel, Ui, WINDOWS, act, confirmed, in_other_windows, read};
use crate::browser::pages::{HISTORY_TEXT, SORT_DOWNLOADS};
use crate::browser::{Browser, Notice};
use crate::controls::{accent_pill, group, group_row, icon_pill, label, pill, pill_grid, segmented, small, switch, title};
use crate::icons;
use crate::shortcuts;
use crate::theme::Theme;
use crate::webview;
use browser_core::closed::ARCHIVE_AFTER;
use browser_core::command::ENGINES;
use browser_core::incoming::LOCALHOST_FROM_APPS;
use browser_core::prefs::{CLOSE_SELECTS_PREVIOUS, GESTURES, NEW_TAB_START_PAGE};
use browser_core::settings::{self as prefs, LIVE_TABS, LIVE_TABS_KEY, SLEEP_AFTER_DEFAULT, SLEEP_AFTER_KEY, SUGGESTIONS};

const NAV_WIDTH: f32 = 180.;
const CARD_SCALE: u32 = 3;

fn card_of(b: &Browser) -> Option<(Card, bool)> {
    let space = b.model.current_space()?;
    let store = b.core.store(space.private);
    let name = if space.private { "Private".to_owned() } else { engine::view_policy::title(&store.profile(space.space.profile_id).ok()?.name) };
    Some((browser_core::shield_card::read(store, space.space.profile_id, space.private, name, browser_core::now())?, space.private))
}

fn words(labels: &[browser_core::shield_card::Label]) -> Vec<tarn_nsh_shared::card::Words<'_>> {
    labels
        .iter()
        .map(|l| tarn_nsh_shared::card::Words {
            x: l.x,
            y: l.y,
            anchor: l.anchor,
            size: l.size,
            weight: l.weight,
            fill: l.fill,
            opacity: l.opacity,
            text: &l.text,
        })
        .collect()
}

fn flip(ui: Ui) {
    ui.card_back.update(|b| *b = !*b);
    act(ui, |_| {});
}

fn shield_card(ui: Ui, card: Card) -> AnyElement {
    super::themed(move || {
        let flipped = ui.card_back.get_untracked();
        let dark = !Theme::is_light();
        let (sky, shapes, labels) = if flipped { back(&card, dark) } else { front(&card, true, dark) };
        let counts = card.counters.iter().map(|(name, n)| format!("{name}: {}", thousands((*n).max(0) as u64))).collect::<Vec<_>>().join(", ");
        let name = if flipped { format!("Shield card, stamps side: {}", card.stamps.join(", ")) } else { format!("Shield card of {}: {counts}", card.name) };
        crate::controls::pressable(name, move || flip(ui))
            .w(px(W as f32))
            .h(px(H as f32))
            .flex_none()
            .rounded(px(Theme::radius_card()))
            .shadow(Theme::elevation(10., 30.))
            .child(tarn_nsh_shared::card::picture(&face(sky, &shapes), W as f32, H as f32, &words(&labels), 1.))
    })
}

fn save_card(ui: Ui, card: Card) {
    let svg = front_svg(&card, true, !Theme::is_light());
    let Some(png) = card_png(&svg) else {
        return act(ui, |b| b.show_toast("The card could not be drawn as an image", None));
    };
    let mut dialog = SaveDialog::new().file_name("Tarn Shield card.png").filter(FileFilter::new("PNG image", ["png"]));
    if let Some(downloads) = std::env::home_dir().map(|h| h.join("Downloads")) {
        dialog = dialog.directory(downloads);
    }
    ui.root.run(|| {
        drop(spawn(async move {
            let Ok(Some(path)) = pick_save_path(dialog).await else { return };
            let saved = std::fs::write(&path, png);
            act(ui, |b| b.show_toast(if saved.is_ok() { "Card saved" } else { "The card could not be saved" }, None));
        }))
    });
}

fn card_png(svg: &str) -> Option<Vec<u8>> {
    use webkit6::prelude::*;
    use webkit6::{gdk, glib};
    let size = format!(r#"width="{W}" height="{H}""#);
    let large = svg.replacen(&size, &format!(r#"width="{}" height="{}""#, W * CARD_SCALE, H * CARD_SCALE), 1);
    let texture = gdk::Texture::from_bytes(&glib::Bytes::from_owned(large.into_bytes())).ok()?;
    Some(texture.save_to_png_bytes().to_vec())
}

fn number(n: i64, name: &str) -> View {
    view()
        .flex_col()
        .flex_1()
        .child(text(thousands(n.max(0) as u64)).text_size(px(20.)).font_weight(FontWeight::BOLD).text_color(Theme::text()))
        .child(small(name.to_owned()))
}

fn hero(ui: Ui, card: Card, level: ShieldLevel, private: bool) -> View {
    let protected = match (card.days, card.since.as_deref()) {
        (Some(days), Some(since)) => format!("Protected for {} · since {since}", days_text(days)),
        _ => "Counting starts with the first page".to_owned(),
    };
    let numbers: Vec<AnyElement> = card
        .counters
        .iter()
        .zip(SHORT)
        .map(|((_, n), (a, b))| {
            let first = if a.chars().all(|c| c.is_uppercase()) { a.to_owned() } else { a.to_lowercase() };
            number(*n, &format!("{first} {b}")).into_element()
        })
        .collect();
    let level_name = if private { "Shield level (private space)" } else { "Shield level" };
    let shared = card.clone();
    let mut actions =
        vec![icon_pill(icons::RELOAD, "Flip", move || flip(ui)), icon_pill(icons::PALETTE, "Change design", move || act(ui, Browser::new_shield_design))];
    actions.extend((!private).then(|| icon_pill(icons::FILE_IMAGE, "Save as image", move || save_card(ui, shared.clone()))));
    let actions = pill_grid(actions);
    view().flex_row().flex_wrap().items_start().gap_5().child(shield_card(ui, card.clone())).child(
        view()
            .flex_col()
            .flex_1()
            .min_w(px(260.))
            .gap_3()
            .child(
                view()
                    .flex_col()
                    .child(text(card.name.clone()).text_size(px(22.)).font_weight(FontWeight::BOLD).text_color(Theme::text()))
                    .child(small(protected)),
            )
            .child(view().flex_row().gap_3().children(numbers))
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(label(level_name).flex_1())
                    .child(segmented(levels(), level, move |l| act(ui, move |b| b.set_default_level(l)))),
            )
            .child(actions),
    )
}

fn boxed(name: &str, aside: Option<String>) -> View {
    let head = view().flex_row().items_baseline().child(label(name.to_owned()).font_weight(FontWeight::SEMIBOLD).flex_1()).children(aside.map(small));
    group().gap_3().p_4().flex_1().min_w(px(260.)).child(head)
}

fn week(ui: Ui, card: &Card) -> View {
    let days = day_totals(&card.week);
    let total: i64 = days.iter().map(|(_, n)| *n).sum();
    let chart = boxed("Last 7 days", Some(format!("{} protected", thousands(total.max(0) as u64)))).child(tarn_nsh_shared::card::day_chart(&days));
    let most = card.sites.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let sites: Vec<AnyElement> = card
        .sites
        .iter()
        .map(|(host, n)| {
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .min_h(px(24.))
                .child(super::site_tile(ui, host, 16.))
                .child(label(browser_core::command::host_tail(&engine::view_policy::title(host), 28)).w(px(130.)).truncate())
                .child(tarn_nsh_shared::card::share_bar(*n, most).flex_1())
                .child(small(thousands((*n).max(0) as u64)).w(px(44.)).text_right())
                .into_element()
        })
        .collect();
    let empty = sites.is_empty().then(|| note("Sites show here once Shield has stopped something on them."));
    let sites = boxed("Most protected sites", None).gap_2().children(sites).children(empty);
    view().flex_row().flex_wrap().gap_3().child(chart).child(sites)
}

fn shortcut_rows(ui: Ui) -> View {
    let Some(stored) = read(ui, |b| shortcuts::stored(&b.core.disk)) else { return view() };
    let map = shortcuts::keymap(&stored);
    let mut out = view().flex_col().gap_2().child(note("Click a shortcut and type the new keys with Ctrl, such as Ctrl+Shift+Y; Enter saves, Esc cancels."));
    if let Some(error) = ui.shortcut_error.get_untracked() {
        out = out.child(note(error).text_color(Theme::warning()));
    }
    for (name, rows) in shortcuts::SECTIONS {
        let mut grp = group();
        for (row_label, action, _) in rows {
            let keys = map.iter().find(|(_, a)| a == action).map(|(k, _)| k.clone()).unwrap_or_default();
            let custom = stored.contains_key(&action.setting_key());
            let action = *action;
            let editing = ui.shortcut_edit.get_untracked() == Some(action);
            let control: AnyElement = if editing {
                super::text_field(ui.shortcut_text, "ctrl+shift+y", 160.)
                    .auto_focus(true)
                    .on_submit(move |v: &str| {
                        let raw = v.to_owned();
                        let result = read(ui, |b| shortcuts::save(&b.core.disk, action, &raw)).unwrap_or(Ok(()));
                        ui.shortcut_error.set(result.err());
                        ui.shortcut_edit.set(None);
                        ui.keys_rev.update(|r| *r += 1);
                        act(ui, |_| {});
                    })
                    .into_element()
            } else {
                pill(shortcuts::display(&keys), move || {
                    ui.shortcut_text.set(String::new());
                    ui.shortcut_error.set(None);
                    ui.shortcut_edit.set(Some(action));
                    act(ui, |_| {});
                })
                .into_element()
            };
            let reset = custom.then(|| {
                pill("Reset", move || {
                    let result = read(ui, |b| shortcuts::reset(&b.core.disk, Some(action))).unwrap_or(Ok(()));
                    ui.shortcut_error.set(result.err());
                    ui.keys_rev.update(|r| *r += 1);
                    act(ui, |_| {});
                })
            });
            grp = grp.child(group_row().child(label(*row_label).flex_1()).children(reset).child(control));
        }
        out = out.child(small(name).pt_2()).child(grp);
    }
    out = out.child(note("Fixed: Ctrl+1 … Ctrl+9 pick a tab, Ctrl+9 the last; Alt+1 … Alt+9 switch to space 1–9."));
    if !stored.is_empty() {
        out = out.child(view().flex_row().justify_end().child(pill("Reset All", move || {
            let result = read(ui, |b| shortcuts::reset(&b.core.disk, None)).unwrap_or(Ok(()));
            ui.shortcut_error.set(result.err());
            ui.keys_rev.update(|r| *r += 1);
            act(ui, |_| {});
        })));
    }
    out
}

fn rules(ui: Ui) -> View {
    let rows = read(ui, Browser::space_rule_rows).unwrap_or_default();
    let mut out = view().flex_col();
    if rows.is_empty() {
        return out.child(note("No rules. “Always open in” in a site's settings opens its new tabs in the space you pick."));
    }
    for (id, host, space) in rows {
        out = out.child(line(format!("{host} → {space}")).child(pill("Remove", move || act(ui, |b| b.delete_space_rule(id)))));
    }
    out
}

fn hidden(ui: Ui, private: bool) -> View {
    if private {
        return view().child(note("A private space keeps no hidden elements."));
    }
    let zaps = read(ui, |b| b.zapped(None)).unwrap_or_default();
    if zaps.is_empty() {
        return view().child(note("No hidden elements. Zap Element in the command bar hides an element on a site in this profile."));
    }
    let rows: Vec<AnyElement> = zaps
        .into_iter()
        .map(|(host, selector)| {
            let (h, s) = (host.clone(), selector.clone());
            line(format!("{host}: {selector}")).child(pill("Undo", move || act(ui, |b| b.unzap(&h, Some(&s))))).into_element()
        })
        .collect();
    view().flex_col().children(rows)
}

fn site_list(ui: Ui, hosts: Vec<String>, empty: &'static str, button: &'static str, run: fn(&mut Browser, &str)) -> View {
    if hosts.is_empty() {
        return view().child(note(empty));
    }
    let rows: Vec<AnyElement> = hosts
        .into_iter()
        .map(|host| {
            let shown = browser_core::text::bounded_title(&host);
            line(shown)
                .child(pill(button, move || {
                    let host = host.clone();
                    act(ui, move |b| run(b, &host))
                }))
                .into_element()
        })
        .collect();
    view().flex_col().children(rows)
}

fn permissions(ui: Ui) -> View {
    let rows = read(ui, |b| b.permissions()).unwrap_or_default();
    if rows.is_empty() {
        return view()
            .child(note("No site has asked yet. When a site asks for the camera, microphone or location and you choose Remember, the answer shows here."));
    }
    let rows: Vec<AnyElement> = rows
        .into_iter()
        .map(|p| {
            let set = |allow: Option<bool>| {
                let (origin, kind) = (p.origin.clone(), p.kind.clone());
                move || act(ui, |b| b.set_permission(&origin, &kind, allow))
            };
            let mark = |on: bool, name: &'static str, allow: bool| {
                if on { accent_pill(name, set(Some(allow))).into_element() } else { pill(name, set(Some(allow))).into_element() }
            };
            line(format!("{} · {}", p.site, p.label))
                .child(mark(p.allowed, "Allow", true))
                .child(mark(!p.allowed, "Block", false))
                .child(pill("Forget", set(None)))
                .into_element()
        })
        .collect();
    view().flex_col().children(rows)
}

fn overrides(ui: Ui) -> View {
    let rows = read(ui, Browser::site_overrides).unwrap_or_default();
    if rows.is_empty() {
        return view().child(note("No site overrides. Use the Shield button in the address bar to set one for a site."));
    }
    let rows: Vec<AnyElement> = rows
        .into_iter()
        .map(|(host, level)| {
            line(engine::view_policy::title(&host))
                .child(small(capitalised(level.as_str())))
                .child(pill("Remove", move || remove_override(ui, host.clone())))
                .into_element()
        })
        .collect();
    view().flex_col().children(rows)
}

fn remove_override(ui: Ui, host: String) {
    let Some(Some((private, profile))) = read(ui, |b| b.model.profile()) else { return };
    act(ui, |b| b.remove_site_override(&host));
    in_other_windows(ui, move |b| b.reload_level(private, profile, &host));
}

fn zooms(ui: Ui, rows: Vec<(String, f64)>) -> View {
    if rows.is_empty() {
        return view().child(note("No saved zoom levels. Zooming in or out remembers the zoom for the site; Actual Size forgets it."));
    }
    let rows: Vec<AnyElement> = rows
        .into_iter()
        .map(|(host, zoom)| {
            line(engine::view_policy::title(&host))
                .child(small(format!("{:.0} %", zoom * 100.)))
                .child(pill("Remove", move || remove_zoom(ui, Some(host.clone()))))
                .into_element()
        })
        .collect();
    view().flex_col().children(rows)
}

fn remove_zoom(ui: Ui, host: Option<String>) {
    let Some(Some((private, profile))) = read(ui, |b| b.model.profile()) else { return };
    act(ui, |b| b.remove_site_zoom(host.as_deref()));
    in_other_windows(ui, move |b| b.reset_zooms(private, profile, host.as_deref()));
}

fn export_profile(ui: Ui) {
    let (name, json) = match read(ui, Browser::export_profile) {
        Some(Ok(export)) => export,
        Some(Err(e)) => return act(ui, |b| b.notice = Some(Notice::Message(format!("Export failed: {e}.")))),
        None => return,
    };
    let mut dialog = SaveDialog::new().file_name(name).filter(FileFilter::new("JSON", ["json"]));
    if let Some(downloads) = std::env::home_dir().map(|h| h.join("Downloads")) {
        dialog = dialog.directory(downloads);
    }
    ui.root.run(|| {
        drop(spawn(async move {
            let Ok(Some(path)) = pick_save_path(dialog).await else { return };
            let message = match write_private(&path, &json) {
                Ok(()) => format!("Exported to {}.", path.display()),
                Err(e) => format!("Export failed: {e}."),
            };
            act(ui, |b| b.notice = Some(Notice::Message(message)));
        }))
    });
}

fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(text.as_bytes())
}

fn delete_profile_data(ui: Ui) {
    let Some(Some((private, profile))) = read(ui, |b| b.model.profile()) else { return };
    let mut wipes = Vec::new();
    act(ui, |b| wipes = b.delete_profile_data());
    in_other_windows(ui, move |b| b.forget_profile(private, profile));
    for window in WINDOWS.with_borrow(Vec::clone) {
        window.history.set(Default::default());
    }
    if !wipes.is_empty() {
        run_on_main_thread(move || wipes.into_iter().for_each(webview::session::wipe));
    }
}

fn data_row(name: &str, action: AnyElement) -> View {
    group_row().child(label(name.to_owned()).flex_1()).child(action)
}

fn overview(ui: Ui) -> View {
    let overrides = match read(ui, |b| b.site_overrides().len()).unwrap_or(0) {
        0 => "None".to_owned(),
        1 => "1 site".to_owned(),
        n => format!("{n} sites"),
    };
    let mut out = view().flex_col();
    if crate::desktop::is_default_browser() == Some(false) {
        let text = view().flex_col().flex_1().child(label("Tarn isn't your default browser")).child(small("Links from other apps open in another browser."));
        out = out.child(section("Default browser")).child(group().child(group_row().child(text).child(accent_pill("Make Default", move || make_default(ui)))));
    }
    let change = pill("Change…", move || {
        ui.settings_pane.set(Pane::Privacy);
        act(ui, |_| {});
    });
    out.child(section("Shield")).child(group().child(group_row().child(label("Site overrides").flex_1()).child(small(overrides)).child(change)))
}

fn make_default(ui: Ui) {
    let made = crate::desktop::make_default_browser();
    act(ui, |b| {
        if let Err(e) = made {
            b.notice = Some(Notice::Message(format!("Tarn could not be made the default browser ({e}).")));
        }
    });
}

fn nav(ui: Ui, current: Pane) -> View {
    let ext_on = ui.ext_pane.get_untracked();
    let mut out = view().flex_col().gap(px(2.)).w(px(NAV_WIDTH)).flex_none().p_2().rounded(px(Theme::radius_group())).bg(Theme::panel_nav());
    for (group, members) in GROUPS {
        out = out.child(view().pt_2().pb_1().px_1().child(tarn_nsh_shared::controls::caps_label(group)));
        for pane in members.iter().copied() {
            let selected = pane == current && !ext_on;
            let name = panes::name(pane);
            out = out.child(
                crate::controls::row(name, move || {
                    ui.ext_pane.set(false);
                    if pane == Pane::Developer {
                        super::devtools::refresh_servers(ui);
                    }
                    ui.settings_pane.set(pane);
                    act(ui, |_| {});
                })
                .h(px(30.))
                .px(px(10.))
                .rounded(px(Theme::radius_row()))
                .bg(if selected { Theme::selected() } else { rgba(0) })
                .hover(|s| s.bg(Theme::hover()))
                .text_color(if selected { Theme::text() } else { Theme::muted() })
                .child(crate::icons::icon(panes::icon(pane), 14.))
                .child(label(name).text_color(if selected { Theme::text() } else { Theme::muted() })),
            );
        }
    }
    out.child(
        crate::controls::row("Extensions", move || {
            ui.ext_pane.set(true);
            act(ui, |_| {});
        })
        .h(px(30.))
        .px(px(10.))
        .rounded(px(Theme::radius_row()))
        .bg(if ext_on { Theme::selected() } else { rgba(0) })
        .hover(|s| s.bg(Theme::hover()))
        .text_color(if ext_on { Theme::text() } else { Theme::muted() })
        .child(crate::icons::icon(crate::icons::SPARKLES, 14.))
        .child(label("Extensions").text_color(if ext_on { Theme::text() } else { Theme::muted() })),
    )
}

pub fn settings_panel(ui: Ui) -> AnyElement {
    let pane = ui.settings_pane.get_untracked();
    let body = if ui.ext_pane.get_untracked() {
        view().flex_col().child(super::panels::heading(ui, title("Extensions"))).child(view().pt_3().child(super::extensions::pane(ui)))
    } else {
        pane_body(ui, pane).unwrap_or_else(|| view().flex_col())
    };
    view()
        .flex_row()
        .flex_1()
        .gap_4()
        .p_4()
        .rounded(px(Theme::radius_card()))
        .bg(Theme::panel())
        .border_1()
        .border_color(Theme::surface_edge())
        .shadow(Theme::elevation(18., 48.))
        .child(nav(ui, pane))
        .child(scroll().flex_1().child(body.child(view().h(px(12.)))))
        .into_element()
}

fn pane_body(ui: Ui, pane: Pane) -> Option<View> {
    let (level, engine, lists, private, card, history_text, sort, archive, youtube) = read(ui, |b| {
        (
            b.default_level(),
            b.search_engine(),
            b.lists(),
            b.model.private(),
            card_of(b),
            !b.setting_off(HISTORY_TEXT),
            !b.setting_off(SORT_DOWNLOADS),
            b.archive_after_index(),
            b.setting_on(shield::groups::YOUTUBE_SETTING),
        )
    })?;
    let (suggestions, live_tabs, sleep_after) = read(ui, |b| {
        let disk = &b.core.disk;
        let sleep_options = prefs::sleep_after_options();
        (
            b.suggestions(),
            prefs::choice(disk, LIVE_TABS_KEY, &LIVE_TABS, prefs::LIVE_TABS_DEFAULT),
            prefs::choice(disk, SLEEP_AFTER_KEY, &sleep_options, SLEEP_AFTER_DEFAULT),
        )
    })?;
    let (start_page, previous, gestures) = read(ui, |b| (b.pref(NEW_TAB_START_PAGE), b.pref(CLOSE_SELECTS_PREVIOUS), b.pref(GESTURES)))?;
    let (sets, paused, muted) = read(ui, |b| (b.set_categories(), b.paused_sites(), b.muted_sites()))?;
    let on = |group: &str| lists.iter().any(|l| l == group);
    let toggle = move |setting: &'static str| move || act(ui, move |b| b.toggle_setting(setting));
    let toggle_off = move |setting: &'static str| move || act(ui, move |b| b.toggle_off_setting(setting));
    let toggle_set = move |category: Category| move || act(ui, move |b| b.toggle_set(category));
    let body = view().flex_col().child(super::panels::heading(ui, title(panes::name(pane))));
    Some(match pane {
        Pane::Profile => match card {
            Some((card, private)) => {
                let kept = if private { "Kept in memory only" } else { "Counted on this computer only" };
                let chip = view()
                    .flex_row()
                    .items_center()
                    .gap_1()
                    .h(px(24.))
                    .px_2()
                    .rounded(px(Theme::radius_small()))
                    .border_1()
                    .border_color(Theme::hairline())
                    .text_color(Theme::muted())
                    .child(crate::icons::icon(crate::icons::LOCK, 11.))
                    .child(small(kept));
                view()
                    .flex_col()
                    .gap_4()
                    .child(super::panels::heading(ui, view().flex_row().items_center().flex_1().child(title("Profile")).child(view().flex_1()).child(chip)))
                    .child(hero(ui, card.clone(), level, private))
                    .child(week(ui, &card))
                    .child(overview(ui))
            }
            None => body.child(note("No profile is open.")),
        },
        Pane::General => body
            .child(section("Downloads"))
            .child(group().child(switch(sort, "Sort downloads into folders", toggle_off(SORT_DOWNLOADS))))
            .child(note("New files go into Images, Videos, Audio, Documents, Archives or Installers inside Downloads; other types stay at the top."))
            .children(browser_core::import::available().then(|| section("Import")))
            .children(browser_core::import::available().then(|| {
                group().child(group_row().child(label("Import browser data").flex_1()).child(pill("Import\u{2026}", move || super::import::open(ui))))
            }))
            .children(
                browser_core::import::available()
                    .then(|| note("Sign-ins, history and bookmarks from Chrome, Edge, Brave, Chromium or Firefox, into this profile.")),
            )
            .child(section("About"))
            .child(group().child(group_row().child(label("Tarn version").flex_1()).child(small(browser_core::VERSION))))
            .child(
                view()
                    .pt_2()
                    .flex_row()
                    .flex_wrap()
                    .gap_2()
                    .child(link_pill(ui, browser_core::LICENCE, browser_core::LICENCE_URL))
                    .child(link_pill(ui, "Source code", browser_core::SOURCE_URL))
                    .child(link_pill(ui, "Privacy notice", browser_core::PRIVACY_URL))
                    .child(link_pill(ui, "Imprint", browser_core::IMPRINT_URL))
                    .child(link_pill(ui, "Open-source licences", browser_core::LINUX_LICENCES_URL)),
            ),
        Pane::Appearance => body.child(view().pt_3().child(super::appearance::pane(ui))),
        Pane::Tabs => {
            let choices: Vec<(usize, String)> = ARCHIVE_AFTER.iter().enumerate().map(|(ix, (name, _))| (ix, (*name).to_owned())).collect();
            body.child(section("Tabs"))
                .child(
                    group()
                        .child(group_row().child(label("Tabs kept awake").flex_1()).child(segmented(options(&LIVE_TABS), live_tabs, move |ix| {
                            set_tab_choice(ui, |b| {
                                b.set_live_tabs(ix);
                                false
                            })
                        })))
                        .child(
                            group_row()
                                .child(label("Sleep background tabs after").flex_1())
                                .child(segmented(options(&prefs::sleep_after_options()), sleep_after, move |ix| set_tab_choice(ui, |b| b.set_sleep_after(ix)))),
                        )
                        .child(group_row().child(label("New tab opens").flex_1()).child(segmented(
                            vec![(false, "Command card".to_owned()), (true, "Start page".to_owned())],
                            start_page,
                            move |on| act(ui, move |b| b.set_pref(NEW_TAB_START_PAGE, on)),
                        )))
                        .child(group_row().child(label("After closing a tab").flex_1()).child(segmented(
                            vec![(false, "Next".to_owned()), (true, "Previous".to_owned())],
                            previous,
                            move |on| act(ui, move |b| b.set_pref(CLOSE_SELECTS_PREVIOUS, on)),
                        )))
                        .child(group_row().child(label("Close unused tabs after").flex_1()).child(segmented(choices, archive, move |ix| act(ui, move |b| b.set_archive_after(ix)))))
                        .child(switch(gestures, "Touchpad gestures", move || act(ui, |b| b.set_pref(GESTURES, !b.pref(GESTURES))))),
                )
                .child(note("Other tabs sleep and reload when you return, unless they are playing video or audio; fewer saves memory."))
                .child(note("A tab left for this long sleeps and reloads when you return, after a minute while memory is short. Pinned tabs and tabs playing video or audio stay awake."))
                .child(note("New Tab shows the command card with the search field, or the start page. After closing a tab, the next or previous one in the list becomes active."))
                .child(note("Unpinned tabs you have not opened for this long move to History › Archived Tabs."))
                .child(note("Touchpad gestures: swipe with two fingers over the sidebar to switch spaces."))
        }
        Pane::Search => {
            let engines: Vec<(usize, String)> = ENGINES.iter().enumerate().map(|(ix, e)| (ix, e.name.to_owned())).collect();
            body.child(section("Search engine"))
                .child(group().child(group_row().child(label("Search engine").flex_1()).child(segmented(engines, engine, move |ix| act(ui, move |b| b.set_search_engine(ix))))))
                .child(section("Suggestions"))
                .child(group().child(switch(suggestions, "Suggest open tabs and history", toggle(SUGGESTIONS))).child(switch(history_text, "Search the text of visited pages", toggle_off(HISTORY_TEXT))))
                .child(note("Shown under the address field while you type. Nothing you type leaves this computer before you press Enter."))
                .child(note("Search the text of visited pages keeps up to 4 KB of each page's text on this computer so history suggestions match what a page said. Never for the private space or Lockdown pages; cleared with history, and when you turn this off."))
        }
        Pane::Privacy => {
            let (localhost, zoom_rows) = read(ui, |b| (b.setting_on(LOCALHOST_FROM_APPS), b.site_zooms())).unwrap_or((true, Vec::new()));
            let has_zooms = !zoom_rows.is_empty();
            let clear_data = move || {
                if let Some(key) = read(ui, |b| b.store_key()).flatten() {
                    run_on_main_thread(move || webview::session::clear(&key));
                    act(ui, |b| b.notice = Some(Notice::Message("Website data of this profile was cleared.".into())));
                }
            };
            let (filter_updates, filter_status) = read(ui, Browser::filter_updates).unwrap_or_default();
            body.child(section("Shield"))
                .child(
                    group()
                        .child(
                            group_row()
                                .child(label(if private { "Shield level (private space)" } else { "Default Shield level" }).flex_1())
                                .child(segmented(levels(), level, move |l| act(ui, move |b| b.set_default_level(l)))),
                        )
                        .child(switch(on("ads"), "Block ads", toggle("block_ads")))
                        .child(switch(on("trackers"), "Block trackers", toggle("block_trackers")))
                        .child(switch(on("annoyances"), "Block cookie banners", toggle("block_annoyances")))
                        .child(switch(youtube, "Block YouTube ads", toggle(shield::groups::YOUTUBE_SETTING)))
                        .child(switch(sets.contains(&Category::Popups), "Block pop-ups and ad redirects", toggle_set(Category::Popups)))
                        .child(switch(sets.contains(&Category::Malware), "Block dangerous sites", toggle_set(Category::Malware)))
                        .child(switch(sets.contains(&Category::Adult), "Block adult content", toggle_set(Category::Adult))),
                )
                .child(note("EasyList Cookie and uBlock Cookie Notices hide consent pop-ups and stop the scripts that show them. Nothing is accepted or refused for you."))
                .child(note("Block YouTube ads removes ads from YouTube's player data before the player sees them and hides ad slots. YouTube changes often; if videos stop playing, turn this off."))
                .child(note("Pages open windows only when you click, never to known pop-up ad sites, and cannot send you to one. Malware, phishing and scam lists stop dangerous sites with a warning page; adult sites get one with no way past it."))
                .child(note("Filter lists apply to pages loaded after a change; they need the signed list bundle."))
                .child(section("Links from other apps"))
                .child(group().child(switch(localhost, "Open localhost links from other apps", toggle(LOCALHOST_FROM_APPS))))
                .child(note("A link to this computer (localhost, 127.0.0.1) from a terminal, editor or other app opens like a typed address, and the page it shows may lead elsewhere on your network as a typed one can. Off: Tarn refuses it, since a link in a mail could reach services running here; Open Anyway on the notice still opens it. Links to other addresses on your network are refused; Open Anyway on the notice opens them. Applies to every profile."))
                .child(section("Site overrides"))
                .child(overrides(ui))
                .child(section("Muted sites"))
                .child(site_list(ui, muted, "No muted sites. Mute Site in a tab's panel or the command bar silences a site in this profile.", "Unmute", |b, host| b.unmute_site(host)))
                .child(section("Sites without ad, tracker and cookie-banner blocking"))
                .child(site_list(ui, paused, "Blocking runs on every site. Turn it off for one site with the Shield button in the address bar.", "Turn On", |b, host| b.set_site_paused(host, false)))
                .child(section("Filter-list updates"))
                .children(filter_updates.map(|on| group().child(switch(on, "Download filter-list updates", toggle(shield::update::SETTING)))))
                .child(note(format!("GitHub, once a day. {filter_status}")))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .child(section("Site zoom levels").flex_1())
                        .children(has_zooms.then(|| confirmed(ui, "zooms".to_owned(), "Reset All", move || remove_zoom(ui, None)))),
                )
                .child(zooms(ui, zoom_rows))
                .child(section("Hidden elements"))
                .child(hidden(ui, private))
                .child(section("Your data"))
                .child(
                    group()
                        .children((!private).then(|| data_row("Export this profile's data as JSON", pill("Export…", move || export_profile(ui)).into_element())))
                        .child(data_row(
                            "Clear browsing history, site icons, zoom levels, protected sites and hidden elements",
                            super::clear_history_choice(ui, "Clear…"),
                        ))
                        .child(data_row("Clear the download list", confirmed(ui, "download list".to_owned(), "Clear", move || act(ui, |b| b.clear_downloads()))))
                        .child(data_row("Clear cookies and website data", confirmed(ui, "data".to_owned(), "Clear", clear_data)))
                        .child(data_row("Delete everything in this profile", confirmed(ui, "delete".to_owned(), "Delete", move || delete_profile_data(ui)))),
                )
                .child(note(
                    "Delete everything removes spaces, tabs, history, cookies, site data and site settings of this profile. Cookies and site data shared with another profile are deleted there too. Files you downloaded stay in Downloads.",
                ))
        }
        Pane::Profiles => body
            .child(view().flex_row().gap_2().pt_3().child(pill("Memory", move || super::show(ui, Panel::Memory))).child(accent_pill(
                "Open Private Space",
                move || {
                    ui.panel.set(Panel::None);
                    act(ui, |b| b.private_space());
                },
            )))
            .child(super::profiles::profiles_pane(ui))
            .child(section("Space Rules"))
            .child(rules(ui)),
        Pane::Permissions => body
            .child(note("This profile's answers to sites. A site you forget asks again next time; Lockdown sites never ask."))
            .child(view().pt_2().child(permissions(ui))),
        Pane::Shortcuts => body.child(view().pt_3().child(shortcut_rows(ui))),
        Pane::Developer => super::devtools::pane(ui, body),
    })
}

fn options(names: &[&str]) -> Vec<(usize, String)> {
    names.iter().map(|n| n.to_string()).enumerate().collect()
}

fn set_tab_choice(ui: Ui, set: impl FnOnce(&mut Browser) -> bool) {
    let mut restart = false;
    act(ui, |b| restart = set(b));
    let windows = super::WINDOWS.with_borrow(Vec::clone);
    let count = windows.len();
    for window in windows {
        act(window, |b| b.apply_tab_choices(restart, count));
    }
}

fn link_pill(ui: Ui, name: &'static str, url: &'static str) -> impl IntoElement {
    pill(name, move || {
        ui.panel.set(Panel::None);
        act(ui, |b| {
            b.new_tab(url);
        });
    })
}
