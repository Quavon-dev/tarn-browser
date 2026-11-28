use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use browser_core::effects::Effect;
use browser_core::github::Hop;
use browser_core::prefs::{self, ACCENTS, COMPACT, FEATURE_HINTS, REDUCE_MOTION, THEME_ACCENT};
use browser_core::theme::{self, Backing, Corners, Origin, Scheme, Selection, Source, ThemeRef};
use browser_core::theme_fetch::{self as fetch, Pending, Release};
use browser_core::theme_library::{self as library, Card};
use nsh_base::input;
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::picker::FilePickRequest;
use nsh_core::prelude::*;
use store::{Id, Store};

use crate::controls::{Segment, accent_pill, caps_label, confirm, group, label, pill, section, secure_button, segmented, switch_row};
use crate::theme::{SYSTEM_FONT, Secure, TEXT_SMALL, Theme};

pub trait Host: Copy + 'static {
    type Transport: Hop;
    fn transport() -> Self::Transport;
    fn catalog_dir(self) -> PathBuf;
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R;
    fn changed(self);
    fn profile(self) -> Option<(Id, String)>;
    fn safe(self) -> bool;
    fn notice(self) -> Option<String>;
    fn system_light(self) -> bool;
    fn themes_dir(self) -> PathBuf;
    fn desktop(self) -> bool;
    fn accent_and_motion(self) -> bool {
        false
    }
    fn backing(self) -> Backing {
        Backing::Opaque
    }
}

#[derive(Clone, Default)]
struct State {
    message: Option<(bool, String)>,
    focused: Option<ThemeRef>,
    colors: Option<Id>,
    watch: Option<(Id, PathBuf, Option<SystemTime>)>,
    busy: bool,
    pending: Option<Pending>,
    catalog: Option<shield::catalog::Index>,
}

#[derive(Clone, Copy)]
struct Pane<H: Host> {
    host: H,
    state: Signal<State>,
    repo: Signal<Repo>,
}

#[derive(Clone, Default)]
struct Repo(String);

fn detached(future: impl Future<Output = ()> + 'static) {
    Owner::detached().run(|| drop(spawn(future)));
}

impl<H: Host> Pane<H> {
    fn get<T>(self, f: impl FnOnce(&State) -> T) -> T {
        self.state.with(f)
    }

    fn set(self, f: impl FnOnce(&mut State)) {
        self.state.update(f);
    }

    fn say(self, message: Option<(bool, String)>) {
        self.set(|s| s.message = message);
    }

    fn report(self, result: Result<(), String>, done: &str) {
        self.say(match result {
            Ok(()) if done.is_empty() => None,
            Ok(()) => Some((false, done.to_owned())),
            Err(e) => Some((true, e)),
        });
        self.host.changed();
    }

    fn quietly(self, result: Result<(), String>) {
        self.report(result, "");
    }

    fn add(self, css: Result<String, String>) {
        let added = css.and_then(|css| self.host.with_store(|s| library::add(s, &css, browser_core::now())));
        match added {
            Ok(id) => {
                self.set(|s| s.focused = Some(ThemeRef::Stored(id)));
                self.report(Ok(()), "Added. Its website styles stay off until you turn them on below.");
            }
            Err(e) => self.report(Err(e), ""),
        }
    }

    fn import_file(self) {
        let request = FilePickRequest::new(false, 1).filter(FileFilter::new("Theme", ["css"]));
        detached(async move {
            if let Some(file) = pick_files(request).await.into_iter().next() {
                self.add(library::read_bytes(&file.name, file.data.bytes().to_vec()));
            }
        });
    }

    fn paste(self) {
        detached(async move {
            let text = clipboard_string().await;
            self.add(Some(text).filter(|t| !t.trim().is_empty()).ok_or_else(|| "The clipboard holds no text.".to_owned()));
        });
    }

    fn duplicate(self, id: ThemeRef) {
        match self.host.with_store(|s| library::duplicate(s, &id, browser_core::now())) {
            Ok(copy) => {
                self.set(|s| {
                    s.focused = Some(ThemeRef::Stored(copy));
                    s.colors = Some(copy);
                });
                let done = if self.host.desktop() { "Copied. Edit its colours below or its CSS in a text editor." } else { "Copied. Edit its colours below." };
                self.report(Ok(()), done);
            }
            Err(e) => self.report(Err(e), ""),
        }
    }

    fn remove(self, id: Id) {
        self.set(|s| {
            s.focused = None;
            s.colors = s.colors.filter(|c| *c != id);
            s.watch = s.watch.take().filter(|(w, _, _)| *w != id);
            s.pending = s.pending.take().filter(|p| p.existing != Some(id));
        });
        std::fs::remove_file(self.host.themes_dir().join(format!("{id}.css"))).ok();
        let removed = self.host.with_store(|s| library::remove(s, id));
        self.report(removed, "Removed.");
    }

    fn edit_in_text_editor(self, id: Id) {
        let Some(css) = self.host.with_store(|s| s.theme(id).ok().flatten().map(|t| t.css)) else { return };
        let dir = self.host.themes_dir();
        let path = dir.join(format!("{id}.css"));
        if let Err(e) = private_dir(&dir).and_then(|()| std::fs::write(&path, css)) {
            return self.say(Some((true, format!("The theme could not be opened in a text editor: {e}"))));
        }
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        self.set(|s| {
            s.watch = Some((id, path.clone(), modified));
            s.message = Some((false, "Saved changes in the text editor show here while Settings is open.".to_owned()));
        });
        detached(async move {
            if let Err(e) = open_path(path).await {
                self.say(Some((true, format!("The theme could not be opened in a text editor: {e}"))));
            }
        });
    }

    fn reload_watched(self) {
        let Some((id, path, seen)) = self.state.with_untracked(|s| s.watch.clone()) else { return };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if modified == seen {
            return;
        }
        self.set(|s| s.watch = Some((id, path.clone(), modified)));
        let saved = library::read_file(&path)
            .and_then(|css| self.host.with_store(|s| theme::save(s, id, "yours", &css, Origin::default(), browser_core::now()).map(drop)));
        self.report(saved, "Reloaded from the text editor.");
    }

    fn save_file(self, id: ThemeRef, name: String) {
        let css = match self.host.with_store(|s| library::exportable(s, &id)) {
            Ok(css) => css,
            Err(e) => return self.say(Some((true, e))),
        };
        let file: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        let mut dialog = SaveDialog::new().file_name(format!("{}.css", file.trim())).filter(FileFilter::new("Theme", ["css"]));
        if let Some(downloads) = std::env::home_dir().map(|h| h.join("Downloads")) {
            dialog = dialog.directory(downloads);
        }
        detached(async move {
            let Ok(Some(path)) = pick_save_path(dialog).await else { return };
            self.say(Some(match std::fs::write(&path, css) {
                Ok(()) => (false, "Saved.".to_owned()),
                Err(e) => (true, format!("The theme could not be saved: {e}")),
            }));
        });
    }
}

impl<H: Host> Pane<H> {
    fn begin(self) -> bool {
        if self.state.with_untracked(|s| s.busy) {
            return false;
        }
        if !self.host.with_store(fetch::downloads_on) {
            self.say(Some((true, "Theme and extension downloads are off in Privacy settings.".to_owned())));
            return false;
        }
        self.set(|s| {
            s.busy = true;
            s.message = Some((false, "Asking GitHub\u{2026}".to_owned()));
        });
        true
    }

    async fn off_thread<T: Send + 'static>(work: impl FnOnce(&H::Transport) -> Result<T, String> + Send + 'static) -> Result<T, String> {
        blocking(move || work(&H::transport())).await.unwrap_or_else(|e| Err(e.to_string()))
    }

    fn fetch_github(self, named: Option<String>, existing: Option<(Id, u64)>) {
        if !self.begin() {
            return;
        }
        self.set(|s| s.pending = None);
        detached(async move {
            let result = Self::off_thread(move |net| match (named, existing) {
                (Some(input), _) => fetch::fetch_theme(net, Release::Named(&input)),
                (None, Some((_, repo_id))) => fetch::fetch_theme(net, Release::Newest(repo_id)),
                (None, None) => Err("Enter a repository.".to_owned()),
            })
            .await;
            let reviewed = self.host.with_store(|s| fetch::pending(s, result, existing.map(|(id, _)| id)));
            self.set(|s| {
                s.busy = false;
                match reviewed {
                    Ok(pending) => {
                        s.pending = Some(pending);
                        s.message = None;
                    }
                    Err(message) => s.message = Some(message),
                }
            });
        });
    }

    fn installed(self, saved: Result<(Id, Option<Scheme>), String>, done: &str) {
        let selected = saved.and_then(|(id, scheme)| match scheme {
            Some(scheme) => {
                self.set(|s| s.focused = Some(ThemeRef::Stored(id)));
                self.host.with_store(|s| library::select(s, ThemeRef::Stored(id), scheme)).map(|()| done)
            }
            None => Ok("Updated."),
        });
        match selected {
            Ok(message) => self.report(Ok(()), message),
            Err(e) => self.report(Err(e), ""),
        }
    }

    fn confirm(self) {
        let Some(pending) = self.state.with_untracked(|s| s.pending.clone()) else { return };
        self.set(|s| s.pending = None);
        let saved = self.host.with_store(|s| fetch::confirm(s, &pending, browser_core::now()));
        self.installed(saved, "Installed. Its website styles stay off until you turn them on below.");
    }

    fn show_official(self) {
        if !self.begin() {
            return;
        }
        let (last, dir) = (self.host.with_store(fetch::last_catalog), self.host.catalog_dir());
        let now = (browser_core::now() / 1000) as u64;
        detached(async move {
            let result = Self::off_thread(move |net| fetch::fetch_catalog(net, &dir, last, now)).await;
            self.set(|s| s.busy = false);
            match result {
                Ok(index) => {
                    let grew = self.host.with_store(|s| fetch::catalog_fetched(s, &index));
                    self.set(|s| {
                        s.catalog = Some(index);
                        s.message = None;
                    });
                    if grew {
                        self.host.changed();
                    }
                }
                Err(e) => self.say(Some((true, e))),
            }
        });
    }

    fn install_official(self, entry: shield::catalog::ThemeEntry, existing: Option<Id>) {
        if !self.begin() {
            return;
        }
        detached(async move {
            let fetched = entry.clone();
            let result = Self::off_thread(move |net| fetch::fetch_official(net, &fetched)).await;
            self.set(|s| s.busy = false);
            let saved = self.host.with_store(|s| fetch::save_official(s, &entry, existing, result, browser_core::now()));
            self.installed(saved, "Installed.");
        });
    }
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn in_use(card: &Card, selection: &Selection) -> Vec<&'static str> {
    [(selection.dark == card.id, if selection.match_system { "Dark" } else { "In use" }), (selection.match_system && selection.light == card.id, "Light")]
        .into_iter()
        .filter_map(|(on, label)| (on && !card.withdrawn).then_some(label))
        .collect()
}

fn small(content: impl Into<SharedString>) -> Text {
    text(content.into()).text_size(px(TEXT_SMALL)).text_color(Theme::muted())
}

fn badge(content: String) -> Text {
    text(content).text_size(px(TEXT_SMALL)).font_family(SYSTEM_FONT).text_color(Secure::muted()).truncate()
}

fn note(content: impl Into<SharedString>, error: bool) -> Text {
    text(content.into()).text_size(px(TEXT_SMALL)).text_color(if error { Secure::warning() } else { Theme::muted() })
}

fn row() -> View {
    view().flex_row().flex_wrap().items_center().gap_2().px_3().py_2()
}

fn swatch(color: u32) -> View {
    view().w(px(14.)).h(px(14.)).rounded_full().bg(rgba(color)).border_1().border_color(Theme::hairline())
}

fn card_view<H: Host>(pane: Pane<H>, card: &Card, selection: &Selection, focused: bool) -> AnyElement {
    let tags = in_use(card, selection).join(" · ");
    let (id, scheme, withdrawn) = (card.id.clone(), card.scheme, card.withdrawn);
    let spoken = format!("{}{}", card.name, if tags.is_empty() { String::new() } else { format!(", {tags}") });
    crate::controls::pressable(spoken, move || {
        pane.set(|s| s.focused = Some(id.clone()));
        if withdrawn {
            return pane.say(Some((true, "This theme was withdrawn as unsafe and cannot be used. Remove it below.".to_owned())));
        }
        let saved = pane.host.with_store(|s| library::select(s, id.clone(), scheme));
        pane.quietly(saved);
    })
    .flex_col()
    .items_start()
    .gap_1()
    .w(px(168.))
    .p_2()
    .rounded(px(Theme::radius_group()))
    .bg(Theme::panel_row())
    .border_1()
    .border_color(if focused { Theme::accent_text() } else { Theme::surface_edge() })
    .child(view().flex_row().gap_1().children(card.swatches.map(|c| swatch(c).into_element())))
    .child(label(card.name.clone()).truncate())
    .child(badge(card.badge.clone()))
    .child(small(if tags.is_empty() { if scheme == Scheme::Light { "Light" } else { "Dark" }.to_owned() } else { tags }).text_color(Theme::accent_text()))
    .into_element()
}

fn details<H: Host>(pane: Pane<H>, card: &Card) -> View {
    let mut out = group().gap_1().py_1();
    if let Some(stored) = card.stored
        && !card.hosts.is_empty()
    {
        let on = card.page_css;
        let detail = if card.unverified {
            format!("Styles for {}. Turn them on only for a theme you trust.", card.hosts.join(", "))
        } else {
            format!("Styles for {}.", card.hosts.join(", "))
        };
        out = out
            .child(switch_row("Styles for websites", move || on, move || pane.quietly(pane.host.with_store(|s| library::set_page_css(s, stored, !on)))))
            .child(view().px_3().pb_2().child(badge(library::shown(&detail))));
    }
    for line in card.warnings.iter().chain(&card.adjusted) {
        out = out.child(view().px_3().child(small(library::shown(line))));
    }
    let id = card.id.clone();
    let mut buttons = row().child(pill(if card.editable { "Duplicate" } else { "Duplicate and Edit" }, move || pane.duplicate(id.clone())));
    if let (Some(stored), true) = (card.stored, card.editable) {
        buttons = buttons.child(pill("Edit Colours", move || pane.set(|s| s.colors = Some(stored))));
        if pane.host.desktop() {
            buttons = buttons.child(pill("Edit in Text Editor", move || pane.edit_in_text_editor(stored)));
        }
    }
    if pane.host.desktop() && !card.withdrawn {
        let (id, name) = (card.id.clone(), card.name.clone());
        buttons = buttons.child(pill("Save as File", move || pane.save_file(id.clone(), name.clone())));
    }
    if let (Some(stored), Some(repo_id)) = (card.stored, card.repo_id) {
        buttons = buttons.child(pill("Check for Updates", move || pane.fetch_github(None, Some((stored, repo_id)))));
    }
    if let Some(stored) = card.stored {
        let question = format!("Remove the theme \u{201C}{}\u{201D}?", browser_core::text::bounded_title(&card.name));
        buttons = buttons.child(pill("Remove\u{2026}", move || confirm(question.clone(), "Its file is deleted.", "Remove", move || pane.remove(stored))));
    }
    out.child(buttons)
}

fn color_editor<H: Host>(pane: Pane<H>, id: Id) -> Option<View> {
    let css = pane.host.with_store(|s| s.theme(id).ok().flatten().filter(|t| t.source == "yours").map(|t| t.css))?;
    let parsed = theme::parse(&css, Source::Yours).ok()?;
    let rows: Vec<AnyElement> = theme::editable_colors(&parsed.tokens)
        .into_iter()
        .map(|(name, color)| {
            let value = signal(library::hex(color));
            let field = input()
                .value(move || value.get())
                .on_change(move |v: &str| value.set(v.to_owned()))
                .on_submit(move |v: &str| {
                    let saved = pane.host.with_store(|s| theme::set_color(s, id, name, v, browser_core::now()));
                    pane.report(saved, "");
                })
                .label(name)
                .w(px(110.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(Theme::radius_row()))
                .bg(Theme::raised())
                .border_1()
                .border_color(Theme::hairline())
                .text_color(Theme::text());
            view().flex_row().items_center().gap_2().px_3().py_1().child(swatch(color)).child(label(name).flex_1()).child(field).into_element()
        })
        .collect();
    let hint = if pane.host.desktop() { "Press Enter to apply a colour." } else { "Press Return to apply a colour." };
    Some(
        view()
            .flex_col()
            .child(section("Colours"))
            .child(group().py_1().children(rows))
            .child(view().px_1().pt_1().child(small(format!("{hint} Text that would be hard to read is made readable.")))),
    )
}

fn saved(ok: bool) -> Result<(), String> {
    if ok { Ok(()) } else { Err("Tarn could not save this setting.".to_owned()) }
}

fn accent_row<H: Host>(pane: Pane<H>) -> View {
    let host = pane.host;
    let (chosen, theme) = host.with_store(|s| (prefs::accent_choice(s), library::current(s, host.system_light(), host.safe()).parsed.tokens.accent >> 8));
    let swatches = std::iter::once((THEME_ACCENT, theme)).chain(ACCENTS).enumerate().map(|(ix, (name, color))| {
        crate::controls::tab_stop(nsh_base::button())
            .accessibility_label(format!("{name} accent"))
            .selected(ix == chosen)
            .on_press(move || pane.quietly(saved(host.with_store(|s| prefs::set_accent_choice(s, ix)))))
            .w(px(18.))
            .h(px(18.))
            .rounded_full()
            .bg(rgba(color << 8 | 0xff))
            .border_color(if ix == chosen { Theme::text() } else { Theme::hairline() })
            .when(ix == chosen, |b| b.border_2())
            .when(ix != chosen, |b| b.border_1())
            .into_element()
    });
    row()
        .child(view().flex_col().flex_1().child(label("Accent")).child(small("Buttons, selection and focus.")))
        .child(view().flex_row().flex_wrap().gap_2().children(swatches))
}

fn reduce_motion_row<H: Host>(pane: Pane<H>) -> View {
    let host = pane.host;
    let on = host.with_store(|s| REDUCE_MOTION.on(s));
    let system = if host.desktop() { "animations are off in the desktop settings" } else { "Reduce Motion is on in the system settings" };
    view()
        .flex_col()
        .pb_2()
        .child(switch_row("Reduce motion", move || on, move || pane.quietly(saved(host.with_store(|s| REDUCE_MOTION.set(s, !on))))))
        .child(view().px_3().child(small(format!("{MOTION_NOTE} Always on while {system}."))))
}

const MOTION_NOTE: &str = "Pop-ups, cards, tabs and the sidebar appear without animation, and the search bar effect holds still.";

fn effect_row<H: Host>(pane: Pane<H>) -> View {
    let stored = pane.host.with_store(library::chosen_effect);
    let choices: Vec<(&str, Option<Effect>)> = std::iter::once(("Theme", None)).chain(Effect::ALL.map(|e| (e.label(), Some(e)))).collect();
    choice_row("Search bar effect", "Around the search field on the start page.", picks(pane, stored, &choices, library::set_effect))
}

fn choice_row(title: &'static str, about: impl Into<SharedString>, choices: View) -> View {
    view().flex_col().gap_2().px_3().py_2().child(label(title)).child(small(about)).child(choices)
}

fn picks<H: Host, T: Copy + PartialEq + 'static>(pane: Pane<H>, current: T, choices: &[(&str, T)], set: fn(&Store, T) -> Result<(), String>) -> View {
    segmented(
        choices
            .iter()
            .map(|&(name, value)| -> Segment {
                (name.to_owned(), Box::new(move || value == current), Box::new(move || pane.quietly(pane.host.with_store(|s| set(s, value)))))
            })
            .collect(),
    )
}

fn design_group<H: Host>(pane: Pane<H>) -> View {
    let host = pane.host;
    let backing = host.backing();
    let (on, glass, reached) = host.with_store(|s| {
        let active = library::current(s, host.system_light(), host.safe());
        (library::glass_on(s), library::glass_settings(s), library::look(s, &active, host.safe(), backing).transparency)
    });
    let about = if backing == Backing::Opaque {
        "Glass gives rounder, softer panels."
    } else {
        "Glass makes the window translucent over your desktop, with rounder, softer panels."
    };
    let mut rows = group().child(choice_row("Design", about, picks(pane, on, &[("Classic", false), ("Glass", true)], library::set_glass_on)));
    let corners = Corners::ALL.map(|c| (c.label(), c));
    if !on {
        let picked = host.with_store(library::classic_corners);
        return rows.child(choice_row("Corners", CORNERS_NOTE, picks(pane, picked, &corners, library::set_classic_corners)));
    }
    if backing != Backing::Opaque {
        let labels = library::TRANSPARENCY_STEPS.map(|v| format!("{v} %"));
        let steps: Vec<(&str, u8)> = labels.iter().map(String::as_str).zip(library::TRANSPARENCY_STEPS).collect();
        let note = library::transparency_note(glass.transparency, reached, host.safe());
        rows = rows.child(choice_row("Transparency", note, picks(pane, glass.transparency, &steps, library::set_transparency)));
    }
    if backing == Backing::Blurred {
        let blur = glass.blur;
        rows = rows.child(switch_row("Blur behind the window", move || blur, move || pane.quietly(host.with_store(|s| library::set_blur(s, !blur)))));
    }
    rows.child(choice_row("Corners", CORNERS_NOTE, picks(pane, glass.corners, &corners, library::set_corners)))
}

const CORNERS_NOTE: &str = "How round panels, cards and tiles are. Theme keeps the theme's own.";

fn pending_card<H: Host>(pane: Pane<H>) -> Option<View> {
    let p = pane.get(|s| s.pending.clone())?;
    let action = if p.existing.is_some() { "Update" } else { "Install" };
    let lines = p.lines.iter().map(|l| text(l.clone()).text_size(px(TEXT_SMALL)).text_color(Secure::muted()).into_element());
    Some(
        view().pt_3().child(
            crate::controls::secure_surface(Theme::radius_group())
                .p_3()
                .flex_col()
                .gap_2()
                .child(text(p.title()).font_weight(FontWeight::SEMIBOLD))
                .children(lines)
                .child(view().flex_row().gap_2().child(secure_button(action, move || pane.confirm(), true)).child(secure_button(
                    "Cancel",
                    move || pane.set(|s| s.pending = None),
                    false,
                ))),
        ),
    )
}

fn official_section<H: Host>(pane: Pane<H>) -> Option<View> {
    if !fetch::catalog_configured() {
        return None;
    }
    let rows = pane.get(|s| s.catalog.clone()).map(|c| pane.host.with_store(|s| fetch::official_rows(s, &c))).unwrap_or_default();
    let list = group()
        .child(
            row()
                .child(
                    view()
                        .flex_col()
                        .flex_1()
                        .child(label("Reviewed and signed by Tarn"))
                        .child(small("Tarn asks GitHub for the list only when you press Show.")),
                )
                .child(pill("Show Official Themes", move || pane.show_official())),
        )
        .children(rows.into_iter().map(|r| {
            let (entry, existing) = (r.entry, r.existing);
            row()
                .child(view().flex_col().flex_1().child(label(r.name)).child(small(r.detail)))
                .child(match r.action {
                    Some(action) => pill(action, move || pane.install_official(entry.clone(), existing)).into_element(),
                    None => small("Installed").into_element(),
                })
                .into_element()
        }));
    Some(view().flex_col().child(section("Official Themes")).child(list))
}

fn github_row<H: Host>(pane: Pane<H>) -> View {
    let busy = pane.get(|s| s.busy);
    let install = move || {
        let input = pane.repo.with_untracked(|r| r.0.trim().to_owned());
        pane.fetch_github(Some(input), None);
    };
    let field = input()
        .value(move || pane.repo.with(|r| r.0.clone()))
        .on_change(move |v: &str| pane.repo.set(Repo(v.to_owned())))
        .on_submit(move |_: &str| install())
        .placeholder("owner/repository")
        .label("GitHub repository")
        .min_w(px(180.))
        .flex_1()
        .px(px(8.))
        .py(px(4.))
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .border_1()
        .border_color(Theme::hairline())
        .text_color(Theme::text());
    row()
        .child(
            view()
                .flex_col()
                .w_full()
                .child(label("From GitHub"))
                .child(small("The theme.css file of a repository's release. Tarn asks GitHub only when you press Install.")),
        )
        .child(field)
        .child(pill(if busy { "Asking\u{2026}" } else { "Install" }, install))
}

fn body<H: Host>(pane: Pane<H>) -> View {
    let host = pane.host;
    let selection = host.with_store(Selection::load);
    let cards = host.with_store(library::cards);
    let focused_id = pane.get(|s| s.focused.clone()).unwrap_or_else(|| selection.active(host.system_light()).clone());
    let match_system = selection.match_system;
    let system = if host.desktop() { "the desktop" } else { "the system" };
    let mut page = view()
        .flex_col()
        .child(view().pt_3().pb_2().child(caps_label("Theme")))
        .child(group().child(switch_row(
            "Match system appearance",
            move || match_system,
            move || pane.quietly(host.with_store(|s| library::set_match_system(s, !match_system))),
        )))
        .child(view().px_1().pt_1().child(small(format!("Use one theme while {system} is light and another while it is dark."))))
        .child(view().flex_row().flex_wrap().gap_2().pt_3().children(cards.iter().map(|c| card_view(pane, c, &selection, c.id == focused_id))))
        .child(view().px_1().pt_2().child(small(if match_system {
            "Choose a light theme to use it in light mode, a dark one for dark mode."
        } else {
            "Choose a theme to use it."
        })));
    page = page.child(view().pt_4().pb_2().child(caps_label("Design"))).child(design_group(pane));
    if host.accent_and_motion() {
        page = page.child(view().pt_3().child(group().child(accent_row(pane))));
    }
    if let Some((profile, name)) = host.profile() {
        let on = host.with_store(|s| library::pages_on(s, profile));
        page = page.child(view().pt_3().child(group().child(switch_row(
            "Theme styles on websites",
            move || on,
            move || pane.quietly(host.with_store(|s| library::set_pages(s, profile, !on))),
        ))));
        page = page.child(view().px_1().pt_1().child(small(format!(
            "In the {} profile, for themes that bring them. Never in private spaces, burner tabs or Lockdown; pages pick up changes as they load.",
            library::shown(&name)
        ))));
    }
    let motion = if host.accent_and_motion() { group().child(reduce_motion_row(pane)) } else { group() };
    page = page.child(view().pt_4().pb_2().child(caps_label("Motion"))).child(motion.child(effect_row(pane)));
    let compact = host.with_store(|s| COMPACT.on(s));
    let set_compact = move || {
        let ok = host.with_store(|s| COMPACT.set(s, !compact));
        if ok {
            Theme::set_compact(!compact);
        }
        pane.quietly(saved(ok));
    };
    page = page
        .child(view().pt_4().pb_2().child(caps_label("Density")))
        .child(group().child(switch_row("Compact density", move || compact, set_compact)))
        .child(view().px_1().pt_1().child(small(prefs::COMPACT_NOTE)));
    let hints = host.with_store(|s| FEATURE_HINTS.on(s));
    page = page
        .child(view().pt_4().pb_2().child(caps_label("Tips")))
        .child(group().child(switch_row("Shortcut tips", move || hints, move || pane.quietly(saved(host.with_store(|s| FEATURE_HINTS.set(s, !hints)))))))
        .child(view().px_1().pt_1().child(small(browser_core::hints::NOTE)));
    if host.safe() {
        let how = if host.desktop() { "Shift was held" } else { "safe mode was on" };
        page = page.child(view().pt_2().child(note(format!("Safe mode: Tarn started with its own theme because {how}. Quit and reopen to use yours."), true)));
    }
    if let Some(notice) = host.notice() {
        page = page.child(view().pt_2().child(note(notice, true)));
    }
    if let Some((error, message)) = pane.get(|s| s.message.clone()) {
        page = page.child(view().pt_2().child(note(message, error)));
    }
    if let Some(card) = cards.iter().find(|c| c.id == focused_id) {
        page = page.child(view().pt_3().child(details(pane, card)));
    }
    if let Some(card) = pending_card(pane) {
        page = page.child(card);
    }
    if let Some(editor) = pane.get(|s| s.colors).and_then(|id| color_editor(pane, id)) {
        page = page.child(editor);
    }
    if let Some(official) = official_section(pane) {
        page = page.child(official);
    }
    let reset_note =
        if host.desktop() { "Tarn's own theme. Holding Shift while Tarn opens also uses it, for that run." } else { "Tarn's own theme, in dark and light." };
    page.child(section("Add Themes"))
        .child(
            group()
                .child(github_row(pane))
                .child(
                    row()
                        .child(
                            view()
                                .flex_col()
                                .flex_1()
                                .child(label("A theme file"))
                                .child(small("A .css theme from someone else. Its website styles stay off until you turn them on.")),
                        )
                        .child(pill("Choose File\u{2026}", move || pane.import_file())),
                )
                .child(
                    row()
                        .child(view().flex_col().flex_1().child(label("From the clipboard")).child(small("Paste a theme you copied.")))
                        .child(accent_pill("Paste Theme", move || pane.paste())),
                ),
        )
        .child(
            row()
                .px_0()
                .pt_3()
                .child(secure_button("Reset Appearance", move || pane.report(host.with_store(library::reset), "Appearance reset to Tarn."), false))
                .child(text(reset_note).text_size(px(TEXT_SMALL)).font_family(SYSTEM_FONT).text_color(Secure::muted()).flex_1()),
        )
}

pub fn appearance<H: Host>(host: H) -> AnyElement {
    let pane = Pane { host, state: global::<State>(), repo: global::<Repo>() };
    if fetch::catalog_configured() && pane.state.with_untracked(|s| s.catalog.is_none()) {
        let cached = fetch::cached_catalog(&host.catalog_dir());
        pane.set(|s| s.catalog = cached);
    }
    if host.desktop() {
        drop(set_interval(Duration::from_secs(1), move || pane.reload_watched()));
    }
    dynamic(move || body(pane))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_name_the_appearance_they_are_used_for() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let cards = library::cards(&store);
        let mut selection = Selection::load(&store);
        let dark = cards.iter().find(|c| c.id == ThemeRef::DARK).unwrap();
        let light = cards.iter().find(|c| c.id == ThemeRef::LIGHT).unwrap();
        assert_eq!((in_use(dark, &selection), in_use(light, &selection)), (vec!["In use"], vec![]));
        selection.match_system = true;
        assert_eq!((in_use(dark, &selection), in_use(light, &selection)), (vec!["Dark"], vec!["Light"]));
        let withdrawn = Card { withdrawn: true, ..dark.clone() };
        assert!(in_use(&withdrawn, &selection).is_empty());
    }
}
