use std::path::PathBuf;

use nsh_core::prelude::*;
use store::{Id, Store};
use tarn_nsh_shared::appearance::{Host, appearance};

use super::{Ui, act, read};

#[derive(Clone, Copy)]
struct Looks(Ui);

impl Host for Looks {
    type Transport = crate::fetch::Soup;
    fn transport() -> crate::fetch::Soup {
        crate::fetch::Soup::new()
    }
    fn catalog_dir(self) -> PathBuf {
        crate::browser::themes::catalog_dir()
    }
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R {
        f(&super::core().disk)
    }
    fn changed(self) {
        for ui in super::WINDOWS.with_borrow(Vec::clone) {
            act(ui, |b| b.apply_theme());
        }
    }
    fn profile(self) -> Option<(Id, String)> {
        read(self.0, |b| {
            let space = b.model.current_space().filter(|s| !s.private)?;
            Some((space.space.profile_id, b.core.disk.profile(space.space.profile_id).ok()?.name))
        })
        .flatten()
    }
    fn safe(self) -> bool {
        read(self.0, |b| b.themes.safe).unwrap_or_default()
    }
    fn notice(self) -> Option<String> {
        read(self.0, |b| b.themes.active.notice.clone()).flatten()
    }
    fn system_light(self) -> bool {
        !dark()
    }
    fn themes_dir(self) -> PathBuf {
        crate::data::root().join("themes")
    }
    fn desktop(self) -> bool {
        true
    }
}

pub(super) fn pane(ui: Ui) -> AnyElement {
    appearance(Looks(ui))
}
