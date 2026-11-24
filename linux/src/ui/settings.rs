use browser_core::shield_card::{Card, H, SHORT, W, back, day_totals, face, front, front_svg, thousands};
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;
use shield::{Category, ShieldLevel};
use tarn_nsh_shared::controls::{described, empty, list_row, row_action, section_with};
use tarn_nsh_shared::panes::{self, GROUPS, Pane};

use super::panels::{capitalised, levels};
use super::tools::note;
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
use browser_core::prefs::{CLOSE_SELECTS_PREVIOUS, GESTURES, NEW_TAB_START_PAGE, TEAR_OFF_TEMPORARY};
use browser_core::settings::{self as prefs, LIVE_TABS, LIVE_TABS_KEY, SEARCH_SHORTCUTS, SLEEP_AFTER_DEFAULT, SLEEP_AFTER_KEY, SUGGESTIONS};

const NAV_WIDTH: f32 = 180.;
const CARD_SCALE: u32 = 3;

#[derive(Clone, Copy)]
struct Notes(Ui);

impl tarn_nsh_shared::snippets::Host for Notes {
    fn profile(self) -> Option<store::Id> {
        read(self.0, |b| b.model.profile().filter(|(private, _)| !private).map(|(_, id)| id)).flatten()
    }
    fn with_store<R>(self, f: impl FnOnce(&store::Store) -> R) -> R {
        f(&super::core().disk)
    }
    fn changed(self) {
        act(self.0, |b| b.snippets_changed());
    }
}

impl tarn_nsh_shared::notes::Host for Notes {
    fn profile(self) -> Option<store::Id> {
        tarn_nsh_shared::snippets::Host::profile(self)
    }
    fn with_store<R>(self, f: impl FnOnce(&store::Store) -> R) -> R {
        tarn_nsh_shared::snippets::Host::with_store(self, f)
    }
    fn ask(self, note: &store::Note) {
        super::assistant::open(self.0, false);
        self.0.chat.attach_note(super::assistant::Host(self.0), note);
    }
    fn export(self, markdown: String) {
        let ui = self.0;
        let dialog = SaveDialog::new()
            .file_name(browser_core::notes::EXPORT_NAME)
            .filter(FileFilter::new("Markdown", ["md"]))
            .directory(webview::session::downloads_dir());
        ui.root.run(|| {
            drop(spawn(async move {
                let said = match pick_save_path(dialog).await {
                    Ok(None) => return,
                    Ok(Some(path)) if std::fs::write(&path, markdown).is_ok() => "Notes exported",
                    _ => "The notes could not be exported",
                };
                act(ui, |b| b.show_toast(said, None));
            }))
        });
    }
}

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
        (Some(days), Some(since)) => browser_core::shield_card::protected_text(days, since),
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
        vec![icon_pill(icons::RELOAD, "Flip", move || flip(ui)), icon_pill(icons::PALETTE, "Change Design", move || act(ui, Browser::new_shield_design))];
    actions.extend((!private).then(|| icon_pill(icons::FILE_IMAGE, "Save as Image", move || save_card(ui, shared.clone()))));
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
                view().flex_row().items_center().gap_2().child(label(level_name).flex_1()).child(segmented(levels(), level, move |l| super::pick_level(ui, l))),
            )
            .child(super::lockdown_confirm(ui))
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
    let mut out =
        view().flex_col().gap_2().child(section_with("Keyboard", "Click a shortcut and type keys with Ctrl, such as Ctrl+Shift+Y. Enter saves, Esc cancels."));
    if let Some(error) = ui.shortcut_error.get_untracked() {
        out = out.child(note(error).text_color(Theme::warning()));
    }
    for (name, rows) in shortcuts::sections() {
        let mut grp = group();
        for (row_label, action) in rows {
            let keys = map.iter().find(|(_, a)| *a == action).map(|(k, _)| k.clone()).unwrap_or_default();
            let custom = stored.contains_key(&action.setting_key());
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
                row_action(icons::UNDO_2, format!("Reset {row_label}"), move || {
                    let result = read(ui, |b| shortcuts::reset(&b.core.disk, Some(action))).unwrap_or(Ok(()));
                    ui.shortcut_error.set(result.err());
                    ui.keys_rev.update(|r| *r += 1);
                    act(ui, |_| {});
                })
            });
            grp = grp.child(group_row().child(label(row_label).flex_1()).children(reset).child(control));
        }
        out = out.child(section_with(name, "")).child(grp);
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
    if rows.is_empty() {
        return group().child(empty(icons::FOLDER, "No rules. “Always open in” in a site's settings sends its new tabs to a space.", None));
    }
    group().children(rows.into_iter().enumerate().map(|(ix, (id, host, space))| {
        list_row(ix, host.clone(), format!("Opens in {space}"))
            .child(row_action(icons::TRASH, format!("Remove the rule for {host}"), move || act(ui, |b| b.delete_space_rule(id))))
            .into_element()
    }))
}

fn hidden(ui: Ui, private: bool) -> View {
    if private {
        return group().child(empty(icons::LOCK, "A private space keeps no hidden elements.", None));
    }
    let zaps = read(ui, |b| b.zapped(None)).unwrap_or_default();
    if zaps.is_empty() {
        return group().child(empty(icons::ZAP, "No hidden elements. Zap Element in the command bar hides one.", None));
    }
    group().children(zaps.into_iter().enumerate().map(|(ix, (host, selector))| {
        let (h, s) = (host.clone(), selector.clone());
        list_row(ix, engine::view_policy::title(&host), selector.clone())
            .child(row_action(icons::UNDO_2, format!("Show {selector} on {host} again"), move || act(ui, |b| b.unzap(&h, Some(&s)))))
            .into_element()
    }))
}

fn site_list(ui: Ui, hosts: Vec<String>, empty_line: &'static str, glyph: &'static str, button: &'static str, run: fn(&mut Browser, &str)) -> View {
    if hosts.is_empty() {
        return group().child(empty(icons::CIRCLE_CHECK, empty_line, None));
    }
    group().children(hosts.into_iter().enumerate().map(|(ix, host)| {
        let shown = browser_core::text::bounded_title(&host);
        list_row(ix, shown.clone(), "")
            .child(row_action(glyph, format!("{button} {shown}"), move || {
                let host = host.clone();
                act(ui, move |b| run(b, &host))
            }))
            .into_element()
    }))
}

fn permissions(ui: Ui) -> View {
    let rows = read(ui, |b| b.permissions()).unwrap_or_default();
    if rows.is_empty() {
        return group().child(empty(icons::CAMERA, "No site has asked yet. Answers you remember show here.", None));
    }
    group().children(rows.into_iter().enumerate().map(|(ix, p)| {
        let (origin, kind) = (p.origin.clone(), p.kind.clone());
        let set = move |allow: Option<bool>| {
            let (origin, kind) = (origin.clone(), kind.clone());
            act(ui, move |b| b.set_permission(&origin, &kind, allow))
        };
        let forget = set.clone();
        list_row(ix, p.site.clone(), p.label.clone())
            .child(segmented(vec![(true, "Allow".to_owned()), (false, "Block".to_owned())], p.allowed, move |allow| set(Some(allow))))
            .child(row_action(icons::TRASH, format!("Forget {} for {}", p.label, p.site), move || forget(None)))
            .into_element()
    }))
}

fn overrides(ui: Ui) -> View {
    let rows = read(ui, Browser::site_overrides).unwrap_or_default();
    if rows.is_empty() {
        return group().child(empty(icons::SHIELD, "No site overrides. Set one with the Shield button in the address bar.", None));
    }
    group().children(rows.into_iter().enumerate().map(|(ix, (host, level))| {
        let name = engine::view_policy::title(&host);
        list_row(ix, name.clone(), capitalised(level.as_str()))
            .child(row_action(icons::TRASH, format!("Remove the override for {name}"), move || remove_override(ui, host.clone())))
            .into_element()
    }))
}

fn remove_override(ui: Ui, host: String) {
    let Some(Some((private, profile))) = read(ui, |b| b.model.profile()) else { return };
    act(ui, |b| b.remove_site_override(&host));
    in_other_windows(ui, move |b| b.reload_level(private, profile, &host));
}

fn zooms(ui: Ui, rows: Vec<(String, f64)>) -> View {
    if rows.is_empty() {
        return group().child(empty(icons::ZOOM_IN, "No saved zoom levels. Zooming remembers the zoom for the site; Actual Size forgets it.", None));
    }
    group().children(rows.into_iter().enumerate().map(|(ix, (host, zoom))| {
        let name = engine::view_policy::title(&host);
        list_row(ix, name.clone(), format!("{:.0} %", zoom * 100.))
            .child(row_action(icons::TRASH, format!("Forget the zoom of {name}"), move || remove_zoom(ui, Some(host.clone()))))
            .into_element()
    }))
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
    super::forget_chats();
    for window in WINDOWS.with_borrow(Vec::clone) {
        window.history.set(Default::default());
    }
    if !wipes.is_empty() {
        run_on_main_thread(move || wipes.into_iter().for_each(webview::session::wipe));
    }
}

fn data_row(name: &str, detail: &str, action: AnyElement) -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_3()
        .px_3()
        .py_2()
        .min_h(px(40.))
        .border_b_1()
        .border_color(Theme::separator())
        .child(view().flex_1().min_w(px(200.)).child(label(name.to_owned())).when(!detail.is_empty(), |v| v.child(small(detail.to_owned()))))
        .child(action)
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
        out = out
            .child(section_with("Default browser", ""))
            .child(group().child(group_row().child(text).child(accent_pill("Make Default", move || make_default(ui)))));
    }
    let change = pill("Change…", move || {
        ui.settings_pane.set(Pane::Privacy);
        act(ui, |_| {});
    });
    out.child(section_with("Shield", "")).child(group().child(list_row(0, "Site overrides", overrides).child(change)))
}

fn make_default(ui: Ui) {
    let made = crate::desktop::make_default_browser();
    act(ui, |b| {
        if let Err(e) = made {
            b.notice = Some(Notice::Message(format!("Tarn could not be made the default browser ({e}).")));
        }
    });
}

fn nav_row(name: &'static str, glyph: &'static str, selected: bool, open: impl Fn() + 'static) -> nsh_base::ButtonElement {
    let tint = if selected { Theme::text() } else { Theme::muted() };
    crate::controls::row(name, open)
        .h(px(30.))
        .px(px(10.))
        .rounded(px(Theme::radius_row()))
        .bg(if selected { Theme::selected() } else { rgba(0) })
        .hover(|s| s.bg(Theme::hover()))
        .text_color(tint)
        .child(crate::icons::icon(glyph, 14.))
        .child(label(name).text_color(tint))
}

fn open_pane(ui: Ui, pane: Option<Pane>) {
    ui.ext_pane.set(pane.is_none());
    if pane == Some(Pane::Developer) {
        super::devtools::refresh_servers(ui);
    }
    if let Some(pane) = pane {
        ui.settings_pane.set(pane);
    }
    act(ui, |_| {});
}

/// The panes Settings search shows for `query`; `None` is Extensions. AI shows only where
/// `assistant::offered`.
fn found(ui: Ui, query: &str) -> Vec<Option<Pane>> {
    let ai = tarn_nsh_shared::assistant::offered(super::assistant::Host(ui));
    let (name, words) = panes::EXTENSIONS;
    let extensions = browser_core::settings::words_match(query, &format!("{name} {words}")).then_some(None);
    GROUPS
        .iter()
        .flat_map(|(_, members)| members.iter().copied())
        .filter(|p| (ai || *p != Pane::Ai) && panes::matches(*p, query))
        .map(Some)
        .chain(extensions)
        .collect()
}

fn nav(ui: Ui, current: Pane) -> View {
    let ext_on = ui.ext_pane.get_untracked();
    let shown = move |pane: Option<Pane>| if ext_on { pane.is_none() } else { pane == Some(current) };
    let field = super::text_field(ui.library_query, "Search", NAV_WIDTH - 16.).on_submit(move |q: &str| {
        let found = found(ui, q);
        if let Some(pane) = found.iter().find(|p| shown(**p)).or(found.first()) {
            open_pane(ui, *pane);
        }
    });
    let rows = dynamic(move || {
        let query = ui.library_query.get();
        let found = found(ui, &query);
        let mut out = view().flex_col().gap(px(2.));
        if found.is_empty() {
            return Some(out.child(note("No settings match.").px_1()));
        }
        for (group, members) in GROUPS {
            if query.trim().is_empty() {
                out = out.child(view().pt_2().pb_1().px_1().child(tarn_nsh_shared::controls::caps_label(group)));
            }
            for pane in members.iter().copied().filter(|p| found.contains(&Some(*p))) {
                out = out.child(nav_row(panes::name(pane), panes::icon(pane), shown(Some(pane)), move || open_pane(ui, Some(pane))));
            }
        }
        if found.contains(&None) {
            out = out.child(nav_row(panes::EXTENSIONS.0, crate::icons::SPARKLES, ext_on, move || open_pane(ui, None)));
        }
        Some(out)
    });
    view().flex_col().gap(px(2.)).w(px(NAV_WIDTH)).flex_none().p_2().rounded(px(Theme::radius_group())).bg(Theme::panel_nav()).child(field).child(rows)
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
    let (suggestions, shortcuts, live_tabs, sleep_after) = read(ui, |b| {
        let disk = &b.core.disk;
        let sleep_options = prefs::sleep_after_options();
        (
            b.suggestions(),
            b.search_shortcuts(),
            prefs::choice(disk, LIVE_TABS_KEY, &LIVE_TABS, prefs::LIVE_TABS_DEFAULT),
            prefs::choice(disk, SLEEP_AFTER_KEY, &sleep_options, SLEEP_AFTER_DEFAULT),
        )
    })?;
    let (start_page, previous, gestures, tear_off) =
        read(ui, |b| (b.pref(NEW_TAB_START_PAGE), b.pref(CLOSE_SELECTS_PREVIOUS), b.pref(GESTURES), b.pref(TEAR_OFF_TEMPORARY)))?;
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
            .child(section_with("Downloads", ""))
            .child(group().child(described(
                switch(sort, "Sort downloads into folders", toggle_off(SORT_DOWNLOADS)),
                "Images, Videos, Audio, Documents, Archives and Installers inside Downloads.",
            )))
            .children(browser_core::import::available().then(|| section_with("Import", "")))
            .children(browser_core::import::available().then(|| {
                group().child(
                    list_row(0, "Import browser data", "Sign-ins, history and bookmarks from Chrome, Edge, Brave, Chromium or Firefox")
                        .child(pill("Import\u{2026}", move || super::import::open(ui))),
                )
            }))
            .child(section_with("About", ""))
            .child(group().child(list_row(0, "Tarn version", browser_core::VERSION)))
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
            let choice =
                |name: &str, note: &str, control: View| described(setting_row(name, control), note.to_owned()).border_b_1().border_color(Theme::separator());
            body.child(section_with("Memory", "Sleeping tabs reload when you return; pinned tabs and tabs playing media stay awake."))
                .child(
                    group()
                        .child(choice(
                            "Tabs kept awake",
                            "Fewer saves memory.",
                            segmented(options(&LIVE_TABS), live_tabs, move |ix| {
                                set_tab_choice(ui, |b| {
                                    b.set_live_tabs(ix);
                                    false
                                })
                            }),
                        ))
                        .child(choice(
                            "Sleep background tabs after",
                            "After five minutes while memory is short.",
                            segmented(options(&prefs::sleep_after_options()), sleep_after, move |ix| set_tab_choice(ui, |b| b.set_sleep_after(ix))),
                        ))
                        .child(choice(
                            "Close unused tabs after",
                            "Unpinned tabs move to History › Archived Tabs.",
                            segmented(choices, archive, move |ix| act(ui, move |b| b.set_archive_after(ix))),
                        )),
                )
                .child(section_with("Opening and closing", ""))
                .child(
                    group()
                        .child(choice(
                            "New tab opens",
                            "The command card with the search field, or the start page.",
                            segmented(vec![(false, "Command card".to_owned()), (true, "Start page".to_owned())], start_page, move |on| {
                                act(ui, move |b| b.set_pref(NEW_TAB_START_PAGE, on))
                            }),
                        ))
                        .child(choice(
                            "After closing a tab",
                            "Which neighbour becomes active.",
                            segmented(vec![(false, "Next".to_owned()), (true, "Previous".to_owned())], previous, move |on| {
                                act(ui, move |b| b.set_pref(CLOSE_SELECTS_PREVIOUS, on))
                            }),
                        ))
                        .child(choice(
                            "A tab dragged out of the window opens",
                            "A temporary window lists only its own temporary space and goes with the window. Pinned tabs and tiles open in a full window, except from a temporary one.",
                            segmented(vec![(true, "Temporary window".to_owned()), (false, "Full window".to_owned())], tear_off, move |on| {
                                act(ui, move |b| b.set_pref(TEAR_OFF_TEMPORARY, on))
                            }),
                        ))
                        .child(described(
                            switch(gestures, "Touchpad gestures", move || act(ui, |b| b.set_pref(GESTURES, !b.pref(GESTURES)))),
                            "Swipe with two fingers over the sidebar to switch spaces.",
                        )),
                )
        }
        Pane::Search => {
            let engines: Vec<(usize, String)> = ENGINES.iter().enumerate().map(|(ix, e)| (ix, e.name.to_owned())).collect();
            body.child(section_with("Search engine", ""))
                .child(group().child(setting_row("Search engine", segmented(engines, engine, move |ix| act(ui, move |b| b.set_search_engine(ix))))))
                .child(section_with("Suggestions", "Nothing you type leaves this computer before you press Enter."))
                .child(
                    group()
                        .child(described(switch(suggestions, "Suggest open tabs and history", toggle(SUGGESTIONS)), "Shown under the address field while you type."))
                        .child(described(
                            switch(history_text, "Search the text of visited pages", toggle_off(HISTORY_TEXT)),
                            "Keeps up to 4 KB of each page's text on this computer. Never for the private space or Lockdown pages; cleared with history and when you turn this off.",
                        )),
                )
                .child(section_with("Shortcuts", ""))
                .child(group().child(described(switch(shortcuts, "Search shortcuts", toggle_off(SEARCH_SHORTCUTS)), prefs::SEARCH_SHORTCUTS_NOTE)))
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
            let filter = |on: bool, name: &str, note: &str, toggle: Box<dyn Fn()>| described(switch(on, name, toggle), note.to_owned());
            body.child(section_with("Shield", "Filter lists apply to pages loaded after a change; they need the signed list bundle."))
                .child(
                    group()
                        .child(switch(on("ads"), "Block ads", toggle("block_ads")))
                        .child(switch(on("trackers"), "Block trackers", toggle("block_trackers")))
                        .child(filter(
                            on("annoyances"),
                            "Block cookie banners",
                            "EasyList Cookie and uBlock Cookie Notices hide consent pop-ups. Nothing is accepted or refused for you.",
                            Box::new(toggle("block_annoyances")),
                        ))
                        .child(filter(
                            youtube,
                            "Block YouTube ads",
                            "Removes ads from YouTube's player data and hides ad slots. If videos stop playing, turn this off.",
                            Box::new(toggle(shield::groups::YOUTUBE_SETTING)),
                        ))
                        .child(filter(
                            sets.contains(&Category::Popups),
                            "Block pop-ups and ad redirects",
                            "Pages open windows only when you click, never to known pop-up ad sites, and cannot send you to one.",
                            Box::new(toggle_set(Category::Popups)),
                        ))
                        .child(filter(
                            sets.contains(&Category::Malware),
                            "Block dangerous sites",
                            "Malware, phishing and scam sites get a warning page.",
                            Box::new(toggle_set(Category::Malware)),
                        ))
                        .child(filter(
                            sets.contains(&Category::Adult),
                            "Block adult content",
                            "Adult sites get a warning page with no way past it.",
                            Box::new(toggle_set(Category::Adult)),
                        )),
                )
                .child(section_with("Filter-list updates", format!("GitHub, once a day. {filter_status}")))
                .children(filter_updates.map(|on| group().child(switch(on, "Download filter-list updates", toggle(shield::update::SETTING)))))
                .child(section_with("Links from other apps", "Applies to every profile."))
                .child(group().child(described(
                    switch(localhost, "Open localhost links from other apps", toggle(LOCALHOST_FROM_APPS)),
                    "A link to this computer (localhost, 127.0.0.1) from a terminal, editor or other app opens like a typed address, and its page may lead elsewhere on your network as a typed one can. Off: Tarn refuses it, since a link in a mail could reach services running here. Links to other addresses on your network are refused. Open Anyway on the notice still opens a refused link.",
                )))
                .child(section_with("Site overrides", "Shield levels you set for single sites."))
                .child(overrides(ui))
                .child(section_with("Muted sites", ""))
                .child(site_list(ui, muted, "No muted sites. Mute Site in a tab's panel or the command bar silences one.", icons::VOLUME_2, "Unmute", |b, host| b.unmute_site(host)))
                .child(section_with("Sites without ad, tracker and cookie-banner blocking", ""))
                .child(site_list(
                    ui,
                    paused,
                    "Blocking runs on every site. Turn it off for one with the Shield button in the address bar.",
                    icons::SHIELD_CHECK,
                    "Turn blocking on for",
                    |b, host| b.set_site_paused(host, false),
                ))
                .child(
                    view()
                        .flex_row()
                        .items_end()
                        .child(section_with("Site zoom levels", "").flex_1())
                        .children(has_zooms.then(|| confirmed(ui, "zooms".to_owned(), "Reset All", move || remove_zoom(ui, None)))),
                )
                .child(zooms(ui, zoom_rows))
                .child(section_with("Hidden elements", ""))
                .child(hidden(ui, private))
                .child(section_with("Your data", "Files you downloaded stay in Downloads."))
                .child(
                    group()
                        .children((!private).then(|| data_row("Export this profile's data", "As JSON", pill("Export…", move || export_profile(ui)).into_element())))
                        .child(data_row(
                            "Clear browsing history",
                            "Also site icons, zoom levels, protected sites and hidden elements",
                            super::clear_history_choice(ui, "Clear…"),
                        ))
                        .child(data_row("Clear the download list", "", confirmed(ui, "download list".to_owned(), "Clear", move || act(ui, |b| b.clear_downloads()))))
                        .child(data_row("Clear cookies and website data", "", confirmed(ui, "data".to_owned(), "Clear", clear_data)))
                        .child(data_row(
                            "Delete everything in this profile",
                            "Spaces, tabs, history, cookies, site data and site settings. Cookies and site data shared with another profile are deleted there too.",
                            confirmed(ui, "delete".to_owned(), "Delete", move || delete_profile_data(ui)),
                        )),
                )
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
            .child(section_with("Space Rules", "Sites whose new tabs open in a space you picked."))
            .child(rules(ui)),
        Pane::Permissions => {
            body.child(section_with("Site answers", "A site you forget asks again next time; Lockdown sites never ask.")).child(permissions(ui))
        }
        Pane::Shortcuts => body.child(view().pt_3().child(shortcut_rows(ui))),
        Pane::Snippets => body.child(view().pt_3().child(tarn_nsh_shared::snippets::pane(Notes(ui)))),
        Pane::Notes => body.child(view().pt_3().child(tarn_nsh_shared::notes::pane(Notes(ui)))),
        Pane::Ai => body.child(view().pt_3().child(tarn_nsh_shared::assistant::settings_pane(super::assistant::Host(ui), ui.chat))),
        Pane::Developer => super::devtools::pane(ui, body),
    })
}

fn setting_row(name: &str, control: View) -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x_3()
        .gap_y_1()
        .px_3()
        .py_2()
        .min_h(px(40.))
        .child(label(name.to_owned()).flex_1().min_w(px(160.)))
        .child(control)
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
