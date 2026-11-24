use browser_core::theme::{Active, Selection};
use std::sync::atomic::{AtomicBool, Ordering};

use gpui::{App, WindowAppearance, WindowBackgroundAppearance};
use ui::theme::Theme;

use crate::shared::AppCore;

/// The theme in use, what decided it, and whether safe mode replaced it for this run.
#[derive(Default)]
pub(crate) struct State {
    pub active: Option<Active>,
    /// Shift was held at launch: the built-in theme shows, without page styles, until quit.
    pub safe: bool,
    pub system_light: bool,
}

pub(crate) fn merge_revocations(core: &AppCore, list: &shield::revocations::Revocations) -> bool {
    browser_core::theme_library::merge_revocations(&core.disk, list)
}

pub(crate) fn system_light(cx: &App) -> bool {
    matches!(cx.window_appearance(), WindowAppearance::Light | WindowAppearance::VibrantLight)
}

/// Loads the chosen theme (or the built-in one in safe mode) into `ui::theme`, then the user's
/// accent on top; the caller refreshes the windows.
pub(crate) fn apply(core: &AppCore, system_light: bool) {
    let mut state = core.themes.borrow_mut();
    state.system_light = system_light;
    let active = browser_core::theme_library::current(&core.disk, system_light, state.safe);
    let p = &active.parsed;
    Theme::set_tokens(p.tokens, p.meta.scheme, p.font.clone());
    GLASS.store(cfg!(target_os = "macos") && matches!(active.id, browser_core::theme::ThemeRef::BuiltIn(_)) && p.tokens.translucent(), Ordering::Relaxed);
    crate::prefs::apply_accent(&core.disk);
    Theme::set_search_effect(browser_core::theme_library::search_effect(&core.disk, &active));
    state.active = Some(active);
}

static GLASS: AtomicBool = AtomicBool::new(false);

/// Only built-in themes see through the window: their text is tested over any desktop. Windows
/// keeps an opaque window until acrylic is tried over CEF views.
pub(crate) fn window_background() -> WindowBackgroundAppearance {
    if GLASS.load(Ordering::Relaxed) { WindowBackgroundAppearance::Blurred } else { WindowBackgroundAppearance::Opaque }
}

/// Deferred, as the caller may be inside the update of one of these windows.
fn refresh(cx: &mut App) {
    cx.defer(|cx| {
        let shells =
            cx.windows().into_iter().filter(|w| w.downcast::<crate::browser::Browser>().is_some() || w.downcast::<crate::settings::SettingsWindow>().is_some());
        for handle in shells.collect::<Vec<_>>() {
            handle.update(cx, |_, window, _| window.set_background_appearance(window_background())).ok();
        }
        cx.refresh_windows();
    });
}

/// Applies the stored theme choice after it or a theme changed, and redraws every window. Pages
/// get changed website styles as they load; Chromium views, which keep the script they were made
/// with, are made again (macOS; on Windows `set_site_styles` restyles open pages and is
/// never true).
pub(crate) fn changed(cx: &mut App) {
    let moved = reapply(cx);
    if !moved.is_empty() {
        crate::shared::each(cx, |b, window, cx| b.restart_views_of(&moved, true, window, cx));
    }
    refresh(cx);
}

/// Loads the theme and hands every open profile its sheets; returns the profiles whose sheets
/// changed.
fn reapply(cx: &mut App) -> Vec<(bool, store::Id)> {
    let core = crate::shared::core(cx);
    apply(&core, system_light(cx));
    let open: Vec<(bool, store::Id)> = core.profiles.borrow().keys().copied().collect();
    open.into_iter().filter(|(private, profile_id)| crate::site_settings::apply_site_styles(&core, *private, *profile_id)).collect()
}

pub(crate) fn pages_on(core: &AppCore, profile_id: store::Id) -> bool {
    browser_core::theme_library::pages_on(&core.disk, profile_id)
}

/// The active theme's website sheets for one profile (`browser_core::theme::page_sheets`).
pub(crate) fn page_sheets(core: &AppCore, private: bool, profile_id: store::Id) -> Vec<(String, String)> {
    let state = core.themes.borrow();
    state.active.as_ref().map(|a| browser_core::theme_library::profile_sheets(&core.disk, a, state.safe, private, profile_id)).unwrap_or_default()
}

pub(crate) fn remember_withdrawn_repos(core: &AppCore) {
    browser_core::theme_library::remember_withdrawn_repos(&core.disk);
}

/// Shift held while the first window opens starts this run in safe mode.
pub(crate) fn enter_safe_mode(core: &AppCore, cx: &mut App) {
    core.themes.borrow_mut().safe = true;
    // The restored tabs loaded with the theme's website styles already; they load again without.
    let moved = reapply(cx);
    if !moved.is_empty() {
        crate::shared::each(cx, |b, window, cx| b.restart_views_of(&moved, false, window, cx));
    }
    refresh(cx);
}

/// Called when macOS switches between light and dark: only a "Match system" choice follows it.
pub(crate) fn appearance_changed(core: &AppCore, cx: &mut App) {
    let light = system_light(cx);
    if core.themes.borrow().system_light == light {
        return;
    }
    let follows = Selection::load(&core.disk).match_system;
    core.themes.borrow_mut().system_light = light;
    if follows {
        // Runs inside a window's update, which `changed` must not lease again.
        cx.defer(changed);
    }
}

/// The accent the theme itself sets, for the "Theme" swatch.
pub(crate) fn theme_accent(core: &AppCore) -> u32 {
    core.themes.borrow().active.as_ref().map_or(browser_core::theme::Tokens::DARK.accent, |a| a.parsed.tokens.accent)
}

pub(crate) use browser_core::theme_fetch::catalog_configured;

pub(crate) fn catalog_dir() -> std::path::PathBuf {
    crate::browser::data_dir().join("catalog-themes")
}

pub(crate) fn cached_catalog() -> Option<shield::catalog::Index> {
    browser_core::theme_fetch::cached_catalog(&catalog_dir())
}
