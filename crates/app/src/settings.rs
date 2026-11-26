use browser_core::library::HistoryRange;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Div, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseMoveEvent, ParentElement, Pixels, PromptLevel, Render, RenderImage, SharedString, Stateful, StatefulInteractiveElement, Styled, Subscription,
    TitlebarOptions, WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, canvas, div, img, point, prelude::FluentBuilder, px, rgb, size,
};
use shield::ShieldLevel;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use store::{Id, Profile, WebEngine};
use ui::command::ENGINES;
use ui::controls::{
    button, disabled_segment, field, group, icon_text_button, note, radio, row, section, segment, segmented, swatch, switch, switch_row, tab_stop, text_block,
    wide_row,
};
use ui::icon::{Icon, icon};
use ui::motion::fade_in;
use ui::text::OneLine;
use ui::text::bounded_title;
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::{ACCENTS, TEXT, TEXT_HEADING, TEXT_SMALL, TEXT_TINY, TEXT_TITLE, Theme};

use crate::browser::Browser;

mod ai;
mod appearance;
mod extensions;
mod notes;
mod privacy;
mod profile_card;
mod shortcuts;
mod snippets;
use crate::filters;
use crate::prefs::{Choice, LIVE_TABS, Pref, Prefs};
use crate::profiles::{EditProfile, NewProfile, profile_dot};
use crate::shield_card::{self, Card};
use crate::shortcuts::Keys::{Bound, Fixed};
use crate::toolbar::shield_badge;
use crate::{CloseOverlay, CloseTab};
use browser_core::shortcuts::{self as keymap, Action as Shortcut};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SettingsPane {
    Profile,
    General,
    Appearance,
    Tabs,
    Search,
    Privacy,
    Permissions,
    Profiles,
    Developer,
    Extensions,
    Snippets,
    Notes,
    Shortcuts,
    Ai,
}

const PANES: [(SettingsPane, Icon, &str, &str, &str); 14] = [
    (
        SettingsPane::Profile,
        Icon::Shield,
        "Protection",
        "Shield level, what it blocked and the default browser.",
        "shield level standard strict lockdown card filter lists site overrides connections default browser",
    ),
    (SettingsPane::Profiles, Icon::Person, "Profiles", "Your profiles and the rules that open sites in a space.", "profile new edit delete space rules"),
    (
        SettingsPane::General,
        Icon::Gear,
        "General",
        "Updates, web engine, downloads, import and this version.",
        "updates nightly web engine chromium webkit downloads import about version licence source privacy imprint",
    ),
    (
        SettingsPane::Appearance,
        Icon::Palette,
        "Appearance",
        "Theme, accent colour, motion and layout.",
        "theme dark light match system accent colour color motion reduce toolbar compact density sidebar search bar effect tips design glass liquid transparency translucent blur corners round",
    ),
    (
        SettingsPane::Tabs,
        Icon::AppWindow,
        "Tabs",
        "When tabs sleep, close and open.",
        "awake sleep memory new tab start page close archive gestures swipe trackpad touchpad drag tear off temporary full window",
    ),
    (SettingsPane::Search, Icon::Search, "Search", "Search engine and suggestions.", "search engine suggestions history text shortcuts bangs"),
    (
        SettingsPane::Privacy,
        Icon::Hand,
        "Privacy",
        "Blocking, what sites keep and your data.",
        "block ads trackers cookie banners youtube pop-ups malware adult zoom muted hidden elements connections history clear export delete data localhost passkeys security keys sign-in",
    ),
    (
        SettingsPane::Permissions,
        Icon::Camera,
        "Permissions",
        "Your answers to sites that asked for a device or pop-ups.",
        "camera microphone location pop-ups site answers allow block",
    ),
    (
        SettingsPane::Ai,
        Icon::Sparkles,
        "AI",
        "The assistant, where questions go, commands and chats.",
        "assistant chat chats claude api key ollama model connect disconnect ask commands prompts",
    ),
    (
        SettingsPane::Developer,
        Icon::Code,
        "Developer",
        "Web Inspector, the developer profile and local servers.",
        "web inspector developer tools profile local servers localhost",
    ),
    (
        SettingsPane::Extensions,
        Icon::Braces,
        "Extensions",
        "Installed extensions, their access and new ones.",
        "install github file bitwarden every site official catalog store updates",
    ),
    (SettingsPane::Snippets, Icon::FileText, "Snippets", "Short names that expand into longer text.", "text expansion expand insert templates signature"),
    (
        SettingsPane::Notes,
        Icon::Pencil,
        "Notes",
        "Your notes, to search, sort, export and ask about.",
        "notes write search sort export markdown ideas assistant",
    ),
    (SettingsPane::Shortcuts, Icon::Keyboard, "Shortcuts", "Keyboard shortcuts, and keys you set yourself.", "keyboard keys"),
];

fn pane_matches(query: &str, name: &str, words: &str) -> bool {
    browser_core::settings::words_match(query, &format!("{name} {words}"))
}

pub(crate) use browser_core::profiles::{LOCKDOWN_WIPE, lockdown_question};

const YOU: usize = 2;

type Pick = fn(&mut Browser, usize, &mut Context<Browser>);

pub(crate) fn level_description(level: ShieldLevel) -> &'static str {
    match level {
        ShieldLevel::Standard => {
            "Blocks trackers and ads with the installed filter lists, removes tracking parameters from links you open and blocks pages from reaching your local network."
        }
        ShieldLevel::Strict => "Standard, plus HTTPS-only browsing and no referrer sent to other sites unless a page asks for one.",
        ShieldLevel::Lockdown => {
            "Strict, plus JavaScript off, no saved page state, no site permission requests and no website data kept after Tarn quits. Choosing it deletes the cookies and site data this profile has stored, except data it shares with another profile."
        }
    }
}

/// `1234567` as `1,234,567`.
pub(crate) use browser_core::shield_card::thousands;

fn value(text: impl Into<SharedString>) -> gpui::Div {
    div().text_color(Theme::muted()).child(text.into())
}

fn label(text: impl Into<SharedString>) -> gpui::Div {
    div().flex_1().one_line(text.into())
}

/// A section title with a button at its right end.
fn heading(title: impl Into<SharedString>, action: impl IntoElement) -> Div {
    section(title).flex().items_center().justify_between().gap_3().child(action)
}

fn warning(text: impl Into<SharedString>) -> Div {
    note(text).text_color(Theme::warning())
}

fn search_field(input: &gpui::Entity<TextInput>, focused: bool) -> Div {
    div().child(field(focused).w(px(320.)).py_0p5().child(input.clone()))
}

type Click = Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App)>;

/// A list editor's buttons: Cancel, then Save while an item is edited or Add.
fn editor_actions(id: &'static str, editing: bool, cancel: Click, save: Click) -> Div {
    div()
        .pt_2()
        .flex()
        .justify_end()
        .gap_2()
        .child(tab_stop(button((id, 0usize), "Cancel", false)).on_click(cancel))
        .child(tab_stop(button((id, 1usize), if editing { "Save" } else { "Add" }, true)).on_click(save))
}

/// One labelled field of a list editor.
fn editor_row(first: bool, name: &'static str, input: &gpui::Entity<TextInput>, focused: bool) -> Div {
    row(first).child(div().w(px(64.)).flex_none().text_color(Theme::muted()).child(name)).child(field(focused).flex_1().min_w_0().py_0p5().child(input.clone()))
}

/// A list's title with its Add button, which opens the list's editor.
fn list_heading(title: &'static str, add: Option<Stateful<Div>>) -> Div {
    heading(title, div().h(px(26.)).children(add))
}

fn licenses() -> Option<std::path::PathBuf> {
    static PATH: std::sync::LazyLock<Option<std::path::PathBuf>> = std::sync::LazyLock::new(|| {
        let exe = std::env::current_exe().ok()?;
        #[cfg(windows)]
        let path = exe.parent()?.join("Third-Party-Licenses.txt");
        #[cfg(not(windows))]
        let path = exe.parent()?.parent()?.join("Resources/Third-Party-Licenses.txt");
        path.is_file().then_some(path)
    });
    PATH.clone()
}

fn available(button: Stateful<Div>, on: bool, reason: &'static str) -> Stateful<Div> {
    if on { button } else { button.opacity(0.45).cursor_default().tooltip(ui::controls::tooltip(reason, None)) }
}

/// Values read from the database, kept until a setting changes or the profile or space does.
#[derive(Clone)]
struct Stored {
    accent: usize,
    live_tabs: usize,
    sleep_after: usize,
    archive_after: usize,
    engine: usize,
    profiles: Vec<Profile>,
}

type StoredKey = (usize, usize, Option<Profile>, bool, u64, u64);

/// What the panes show, read from the browser once per render.
struct Snapshot {
    prefs: Prefs,
    reduce: bool,
    accent: usize,
    live_tabs: usize,
    sleep_after: usize,
    archive_after: usize,
    engine: usize,
    web_engine: WebEngine,
    updater: bool,
    update_ready: Option<String>,
    update_checks: bool,
    chromium_allowed: bool,
    chromium_closed: Option<&'static str>,
    profile: Option<Profile>,
    private: bool,
    burner: bool,
    ai_provider: browser_core::assistant::settings::Provider,
    ollama_model: String,
    claude_model: usize,
    ai_commands: Vec<store::AiCommand>,
    chats: usize,
    overrides: Vec<(String, ShieldLevel)>,
    zooms: Vec<(String, f64)>,
    muted: Vec<String>,
    permissions: Vec<browser_core::permissions::SitePermission>,
    cert_exceptions: Vec<(String, u16, String)>,
    zapped: Vec<(String, String)>,
    paused: Vec<String>,
    profiles: Vec<Profile>,
    space_rules: Vec<(Id, String, String)>,
    filters: filters::Status,
    notice: Option<String>,
    shared_data: bool,
    developer: bool,
    local_servers: Option<Vec<u16>>,
    extensions: Vec<crate::extensions::Row>,
    extensions_allowed: bool,
    extension_downloads: bool,
    all_sites_notice: bool,
    extension_notices: Vec<String>,
    extension_busy: Option<String>,
    extension_choice: Vec<String>,
    consent: Option<crate::extensions::Consent>,
    store_sheet: Option<(crate::extensions::StoreToken, String, Vec<String>)>,
    catalog: (Vec<crate::extensions::CatalogRow>, bool),
    card: Option<Card>,
    /// `None` outside an app bundle, which cannot become the default browser.
    default_browser: Option<bool>,
    notes: Vec<store::Note>,
    snippets: Rc<Vec<store::Snippet>>,
}

pub struct SettingsWindow {
    browser: WeakEntity<Browser>,
    main: WindowHandle<Browser>,
    pane: SettingsPane,
    focus: FocusHandle,
    stored: Option<(StoredKey, Stored)>,
    repo_input: gpui::Entity<TextInput>,
    search: gpui::Entity<TextInput>,
    card: Option<(CardKey, Option<Card>)>,
    motion: CardMotion,
    /// `None` until asked; asking LaunchServices on every render would be wasteful.
    default_browser: Option<Option<bool>>,
    polling_default: bool,
    card_notice: Option<String>,
    /// The action whose new keys the next keystroke sets, and the interceptor that catches it.
    recording: Option<(Shortcut, Subscription)>,
    shortcut_error: Option<String>,
    appearance: appearance::Pane,
    snippets: snippets::Pane,
    notes: notes::Pane,
    ai: ai::Pane,
    _inputs: [Subscription; 2],
    _observe: Option<Subscription>,
    _activation: Subscription,
    _release: Subscription,
}

impl SettingsWindow {
    pub fn open(browser: WeakEntity<Browser>, main: WindowHandle<Browser>, cx: &mut App) -> Option<WindowHandle<SettingsWindow>> {
        let sheet = size(px(1000.), px(700.));
        let over = main
            .update(cx, |_, window, cx| window.display(cx).map(|d| (window.bounds().center(), crate::shared::usable_bounds(&*d), Some(d.id()))))
            .ok()
            .flatten();
        let primary = || cx.primary_display().map(|d| crate::shared::usable_bounds(&*d)).map(|s| (s.center(), s, None));
        let (bounds, display_id) = match over.or_else(primary) {
            Some((center, screen, display)) => {
                let sheet = size(sheet.width.min(screen.size.width), sheet.height.min(screen.size.height));
                let at = center - point(sheet.width / 2., sheet.height / 2.);
                let x = at.x.clamp(screen.origin.x, screen.right() - sheet.width);
                let y = at.y.clamp(screen.origin.y, screen.bottom() - sheet.height);
                (Bounds::new(point(x, y), sheet), display)
            }
            None => (Bounds::centered(None, sheet, cx), None),
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            display_id,
            window_min_size: Some(size(px(720.), px(480.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Settings".into()),
                appears_transparent: true,
                // AppKit only hit-tests the buttons inside its titlebar strip, so they stay within it.
                traffic_light_position: Some(point(px(18.), px(10.))),
            }),
            window_background: crate::themes::window_background(),
            is_minimizable: false,
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            cx.new(|cx| {
                let observe = browser.upgrade().map(|b| cx.observe(&b, |_, _, cx| cx.notify()));
                // The default browser can change in System Settings while this window is open.
                let activation = cx.observe_window_activation(window, |this: &mut SettingsWindow, window, cx| {
                    if window.is_window_active() {
                        this.default_browser = None;
                        cx.notify();
                        crate::shared::each(cx, |_, _, cx| cx.notify());
                    }
                });
                // The pane list is a tab stop, so Tab and Shift+Tab come back to its Up and Down.
                let focus = cx.focus_handle().tab_stop(true);
                window.focus(&focus);
                let repo_input = cx.new(|cx| TextInput::new("owner/repository or a github.com address", cx));
                let search = cx.new(|cx| TextInput::new("Search", cx));
                let inputs = [
                    cx.subscribe(&repo_input, |this: &mut SettingsWindow, _, event, cx| {
                        if matches!(event, TextInputEvent::Submit) {
                            this.install_from_github(cx);
                        }
                    }),
                    cx.subscribe(&search, |this: &mut SettingsWindow, _, event, cx| this.on_search(event, cx)),
                ];
                SettingsWindow {
                    browser,
                    main,
                    pane: SettingsPane::Profile,
                    focus,
                    stored: None,
                    repo_input,
                    search,
                    motion: CardMotion::default(),
                    card: None,
                    default_browser: None,
                    polling_default: false,
                    card_notice: None,
                    recording: None,
                    shortcut_error: None,
                    appearance: appearance::Pane::new(cx),
                    snippets: snippets::Pane::new(cx),
                    notes: notes::Pane::new(cx),
                    ai: ai::Pane::new(cx),
                    _inputs: inputs,
                    _observe: observe,
                    _activation: activation,
                    _release: cx.on_release(|this: &mut SettingsWindow, cx| {
                        this.main.update(cx, |b, _, cx| b.cancel_store_offer(cx)).ok();
                    }),
                }
            })
        })
        .ok()
    }

    fn snapshot(&mut self, cx: &App) -> Option<Snapshot> {
        let b = self.browser.upgrade()?;
        let b = b.read(cx);
        let current = b.model.current_space().map(|s| (s.private, s.space.profile_id));
        let key = (
            b.model.space_ix,
            b.model.spaces.len(),
            b.current_profile(),
            b.profile_editor.is_some(),
            b.store(false).settings_changes(),
            b.store(true).settings_changes(),
        );
        let stored = match &self.stored {
            Some((k, stored)) if *k == key => stored.clone(),
            _ => {
                let stored = Stored {
                    accent: b.choice(Choice::Accent),
                    live_tabs: b.choice(Choice::LiveTabs),
                    sleep_after: b.choice(Choice::SleepAfter),
                    archive_after: b.choice(Choice::ArchiveAfter),
                    engine: b.search_engine(),
                    profiles: b.all_profiles(),
                };
                self.stored = Some((key, stored.clone()));
                stored
            }
        };
        let profile_pane = self.pane == SettingsPane::Profile;
        let card_key = (shield_card::changes(), browser_core::now() / 60_000, b.current_profile(), current);
        let card = if !profile_pane {
            None
        } else if let Some((_, card)) = self.card.as_ref().filter(|(k, _)| *k == card_key) {
            card.clone()
        } else {
            let card = shield_card::for_profile(b);
            self.card = Some((card_key, card.clone()));
            card
        };
        Some(Snapshot {
            card,
            default_browser: if profile_pane { *self.default_browser.get_or_insert_with(default_browser) } else { None },
            prefs: b.core.prefs.get(),
            reduce: b.pref(Pref::ReduceMotion),
            accent: stored.accent,
            live_tabs: stored.live_tabs,
            sleep_after: stored.sleep_after,
            archive_after: stored.archive_after,
            engine: stored.engine,
            web_engine: b.core.default_web_engine.get(),
            updater: b.core.updater.is_some(),
            update_ready: if self.pane == SettingsPane::General { b.core.updater.as_ref().and_then(platform::updater::Updater::ready_version) } else { None },
            update_checks: b.core.update_checks(),
            chromium_allowed: b.core.chromium_allowed(),
            chromium_closed: b.chromium_closed(),
            profile: b.current_profile(),
            private: b.model.current_space().is_some_and(|s| s.private),
            burner: b.model.burner_shown(),
            ai_provider: browser_core::assistant::settings::provider(&b.core.disk),
            ollama_model: if self.pane == SettingsPane::Ai { browser_core::assistant::settings::ollama_model(&b.core.disk) } else { String::new() },
            claude_model: if self.pane == SettingsPane::Ai { browser_core::assistant::settings::claude_model(&b.core.disk) } else { 0 },
            ai_commands: match (self.pane, b.model.profile()) {
                (SettingsPane::Ai, Some((false, id))) => b.core.disk.ai_commands(id).unwrap_or_default(),
                _ => Vec::new(),
            },
            chats: match (self.pane, b.model.profile()) {
                (SettingsPane::Ai, Some((false, id))) => b.core.disk.chat_titles(id).map(|c| c.len()).unwrap_or_default(),
                _ => 0,
            },
            overrides: if matches!(self.pane, SettingsPane::Privacy | SettingsPane::Profile) { b.site_overrides() } else { Vec::new() },
            zooms: if self.pane == SettingsPane::Privacy { b.site_zooms() } else { Vec::new() },
            muted: if self.pane == SettingsPane::Privacy { b.muted_sites() } else { Vec::new() },
            permissions: if self.pane == SettingsPane::Permissions { b.profile_permissions() } else { Vec::new() },
            cert_exceptions: match (self.pane, b.model.profile()) {
                (SettingsPane::Permissions, Some((false, id))) => b.core.disk.cert_exceptions(id).unwrap_or_default(),
                _ => Vec::new(),
            },
            zapped: if self.pane == SettingsPane::Privacy { b.zapped(None) } else { Vec::new() },
            paused: if self.pane == SettingsPane::Privacy { b.paused_sites() } else { Vec::new() },
            notes: match (self.pane, b.model.profile()) {
                (SettingsPane::Notes, Some((false, id))) => b.core.disk.notes(id).unwrap_or_default(),
                _ => Vec::new(),
            },
            snippets: if self.pane == SettingsPane::Snippets { b.core.snippets(b.model.profile()) } else { Rc::default() },
            profiles: stored.profiles,
            space_rules: if self.pane == SettingsPane::Profiles { b.space_rule_rows() } else { Vec::new() },
            developer: b.developer,
            local_servers: b.local_servers.clone(),
            extensions: if self.pane == SettingsPane::Extensions { b.extension_rows() } else { Vec::new() },
            extensions_allowed: b.extensions_allowed(),
            extension_downloads: b.extension_downloads(),
            all_sites_notice: self.pane == SettingsPane::Extensions && !b.pref(Pref::ExtensionsEverySite) && b.all_sites_notice(),
            extension_notices: b.core.extensions.borrow().notices.clone(),
            extension_busy: b.core.extensions.borrow().busy.clone(),
            extension_choice: b
                .core
                .extensions
                .borrow()
                .choice
                .as_ref()
                .map(|(_, a)| a.iter().map(|a| crate::extensions::shown(&a.name)).collect())
                .unwrap_or_default(),
            consent: if self.pane == SettingsPane::Extensions { b.core.extensions.borrow().consent() } else { None },
            store_sheet: if self.pane == SettingsPane::Extensions { b.store_sheet() } else { None },
            catalog: if self.pane == SettingsPane::Extensions { b.catalog_rows() } else { (Vec::new(), false) },
            filters: b.core.filters.borrow().clone(),
            shared_data: self.pane == SettingsPane::Privacy
                && b.model.current_space().is_some_and(|s| !s.private)
                && b.current_profile().is_some_and(|p| b.store_is_shared(p.id, &p.data_store_uuid)),
            notice: b.privacy_notice.as_ref().filter(|(owner, _)| Some(*owner) == current).map(|(_, n)| n.clone()),
        })
    }

    fn with_browser(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Browser, &mut Window, &mut Context<Browser>)) {
        self.stored = None;
        self.main.update(cx, |b, window, cx| f(b, window, cx)).ok();
    }

    fn pref_row(&self, first: bool, pref: Pref, title: &'static str, detail: impl Into<SharedString>, on: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        switch_row(title, first, text_block(title, detail), on)
            .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.set_pref(pref, !on, cx))))
    }

    #[allow(clippy::too_many_arguments)]
    fn choice_row(
        &self,
        first: bool,
        title: &'static str,
        detail: impl Into<SharedString>,
        options: &[&'static str],
        selected: usize,
        pick: Pick,
        cx: &mut Context<Self>,
    ) -> Div {
        wide_row(
            first,
            text_block(title, detail),
            segmented().children(options.iter().enumerate().map(|(ix, option)| {
                tab_stop(segment((title, ix), *option, ix == selected))
                    .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| pick(b, ix, cx))))
            })),
        )
    }

    pub(crate) fn set_pane(&mut self, pane: SettingsPane, cx: &mut Context<Self>) {
        if self.pane == SettingsPane::Extensions && pane != SettingsPane::Extensions {
            self.with_browser(cx, |b, _, cx| b.cancel_store_offer(cx));
        }
        if pane == SettingsPane::Developer {
            self.with_browser(cx, |b, _, cx| b.refresh_local_servers(cx));
        }
        self.pane = pane;
        self.recording = None;
        self.shortcut_error = None;
        self.card_notice = None;
        (self.motion.hover, self.motion.pressed) = (false, false);
        self.default_browser = None;
        cx.notify();
    }

    fn found_panes(&self, cx: &App) -> Vec<SettingsPane> {
        let query = self.search.read(cx).text();
        PANES.iter().filter(|(_, _, name, _, words)| pane_matches(query, name, words)).map(|(pane, ..)| *pane).collect()
    }

    fn on_search(&mut self, event: &TextInputEvent, cx: &mut Context<Self>) {
        let found = self.found_panes(cx);
        let at = found.iter().position(|p| *p == self.pane);
        let pick = match event {
            TextInputEvent::Changed if at.is_none() => found.first(),
            TextInputEvent::Submit => at.or(Some(0)).and_then(|ix| found.get(ix)),
            TextInputEvent::Down => found.get(at.map_or(0, |ix| ix + 1)),
            TextInputEvent::Up => at.and_then(|ix| ix.checked_sub(1)).and_then(|ix| found.get(ix)),
            TextInputEvent::Cancel => {
                self.search.update(cx, |s, cx| s.set_text("", cx));
                None
            }
            _ => None,
        };
        match pick {
            Some(pane) => self.set_pane(*pane, cx),
            None => cx.notify(),
        }
    }

    fn install_from_github(&mut self, cx: &mut Context<Self>) {
        let repo = self.repo_input.read(cx).text().trim().to_owned();
        if !repo.is_empty() {
            self.with_browser(cx, move |b, _, cx| b.install_extension(crate::extensions::Install::Github(repo), None, cx));
        }
    }

    fn ask(
        &self,
        message: &str,
        detail: Option<&str>,
        answers: &[&str],
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, usize, &mut Context<Self>) + 'static,
    ) {
        let count = answers.len();
        let buttons: Vec<&str> = answers.iter().copied().chain(["Cancel"]).collect();
        let answer = crate::dialog::ask(PromptLevel::Warning, message, detail, &buttons, window, cx);
        cx.spawn(async move |this, cx| {
            if let Ok(ix) = answer.await
                && ix < count
            {
                this.update(cx, |this, cx| then(this, ix, cx)).ok();
            }
        })
        .detach();
    }

    fn set_level(&mut self, level: ShieldLevel, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.browser.upgrade().and_then(|b| b.read(cx).shield_target()) else { return };
        let apply = move |this: &mut Self, _: usize, cx: &mut Context<Self>| this.with_browser(cx, |b, w, cx| b.apply_profile_shield(target, level, w, cx));
        if self.browser.upgrade().is_some_and(|b| b.read(cx).shield_wipes_data(level)) {
            self.ask(&lockdown_question(name), Some(LOCKDOWN_WIPE), &["Use Lockdown"], window, cx, apply);
        } else {
            apply(self, 0, cx);
        }
    }

    /// Renders the card off the main thread and writes it to Downloads; nothing is uploaded.
    fn save_card(&mut self, card: Card, cx: &mut Context<Self>) {
        self.card_notice = Some("Saving…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let name = engine::downloads::sanitize_filename(&format!("Tarn Shield {}.png", card.name));
            let dark = !Theme::is_light();
            let png = cx.background_spawn(async move { shield_card::card_png(&card, dark) }).await;
            let notice = match png.map(|png| crate::capture::write_download(&name, &png)) {
                Some(Ok(path)) => format!("Saved to {}.", path.display()),
                Some(Err(e)) => format!("Could not save the image: {e}."),
                None => "Could not draw the image.".into(),
            };
            this.update(cx, |this, cx| {
                this.card_notice = Some(notice);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// macOS asks the user before changing the default browser, so the answer is polled for up
    /// to a minute; another click while it polls only asks macOS again.
    fn make_default(&mut self, cx: &mut Context<Self>) {
        engine::make_default_browser();
        if std::mem::replace(&mut self.polling_default, true) {
            return;
        }
        cx.spawn(async move |this, cx| {
            for _ in 0..120 {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let now = default_browser();
                if this
                    .update(cx, |this, cx| {
                        this.default_browser = Some(now);
                        cx.notify();
                    })
                    .is_err()
                    || now == Some(true)
                {
                    break;
                }
            }
            this.update(cx, |this, _| this.polling_default = false).ok();
        })
        .detach();
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.pane;
        let snapshot = self.snapshot(cx);
        let reduce = snapshot.as_ref().is_some_and(|s| s.reduce);
        let found = self.found_panes(cx);
        let searching = found.len() < PANES.len();
        let search_focused = self.search.focus_handle(cx).is_focused(window);
        // With no focus path, the window's own actions (Close Window, ⌘W, Esc) never reach it.
        if window.focused(cx).is_none() {
            window.focus(&self.focus);
        }

        let nav = div()
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .h_full()
            .pt(px(HEADER_HEIGHT))
            .px_2()
            .flex()
            .flex_col()
            .gap(px(2.))
            .bg(Theme::panel_nav())
            .border_r_1()
            .border_color(Theme::separator())
            .window_control_area(gpui::WindowControlArea::Drag)
            .child(field(search_focused).mb_1().occlude().py_0p5().child(self.search.clone()))
            .when(found.is_empty(), |d| d.child(note("No settings match.").px_2()))
            .children(PANES.iter().enumerate().filter(|(_, (pane, ..))| found.contains(pane)).flat_map(|(ix, &(pane, glyph, text, ..))| {
                let selected = pane == current;
                let group = match ix {
                    _ if searching => None,
                    0 => Some("YOU"),
                    YOU => Some("BROWSER"),
                    _ => None,
                };
                let caption = group.map(|g| {
                    div()
                        .px_2()
                        .pt(px(if ix == 0 { 4. } else { 12. }))
                        .pb_1()
                        .text_size(px(TEXT_TINY))
                        .font_weight(FontWeight::BOLD)
                        .text_color(Theme::faint())
                        .child(g)
                        .into_any_element()
                });
                let item = div()
                    .id(text)
                    .occlude()
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(px(Theme::radius_small()))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(Theme::selected()).font_weight(FontWeight::MEDIUM))
                    .when(!selected, |d| {
                        d.text_color(Theme::muted()).hover(|s| s.bg(Theme::hover()).text_color(Theme::text())).active(|s| s.bg(Theme::pressed()))
                    })
                    .child(icon(glyph, 15., if selected { Theme::text() } else { Theme::muted() }))
                    .child(label(text))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.focus);
                        this.set_pane(pane, cx);
                    }))
                    .into_any_element();
                caption.into_iter().chain([item])
            }));

        let body = match (current, snapshot) {
            (_, None) => note("The browser window is closed.").into_any_element(),
            (SettingsPane::Profile, Some(s)) => self.profile_tab(&s, window, cx),
            (SettingsPane::General, Some(s)) => self.general(&s, cx),
            (SettingsPane::Appearance, Some(s)) => self.appearance(&s, window, cx),
            (SettingsPane::Tabs, Some(s)) => self.tabs(&s, cx),
            (SettingsPane::Search, Some(s)) => self.search(&s, cx),
            (SettingsPane::Privacy, Some(s)) => self.privacy(s, cx),
            (SettingsPane::Permissions, Some(s)) => self.permissions(s, cx),
            (SettingsPane::Profiles, Some(s)) => self.profiles(&s, cx),
            (SettingsPane::Developer, Some(s)) => self.developer(s, cx),
            (SettingsPane::Extensions, Some(s)) => self.extensions(s, window, cx),
            (SettingsPane::Snippets, Some(s)) => self.snippets(&s, window, cx),
            (SettingsPane::Notes, Some(s)) => self.notes(&s, window, cx),
            (SettingsPane::Shortcuts, Some(_)) => self.shortcuts(cx),
            (SettingsPane::Ai, Some(s)) => self.ai(&s, window, cx),
        };
        let (name, subtitle) = PANES.iter().find(|(p, ..)| *p == current).map_or(("", ""), |(_, _, t, sub, _)| (*t, *sub));

        let content = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(PANE_HEADER_HEIGHT))
                    .flex_none()
                    .px_8()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(Theme::separator())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(div().text_size(px(TEXT_TITLE)).font_weight(FontWeight::SEMIBOLD).one_line(name))
                            .child(div().text_size(px(TEXT_SMALL)).text_color(Theme::muted()).one_line(subtitle)),
                    )
                    .when(cfg!(windows), |d| {
                        d.pr_2().window_control_area(gpui::WindowControlArea::Drag).child(div().flex_1()).children(ui::controls::window_controls())
                    }),
            )
            .child(fade_in(
                div()
                    .id("settings-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(div().w_full().max_w(px(CONTENT_WIDTH)).mx_auto().px_8().pb_10().child(body)),
                name,
                reduce,
            ));

        div()
            .key_context("Settings")
            .track_focus(&self.focus)
            .on_action(cx.listener(|_, _: &CloseTab, window, _| window.remove_window()))
            .on_action(cx.listener(|_, _: &crate::CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|_, _: &CloseOverlay, window, _| window.remove_window()))
            .on_action(cx.listener(|this, _: &NewProfile, _, cx| this.with_browser(cx, |b, w, cx| b.open_profile_editor(None, w, cx))))
            .on_action(cx.listener(|this, _: &EditProfile, _, cx| this.with_browser(cx, |b, w, cx| b.edit_current_profile(w, cx))))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let step = match event.keystroke.key.as_str() {
                    "up" => -1,
                    "down" => 1,
                    "tab" if this.recording.is_none() && event.keystroke.modifiers.shift => return window.focus_prev(),
                    "tab" if this.recording.is_none() => return window.focus_next(),
                    _ => return,
                };
                if !this.focus.is_focused(window) || this.recording.is_some() || event.keystroke.modifiers.modified() {
                    return;
                }
                let found = this.found_panes(cx);
                let at = found.iter().position(|p| *p == this.pane).unwrap_or(0) as isize;
                if let Some(pane) = found.get((at + step).min(found.len() as isize - 1).max(0) as usize) {
                    this.set_pane(*pane, cx);
                }
            }))
            .size_full()
            .flex()
            .bg(Theme::panel())
            .text_size(px(TEXT))
            .text_color(Theme::text())
            .font_family(Theme::font())
            .child(nav)
            .child(content)
    }
}

/// The minute is part of the key so the days shown move on, on the next repaint, without a new count.
type CardKey = (u64, i64, Option<Profile>, Option<(bool, Id)>);

/// The Shield card as it tilts under the pointer and flips on a click.
#[derive(Default)]
struct CardMotion {
    faces: Option<((Card, bool), Arc<shield_card::Faces>)>,
    /// The card whose faces are being drawn, or could not be.
    drawing: Option<(Card, bool)>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Where the pointer is over the card, each axis from -1 to 1.
    aim: (f32, f32),
    hover: bool,
    pressed: bool,
    flipped: bool,
    /// Tilt across and down, glow, flip and press as drawn now; each eases towards its goal.
    at: [f32; 5],
    stepped: Option<Instant>,
    drawn: Option<([f32; 5], Arc<shield_card::Faces>, Arc<RenderImage>)>,
}

/// Whether Tarn is the default browser; `None` outside an app bundle.
fn default_browser() -> Option<bool> {
    engine::bundle_identifier().map(|_| engine::is_default_browser())
}

const SIDEBAR_WIDTH: f32 = 196.;
/// Tall enough to clear the traffic lights, which sit 18 pt from the top.
const HEADER_HEIGHT: f32 = 52.;
const PANE_HEADER_HEIGHT: f32 = 64.;
const CONTENT_WIDTH: f32 = 760.;

impl SettingsWindow {
    fn general(&self, s: &Snapshot, cx: &mut Context<Self>) -> AnyElement {
        div()
            .child(section("Updates"))
            .child(if s.updater {
                group()
                    .child(self.pref_row(
                        true,
                        Pref::UpdateChecks,
                        "Check for updates automatically",
                        if cfg!(windows) {
                            "Once a day from GitHub. Updates are signed; Tarn offers to restart once one is downloaded."
                        } else {
                            "Once a day from GitHub. Updates are signed and install when you quit Tarn."
                        },
                        s.prefs.get(Pref::UpdateChecks),
                        cx,
                    ))
                    .child(self.pref_row(
                        false,
                        Pref::NightlyUpdates,
                        "Nightly builds",
                        "Signed builds of the latest changes, every day, before they are tested in a release. Switching back keeps this build until a newer release.",
                        s.prefs.get(Pref::NightlyUpdates),
                        cx,
                    ))
                    .child(Self::update_row(s))
            } else {
                group().child(row(true).child(label("Check for updates automatically")).child(value("Not set up in this build")))
            })
            .when(cfg!(feature = "chromium"), |d| d.child(section("Web engine")).child(group().child(self.web_engine_row(s, cx))))
            .child(section("Downloads"))
            .child(group().child(self.pref_row(
                true,
                Pref::SortDownloads,
                "Sort downloads into folders",
                "New files go into Images, Videos, Audio, Documents, Archives or Installers inside Downloads; other types stay at the top.",
                s.prefs.get(Pref::SortDownloads),
                cx,
            )))
            .when(browser_core::import::available(), |d| {
                d.child(section("Import")).child(
                    group().child(
                        row(true)
                            .child(text_block(
                                "Import browser data",
                                if cfg!(windows) {
                                    "History and bookmarks from Chrome, Edge, Brave or Firefox, into this profile."
                                } else {
                                    "Sign-ins, history and bookmarks from Chrome, Edge, Brave, Arc, Dia or Firefox, into this profile."
                                },
                            ))
                            .child(button("import-browser-data", "Import…", false).on_click(
                                cx.listener(|this, _, _, cx| this.with_browser(cx, |b, w, cx| b.open_library(crate::library::Page::Import, None, w, cx))),
                            )),
                    ),
                )
            })
            .child(section("About"))
            .child(
                group()
                    .child(row(true).child(label("Tarn version")).child(value(platform::updater::VERSION)))
                    .child(self.link_row("about-licence", browser_core::LICENCE, "Free software you may share and change.", browser_core::LICENCE_URL, cx))
                    .child(self.link_row("about-source", "Source code", "Tarn's full source is public on GitHub.", browser_core::SOURCE_URL, cx))
                    .child(self.link_row("about-privacy", "Privacy notice", "What Tarn stores and which connections it makes.", browser_core::PRIVACY_URL, cx))
                    .child(self.link_row("about-imprint", "Imprint", "Quavon UG (haftungsbeschränkt), Tarn's publisher.", browser_core::IMPRINT_URL, cx))
                    .when_some(licenses(), |d, path| {
                        d.child(
                            row(false)
                                .child(text_block("Open-source licences", "Chromium, CEF, Sparkle and the Rust crates Tarn is built from."))
                                .child(button("show-licenses", ui::controls::REVEAL, false).on_click(move |_, _, _| engine::reveal_in_finder(&path))),
                        )
                    }),
            )
            .into_any_element()
    }

    fn update_row(s: &Snapshot) -> Div {
        let check = button("update-check", "Check Now", false).on_click(|_, _, cx| crate::shared::check_for_updates(cx));
        let text = match (&s.update_ready, s.prefs.get(Pref::NightlyUpdates)) {
            (Some(version), _) => text_block(format!("Tarn {version} is ready"), "Downloaded and verified; it installs when Tarn restarts."),
            (None, true) => text_block("Nightly channel", "A new release reaches it as the next nightly build."),
            (None, false) => text_block("Release channel", "Tested releases only."),
        };
        row(false).child(text).child(div().flex().gap_2().child(check).when(s.update_ready.is_some(), |d| {
            d.child(button("update-restart", "Restart to Update", true).on_click(|_, _, cx| crate::shared::restart_to_update(cx)))
        }))
    }

    fn link_row(&self, id: &'static str, title: &'static str, detail: &'static str, url: &'static str, cx: &mut Context<Self>) -> Div {
        row(false).child(text_block(title, detail)).child(ui::controls::row_action(id, Icon::ExternalLink, "Open").on_click(cx.listener(
            move |this, _, _, cx| {
                if let Ok(url) = url::Url::parse(url) {
                    this.with_browser(cx, |b, w, cx| b.new_tab(&url, w, cx));
                }
            },
        )))
    }

    fn web_engine_row(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        let closed = s.chromium_closed.filter(|_| s.chromium_allowed);
        let detail = if let Some(closed) = closed {
            closed
        } else if s.chromium_allowed {
            "For new tabs and tabs restored without an engine of their own. Lockdown sites always open in WebKit."
        } else if s.updater {
            "Chromium can be the default only while Tarn checks for updates automatically."
        } else {
            "Chromium can become the default once Tarn checks for updates automatically, which this build does not."
        };
        let options = [WebEngine::WebKit, WebEngine::Chromium].into_iter().enumerate().map(|(ix, engine)| {
            let available = engine == WebEngine::WebKit || (s.chromium_allowed && closed.is_none());
            let (name, selected) = (crate::tabs::web_engine_name(engine), engine == s.web_engine);
            if !available && !selected {
                return disabled_segment(("web-engine", ix), name);
            }
            tab_stop(segment(("web-engine", ix), name, selected))
                .when(available, |d| d.on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.set_default_web_engine(engine, w, cx)))))
        });
        wide_row(true, text_block("Default web engine", detail), segmented().children(options))
    }

    fn appearance(&self, s: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme_accent = crate::themes::theme_accent(&crate::shared::core(cx)) >> 8;
        let colors = std::iter::once((crate::prefs::THEME_ACCENT, theme_accent)).chain(ACCENTS.iter().copied());
        let accents = div().flex().gap_2().children(colors.enumerate().map(|(ix, (name, color))| {
            let glyph = (ix == 0).then(|| icon(Icon::Palette, 11., Theme::on_fill(rgb(color))));
            tab_stop(swatch(("accent", ix), rgb(color), ix == s.accent, 18.))
                .children(glyph)
                .tooltip(ui::controls::tooltip(if ix == 0 { "The theme's own accent" } else { name }, None))
                .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.set_choice(Choice::Accent, ix, cx))))
        }));
        let compact = s.prefs.get(Pref::CompactToolbar) as usize;
        let motion = "Pop-ups, cards, tabs and the sidebar appear without animation, and the search bar effect holds still.";
        let reduce_motion = if s.reduce && !s.prefs.get(Pref::ReduceMotion) {
            let system = if cfg!(windows) {
                "On while animations are off in Windows settings."
            } else {
                "On while Reduce motion is on in macOS Accessibility settings."
            };
            row(true).child(text_block("Reduce motion", format!("{motion} {system}"))).child(switch(true).opacity(0.5)).into_any_element()
        } else {
            self.pref_row(true, Pref::ReduceMotion, "Reduce motion", motion, s.prefs.get(Pref::ReduceMotion), cx).into_any_element()
        };
        div()
            .child(section("Theme"))
            .child(group().children(self.theme_rows(cx)).child(row(false).child(text_block("Accent", "Buttons, selection and focus.")).child(accents)))
            .child(self.theme_sections(window, cx))
            .child(section("Design"))
            .child(group().children(self.design_rows(cx)))
            .child(section("Motion"))
            .child(group().child(reduce_motion).child(self.search_effect_row(false, cx)))
            .child(section("Tips"))
            .child(group().child(self.pref_row(true, Pref::FeatureHints, "Shortcut tips", browser_core::hints::NOTE, s.prefs.get(Pref::FeatureHints), cx)))
            .child(section("Layout"))
            .child(
                group()
                    .child(self.choice_row(
                        true,
                        "Toolbar",
                        "Compact puts the address field into a slimmer bar.",
                        &["Classic", "Compact"],
                        compact,
                        |b, ix, cx| b.set_pref(Pref::CompactToolbar, ix == 1, cx),
                        cx,
                    ))
                    .child(self.pref_row(
                        false,
                        Pref::CompactDensity,
                        "Compact density",
                        browser_core::prefs::COMPACT_NOTE,
                        s.prefs.get(Pref::CompactDensity),
                        cx,
                    ))
                    .child(self.pref_row(
                        false,
                        Pref::AutoHideSidebar,
                        "Hide sidebar automatically",
                        format!(
                            "With the sidebar hidden ({}), it slides in over the page while the pointer rests at the window's left edge.",
                            crate::shortcuts::shown(&crate::ToggleSidebar, cx)
                        ),
                        s.prefs.get(Pref::AutoHideSidebar),
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn tabs(&self, s: &Snapshot, cx: &mut Context<Self>) -> AnyElement {
        div()
            .child(section("Tabs"))
            .child(
                group()
                    .child(self.choice_row(
                        true,
                        "Tabs kept awake",
                        "Others sleep and reload when you return, unless they are playing video or audio; fewer saves memory.",
                        &LIVE_TABS,
                        s.live_tabs,
                        |b, ix, cx| b.set_choice(Choice::LiveTabs, ix, cx),
                        cx,
                    ))
                    .child(self.choice_row(
                        false,
                        "Sleep background tabs after",
                        "A tab left for this long sleeps and reloads when you return; past 1 GB of background tabs, the heaviest sleep after ten minutes (pinned tabs after 30). Tabs playing video or audio stay awake.",
                        &Choice::SleepAfter.options(),
                        s.sleep_after,
                        |b, ix, cx| b.set_choice(Choice::SleepAfter, ix, cx),
                        cx,
                    ))
                    .child(self.choice_row(
                        false,
                        "New tab opens",
                        format!("{} shows the command card with the search field, or the start page.", crate::shortcuts::shown(&crate::NewTab, cx)),
                        &["Command card", "Start page"],
                        s.prefs.get(Pref::NewTabStartPage) as usize,
                        |b, ix, cx| b.set_pref(Pref::NewTabStartPage, ix == 1, cx),
                        cx,
                    ))
                    .child(self.choice_row(
                        false,
                        "After closing a tab",
                        "Which neighbour in the list becomes active.",
                        &["Next", "Previous"],
                        s.prefs.get(Pref::ClosePrevious) as usize,
                        |b, ix, cx| b.set_pref(Pref::ClosePrevious, ix == 1, cx),
                        cx,
                    ))
                    .child(self.choice_row(
                        false,
                        "A tab dragged out of the window opens",
                        "A temporary window lists only that tab's own temporary space, which goes with the window; a full window lists every space and profile. Pinned tabs and tiles open in a full window, except from a temporary one.",
                        &["Temporary window", "Full window"],
                        usize::from(!s.prefs.get(Pref::TearOffTemporary)),
                        |b, ix, cx| b.set_pref(Pref::TearOffTemporary, ix == 0, cx),
                        cx,
                    ))
                    .child(self.choice_row(
                        false,
                        "Close unused tabs after",
                        "Unpinned tabs you have not opened for this long move to Library › Bookmarks › Archived Tabs.",
                        &Choice::ArchiveAfter.options(),
                        s.archive_after,
                        |b, ix, cx| b.set_choice(Choice::ArchiveAfter, ix, cx),
                        cx,
                    ))
                    .child(self.pref_row(
                        false,
                        Pref::Gestures,
                        if cfg!(windows) { "Touchpad gestures" } else { "Trackpad gestures" },
                        if cfg!(windows) {
                            "Swipe between pages and tabs. Swiping between pages follows this after Tarn restarts."
                        } else {
                            "Swipe between pages and tabs."
                        },
                        s.prefs.get(Pref::Gestures),
                        cx,
                    ))
                    .child(self.pref_row(
                        false,
                        Pref::AutoPictureInPicture,
                        "Picture in Picture when switching tabs",
                        "A playing video with sound keeps playing in a floating window while you are in another tab, and goes back into its page when you return.",
                        s.prefs.get(Pref::AutoPictureInPicture),
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn search(&self, s: &Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let name = s.profile.as_ref().map(|p| bounded_title(&p.name)).unwrap_or_default();
        let engines = group().child(row(true).child(segmented().children(ENGINES.iter().enumerate().map(|(ix, engine)| {
            tab_stop(segment(("engine", ix), engine.name, ix == s.engine))
                .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.set_search_engine(ix, cx))))
        }))));
        div()
            .child(section("Search engine"))
            .child(engines)
            .child(note(format!("Applies to the profile “{name}”. Typed words that are not an address go to this engine.")))
            .child(section("Suggestions"))
            .child(
                group()
                    .child(self.pref_row(true, Pref::Suggestions, "Suggest open tabs and history", format!("Shown under the command field while you type. Nothing you type leaves {} before you press {}.", ui::controls::THIS_DEVICE, if cfg!(windows) { "Enter" } else { "Return" }), s.prefs.get(Pref::Suggestions), cx))
                    .child(self.pref_row(false, Pref::SearchShortcuts, "Search shortcuts", browser_core::settings::SEARCH_SHORTCUTS_NOTE, s.prefs.get(Pref::SearchShortcuts), cx))
                    .child(self.pref_row(false, Pref::HistoryText, "Search the text of visited pages", format!("Keeps up to 4 KB of each page's text on {} so history suggestions match what a page said. Never for the private space or Lockdown pages; cleared with history, and when you turn this off.", ui::controls::THIS_DEVICE), s.prefs.get(Pref::HistoryText), cx)),
            )
            .into_any_element()
    }
}

/// Arms on the first call for `target`; returns true on a second call for the same target.
pub(crate) fn confirm_twice<T: PartialEq>(armed: &mut Option<T>, target: T) -> bool {
    if armed.take().is_some_and(|a| a == target) {
        return true;
    }
    *armed = Some(target);
    false
}

#[cfg(test)]
mod tests {
    use super::confirm_twice;

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(super::thousands(0), "0");
        assert_eq!(super::thousands(999), "999");
        assert_eq!(super::thousands(1000), "1,000");
        assert_eq!(super::thousands(502_431), "502,431");
        assert_eq!(super::thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn search_finds_panes_by_word_starts() {
        let found = |q: &str| {
            super::PANES.iter().filter(|(_, _, name, _, words)| super::pane_matches(q, name, words)).map(|(_, _, name, ..)| *name).collect::<Vec<_>>()
        };
        assert_eq!(found("lockd"), ["Protection"]);
        assert_eq!(found("COOKIE ban"), ["Privacy"]);
        assert_eq!(found("density"), ["Appearance"]);
        assert_eq!(found("").len(), super::PANES.len());
        assert!(found("ock").is_empty(), "words match from their start");
    }

    #[test]
    fn clear_confirmation_is_bound_to_one_profile() {
        let mut armed = None;
        assert!(!confirm_twice(&mut armed, (false, 1)));
        assert!(!confirm_twice(&mut armed, (false, 2)), "a different profile re-arms instead of clearing");
        assert!(confirm_twice(&mut armed, (false, 2)));
        assert_eq!(armed, None);
    }

    #[test]
    fn shortcuts_list_every_key_binding() {
        let listed = crate::shortcuts::sections()
            .iter()
            .flat_map(|(_, items)| items.iter())
            .map(|(_, keys)| match keys {
                super::Fixed(keys) => (*keys).to_owned(),
                super::Bound(action) => crate::shortcuts::display(&browser_core::shortcuts::default(&crate::shortcuts::MAC, *action)),
            })
            .chain(crate::shortcuts::FIXED.iter().map(|(_, _, keys)| (*keys).to_owned()))
            .collect::<Vec<_>>()
            .join("  ");
        for binding in include_str!("lib.rs").replace("KeyBinding::new(&shortcuts::platform(\"", "KeyBinding::new(\"").split("KeyBinding::new(\"").skip(1) {
            let binding = binding.split('"').next().unwrap_or_default();
            let (mods, key) = match binding.strip_suffix("--") {
                Some(mods) => (mods, "-"),
                None => binding.rsplit_once('-').unwrap_or(("", binding)),
            };
            let symbols: String = [("ctrl", "⌃"), ("alt", "⌥"), ("shift", "⇧"), ("cmd", "⌘")]
                .iter()
                .filter(|(m, _)| mods.split('-').any(|p| p == *m || (p == "secondary" && *m == "cmd")))
                .map(|(_, s)| *s)
                .collect();
            let key = match key {
                "escape" => "Esc".to_owned(),
                "tab" => "⇥".to_owned(),
                "left" => "←".to_owned(),
                "right" => "→".to_owned(),
                k if mods == "ctrl" && k.parse::<u8>().is_ok() => "1".to_owned(),
                k => k.to_uppercase(),
            };
            let shown = format!("{symbols}{key}");
            assert!(listed.split_whitespace().any(|k| k == shown), "{binding} is missing from SHORTCUTS");
        }
    }
}
