use browser_core::library::HistoryRange;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, Div, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    MouseMoveEvent, ParentElement, Pixels, PromptLevel, Render, RenderImage, SharedString, Stateful, StatefulInteractiveElement, Styled, Subscription,
    TitlebarOptions, WeakEntity, Window, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowOptions, canvas, div, img, point,
    prelude::FluentBuilder, px, rgb, size,
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

mod appearance;
use crate::filters;
use crate::prefs::{Choice, LIVE_TABS, Pref, Prefs};
use crate::profiles::{EditProfile, NewProfile, profile_dot};
use crate::shield_card::{self, Card};
use crate::shortcuts::Keys::{self, Bound, Fixed};
use crate::toolbar::shield_badge;
use crate::{CloseOverlay, CloseTab};

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
    Shortcuts,
}

const PANES: [(SettingsPane, Icon, &str, &str); 11] = [
    (SettingsPane::Profile, Icon::Shield, "Protection", "shield level standard strict lockdown card filter lists site overrides connections default browser"),
    (SettingsPane::Profiles, Icon::Person, "Profiles", "profile new edit delete space rules"),
    (SettingsPane::General, Icon::Gear, "General", "updates nightly web engine chromium webkit downloads import about version licence source privacy imprint"),
    (
        SettingsPane::Appearance,
        Icon::Palette,
        "Appearance",
        "theme dark light match system accent colour color motion reduce toolbar compact sidebar search bar effect",
    ),
    (
        SettingsPane::Tabs,
        Icon::AppWindow,
        "Tabs",
        "awake sleep memory new tab start page close archive gestures swipe trackpad touchpad drag tear off temporary full window",
    ),
    (SettingsPane::Search, Icon::Search, "Search", "search engine suggestions history text"),
    (
        SettingsPane::Privacy,
        Icon::Hand,
        "Privacy",
        "block ads trackers cookie banners youtube pop-ups malware adult zoom muted hidden elements connections history clear export delete data localhost",
    ),
    (SettingsPane::Permissions, Icon::Camera, "Permissions", "camera microphone location pop-ups site answers allow block"),
    (SettingsPane::Developer, Icon::Code, "Developer", "web inspector developer tools profile local servers localhost"),
    (SettingsPane::Extensions, Icon::Braces, "Extensions", "install github file bitwarden every site official catalog store updates"),
    (SettingsPane::Shortcuts, Icon::Keyboard, "Shortcuts", "keyboard keys"),
];

fn pane_matches(query: &str, name: &str, words: &str) -> bool {
    browser_core::settings::words_match(query, &format!("{name} {words}"))
}

pub(crate) use browser_core::profiles::{LOCKDOWN_WIPE, lockdown_question};

const YOU: usize = 2;

type Pick = fn(&mut Browser, usize, &mut Context<Browser>);

pub(crate) const SHORTCUTS: [(&str, &[(&str, Keys)]); 4] = [
    (
        "Tabs",
        &[
            ("New tab", Bound("cmd-t", &crate::NewTab)),
            ("Open location", Bound("cmd-l", &crate::OpenLocation)),
            ("Open file", Bound("cmd-o", &crate::OpenFile)),
            ("Close tab", Bound("cmd-w", &crate::CloseTab)),
            ("Reopen closed tab", Bound("cmd-shift-t", &crate::ReopenClosedTab)),
            ("Close all tabs", Bound("cmd-shift-k", &crate::CloseAllTabs)),
            ("Select tab 1–8", Fixed("⌘1 ⌘2 ⌘3 ⌘4 ⌘5 ⌘6 ⌘7 ⌘8")),
            ("Select last tab", Bound("cmd-9", &crate::LastTab)),
            ("Next tab", Bound("ctrl-tab", &crate::NextTab)),
            ("Previous tab", Bound("ctrl-shift-tab", &crate::PreviousTab)),
            ("Next or previous tab, other keys", Fixed("⌘} ⌘{ ⌥⌘→ ⌥⌘←")),
            ("Pin or unpin tab", Bound("cmd-d", &crate::TogglePin)),
            ("New tab group", Bound("cmd-alt-g", &crate::NewGroup)),
            ("Open or close split view", Bound("cmd-ctrl-n", &crate::split::ToggleSplitView)),
            ("Focus next split pane", Bound("cmd-ctrl-]", &crate::split::FocusNextPane)),
            ("Focus previous split pane", Bound("cmd-ctrl-[", &crate::split::FocusPreviousPane)),
            ("Swap split panes", Bound("cmd-ctrl-s", &crate::split::SwapPanes)),
            ("Rotate split layout", Bound("cmd-ctrl-r", &crate::split::RotateSplitLayout)),
            ("Remove tab from split view", Bound("cmd-ctrl-w", &crate::split::RemoveFromSplitView)),
        ],
    ),
    (
        "Page",
        &[
            ("Reload", Bound("cmd-r", &crate::Reload)),
            ("Reload without cache", Bound("cmd-shift-r", &crate::ReloadFromOrigin)),
            ("Stop loading", Bound("cmd-.", &crate::StopLoading)),
            ("Back", Bound("cmd-[", &crate::Back)),
            ("Forward", Bound("cmd-]", &crate::Forward)),
            ("Find in page", Bound("cmd-f", &crate::Find)),
            ("Find next", Bound("cmd-g", &crate::FindNext)),
            ("Find previous", Bound("cmd-shift-g", &crate::FindPrevious)),
            ("Zoom in", Bound("cmd-=", &crate::ZoomIn)),
            ("Zoom in, other key", Fixed("⌘+")),
            ("Zoom out", Bound("cmd--", &crate::ZoomOut)),
            ("Actual size", Bound("cmd-0", &crate::ZoomReset)),
            ("Copy link", Bound("cmd-shift-c", &crate::CopyLink)),
            ("Copy clean link", Bound("ctrl-shift-cmd-c", &crate::CopyCleanLink)),
            ("Copy link as Markdown", Bound("cmd-shift-alt-c", &crate::capture::CopyMarkdownLink)),
            ("Take screenshot", Bound("cmd-alt-s", &crate::capture::TakeScreenshot)),
            ("Reader", Bound("alt-cmd-r", &crate::reader::ToggleReader)),
            ("Picture in Picture", Bound("cmd-ctrl-p", &crate::capture::PictureInPicture)),
            ("Print", Bound("cmd-p", &crate::Print)),
            ("Open link in a peek", Fixed("⇧-click")),
        ],
    ),
    (
        "Library",
        &[
            ("Bookmark this tab", Bound("cmd-shift-d", &crate::BookmarkTab)),
            ("Show library", Bound("cmd-shift-a", &crate::ToggleLibrary)),
            ("Show downloads", Bound("cmd-shift-j", &crate::ToggleDownloads)),
            ("Manage bookmarks", Bound("cmd-alt-b", &crate::menubar::ManageBookmarks)),
            ("Show history", Bound("cmd-y", &crate::menubar::ShowHistory)),
        ],
    ),
    (
        "Window",
        &[
            ("New window", Bound("cmd-n", &crate::NewWindow)),
            ("Close window", Bound("cmd-shift-w", &crate::CloseWindow)),
            ("Toggle sidebar", Bound("cmd-s", &crate::ToggleSidebar)),
            ("New space", Bound("cmd-shift-s", &crate::NewSpace)),
            ("Private space", Bound("cmd-shift-n", &crate::PrivateSpace)),
            ("Burner tab", Bound("cmd-alt-shift-n", &crate::BurnerTab)),
            ("Switch to space 1–9", Fixed("⌃1 … ⌃9")),
            ("Minimize window", Bound("cmd-m", &crate::menubar::Minimize)),
            ("Enter or exit full screen", Bound("cmd-ctrl-f", &crate::ToggleFullScreen)),
            ("Hide Tarn", Bound("cmd-h", &crate::HideTarn)),
            ("Hide others", Bound("cmd-alt-h", &crate::HideOthers)),
            ("Settings", Bound("cmd-,", &crate::Settings)),
            ("Keyboard shortcuts", Bound("cmd-/", &crate::shortcuts::ToggleShortcuts)),
            ("Open the menu", Fixed("F10")),
            ("Move between page, address field and sidebar", Fixed("F6  ⇧F6")),
            ("Close command bar, overlay, popover or link peek", Fixed("Esc")),
            ("Undo / Redo", Fixed("⌘Z  ⇧⌘Z")),
            ("Quit", Fixed("⌘Q")),
        ],
    ),
];

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
    update_checks: bool,
    chromium_allowed: bool,
    chromium_closed: Option<&'static str>,
    profile: Option<Profile>,
    private: bool,
    overrides: Vec<(String, ShieldLevel)>,
    zooms: Vec<(String, f64)>,
    muted: Vec<String>,
    permissions: Vec<browser_core::permissions::SitePermission>,
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
    recording: Option<(&'static dyn gpui::Action, Subscription)>,
    shortcut_error: Option<String>,
    appearance: appearance::Pane,
    _inputs: [Subscription; 2],
    _observe: Option<Subscription>,
    _activation: Subscription,
    _release: Subscription,
}

impl SettingsWindow {
    pub fn open(browser: WeakEntity<Browser>, main: WindowHandle<Browser>, cx: &mut App) -> Option<WindowHandle<SettingsWindow>> {
        let sheet = size(px(1000.), px(700.));
        let over = main.update(cx, |_, window, cx| (window.bounds(), window.display(cx).map(|d| d.bounds()))).ok();
        let bounds = match over {
            Some((over, Some(screen))) => {
                let sheet = size(sheet.width.min(screen.size.width), sheet.height.min(screen.size.height));
                let at = over.center() - point(sheet.width / 2., sheet.height / 2.);
                let x = at.x.clamp(screen.origin.x, screen.right() - sheet.width);
                let y = at.y.clamp(screen.origin.y, screen.bottom() - sheet.height);
                Bounds::new(point(x, y), sheet)
            }
            _ => Bounds::centered(None, sheet, cx),
        };
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(px(720.), px(480.))),
            titlebar: Some(TitlebarOptions {
                title: Some("Settings".into()),
                appears_transparent: true,
                // AppKit only hit-tests the buttons inside its titlebar strip, so they stay within it.
                traffic_light_position: Some(point(px(18.), px(10.))),
            }),
            window_background: WindowBackgroundAppearance::Opaque,
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
            update_checks: b.core.update_checks(),
            chromium_allowed: b.core.chromium_allowed(),
            chromium_closed: b.chromium_closed(),
            profile: b.current_profile(),
            private: b.model.current_space().is_some_and(|s| s.private),
            overrides: if matches!(self.pane, SettingsPane::Privacy | SettingsPane::Profile) { b.site_overrides() } else { Vec::new() },
            zooms: if self.pane == SettingsPane::Privacy { b.site_zooms() } else { Vec::new() },
            muted: if self.pane == SettingsPane::Privacy { b.muted_sites() } else { Vec::new() },
            permissions: if self.pane == SettingsPane::Permissions { b.profile_permissions() } else { Vec::new() },
            zapped: if self.pane == SettingsPane::Privacy { b.zapped(None) } else { Vec::new() },
            paused: if self.pane == SettingsPane::Privacy { b.paused_sites() } else { Vec::new() },
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
        PANES.iter().filter(|(_, _, name, words)| pane_matches(query, name, words)).map(|(pane, ..)| *pane).collect()
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

    fn shortcuts(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let stored = crate::shortcuts::stored(&crate::shared::core(cx).disk);
        let defaults = crate::shortcuts::keymap(&Default::default());
        let changed: Vec<&dyn gpui::Action> =
            crate::shortcuts::keymap(&stored).into_iter().zip(&defaults).filter(|((k, _), (d, _))| k != d).map(|((_, a), _)| a).collect();
        let recording = self.recording.as_ref().map(|(a, _)| *a);
        let mut n = 0usize;
        let mut pane = div()
            .child(
                div()
                    .pt_3()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        note(if cfg!(windows) {
                            "Click a shortcut and press the new keys, with Ctrl, or Alt with Shift or a digit; Esc cancels."
                        } else {
                            "Click a shortcut and press the new keys, with ⌘ or ⌃; Esc cancels."
                        })
                        .flex_grow()
                        .flex_basis(px(0.))
                        .pt_0(),
                    )
                    .when(!changed.is_empty(), |d| {
                        d.child(button("shortcut-reset-all", "Reset All", false).on_click(cx.listener(|this, _, _, cx| this.reset_shortcuts(None, cx))))
                    }),
            )
            .children(self.shortcut_error.clone().map(|e| note(e).text_color(Theme::warning())));
        for (title, items) in SHORTCUTS.iter() {
            let mut list = group();
            for (ix, (name, keys)) in items.iter().enumerate() {
                let mut line = row(ix == 0).child(label(*name));
                line = match keys {
                    keys if !crate::shortcuts::listed(keys) => continue,
                    Fixed(keys) => line.child(value(ui::controls::keys(keys))),
                    Bound(_, action) => {
                        let action = *action;
                        n += 1;
                        let on = recording.is_some_and(|a| a.partial_eq(action));
                        let custom = changed.iter().any(|a| a.partial_eq(action));
                        let shown = if on { "Press keys…".to_owned() } else { crate::shortcuts::shown(action, cx) };
                        line.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(custom, |d| {
                                    d.child(
                                        button(("shortcut-reset", n), "Reset", false)
                                            .on_click(cx.listener(move |this, _, _, cx| this.reset_shortcuts(Some(action), cx))),
                                    )
                                })
                                .child(button(("shortcut", n), shown, on).on_click(cx.listener(move |this, _, window, cx| this.record(action, window, cx)))),
                        )
                    }
                };
                list = list.child(line);
            }
            pane = pane.child(section(*title)).child(list);
        }
        pane.into_any_element()
    }

    /// Catches the next keystroke in the Settings window as `action`'s new keys; a key in another
    /// window stops recording and goes on as usual.
    fn record(&mut self, action: &'static dyn gpui::Action, window: &mut Window, cx: &mut Context<Self>) {
        let me = cx.entity().downgrade();
        let settings = window.window_handle().window_id();
        let interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle().window_id() != settings {
                me.update(cx, |this, cx| {
                    if let Some((_, interceptor)) = this.recording.take() {
                        cx.defer(move |_| drop(interceptor));
                        cx.notify();
                    }
                })
                .ok();
                return;
            }
            cx.stop_propagation();
            let keystroke = event.keystroke.clone();
            if me.update(cx, |this, cx| this.recorded(&keystroke, cx)).unwrap_or(false) {
                cx.defer(crate::shortcuts::rebind);
            }
        });
        self.recording = Some((action, interceptor));
        self.shortcut_error = None;
        cx.notify();
    }

    /// Saves `keystroke` for the recording action unless it is Esc, invalid or taken; true when saved.
    fn recorded(&mut self, keystroke: &gpui::Keystroke, cx: &mut Context<Self>) -> bool {
        let Some((action, interceptor)) = self.recording.take() else { return false };
        cx.defer(move |_| drop(interceptor));
        cx.notify();
        if keystroke.key == "escape" && keystroke.modifiers == gpui::Modifiers::none() {
            return false;
        }
        if cfg!(windows) && keystroke.modifiers.platform {
            self.shortcut_error = Some("The Windows key belongs to Windows; use Ctrl, or Alt with Shift or a digit.".to_owned());
            return false;
        }
        use crate::shortcuts::{bindable, conflict, display, keymap, parse_keys, setting_key, stored};
        let disk = &crate::shared::core(cx).disk;
        let keys = match parse_keys(&crate::shortcuts::canonical(&keystroke.unparse())) {
            Ok(keys) => keys,
            Err(e) => {
                self.shortcut_error = Some(e.to_owned());
                return false;
            }
        };
        if let Some(user) = conflict(&keys, action, &keymap(&stored(disk))) {
            self.shortcut_error = Some(format!("{} is already used by {user}.", ui::controls::keys(&display(&keys))));
            return false;
        }
        let default = bindable().find(|(_, _, a)| a.partial_eq(action)).and_then(|(_, keys, _)| parse_keys(keys).ok());
        let saved =
            if default.as_deref() == Some(keys.as_str()) { disk.delete_setting(&setting_key(action)) } else { disk.set_setting(&setting_key(action), &keys) };
        if let Err(e) = saved {
            self.shortcut_error = Some(format!("The shortcut could not be saved ({e})."));
        }
        true
    }

    /// Back to the default keys for `action`, unless another action took them meanwhile, or for
    /// every action.
    fn reset_shortcuts(&mut self, action: Option<&'static dyn gpui::Action>, cx: &mut Context<Self>) {
        use crate::shortcuts::{bindable, conflict, display, keymap, parse_keys, setting_key, stored};
        let disk = &crate::shared::core(cx).disk;
        self.recording = None;
        self.shortcut_error = None;
        let default = action.and_then(|action| bindable().find(|(_, _, a)| a.partial_eq(action))).and_then(|(_, keys, _)| parse_keys(keys).ok());
        if let (Some(action), Some(keys)) = (action, &default)
            && let Some(user) = conflict(keys, action, &keymap(&stored(disk)))
        {
            self.shortcut_error = Some(format!("{} is now used by {user}; change that first.", ui::controls::keys(&display(keys))));
            return cx.notify();
        }
        for (_, _, a) in bindable().filter(|(_, _, a)| action.is_none_or(|action| a.partial_eq(action))) {
            if let Err(e) = disk.delete_setting(&setting_key(a)) {
                self.shortcut_error = Some(format!("The shortcut could not be reset ({e})."));
            }
        }
        cx.defer(crate::shortcuts::rebind);
        cx.notify();
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
            .children(PANES.iter().enumerate().filter(|(_, (pane, ..))| found.contains(pane)).flat_map(|(ix, &(pane, glyph, text, _))| {
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
            (SettingsPane::Shortcuts, Some(_)) => self.shortcuts(cx),
        };
        let name = PANES.iter().find(|(p, ..)| *p == current).map_or("", |(_, _, t, _)| *t);

        let content = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(HEADER_HEIGHT))
                    .flex_none()
                    .px_8()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(Theme::separator())
                    .text_size(px(TEXT_TITLE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(div().flex_1().one_line(name))
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
                    .child(div().w_full().max_w(px(CONTENT_WIDTH)).mx_auto().px_8().pt_1().pb_8().child(body)),
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
const CONTENT_WIDTH: f32 = 760.;

fn week_chart(card: &Card) -> Div {
    const BARS: f32 = 92.;
    let days = browser_core::shield_card::day_totals(&card.week);
    if days.iter().all(|(_, n)| *n <= 0) {
        return group().child(row(true).child(text_block("Last 7 days", "Nothing stopped yet. Each day Shield stops something shows here.")));
    }
    let busiest = days.iter().map(|(_, n)| *n).max().unwrap_or(0);
    let bars = days.iter().map(|(day, n)| {
        let fill = if *n == 0 { 2. } else { (BARS * *n as f32 / busiest.max(1) as f32).max(4.) };
        let top = *n == busiest && busiest > 0;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_end()
            .gap_1()
            .text_size(px(TEXT_TINY))
            .text_color(Theme::muted())
            .child(if *n > 0 { thousands(*n as u64) } else { String::new() })
            .child(div().w(px(22.)).h(px(fill)).rounded(px(Theme::radius_small())).bg(if top { Theme::space_label() } else { Theme::accent() }))
            .child(*day)
    });
    let total: i64 = days.iter().map(|(_, n)| *n).sum();
    group()
        .p_4()
        .gap_3()
        .child(
            div()
                .flex()
                .justify_between()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Last 7 days"))
                .child(value(format!("{} protected", thousands(total.max(0) as u64))).text_size(px(TEXT_SMALL))),
        )
        .child(div().h(px(BARS + 34.)).flex().items_end().gap_1().children(bars))
}

fn protected_sites(card: &Card) -> Div {
    let most = card.sites.iter().map(|(_, n)| *n).max().unwrap_or(0).max(1);
    let rows = card.sites.iter().map(|(host, n)| {
        let host = crate::toolbar::host_tail(&bounded_title(host), 40);
        let share = (*n).max(0) as f32 / most as f32;
        div()
            .h(px(28.))
            .flex()
            .items_center()
            .gap_2()
            .child(ui::controls::favicon(&host, 16.))
            .child(div().flex_none().w(px(110.)).one_line(host))
            .child(
                div()
                    .flex_1()
                    .h(px(4.))
                    .rounded_full()
                    .bg(Theme::separator())
                    .child(div().h_full().w(gpui::relative(share)).rounded_full().bg(Theme::accent())),
            )
            .child(value(thousands((*n).max(0) as u64)).w(px(44.)).flex_none().text_right())
    });
    group().p_4().gap_1().child(div().pb_1().font_weight(FontWeight::SEMIBOLD).child("Most protected sites")).children(rows)
}

impl SettingsWindow {
    /// The card's next frame: draws its faces off the main thread when the card changes, moves
    /// the tilt, glow, flip and press a step towards their goals, and asks for another frame
    /// until they arrive.
    fn card_frame(&mut self, card: &Card, reduce: bool, window: &mut Window, cx: &mut Context<Self>) -> Option<Arc<RenderImage>> {
        const RATES: [f32; 5] = [12., 12., 8., 6.5, 25.];
        let m = &mut self.motion;
        let key = (card.clone(), !Theme::is_light());
        if m.faces.as_ref().is_none_or(|(k, _)| *k != key) && m.drawing.as_ref() != Some(&key) {
            m.drawing = Some(key.clone());
            let card = key;
            cx.spawn(async move |this, cx| {
                let made = cx.background_spawn({
                    let (card, dark) = card.clone();
                    async move { shield_card::faces(&card, dark) }
                });
                let Some(faces) = made.await else { return };
                this.update(cx, |this, cx| {
                    if this.motion.drawing.as_ref() == Some(&card) {
                        this.motion.drawing = None;
                        this.motion.faces = Some((card, Arc::new(faces)));
                        cx.notify();
                    }
                })
                .ok();
            })
            .detach();
        }
        let faces = m.faces.as_ref()?.1.clone();
        let flip = if m.flipped { 1. } else { 0. };
        let goal = if reduce || !m.hover { [0., 0., 0., flip, 0.] } else { [m.aim.0, m.aim.1, 1., flip, if m.pressed { 1. } else { 0. }] };
        let passed = m.stepped.replace(Instant::now()).map_or(0.05, |t| t.elapsed().as_secs_f32()).min(0.05);
        for ((at, goal), rate) in m.at.iter_mut().zip(goal).zip(RATES) {
            *at = if reduce || (goal - *at).abs() < 0.002 { goal } else { *at + (goal - *at) * (1. - (-passed * rate).exp()) };
        }
        if m.at != goal {
            window.request_animation_frame();
        }
        if m.drawn.as_ref().is_none_or(|(at, from, _)| *at != m.at || !Arc::ptr_eq(from, &faces)) {
            let [across, down, glow, flip, press] = m.at;
            let image =
                Arc::new(RenderImage::new([image::Frame::new(shield_card::frame(&faces, &ui::tilt::Pose { tilt: (across, down), glow, flip, press }))]));
            if let Some((_, _, old)) = m.drawn.replace((m.at, faces, image)) {
                cx.drop_image(old, Some(window));
            }
        }
        m.drawn.as_ref().map(|(_, _, image)| image.clone())
    }

    /// The first pane: the profile's Shield card, its week, and what Tarn does for it and talks to.
    fn profile_tab(&mut self, s: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(card) = s.card.clone() else { return note("No profile is open.").into_any_element() };
        let target = s.profile.as_ref().map(|p| (s.private, p.id));
        let frame = self.card_frame(&card, s.reduce, window, cx);
        let [_, _, glow, flip, _] = self.motion.at;
        let facing = (flip * std::f32::consts::PI).cos().abs();
        let (width, height, margin) = (shield_card::W as f32, shield_card::H as f32, shield_card::MARGIN as f32);
        let bounds = self.motion.bounds.clone();
        let face = div()
            .id("shield-card")
            .relative()
            .w(px(width))
            .h(px(height))
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .h_full()
                    .left(px(width / 2. * (1. - facing)))
                    .w(px(width * facing))
                    .rounded(px(18.))
                    .shadow(Theme::elevation(5. + 5. * glow, 9. + 7. * glow)),
            )
            .children(frame.map(|f| img(f).absolute().left(px(-margin)).top(px(-margin)).w(px(width + 2. * margin)).h(px(height + 2. * margin))))
            .child(canvas(move |at, _, _| bounds.set(at), |_, _, _, _| {}).absolute().size_full())
            .on_mouse_move(cx.listener(|this, moved: &MouseMoveEvent, _, cx| {
                let card = this.motion.bounds.get();
                let from_centre = moved.position - card.center();
                this.motion.aim = ((from_centre.x / (card.size.width / 2.)).clamp(-1., 1.), (from_centre.y / (card.size.height / 2.)).clamp(-1., 1.));
                cx.notify();
            }))
            .on_hover(cx.listener(|this, over: &bool, _, cx| {
                this.motion.hover = *over;
                this.motion.pressed &= *over;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.motion.pressed = true;
                    cx.notify();
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.motion.pressed = false;
                this.motion.flipped ^= true;
                cx.notify();
            }));
        let hero = self.hero(&card, face, target, cx);
        let mut page = div()
            .child(hero)
            .children(self.card_notice.clone().map(|n| note(n).pt_2()))
            .child(div().pt_5().flex().gap_4().child(week_chart(&card).flex_1()).when(!card.sites.is_empty(), |d| d.child(protected_sites(&card).flex_1())));
        if s.default_browser == Some(false) {
            page = page.child(section("Default browser")).child(
                group().child(
                    row(true)
                        .child(text_block("Tarn isn't your default browser", "Links from other apps open in another browser."))
                        .child(button("make-default", "Make Default", true).on_click(cx.listener(|this, _, _, cx| this.make_default(cx)))),
                ),
            );
        }
        let levels = self.shield_levels(s, cx);
        let to_privacy =
            |id: &'static str, text: &'static str| button(id, text, false).on_click(cx.listener(|this, _, _, cx| this.set_pane(SettingsPane::Privacy, cx)));
        let overrides = match s.overrides.len() {
            0 => "None".to_owned(),
            1 => "1 site".to_owned(),
            n => format!("{n} sites"),
        };
        page = page
            .child(note({
                let kept = if s.private { "Counts stay in memory and are gone once the private space closes.".to_owned() } else { format!("Counted on {} only and never sent anywhere.", ui::controls::THIS_DEVICE) };
                if cfg!(windows) {
                    format!("{kept} Only what Tarn stops itself counts.")
                } else {
                    format!("{kept} Only what Tarn stops itself counts: ad and tracker blocks in WebKit tabs are not counted, because WebKit does not report what its blocking rules stop.")
                }
            }))
            .child(section("Shield level"))
            .child(levels)
            .child(note("Applies to this profile. Open pages reload with the new level."))
            .child(section("Shield"))
            .child(
                group()
                    .child(row(true).child(label("Site overrides")).child(value(overrides)).child(to_privacy("shield-change", "Change…")))
                    .child(row(false).child(text_block("Filter lists", s.filters.summary()))),
            );
        let filter_lists = match (filters::configured(), s.filters.updates) {
            (false, _) => "Not set up",
            (true, true) => "GitHub, once a day",
            (true, false) => "Off",
        };
        let updates = match (s.updater, s.update_checks) {
            (false, _) => "Not set up",
            (true, true) => "GitHub, once a day",
            (true, false) => "Only when you ask",
        };
        page.child(section("Connections Tarn makes"))
            .child(
                group()
                    .child(row(true).child(label("Filter-list updates")).child(value(filter_lists)))
                    .child(row(false).child(label("Extensions and themes")).child(value(if s.extension_downloads {
                        "GitHub, when you install"
                    } else {
                        "Off"
                    })))
                    .child(row(false).child(label("Site icons")).child(value("The site you are on or its CDN")))
                    .child(row(false).child(label("Update checks")).child(value(updates)))
                    .child(
                        row(false)
                            .child(text_block("Nothing else", "No accounts, no telemetry, no AI requests."))
                            .child(to_privacy("connections-details", "Details…")),
                    ),
            )
            .into_any_element()
    }

    fn hero(&self, card: &Card, face: impl IntoElement, target: Option<(bool, Id)>, cx: &mut Context<Self>) -> Div {
        let private = target.is_some_and(|(private, _)| private);
        let since = match (card.days, &card.since) {
            (Some(days), Some(since)) => browser_core::shield_card::protected_text(days, since),
            _ if private => "Counts stay in memory until the private space closes.".to_owned(),
            _ => "Counting starts with the first thing Shield stops.".to_owned(),
        };
        let saved = card.clone();
        let shared = card.clone();
        let system = cfg!(target_os = "macos");
        let share = if system { "Share Card" } else { "Save as Image" };
        let action = |id: &'static str, glyph, text: &'static str| tab_stop(icon_text_button(id, glyph, text)).justify_center();
        let actions = div()
            .grid()
            .grid_cols(2)
            .gap_2()
            .child(action("flip-card", Icon::Reload, "Flip").on_click(cx.listener(|this, _, _, cx| {
                this.motion.flipped ^= true;
                cx.notify();
            })))
            .child(action("new-design", Icon::Palette, "Change Design").on_click(cx.listener(move |this, _, _, cx| {
                let Some((private, id)) = target else { return };
                this.with_browser(cx, |b, _, cx| {
                    b.store(private).new_shield_design(id).ok();
                    shield_card::touch();
                    cx.notify();
                });
            })))
            .when(!private, |d| {
                d.child(action("share-card", if system { Icon::Share } else { Icon::FileImage }, share).on_click(cx.listener(move |this, _, window, cx| {
                    if system {
                        this.share_card(shared.clone(), window, cx);
                    } else {
                        this.save_card(shared.clone(), cx);
                    }
                })))
            })
            .when(system && !private, |d| {
                d.child(action("save-card", Icon::FileImage, "Save as Image").on_click(cx.listener(move |this, _, _, cx| this.save_card(saved.clone(), cx))))
            });
        div().pt_4().flex().flex_wrap().items_start().gap_6().child(face).child(
            div()
                .flex_1()
                .min_w(px(300.))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .child(div().text_size(px(TEXT_HEADING)).font_weight(FontWeight::BOLD).one_line(card.name.clone()))
                        .child(value(since).text_size(px(TEXT_SMALL))),
                )
                .child(actions),
        )
    }

    fn share_card(&mut self, card: Card, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = engine::Host::from_window(window) else { return };
        let at = self.motion.bounds.get();
        cx.spawn(async move |this, cx| {
            let name = engine::downloads::sanitize_filename(&format!("Tarn Shield {}.png", card.name));
            let dark = !Theme::is_light();
            let written = cx
                .background_spawn(async move {
                    let png = shield_card::card_png(&card, dark)?;
                    let path = browser_core::shield_card::export_path(&name).ok()?;
                    std::fs::write(&path, png).ok()?;
                    url::Url::from_file_path(path).ok()
                })
                .await;
            let Some(url) = written else {
                this.update(cx, |this, cx| {
                    this.card_notice = Some("Could not draw the image.".into());
                    cx.notify();
                })
                .ok();
                return;
            };
            let (x, y, w, h) = (f32::from(at.origin.x), f32::from(at.origin.y), f32::from(at.size.width), f32::from(at.size.height));
            host.share(url.as_str(), x.into(), y.into(), w.into(), h.into());
        })
        .detach();
    }

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

    fn link_row(&self, id: &'static str, title: &'static str, detail: &'static str, url: &'static str, cx: &mut Context<Self>) -> Div {
        row(false).child(text_block(title, detail)).child(button(id, "Open", false).on_click(cx.listener(move |this, _, _, cx| {
            if let Ok(url) = url::Url::parse(url) {
                this.with_browser(cx, |b, w, cx| b.new_tab(&url, w, cx));
            }
        })))
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
        let motion = "Cards, banners, the sidebar and tab switches appear without animation.";
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
            .child(section("Motion"))
            .child(group().child(reduce_motion).child(self.search_effect_row(false, cx)))
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
                    .child(self.pref_row(false, Pref::HistoryText, "Search the text of visited pages", format!("Keeps up to 4 KB of each page's text on {} so history suggestions match what a page said. Never for the private space or Lockdown pages; cleared with history, and when you turn this off.", ui::controls::THIS_DEVICE), s.prefs.get(Pref::HistoryText), cx)),
            )
            .into_any_element()
    }
}

impl SettingsWindow {
    fn permissions(&self, s: Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let body = if s.permissions.is_empty() {
            note("No site has asked yet. When a site asks for the camera, microphone, location or pop-ups and you choose Remember, the answer shows here.")
        } else {
            let mut list = group();
            for (ix, p) in s.permissions.into_iter().enumerate() {
                let set = |allow: Option<bool>, origin: String, kind: String| {
                    cx.listener(move |this: &mut Self, _, _, cx| {
                        let (origin, kind) = (origin.clone(), kind.clone());
                        this.with_browser(cx, move |b, _, cx| b.set_profile_permission(&origin, &kind, allow, cx));
                    })
                };
                list = list.child(
                    row(ix == 0)
                        .child(text_block(bounded_title(&p.site), p.label.clone()))
                        .child(button(("allow", ix), "Allow", p.allowed).on_click(set(Some(true), p.origin.clone(), p.kind.clone())))
                        .child(button(("block", ix), "Block", !p.allowed).on_click(set(Some(false), p.origin.clone(), p.kind.clone())))
                        .child(button(("forget", ix), "Forget", false).on_click(set(None, p.origin, p.kind))),
                );
            }
            list
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(note("This profile's answers to sites. A site you forget asks again next time; Lockdown sites never ask."))
            .child(body)
            .into_any_element()
    }

    fn shield_levels(&self, s: &Snapshot, cx: &mut Context<Self>) -> Div {
        let level = s.profile.as_ref().map(|p| p.shield_level).unwrap_or_default();
        let name = s.profile.as_ref().map(|p| bounded_title(&p.name)).unwrap_or_default();
        ShieldLevel::ALL.into_iter().enumerate().fold(group(), |levels, (ix, l)| {
            let name = name.clone();
            levels.child(
                tab_stop(row(ix == 0).id(l.as_str()))
                    .cursor_pointer()
                    .hover(|s| s.bg(Theme::hover()))
                    .active(|s| s.bg(Theme::pressed()))
                    .child(radio(l == level))
                    .child(text_block(shield_badge(l), level_description(l)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if l != level {
                            this.set_level(l, &name, window, cx);
                        }
                    })),
            )
        })
    }

    fn privacy(&self, s: Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let target = s.profile.as_ref().map(|p| (s.private, p.id));
        let profile_name = s.profile.as_ref().map(|p| bounded_title(&p.name)).unwrap_or_default();

        let sites = if s.overrides.is_empty() {
            note("No site overrides. Use the Shield button in the address bar to set one for a site.")
        } else {
            let mut list = group();
            for (ix, (host, l)) in s.overrides.into_iter().enumerate() {
                list = list.child(
                    row(ix == 0).child(label(bounded_title(&host))).child(value(shield_badge(l))).child(
                        button(("remove-override", ix), "Remove", false)
                            .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.remove_site_override(&host, w, cx)))),
                    ),
                );
            }
            list
        };

        let reset_zooms = (!s.zooms.is_empty()).then(|| {
            button("reset-zooms", "Reset All…", false).on_click(cx.listener(move |this, _, window, cx| {
                let Some((private, id)) = target else { return };
                this.ask("Reset every saved zoom level?", None, &["Reset All"], window, cx, move |this, _, cx| {
                    this.with_browser(cx, |b, _, cx| b.remove_site_zoom(private, id, None, cx))
                });
            }))
        });
        let zooms = if s.zooms.is_empty() {
            note(format!(
                "No saved zoom levels. {} and {} remember the zoom for the site; {} forgets it.",
                crate::shortcuts::shown(&crate::ZoomIn, cx),
                crate::shortcuts::shown(&crate::ZoomOut, cx),
                crate::shortcuts::shown(&crate::ZoomReset, cx)
            ))
        } else {
            let mut list = group();
            for (ix, (host, zoom)) in s.zooms.into_iter().enumerate() {
                list = list.child(row(ix == 0).child(label(bounded_title(&host))).child(value(format!("{:.0} %", zoom * 100.))).child(
                    button(("remove-zoom", ix), "Remove", false).on_click(cx.listener(move |this, _, _, cx| {
                        let Some((private, id)) = target else { return };
                        this.with_browser(cx, |b, _, cx| b.remove_site_zoom(private, id, Some(&host), cx));
                    })),
                ));
            }
            list
        };

        let zapped = if s.private {
            note("A private space keeps no hidden elements.")
        } else if s.zapped.is_empty() {
            note("No hidden elements. Zap Element in the command bar hides an element on a site in this profile.")
        } else {
            let mut list = group();
            for (ix, (host, selector)) in s.zapped.into_iter().enumerate() {
                list = list.child(row(ix == 0).child(label(bounded_title(&format!("{host} — {selector}")))).child(
                    button(("unzap", ix), "Show Again", false).on_click(cx.listener(move |this, _, _, cx| {
                        let (host, selector) = (host.clone(), selector.clone());
                        this.with_browser(cx, move |b, _, cx| b.unzap(&host, Some(&selector), cx));
                    })),
                ));
            }
            list
        };

        let muted = if s.muted.is_empty() {
            note("No muted sites. Mute Site in the Tabs menu or a tab's menu silences a site in this profile.")
        } else {
            let mut list = group();
            for (ix, host) in s.muted.into_iter().enumerate() {
                list = list.child(row(ix == 0).child(label(bounded_title(&host))).child(button(("unmute", ix), "Unmute", false).on_click(cx.listener(
                    move |this, _, _, cx| {
                        let Some((private, id)) = target else { return };
                        this.with_browser(cx, |b, _, cx| {
                            b.set_site_muted(private, id, Some(&host), false, true);
                            cx.notify();
                        });
                    },
                ))));
            }
            list
        };

        let count = |category: &str, unit: &str| match s.filters.counts.get(category) {
            Some(n) => format!(" {} {unit}.", thousands(*n)),
            None if filters::configured() => " Lists not installed yet.".into(),
            None => " Needs the filter lists, which this build does not download.".into(),
        };
        let on = |pref| s.prefs.get(pref);
        let blocking = group()
            .child(self.pref_row(true, Pref::BlockAds, "Block ads", format!("EasyList, AdGuard Base and uBlock filters stop ad requests and hide ad placeholders.{}", count("ads", "rules")), on(Pref::BlockAds), cx))
            .child(self.pref_row(false, Pref::BlockTrackers, "Block trackers", format!("EasyPrivacy and uBlock Privacy stop tracking requests.{}", count("trackers", "rules")), on(Pref::BlockTrackers), cx))
            .child(self.pref_row(false, Pref::BlockAnnoyances, "Block cookie banners", format!("EasyList Cookie and uBlock Cookie Notices hide consent pop-ups and stop the scripts that show them. Nothing is accepted or refused for you.{}", count("annoyances", "rules")), on(Pref::BlockAnnoyances), cx))
            .child(self.pref_row(false, Pref::YouTubeAds, "Block YouTube ads", "Removes ads from YouTube's player data before the player sees them and hides ad slots. YouTube changes often; if videos stop playing, turn this off.", on(Pref::YouTubeAds), cx))
            .child(self.pref_row(false, Pref::BlockPopups, "Block pop-ups and ad redirects", format!("Pages open windows only when you click, never to known pop-up ad sites, and cannot send you to one.{}", count("popups", "domains")), on(Pref::BlockPopups), cx))
            .child(self.pref_row(false, Pref::BlockMalware, "Block dangerous sites", format!("Malware, phishing and scam lists{}.{}", if cfg!(windows) { "" } else { ", plus WebKit's fraudulent-website check, which asks Apple's Safe Browsing service about hashed addresses" }, count("malware", "domains")), on(Pref::BlockMalware), cx))
            .child(self.pref_row(false, Pref::BlockAdult, "Block adult content", format!("Pornographic and other adult websites, with no way past the warning page.{}", count("adult", "domains")), on(Pref::BlockAdult), cx));
        let paused = if s.paused.is_empty() {
            note("Blocking runs on every site. Turn it off for one site with the Shield button in the address bar.")
        } else {
            let mut list = group();
            for (ix, host) in s.paused.into_iter().enumerate() {
                list = list.child(
                    row(ix == 0).child(label(bounded_title(&host))).child(
                        button(("resume-blocking", ix), "Turn On", false)
                            .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.set_site_paused(&host, false, w, cx)))),
                    ),
                );
            }
            list
        };

        let updates = s.filters.updates;
        let filter_state = s.filters.summary();
        let connection = |first: bool, name: &'static str, detail: String| row(first).child(text_block(name, detail));
        let connections = group()
            .child(if filters::configured() {
                switch_row("filter-updates", true, text_block("Filter-list updates", format!("GitHub, once a day. {filter_state}")), updates)
                    .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.set_filter_updates(!updates, cx))))
                    .into_any_element()
            } else {
                connection(true, "Filter-list updates", format!("GitHub, once a day. {filter_state}")).into_any_element()
            })
            .child({
                let on = s.extension_downloads;
                switch_row(
                    "extension-downloads",
                    false,
                    text_block(
                        "Extension and theme downloads",
                        "GitHub, only when you click Install, Check for updates or Show official extensions in Extensions or Appearance settings. GitHub learns your address and the repository.",
                    ),
                    on,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.set_extension_downloads(!on, cx))))
            })
            .child(connection(false, "Site icons", "The site you are on, or a server it already loaded from such as its CDN (without cookies), through the tab's web engine with its Shield rules; never under Lockdown.".into()))
            .child(connection(false, "Update checks", match (s.updater, s.update_checks) {
                (false, _) => "Not set up in this build; Tarn does not check for updates.".into(),
                (true, true) => "GitHub, once a day. Turn off in General.".into(),
                (true, false) => "Off; only Check for Updates in the Tarn menu asks GitHub.".into(),
            }))
            .when(cfg!(feature = "translation"), |d| {
                d.child(connection(false, "Translation", "None. Pages are translated on this Mac with the language packs installed in System Settings; Tarn never downloads them.".into()))
            })
            .child(connection(false, "AI requests", format!("Not built yet; no page content leaves {}.", ui::controls::THIS_DEVICE)));

        let mut data = group();
        if !s.private {
            data = data.child(row(true).child(label("Export this profile's data to Downloads as JSON")).child(
                button("export-profile", "Export", false).on_click(cx.listener(move |this, _, _, cx| {
                    let Some((private, id)) = target else { return };
                    this.with_browser(cx, |b, _, cx| b.export_profile(private, id, cx));
                })),
            ));
        }
        let clear_name = profile_name.clone();
        let delete_name = profile_name.clone();
        data = data
            .child(
                row(s.private)
                    .child(text_block(
                        "Clear browsing history",
                        "The last hour or today removes visited pages; everything also removes site icons, zoom levels, muted sites, protected sites and hidden elements.",
                    ))
                    .child(button("clear-history", "Clear…", false).on_click(cx.listener(move |this, _, window, cx| {
                        let Some((private, id)) = target else { return };
                        let ranges = HistoryRange::ALL.map(HistoryRange::label);
                        let question = format!("Clear the browsing history of \u{201C}{clear_name}\u{201D}?");
                        this.ask(&question, Some("Choose how far back to clear."), &ranges, window, cx, move |this, ix, cx| {
                            this.with_browser(cx, |b, _, cx| b.clear_history(private, id, HistoryRange::ALL[ix], cx))
                        });
                    }))),
            )
            .child(
                row(false).child(label(format!("Delete everything in the profile \u{201C}{profile_name}\u{201D}"))).child(
                    button("delete-profile", "Delete…", false).on_click(cx.listener(move |this, _, window, cx| {
                        let Some((private, id)) = target else { return };
                        let question = format!("Delete everything in the profile \u{201C}{delete_name}\u{201D}?");
                        let detail = "Its spaces, tabs, history, cookies, site data and site settings are deleted. This cannot be undone.";
                        this.ask(&question, Some(detail), &["Delete Everything"], window, cx, move |this, _, cx| {
                            this.with_browser(cx, |b, w, cx| b.delete_profile_data(private, id, w, cx))
                        });
                    })),
                ),
            );

        div()
            .child(section("Blocking"))
            .child(blocking)
            .child(note("Applies to every profile. Malware, phishing and adult lists stop whole sites with a warning page; ad, tracker and cookie-banner lists act inside pages."))
            .child(section("Sites without ad, tracker and cookie-banner blocking"))
            .child(paused)
            .child(section("Links from other apps"))
            .child(group().child(self.pref_row(true, Pref::LocalhostFromApps, "Open localhost links from other apps", format!("A link to {} (localhost, 127.0.0.1) from a terminal, editor or other app opens like a typed address, and the page it shows may lead elsewhere on your network as a typed one can. Off: Tarn refuses it, since a link in a mail could reach services running here; Open Anyway on the notice still opens it. Links to other addresses on your network are refused; Open Anyway on the notice opens them. Applies to every profile.", ui::controls::THIS_DEVICE), on(Pref::LocalhostFromApps), cx)))
            .child(section("Site overrides"))
            .child(sites)
            .child(heading("Site zoom levels", div().children(reset_zooms)))
            .child(zooms)
            .child(section("Muted sites"))
            .child(muted)
            .child(section("Hidden elements"))
            .child(zapped)
            .child(section("Connections Tarn makes"))
            .child(connections)
            .child(note("Pages you open connect wherever they link to; this list covers Tarn itself. GitHub sees your IP address when Tarn checks for filter lists; turning updates off keeps the installed lists."))
            .child(section("Your data"))
            .child(data)
            .child(note(s.notice.clone().unwrap_or_else(|| {
                let shared = if s.shared_data { " Cookies and site data are shared with another profile and are deleted there too." } else { "" };
                format!("Delete everything removes spaces, tabs, history, cookies, site data and site settings of this profile.{shared} Files you downloaded stay in Downloads.")
            })))
            .into_any_element()
    }
}

impl SettingsWindow {
    fn developer(&self, s: Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let on = s.developer;
        let dev_profile = s.profiles.iter().find(|p| p.is_dev).map(|p| bounded_title(&p.name));
        let servers = match &s.local_servers {
            None => note("Not checked yet."),
            Some(ports) if ports.is_empty() => note("Nothing is listening on the common dev ports (3000, 5173, 8080 and others)."),
            Some(ports) => {
                let mut list = group();
                for (ix, port) in ports.iter().copied().enumerate() {
                    list = list.child(
                        row(ix == 0).child(label(format!("localhost:{port}"))).child(
                            button(("open-server", ix), "Open", false)
                                .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.open_local_server(port, w, cx)))),
                        ),
                    );
                }
                list
            }
        };
        div()
            .child(section("Developer tools"))
            .child(group().child(
                switch_row("developer-tools", true, label("Web Inspector, JSON viewer, responsive sizes and local servers"), on)
                    .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.set_developer(!on, w, cx)))),
            ))
            .child(note("Applies to every profile; open pages reload. Right-click a page and choose Inspect Element; the toolbar gains responsive sizes and Clear Site Data."))
            .child(section("Developer profile"))
            .child(match dev_profile {
                Some(name) => group().child(row(true).child(label(name)).child(value("Created"))),
                None => group().child(row(true).child(label("A separate profile for development")).child(
                    button("create-dev-profile", "Create", false)
                        .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, w, cx| b.create_dev_profile(w, cx)))),
                )),
            })
            .child(note("Web Inspector and the developer tools are always on there; caches are cleared after every page load and Reload bypasses the cache."))
            .child(heading(
                "Local servers",
                button("refresh-servers", "Refresh", false).on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.refresh_local_servers(cx)))),
            ))
            .child(servers)
            .into_any_element()
    }
}

impl SettingsWindow {
    fn extensions(&self, s: Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if !s.extensions_allowed {
            return div().child(section("Extensions")).child(note("Extensions are off in private and Lockdown profiles.")).into_any_element();
        }
        let mut page = div();
        if cfg!(windows) {
            page = page.child(note("Windows runs Tarn extensions only; Safari and Chrome extensions need their own engine and are not available here."));
        }
        if s.all_sites_notice {
            page = page.child(
                group().child(
                    row(true)
                        .child(text_block(
                            "Extensions you had turned on keep access to every site they list",
                            "Their \u{201C}Allow on every site\u{201D} switch is on. Turn it off to be asked per site instead.",
                        ))
                        .child(
                            button("all-sites-notice", "OK", false)
                                .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.dismiss_all_sites_notice(cx)))),
                        ),
                ),
            );
        }
        if !s.extension_notices.is_empty() {
            let mut list = group();
            for (ix, n) in s.extension_notices.iter().enumerate() {
                list = list.child(row(ix == 0).child(div().flex_1().text_color(Theme::warning()).child(n.clone())));
            }
            page = page.child(
                list.child(
                    row(false).child(div().flex_1()).child(
                        button("extension-notices", "Dismiss", false)
                            .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.dismiss_extension_notices(cx)))),
                    ),
                ),
            );
        }
        if let Some((token, title, lines)) = s.store_sheet {
            let card = group()
                .child(row(true).child(text_block(title, "From Tarn's signed official catalog.")))
                .children(lines.into_iter().map(|l| row(false).child(div().flex_1().child(l))))
                .child(
                    row(false)
                        .child(div().flex_1())
                        .child(
                            button("store-cancel", "Cancel", true)
                                .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.cancel_store_offer(cx)))),
                        )
                        .child(button("store-install", "Install", false).on_click(cx.listener(move |this, _, _, cx| {
                            let token = token.clone();
                            this.with_browser(cx, move |b, _, cx| b.confirm_store_offer(&token, cx))
                        }))),
                );
            return page.child(section("Confirm")).child(card).into_any_element();
        }
        if let Some(c) = s.consent {
            let list = |title: &'static str, items: &[String]| {
                (!items.is_empty()).then(|| text_block(title, items.iter().map(|i| crate::extensions::shown(i)).collect::<Vec<_>>().join(", ")))
            };
            let mut card = group()
                .child(row(true).child(text_block(format!("Install {} {}?", c.name, c.version), format!("The extension's own words. Source: {}", c.badge))))
                .children(list("Permissions", &c.apis).map(|d| row(false).child(d)))
                .children(list("Optional permissions", &c.optional_apis).map(|d| row(false).child(d)))
                .children(list("Sites it asks for (you are asked per site)", &c.hosts).map(|d| row(false).child(d)))
                .children(list("Optional sites", &c.optional_hosts).map(|d| row(false).child(d)))
                .children(list("Changes", &c.changes).map(|d| row(false).child(d)));
            if c.active_tab {
                card = card.child(row(false).child(note("Clicking its toolbar button gives it the page you are on, in that tab.")));
            }
            if c.unreviewed {
                card = card.child(row(false).child(div().flex_1().text_color(Theme::warning()).child(crate::extensions::UNREVIEWED)));
            }
            card = card.child(
                row(false)
                    .child(div().flex_1())
                    .child(
                        button("consent-cancel", "Cancel", true)
                            .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.cancel_extension_install(cx)))),
                    )
                    .child(
                        button("consent-install", "Install", false)
                            .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.confirm_extension_install(cx)))),
                    ),
            );
            return page.child(section("Confirm")).child(card).into_any_element();
        }
        if !s.extension_choice.is_empty() {
            let mut list = group().child(row(true).child(label("The release offers several packages. Pick the one for Chromium or Safari:")));
            for (ix, name) in s.extension_choice.iter().enumerate() {
                list = list.child(
                    row(false).child(label(name.clone())).child(
                        button(("extension-asset", ix), "Install", false)
                            .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, move |b, _, cx| b.choose_extension_asset(ix, cx)))),
                    ),
                );
            }
            list = list.child(
                row(false).child(div().flex_1()).child(
                    button("extension-asset-cancel", "Cancel", true)
                        .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.cancel_extension_install(cx)))),
                ),
            );
            page = page.child(list);
        }
        if let Some(busy) = s.extension_busy {
            page = page.child(
                group().child(
                    row(true).child(label(busy)).child(
                        button("install-cancel", "Cancel", false)
                            .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.cancel_extension_install(cx)))),
                    ),
                ),
            );
        }
        let access = group()
            .child(self.pref_row(
                true,
                Pref::ExtensionsEverySite,
                "Allow extensions on every site",
                "App and official extensions read and change every site they list without asking. Unverified extensions always ask per site.",
                s.prefs.get(Pref::ExtensionsEverySite),
                cx,
            ))
            .when(cfg!(target_os = "macos"), |g| {
                g.child(self.pref_row(
                    false,
                    Pref::ExtensionsBitwarden,
                    "Bitwarden integration",
                    "Lets Bitwarden's app extension unlock with Touch ID through the Bitwarden app, use the clipboard and offer passkeys on sites. Off, it runs like any other app extension.",
                    s.prefs.get(Pref::ExtensionsBitwarden),
                    cx,
                ))
            });
        let installed = if s.extensions.is_empty() {
            note("No extensions yet. Apps with a Safari web extension, such as Bitwarden from the Mac App Store, appear here.")
        } else {
            let mut list = group();
            for (ix, e) in s.extensions.into_iter().enumerate() {
                let on = e.enabled;
                let (install_id, app_path) = (e.install_id.clone(), e.app_path.clone());
                let mut actions = div().flex().flex_wrap().items_center().gap_2();
                if let (Some(all), Some(id)) = (e.all_sites, e.install_id.clone()) {
                    actions = actions.child(
                        tab_stop(div().id(("extension-all", ix)))
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .cursor_pointer()
                            .text_size(px(TEXT_SMALL))
                            .child(switch(all))
                            .child("Every site")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let id = id.clone();
                                this.with_browser(cx, move |b, w, cx| b.set_extension_all_sites(&id, !all, w, cx))
                            })),
                    );
                }
                if let Some(id) = e.install_id.clone() {
                    actions = actions.child(button(("extension-forget", ix), "Forget Site Answers", false).on_click(cx.listener(move |this, _, _, cx| {
                        let id = id.clone();
                        this.with_browser(cx, move |b, w, cx| b.forget_extension_answers(&id, w, cx))
                    })));
                }
                if let Some(id) = e.install_id {
                    let question = format!("Remove the extension \u{201C}{}\u{201D}?", bounded_title(&e.name));
                    actions = actions.child(button(("extension-remove", ix), "Remove\u{2026}", false).on_click(cx.listener(move |this, _, window, cx| {
                        let id = id.clone();
                        this.ask(&question, Some("Its files and the data it stored are deleted."), &["Remove"], window, cx, move |this, _, cx| {
                            this.with_browser(cx, move |b, _, cx| b.remove_extension(&id, cx))
                        });
                    })));
                }
                let r = row(ix == 0)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(text_block(format!("{} \u{00B7} {}", e.name, e.badge), e.permissions).flex_initial())
                            .child(actions),
                    )
                    .child(tab_stop(div().id(("extension-switch", ix))).cursor_pointer().child(switch(on)).on_click(cx.listener(move |this, _, _, cx| {
                        let (install_id, app_path) = (install_id.clone(), app_path.clone());
                        this.with_browser(cx, move |b, w, cx| match (install_id, app_path) {
                            (Some(id), _) => b.set_extension(&id, !on, w, cx),
                            (None, Some(path)) => b.enable_app_extension(path, w, cx),
                            _ => {}
                        })
                    })));
                list = list.child(r);
            }
            list
        };
        let downloads = s.extension_downloads;
        const OFF: &str = "Extension downloads are off in Privacy settings.";
        let input = self.repo_input.clone();
        let focused = input.focus_handle(cx).is_focused(window);
        let add = group()
            .child(row(true).child(field(focused).flex_1().py_0p5().child(input)).child(available(
                button("install-github", "Install", false).when(downloads, |d| d.on_click(cx.listener(|this, _, _, cx| this.install_from_github(cx)))),
                downloads,
                OFF,
            )))
            .child(row(false).child(label("A .zip or .crx file")).child(button("install-file", "Choose File\u{2026}", false).on_click(cx.listener(
                |this, _, _, cx| {
                    let paths = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
                    let main = this.main;
                    cx.spawn(async move |_, cx| {
                        let Ok(Ok(Some(paths))) = paths.await else { return };
                        let Some(path) = paths.into_iter().next() else { return };
                        main.update(cx, |b, _, cx| {
                            if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("crx")) {
                                b.install_extension(crate::extensions::Install::File(path), None, cx);
                            } else {
                                b.core.extensions.borrow_mut().notices.push("Choose a .zip or .crx extension file.".to_owned());
                                cx.notify();
                            }
                        })
                        .ok();
                    })
                    .detach();
                },
            ))))
            .child(
                row(false).child(label("Installed from GitHub or the catalog")).child(available(
                    button("extension-updates", "Check for Updates", false)
                        .when(downloads, |d| d.on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.check_extension_updates(cx))))),
                    downloads,
                    OFF,
                )),
            );
        page = page.child(section("Installed")).child(installed).child(section("Access")).child(access).child(section("Add")).child(add);
        if !downloads {
            page = page.child(note("Extension downloads are off in Privacy settings; installing from a file still works."));
        }
        if crate::extensions::catalog_configured() {
            let (rows, old) = s.catalog;
            let mut official = group().child(
                row(true).child(label("Reviewed and signed by Tarn")).child(available(
                    button("catalog", "Show Official Extensions", false)
                        .when(downloads, |d| d.on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, _, cx| b.fetch_catalog(cx))))),
                    downloads,
                    OFF,
                )),
            );
            for (ix, e) in rows.into_iter().enumerate() {
                let id = e.id.clone();
                official = official.child(row(false).child(text_block(format!("{} {}", e.name, e.version), e.description)).child(if e.installed {
                    value("Installed").into_any_element()
                } else {
                    available(
                        button(("catalog-install", ix), "Install", false).when(downloads, |d| {
                            d.on_click(cx.listener(move |this, _, _, cx| {
                                let id = id.clone();
                                this.with_browser(cx, move |b, _, cx| b.install_official(&id, cx))
                            }))
                        }),
                        downloads,
                        OFF,
                    )
                    .into_any_element()
                }));
            }
            official = official.child(self.link_row(
                "extension-store",
                "Browse the store",
                "Official extensions with screenshots and search; Install in Tarn asks here first.",
                browser_core::extensions::STORE_URL,
                cx,
            ));
            page = page.child(section("Official")).child(official);
            if old {
                page = page.child(note("This list is more than 30 days old. Show official extensions to get the current one."));
            }
        }
        page.child(note("Extensions run in WebKit tabs of this profile only, never in the private space or in Lockdown profiles, and never on sites set to Lockdown. An extension that may not use every site asks for each site when it first meets it. Open pages reload when an extension is turned on or off."))
            .into_any_element()
    }

    fn profile_row(&self, s: &Snapshot, ix: usize, p: &Profile, deletable: bool, cx: &mut Context<Self>) -> gpui::Div {
        let id = p.id;
        let current = s.profile.as_ref().is_some_and(|c| c.id == id && c.is_private == p.is_private);
        row(ix == 0)
            .child(if p.is_private { icon(Icon::Person, 14., Theme::warning()).into_any_element() } else { profile_dot(&p.color, 12.).into_any_element() })
            .child(label(bounded_title(&p.name)))
            .when(current, |d| d.child(value("This window")))
            .when(p.is_private, |d| d.child(value("Private, not saved")))
            .when(p.is_dev, |d| d.child(value("Developer")))
            .child(value(shield_badge(p.shield_level)))
            .when(!p.is_private, |d| {
                d.child(
                    button(("edit-profile", ix), "Edit…", false)
                        .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, w, cx| b.open_profile_editor(Some(id), w, cx)))),
                )
            })
            .when(!p.is_private && deletable, |d| {
                let question = format!("Delete the profile \u{201C}{}\u{201D}?", bounded_title(&p.name));
                d.child(button(("delete-profile", ix), "Delete…", false).on_click(cx.listener(move |this, _, window, cx| {
                    let detail = "Its spaces, tabs, history and website data are deleted. Website data another profile shares stays for that profile.";
                    this.ask(&question, Some(detail), &["Delete Profile"], window, cx, move |this, _, cx| {
                        this.with_browser(cx, |b, w, cx| b.delete_profile(id, w, cx))
                    });
                })))
            })
    }

    fn profiles(&self, s: &Snapshot, cx: &mut Context<Self>) -> AnyElement {
        let deletable = s.profiles.iter().filter(|p| !p.is_private).count() > 1;
        let list = s.profiles.iter().enumerate().fold(group(), |list, (ix, p)| list.child(self.profile_row(s, ix, p, deletable, cx)));
        let rules = if s.space_rules.is_empty() {
            note("No rules. \u{201c}Always open in\u{201d} in a site's settings opens its new tabs in the space you pick.")
        } else {
            let mut list = group();
            for (ix, (id, pattern, space)) in s.space_rules.iter().cloned().enumerate() {
                list = list.child(
                    row(ix == 0).child(label(bounded_title(&pattern))).child(value(space)).child(
                        button(("delete-space-rule", ix), "Remove", false)
                            .on_click(cx.listener(move |this, _, _, cx| this.with_browser(cx, |b, _, cx| b.delete_space_rule(id, cx)))),
                    ),
                );
            }
            list
        };
        div()
            .child(heading(
                "Profiles",
                button("new-profile", "New Profile…", true)
                    .on_click(cx.listener(|this, _, _, cx| this.with_browser(cx, |b, w, cx| b.open_profile_editor(None, w, cx)))),
            ))
            .child(list)
            .child(note(
                "Delete removes a profile with its spaces, tabs, history and website data. Website data another profile shares stays for that profile.",
            ))
            .child(section("Space Rules"))
            .child(rules)
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
        let found =
            |q: &str| super::PANES.iter().filter(|(_, _, name, words)| super::pane_matches(q, name, words)).map(|(_, _, name, _)| *name).collect::<Vec<_>>();
        assert_eq!(found("lockd"), ["Protection"]);
        assert_eq!(found("COOKIE ban"), ["Privacy"]);
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
        let listed = super::SHORTCUTS
            .iter()
            .flat_map(|(_, items)| items.iter())
            .map(|(_, keys)| match keys {
                super::Fixed(keys) => (*keys).to_owned(),
                super::Bound(keys, _) => crate::shortcuts::display(keys),
            })
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
