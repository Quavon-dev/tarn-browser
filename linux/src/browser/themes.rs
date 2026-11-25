use browser_core::prefs;
use browser_core::tabs::TabKey;
use browser_core::theme::{Active, Backing};
use browser_core::theme_library as library;
use serde_json::{Value, json};
use tarn_nsh_shared::theme::Theme;

use super::Browser;

#[derive(Clone)]
pub struct Themes {
    pub active: Active,
    pub safe: bool,
    pub light: bool,
    pub window: Backing,
}

pub fn catalog_dir() -> std::path::PathBuf {
    crate::data::root().join("catalog-themes")
}

impl Themes {
    pub fn open(store: &store::Store, filters: &std::path::Path) -> Themes {
        library::merge_cached_bundle(store, filters);
        browser_core::theme_fetch::merge_cached_catalog(store, &catalog_dir());
        Themes { active: library::current(store, false, false), safe: false, light: false, window: Backing::Opaque }
    }
}

/// GTK 4 has no blur behind a window, so the most a composited display offers is a clear one.
pub fn desktop_backing() -> Backing {
    use webkit6::prelude::*;
    if webkit6::gdk::Display::default().is_some_and(|d| d.is_composited()) { Backing::Clear } else { Backing::Opaque }
}

impl Browser {
    pub fn apply_theme(&mut self) {
        let store = &self.core.disk;
        let active = library::current(store, self.themes.light, self.themes.safe);
        let look = library::look(store, &active, self.themes.safe, desktop_backing());
        let p = &active.parsed;
        Theme::set_tokens(prefs::accented(look.tokens, store), p.meta.scheme, p.font.clone());
        Theme::set_search_effect(library::search_effect(store, &active));
        tarn_nsh_shared::motion::set_reduced(prefs::REDUCE_MOTION.on(store));
        Theme::set_compact(prefs::COMPACT.on(store));
        self.themes.active = active;
        self.themes.window = look.window;
    }

    pub fn toggle_glass_design(&mut self) {
        if let Err(e) = library::set_glass_on(&self.core.disk, !library::glass_on(&self.core.disk)) {
            self.show_toast_as(browser_core::toast::Tone::Warning, e, None);
        }
    }

    pub fn follow_appearance(&mut self, light: bool) {
        if self.themes.light != light {
            self.themes.light = light;
            self.apply_theme();
        }
    }

    pub fn enter_safe_mode(&mut self) {
        self.themes.safe = true;
        self.apply_theme();
    }

    pub fn theme_styles(&self, key: TabKey) -> Value {
        let Some(profile) = self.model.profile_of(&self.core, key) else { return json!([]) };
        let sheets = library::profile_sheets(&self.core.disk, &self.themes.active, self.themes.safe, key.private, profile);
        Value::Array(sheets.into_iter().map(|(hosts, css)| json!([hosts, css])).collect())
    }
}
