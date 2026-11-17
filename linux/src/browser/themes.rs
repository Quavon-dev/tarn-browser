use browser_core::prefs;
use browser_core::tabs::TabKey;
use browser_core::theme::Active;
use browser_core::theme_library as library;
use serde_json::{Value, json};
use tarn_nsh_shared::theme::Theme;

use super::Browser;

#[derive(Clone)]
pub struct Themes {
    pub active: Active,
    pub safe: bool,
    pub light: bool,
}

pub fn catalog_dir() -> std::path::PathBuf {
    crate::data::root().join("catalog-themes")
}

impl Themes {
    pub fn open(store: &store::Store, filters: &std::path::Path) -> Themes {
        library::merge_cached_bundle(store, filters);
        browser_core::theme_fetch::merge_cached_catalog(store, &catalog_dir());
        Themes { active: library::current(store, false, false), safe: false, light: false }
    }
}

impl Browser {
    pub fn apply_theme(&mut self) {
        let store = &self.core.disk;
        let active = library::current(store, self.themes.light, self.themes.safe);
        let p = &active.parsed;
        Theme::set_tokens(prefs::accented(p.tokens, store), p.meta.scheme, p.font.clone());
        Theme::set_search_effect(library::search_effect(store, &active));
        tarn_nsh_shared::motion::set_reduced(prefs::REDUCE_MOTION.on(store));
        self.themes.active = active;
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
