use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use browser_core::theme::{self, Origin, Scheme, Selection, Source, ThemeRef};
use browser_core::theme_fetch::{self as fetch, Pending, Release};
use browser_core::theme_library::{self as library, Card, hex, read_file as read_theme_file};
use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, prelude::FluentBuilder, px, rgba,
};
use store::Id;
use ui::controls::{button, field, group, note, row, section, secure_button, segment, segmented, swatch, switch_row, tab_stop};
use ui::text::bounded_title;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{Secure, TEXT_SMALL, Theme};

use ui::controls::text_block;

use super::{SettingsWindow, label};

/// The colour editor's theme, one field per colour, and their subscriptions.
type ColorEditor = (Id, Vec<(&'static str, Entity<TextInput>)>, Vec<Subscription>);
/// Cards cached for a pane revision and theme generation.
type Cards = ((u32, u32), Rc<Vec<Card>>);

pub(super) struct Pane {
    /// The official theme index, fetched or cached; `None` while this build has no catalog key.
    catalog: Option<shield::catalog::Index>,
    repo: Entity<TextInput>,
    busy: bool,
    pending: Option<Pending>,
    focused: Option<ThemeRef>,
    colors: Option<ColorEditor>,
    /// The user's theme open in a text editor, its file and the time it was last read.
    watch: Option<(Id, PathBuf, Option<SystemTime>)>,
    message: Option<(bool, String)>,
    revision: u32,
    cards: RefCell<Option<Cards>>,
    _repo_submit: Subscription,
}

impl Pane {
    pub(super) fn new(cx: &mut Context<SettingsWindow>) -> Pane {
        let repo = cx.new(|cx| TextInput::new("owner/repository or a github.com address", cx));
        let repo_submit = cx.subscribe(&repo, |this: &mut SettingsWindow, repo, event, cx| {
            if matches!(event, TextInputEvent::Submit) {
                let input = repo.read(cx).text().trim().to_owned();
                this.fetch_github(Some(input), None, cx);
            }
        });
        Pane {
            catalog: crate::themes::catalog_configured().then(crate::themes::cached_catalog).flatten(),
            repo,
            busy: false,
            pending: None,
            focused: None,
            colors: None,
            watch: None,
            message: None,
            revision: 0,
            cards: RefCell::default(),
            _repo_submit: repo_submit,
        }
    }
}

/// Where a theme came from, in the fixed security palette and the system font, so no theme can
/// make "Unverified" look like "Official" (threat model S29).
fn badge_line(text: String) -> gpui::Div {
    div()
        .text_size(px(TEXT_SMALL))
        .text_color(Secure::muted())
        .font_family(ui::theme::SYSTEM_FONT)
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text)
}

#[cfg(target_os = "macos")]
fn open_in_text_editor(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("/usr/bin/open").arg("-t").arg(path).spawn().map(drop)
}

#[cfg(windows)]
fn open_in_text_editor(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("notepad.exe").arg(path).spawn().map(drop)
}

/// Creates `dir` readable by the user only.
fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn themes_dir() -> PathBuf {
    crate::browser::data_dir().join("Themes")
}

impl SettingsWindow {
    fn cards(&self, cx: &Context<Self>) -> Rc<Vec<Card>> {
        let key = (self.appearance.revision, Theme::generation());
        if let Some((k, cards)) = self.appearance.cards.borrow().as_ref()
            && *k == key
        {
            return cards.clone();
        }
        let cards = library::cards(&crate::shared::core(cx).disk);
        let cards = Rc::new(cards);
        *self.appearance.cards.borrow_mut() = Some((key, cards.clone()));
        cards
    }

    /// Applies the stored choice everywhere after a theme or selection changed.
    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        self.appearance.revision += 1;
        crate::themes::changed(cx);
        cx.notify();
    }

    fn report(&mut self, result: Result<(), String>, done: &str, cx: &mut Context<Self>) {
        self.appearance.message = Some(match result {
            Ok(()) => (false, done.to_owned()),
            Err(e) => (true, e),
        });
        self.theme_changed(cx);
    }

    fn select_theme(&mut self, id: ThemeRef, scheme: Scheme, cx: &mut Context<Self>) {
        let core = crate::shared::core(cx);
        let mut selection = Selection::load(&core.disk);
        if selection.match_system && scheme == Scheme::Light {
            selection.light = id.clone();
        } else {
            selection.dark = id.clone();
        }
        self.appearance.focused = Some(id);
        self.appearance.message = None;
        let saved = selection.save(&core.disk).map_err(|e| e.to_string());
        self.report(saved, "", cx);
        self.appearance.message = None;
    }

    fn add_theme(&mut self, css: Result<String, String>, cx: &mut Context<Self>) {
        let core = crate::shared::core(cx);
        let added = css.and_then(|css| library::add(&core.disk, &css, browser_core::now()));
        match added {
            Ok(id) => {
                let scheme = core.disk.theme(id).ok().flatten().map_or(Scheme::Dark, |t| if t.scheme == "light" { Scheme::Light } else { Scheme::Dark });
                self.select_theme(ThemeRef::Stored(id), scheme, cx);
                self.appearance.message = Some((false, "Added. Its website styles stay off until you turn them on below.".to_owned()));
            }
            Err(e) => self.report(Err(e), "", cx),
        }
    }

    /// Asks GitHub for a theme, off the main thread, after a click; the answer waits for
    /// confirmation in `Pending`.
    fn fetch_github(&mut self, named: Option<String>, existing: Option<(Id, u64)>, cx: &mut Context<Self>) {
        if self.appearance.busy || !self.downloads_allowed(cx) {
            return;
        }
        self.appearance.busy = true;
        self.appearance.pending = None;
        self.appearance.message = Some((false, "Asking GitHub\u{2026}".to_owned()));
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let net = crate::ext_sources::Net::new();
            match (&named, existing) {
                (Some(input), _) => fetch::fetch_theme(&net, Release::Named(input)),
                (None, Some((_, repo_id))) => fetch::fetch_theme(&net, Release::Newest(repo_id)),
                (None, None) => Err("Enter a repository.".to_owned()),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.fetched(result, existing.map(|(id, _)| id), cx)).ok();
        })
        .detach();
    }

    fn fetched(&mut self, result: Result<fetch::Download, String>, existing: Option<Id>, cx: &mut Context<Self>) {
        self.appearance.busy = false;
        let core = crate::shared::core(cx);
        match fetch::pending(&core.disk, result, existing) {
            Ok(pending) => {
                self.appearance.pending = Some(pending);
                self.appearance.message = None;
            }
            Err(message) => self.appearance.message = Some(message),
        }
        cx.notify();
    }

    fn downloads_allowed(&mut self, cx: &mut Context<Self>) -> bool {
        let core = crate::shared::core(cx);
        let on = fetch::downloads_on(&core.disk);
        if !on {
            self.appearance.message = Some((true, "Theme and extension downloads are off in Privacy settings.".to_owned()));
            cx.notify();
        }
        on
    }

    fn fetch_official_catalog(&mut self, cx: &mut Context<Self>) {
        if self.appearance.busy || !self.downloads_allowed(cx) {
            return;
        }
        let core = crate::shared::core(cx);
        let last = fetch::last_catalog(&core.disk);
        let now = (browser_core::now() / 1000) as u64;
        self.appearance.busy = true;
        self.appearance.message = Some((false, "Asking GitHub\u{2026}".to_owned()));
        cx.notify();
        let task =
            cx.background_executor().spawn(async move { fetch::fetch_catalog(&crate::ext_sources::Net::new(), &crate::themes::catalog_dir(), last, now) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.appearance.busy = false;
                let core = crate::shared::core(cx);
                match result {
                    Ok(index) => {
                        let grew = fetch::catalog_fetched(&core.disk, &index);
                        this.appearance.catalog = Some(index);
                        this.appearance.message = None;
                        if grew {
                            this.theme_changed(cx);
                        }
                    }
                    Err(e) => this.appearance.message = Some((true, e)),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn install_official(&mut self, entry: shield::catalog::ThemeEntry, existing: Option<Id>, cx: &mut Context<Self>) {
        if self.appearance.busy || !self.downloads_allowed(cx) {
            return;
        }
        self.appearance.busy = true;
        cx.notify();
        let fetched = entry.clone();
        let task = cx.background_executor().spawn(async move { fetch::fetch_official(&crate::ext_sources::Net::new(), &fetched) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.appearance.busy = false;
                let core = crate::shared::core(cx);
                match fetch::save_official(&core.disk, &entry, existing, result, browser_core::now()) {
                    Ok((id, Some(scheme))) => {
                        this.select_theme(ThemeRef::Stored(id), scheme, cx);
                        this.appearance.message = Some((false, "Installed.".to_owned()));
                    }
                    Ok(_) => this.report(Ok(()), "Updated.", cx),
                    Err(e) => this.report(Err(e), "", cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn official_section(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !crate::themes::catalog_configured() {
            return None;
        }
        let core = crate::shared::core(cx);
        let mut list = group().child(
            row(true)
                .child(text_block("Reviewed and signed by Tarn", "Tarn asks GitHub for the list only when you click."))
                .child(button("theme-catalog", "Show Official Themes", false).on_click(cx.listener(|this, _, _, cx| this.fetch_official_catalog(cx)))),
        );
        let rows = self.appearance.catalog.as_ref().map(|c| fetch::official_rows(&core.disk, c)).unwrap_or_default();
        for (ix, official) in rows.into_iter().enumerate() {
            let (entry, existing) = (official.entry, official.existing);
            list = list.child(
                row(false).child(text_block(official.name, official.detail)).child(match official.action {
                    Some(label) => button(("theme-official", ix), label, false)
                        .on_click(cx.listener(move |this, _, _, cx| this.install_official(entry.clone(), existing, cx)))
                        .into_any_element(),
                    None => div().text_color(Theme::muted()).child("Installed").into_any_element(),
                }),
            );
        }
        Some(div().child(section("Official Themes")).child(list).into_any_element())
    }

    fn confirm_pending(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.appearance.pending.take() else { return };
        let core = crate::shared::core(cx);
        match fetch::confirm(&core.disk, &p, browser_core::now()) {
            Ok((id, Some(scheme))) => {
                self.select_theme(ThemeRef::Stored(id), scheme, cx);
                self.appearance.message = Some((false, "Installed. Its website styles stay off until you turn them on below.".to_owned()));
            }
            Ok(_) => self.report(Ok(()), "Updated.", cx),
            Err(e) => self.report(Err(e), "", cx),
        }
    }

    fn pending_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.appearance.pending.as_ref()?;
        let title = p.title();
        let mut card = ui::controls::secure_surface(Theme::radius_group())
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(title));
        for line in &p.lines {
            card = card.child(div().text_size(px(TEXT_SMALL)).text_color(Secure::muted()).child(SharedString::from(line.clone())));
        }
        card = card.child(
            div()
                .flex()
                .gap_2()
                .child(
                    secure_button("theme-confirm", if p.existing.is_some() { "Update" } else { "Install" }, true)
                        .on_click(cx.listener(|this, _, _, cx| this.confirm_pending(cx))),
                )
                .child(secure_button("theme-cancel", "Cancel", false).on_click(cx.listener(|this, _, _, cx| {
                    this.appearance.pending = None;
                    cx.notify();
                }))),
        );
        Some(div().pt_3().child(card).into_any_element())
    }

    fn duplicate_theme(&mut self, id: ThemeRef, window: &mut Window, cx: &mut Context<Self>) {
        let core = crate::shared::core(cx);
        match library::duplicate(&core.disk, &id, browser_core::now()) {
            Ok(copy) => {
                self.appearance.focused = Some(ThemeRef::Stored(copy));
                self.edit_colors(copy, window, cx);
                self.report(Ok(()), "Copied. Edit its colours below or its CSS in a text editor.", cx);
            }
            Err(e) => self.report(Err(e), "", cx),
        }
    }

    fn edit_colors(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let core = crate::shared::core(cx);
        let Some(row) = core.disk.theme(id).ok().flatten() else { return };
        let Ok(parsed) = theme::parse(&row.css, Source::Yours) else { return };
        let mut inputs = Vec::new();
        let mut subscriptions = Vec::new();
        for (name, color) in theme::editable_colors(&parsed.tokens) {
            let input = cx.new(|cx| {
                let mut input = TextInput::new("#rrggbb", cx);
                input.set_text(hex(color), cx);
                input
            });
            subscriptions.push(cx.subscribe(&input, move |this: &mut SettingsWindow, input, event, cx| {
                if matches!(event, TextInputEvent::Submit) {
                    this.save_color(id, name, &input, cx);
                }
            }));
            let typed = input.clone();
            subscriptions.push(cx.on_blur(&input.focus_handle(cx), window, move |this, _, cx| this.save_color(id, name, &typed, cx)));
            inputs.push((name, input));
        }
        self.appearance.colors = Some((id, inputs, subscriptions));
        cx.notify();
    }

    fn save_color(&mut self, id: Id, name: &'static str, input: &Entity<TextInput>, cx: &mut Context<Self>) {
        let value = input.read(cx).text().trim().to_owned();
        let core = crate::shared::core(cx);
        let stored = core
            .disk
            .theme(id)
            .ok()
            .flatten()
            .and_then(|t| theme::parse(&t.css, Source::Yours).ok())
            .and_then(|p| theme::editable_colors(&p.tokens).into_iter().find(|(n, _)| *n == name).map(|(_, c)| hex(c)));
        if stored.is_some_and(|c| c.eq_ignore_ascii_case(&value)) {
            return;
        }
        let saved = theme::set_color(&core.disk, id, name, &value, browser_core::now());
        self.report(saved, "", cx);
        self.appearance.message = self.appearance.message.take().filter(|(error, _)| *error);
    }

    fn edit_in_text_editor(&mut self, id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let core = crate::shared::core(cx);
        let Some(row) = core.disk.theme(id).ok().flatten() else { return };
        let dir = themes_dir();
        let path = dir.join(format!("{id}.css"));
        let written = private_dir(&dir).and_then(|()| std::fs::write(&path, &row.css)).and_then(|()| open_in_text_editor(&path));
        if let Err(e) = written {
            self.appearance.message = Some((true, format!("The theme could not be opened in a text editor: {e}")));
            return cx.notify();
        }
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let watching = self.appearance.watch.is_some();
        self.appearance.watch = Some((id, path, modified));
        self.appearance.message = Some((false, "Saved changes in the text editor show here while Settings is open.".to_owned()));
        cx.notify();
        if watching {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                match this.update_in(cx, |this, window, cx| this.reload_theme_file(window, cx)) {
                    Ok(true) => {}
                    _ => break,
                }
            }
        })
        .detach();
    }

    /// Reads the watched file again when it changed; false once nothing is watched.
    fn reload_theme_file(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((id, path, seen)) = self.appearance.watch.clone() else { return false };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if modified == seen {
            return true;
        }
        self.appearance.watch = Some((id, path.clone(), modified));
        let core = crate::shared::core(cx);
        let saved = read_theme_file(&path).and_then(|css| theme::save(&core.disk, id, "yours", &css, Origin::default(), browser_core::now()).map(drop));
        if self.appearance.colors.as_ref().is_some_and(|(c, _, _)| *c == id) {
            self.edit_colors(id, window, cx);
        }
        self.report(saved, "Reloaded from the text editor.", cx);
        true
    }

    fn save_theme_file(&mut self, id: ThemeRef, name: String, cx: &mut Context<Self>) {
        let core = crate::shared::core(cx);
        let Some(css) = theme::css_of(&core.disk, &id) else { return };
        let dir = std::env::home_dir().map(|h| h.join("Downloads")).unwrap_or_default();
        let file_name: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        let path = cx.prompt_for_new_path(&dir, Some(&format!("{}.css", file_name.trim())));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else { return };
            let written = std::fs::write(&path, css).map_err(|e| format!("The theme could not be saved: {e}"));
            this.update(cx, |this, cx| {
                this.appearance.message = Some(match written {
                    Ok(()) => (false, "Saved.".to_owned()),
                    Err(e) => (true, e),
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn import_theme_file(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let css =
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("css")) { read_theme_file(&path) } else { Err("Choose a .css file.".to_owned()) };
            this.update(cx, |this, cx| this.add_theme(css, cx)).ok();
        })
        .detach();
    }

    fn remove_theme(&mut self, id: Id, cx: &mut Context<Self>) {
        if self.appearance.watch.as_ref().is_some_and(|(w, _, _)| *w == id) {
            self.appearance.watch = None;
        }
        if self.appearance.colors.as_ref().is_some_and(|(c, _, _)| *c == id) {
            self.appearance.colors = None;
        }
        self.appearance.focused = None;
        let removed = library::remove(&crate::shared::core(cx).disk, id);
        std::fs::remove_file(themes_dir().join(format!("{id}.css"))).ok();
        self.report(removed, "Removed.", cx);
    }

    fn set_page_css(&mut self, id: Id, on: bool, cx: &mut Context<Self>) {
        let saved = library::set_page_css(&crate::shared::core(cx).disk, id, on);
        self.report(saved, "", cx);
        self.appearance.message = self.appearance.message.take().filter(|(error, _)| *error);
    }

    fn reset_appearance(&mut self, cx: &mut Context<Self>) {
        let core = crate::shared::core(cx);
        let saved = library::reset(&core.disk);
        self.with_browser(cx, |b, _, cx| b.set_choice(crate::prefs::Choice::Accent, 0, cx));
        self.appearance.focused = None;
        self.report(saved, "Appearance reset to Tarn.", cx);
    }

    pub(super) fn search_effect_row(&self, first: bool, cx: &mut Context<Self>) -> gpui::Div {
        use browser_core::effects::Effect;
        let stored = library::chosen_effect(&crate::shared::core(cx).disk);
        let choices = std::iter::once((None, "Theme")).chain(Effect::ALL.map(|e| (Some(e), e.label())));
        row(first).child(text_block("Search bar effect", "Around the search field on the start page and in the command bar.")).child(segmented().children(
            choices.enumerate().map(|(ix, (effect, label))| {
                tab_stop(segment(("search-effect", ix), label, effect == stored)).on_click(cx.listener(move |this, _, _, cx| {
                    let saved = library::set_effect(&crate::shared::core(cx).disk, effect);
                    this.report(saved, "", cx);
                    this.appearance.message = this.appearance.message.take().filter(|(error, _)| *error);
                }))
            }),
        ))
    }

    pub(super) fn theme_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let core = crate::shared::core(cx);
        let selection = Selection::load(&core.disk);
        let cards = self.cards(cx);
        let match_system = selection.match_system;
        let system = if cfg!(windows) { "Windows" } else { "macOS" };
        let mut rows = vec![
            switch_row("theme-match-system", true, text_block("Match system", format!("Follow {system}'s light and dark appearance.")), match_system)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let core = crate::shared::core(cx);
                    let mut selection = Selection::load(&core.disk);
                    let dark_scheme = this.cards(cx).iter().find(|c| c.id == selection.dark).map_or(Scheme::Dark, |c| c.scheme);
                    selection.set_match_system(!match_system, dark_scheme);
                    let saved = selection.save(&core.disk).map_err(|e| e.to_string());
                    this.report(saved, "", cx);
                    this.appearance.message = None;
                }))
                .into_any_element(),
        ];
        let slots = [
            ("theme-dark", "Dark theme", "Shown in dark appearance.", Scheme::Dark, selection.dark.clone()),
            ("theme-light", "Light theme", "Shown in light appearance.", Scheme::Light, selection.light.clone()),
        ];
        for (id, title, detail, scheme, current) in slots.into_iter().filter(|_| match_system) {
            let choices: Vec<(ThemeRef, String)> = cards.iter().filter(|c| c.scheme == scheme).map(|c| (c.id.clone(), bounded_title(&c.name))).collect();
            let shown = choices.iter().find(|(c, _)| *c == current).map_or_else(|| "Custom".to_owned(), |(_, name)| name.clone());
            let menu: Vec<(ThemeRef, String, Scheme)> =
                cards.iter().filter(|c| !c.withdrawn && c.scheme == scheme).map(|c| (c.id.clone(), bounded_title(&c.name), c.scheme)).collect();
            rows.push(
                row(false)
                    .child(text_block(title, detail))
                    .child(tab_stop(button(id, shown, false)).child(ui::icon::icon(ui::icon::Icon::ChevronsUpDown, 12., Theme::muted())).on_click(cx.listener(
                        move |_, _, window, cx| {
                            let Some(host) = engine::Host::from_window(window) else { return };
                            let items = menu
                                .iter()
                                .enumerate()
                                .map(|(ix, (id, name, _))| engine::menu::Item::action(name.clone(), ix).checked(*id == current))
                                .collect();
                            let menu = menu.clone();
                            cx.spawn_in(window, async move |this, cx| {
                                let (tx, rx) = futures::channel::oneshot::channel();
                                engine::menu::show(&host, items, None, move |ix| {
                                    tx.send(ix).ok();
                                });
                                let Ok(ix) = rx.await else { return };
                                let Some((id, _, scheme)) = menu.get(ix).cloned() else { return };
                                this.update(cx, |this, cx| this.select_theme(id, scheme, cx)).ok();
                            })
                            .detach();
                        },
                    )))
                    .into_any_element(),
            );
        }
        rows
    }

    fn theme_card(&self, card: &Card, selection: &Selection, focused: bool, ix: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let in_use: Vec<&str> = [
            (selection.dark == card.id, if selection.match_system { "Dark" } else { "In use" }),
            (selection.match_system && selection.light == card.id, "Light"),
        ]
        .into_iter()
        .filter_map(|(on, label)| (on && !card.withdrawn).then_some(label))
        .collect();
        let (id, scheme, withdrawn) = (card.id.clone(), card.scheme, card.withdrawn);
        let [sidebar, surface, accent, text] = card.swatches;
        tab_stop(div().id(("theme-card", ix)))
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded(px(Theme::radius_group()))
            .border_1()
            .border_color(if focused { Theme::accent() } else { Theme::surface_edge() })
            .bg(Theme::panel_row())
            .cursor_pointer()
            .hover(|s| s.bg(Theme::hover()))
            .child(
                div().h(px(46.)).flex().rounded(px(Theme::radius_row())).overflow_hidden().child(div().w(px(36.)).h_full().bg(rgba(sidebar))).child(
                    div()
                        .flex_1()
                        .h_full()
                        .bg(rgba(surface))
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .child(div().w(px(22.)).h(px(10.)).rounded_full().bg(rgba(accent)))
                        .child(div().w(px(30.)).h(px(4.)).rounded_full().bg(rgba(text))),
                ),
            )
            .child(div().overflow_hidden().whitespace_nowrap().text_ellipsis().child(card.name.clone()))
            .child(badge_line(if in_use.is_empty() { card.badge.clone() } else { format!("{} · {}", in_use.join(", "), card.badge) }))
            .on_click(cx.listener(move |this, _, _, cx| {
                if withdrawn {
                    this.appearance.focused = Some(id.clone());
                    this.appearance.message = Some((true, "This theme was withdrawn as unsafe and cannot be used. Remove it below.".to_owned()));
                    cx.notify();
                } else {
                    this.select_theme(id.clone(), scheme, cx);
                }
            }))
    }

    fn theme_details(&self, card: &Card, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut details =
            group().child(row(true).child(div().flex_1().min_w_0().flex().flex_col().child(card.name.clone()).child(badge_line(card.badge.clone()))));
        if !card.adjusted.is_empty() {
            details = details.child(row(false).child(label(format!("Made readable: {} (its contrast was too low).", card.adjusted.join(", ")))));
        }
        for warning in card.warnings.iter().take(5) {
            details = details.child(row(false).child(div().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).child(bounded_title(warning))));
        }
        if card.warnings.len() > 5 {
            details = details.child(row(false).child(label(format!("{} more notes", card.warnings.len() - 5))));
        }
        if let (Some(id), false) = (card.stored, card.hosts.is_empty()) {
            let on = card.page_css;
            let detail = format!(
                "{}{}",
                if card.unverified { "Nobody reviewed these styles; they may only change colours and fonts. " } else { "" },
                bounded_title(&format!("Sites: {}", card.hosts.join(", ")))
            );
            details = details.child(
                switch_row("theme-page-css", false, text_block("Styles for websites", detail), on)
                    .on_click(cx.listener(move |this, _, _, cx| this.set_page_css(id, !on, cx))),
            );
        }
        let (id, name) = (card.id.clone(), card.name.clone());
        let duplicate_id = id.clone();
        let mut actions = div().flex().flex_wrap().gap_2().when(!card.withdrawn, |d| {
            d.child(
                button("theme-duplicate", if card.editable { "Duplicate" } else { "Duplicate and Edit" }, false)
                    .on_click(cx.listener(move |this, _, window, cx| this.duplicate_theme(duplicate_id.clone(), window, cx))),
            )
        });
        if let (Some(stored), true) = (card.stored, card.editable) {
            actions = actions
                .child(button("theme-colors", "Edit Colours", false).on_click(cx.listener(move |this, _, window, cx| this.edit_colors(stored, window, cx))))
                .child(
                    button("theme-editor", "Edit in Text Editor", false)
                        .on_click(cx.listener(move |this, _, window, cx| this.edit_in_text_editor(stored, window, cx))),
                );
        }
        if let (Some(stored), Some(repo_id)) = (card.stored, card.repo_id) {
            actions = actions.child(
                button("theme-update", "Check for Updates", false)
                    .on_click(cx.listener(move |this, _, _, cx| this.fetch_github(None, Some((stored, repo_id)), cx))),
            );
        }
        if !card.withdrawn {
            actions = actions.child(
                button("theme-save", "Save as File\u{2026}", false)
                    .on_click(cx.listener(move |this, _, _, cx| this.save_theme_file(id.clone(), name.clone(), cx))),
            );
        }
        if let Some(stored) = card.stored {
            let question = format!("Remove the theme \u{201C}{}\u{201D}?", bounded_title(&card.name));
            actions = actions.child(button("theme-remove", "Remove\u{2026}", false).on_click(cx.listener(move |this, _, window, cx| {
                this.ask(&question, None, &["Remove"], window, cx, move |this, _, cx| this.remove_theme(stored, cx));
            })));
        }
        details.child(row(false).child(actions))
    }

    fn color_editor(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, inputs, _) = self.appearance.colors.as_ref()?;
        if self.appearance.focused != Some(ThemeRef::Stored(*id)) {
            return None;
        }
        let core = crate::shared::core(cx);
        let tokens = core.disk.theme(*id).ok().flatten().and_then(|t| theme::parse(&t.css, Source::Yours).ok())?.tokens;
        let colors = theme::editable_colors(&tokens);
        let mut editor = group();
        for (ix, ((name, input), (_, color))) in inputs.iter().zip(colors).enumerate() {
            editor = editor.child(
                row(ix == 0)
                    .child(swatch(("theme-color", ix), rgba(color), false, 16.))
                    .child(label(name.replace('-', " ")))
                    .child(field(input.focus_handle(cx).is_focused(window)).w(px(120.)).py_0p5().child(input.clone())),
            );
        }
        Some(
            div()
                .child(section("Colours"))
                .child(editor)
                .child(note(if cfg!(windows) {
                    "A colour applies on Enter or when you leave its field. Text that would be hard to read is made readable."
                } else {
                    "A colour applies on Return or when you leave its field. Text that would be hard to read is made readable."
                }))
                .into_any_element(),
        )
    }

    pub(super) fn theme_sections(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let core = crate::shared::core(cx);
        let selection = Selection::load(&core.disk);
        let cards = self.cards(cx);
        let (safe, notice) = {
            let state = core.themes.borrow();
            (state.safe, state.active.as_ref().and_then(|a| a.notice.clone()))
        };
        let focused_id = self.appearance.focused.clone().unwrap_or_else(|| selection.active(crate::themes::system_light(cx)).clone());
        let focused = cards.iter().find(|c| c.id == focused_id);
        let gallery = div()
            .grid()
            .grid_cols(4)
            .gap_2()
            .children(cards.iter().enumerate().map(|(ix, card)| self.theme_card(card, &selection, card.id == focused_id, ix, cx)));
        let match_system = selection.match_system;
        let mut page = div().child(div().pt_3().child(gallery)).child(note(if match_system {
            "Click a light theme to use it in light mode, a dark one for dark mode."
        } else {
            "Click a theme to use it."
        }));
        let profile = self.browser.upgrade().and_then(|b| {
            let b = b.read(cx);
            let private = b.model.current_space().is_some_and(|s| s.private);
            b.current_profile().filter(|_| !private)
        });
        if let Some(profile) = profile {
            let on = crate::themes::pages_on(&core, profile.id);
            let id = profile.id;
            page = page.child(
                div().pt_3().child(
                    group().child(
                        switch_row(
                            "theme-pages-profile",
                            true,
                            text_block(
                                "Theme styles on websites",
                                format!("In the {} profile, for themes that bring them. Never in private spaces, burner tabs or Lockdown; pages pick up changes as they load.", bounded_title(&profile.name)),
                            ),
                            on,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                                let core = crate::shared::core(cx);
                                let saved = core.disk.set_setting(&browser_core::theme::pages_setting(id), if on { "off" } else { "on" }).map_err(|e| e.to_string());
                                this.report(saved, "", cx);
                                this.appearance.message = this.appearance.message.take().filter(|(error, _)| *error);
                            })),
                    ),
                ),
            );
        }
        if safe {
            page = page.child(
                div()
                    .pt_2()
                    .text_color(Secure::warning())
                    .child("Safe mode: Tarn started with its own theme because Shift was held. Quit and reopen to use yours."),
            );
        }
        if let Some(notice) = notice {
            page = page.child(div().pt_2().text_color(Secure::warning()).child(notice));
        }
        if let Some((error, message)) = self.appearance.message.clone().filter(|(_, m)| !m.is_empty()) {
            page = page.child(div().pt_2().text_color(if error { Secure::warning() } else { Theme::muted() }).child(SharedString::from(message)));
        }
        if let Some(card) = focused {
            page = page.child(div().pt_3().child(self.theme_details(card, cx)));
        }
        if let Some(pending) = self.pending_card(cx) {
            page = page.child(pending);
        }
        if let Some(editor) = self.color_editor(window, cx) {
            page = page.child(editor);
        }
        let repo = self.appearance.repo.clone();
        let busy = self.appearance.busy;
        if let Some(official) = self.official_section(cx) {
            page = page.child(official);
        }
        page.child(section("Add Themes"))
            .child(
                group()
                    .child(
                        row(true)
                            .child(text_block("From GitHub", "The theme.css file of a repository's release. Tarn asks GitHub only when you click Install."))
                            .child(field(repo.focus_handle(cx).is_focused(window)).w(px(240.)).py_0p5().child(repo.clone()))
                            .child(button("theme-github", if busy { "Asking\u{2026}" } else { "Install" }, false).on_click(cx.listener(
                                move |this, _, _, cx| {
                                    let input = repo.read(cx).text().trim().to_owned();
                                    this.fetch_github(Some(input), None, cx);
                                },
                            ))),
                    )
                    .child(
                        row(false)
                            .child(text_block("A theme file", "A .css theme from someone else. Its website styles stay off until you turn them on."))
                            .child(button("theme-import", "Choose File\u{2026}", false).on_click(cx.listener(|this, _, _, cx| this.import_theme_file(cx)))),
                    )
                    .child(row(false).child(text_block("From the clipboard", "Paste a theme you copied.")).child(
                        button("theme-paste", "Paste Theme", false).on_click(cx.listener(|this, _, _, cx| {
                            let css = cx
                                .read_from_clipboard()
                                .and_then(|c| c.text())
                                .filter(|t| !t.trim().is_empty())
                                .ok_or_else(|| "The clipboard holds no text.".to_owned());
                            this.add_theme(css, cx);
                        })),
                    )),
            )
            .child(
                div()
                    .pt_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(secure_button("theme-reset", "Reset Appearance", false).on_click(cx.listener(|this, _, _, cx| this.reset_appearance(cx))))
                    .child(
                        div()
                            .text_size(px(TEXT_SMALL))
                            .text_color(Secure::muted())
                            .font_family(ui::theme::SYSTEM_FONT)
                            .child("Tarn's own theme and accent. Holding Shift while Tarn opens also uses it, for that run."),
                    ),
            )
            .into_any_element()
    }
}
