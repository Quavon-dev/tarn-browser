use std::path::{Path, PathBuf};

use browser_core::theme::Active;
use browser_core::theme_library as library;
use store::Store;
use tarn_nsh_shared::theme::Theme;

use crate::data::Platform;
use crate::shell::Shell;

pub struct Themes {
    pub active: Active,
    pub light: bool,
    pub catalog_dir: PathBuf,
}

impl Themes {
    pub fn open(store: &Store, dir: &Path) -> Themes {
        let catalog_dir = dir.join("catalog-themes");
        browser_core::theme_fetch::merge_cached_catalog(store, &catalog_dir);
        Themes { active: library::current(store, false, false), light: false, catalog_dir }
    }
}

impl<D: Platform> Shell<D> {
    pub fn apply_theme(&mut self) {
        let store = &self.core.disk;
        let active = library::current(store, self.themes.light, false);
        let p = &active.parsed;
        Theme::set_tokens(p.tokens, p.meta.scheme, p.font.clone());
        Theme::set_search_effect(library::search_effect(store, &active));
        self.themes.active = active;
    }

    pub fn follow_appearance(&mut self, light: bool) {
        if self.themes.light != light {
            self.themes.light = light;
            self.apply_theme();
        }
    }

    pub fn theme_styles(&self, private: bool, profile_id: store::Id) -> Vec<(String, String)> {
        library::profile_sheets(&self.core.disk, &self.themes.active, false, private, profile_id)
    }
}
