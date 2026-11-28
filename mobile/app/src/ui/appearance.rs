use std::path::PathBuf;

use nsh_core::prelude::*;
use store::{Id, Store};
use tarn_nsh_shared::appearance::{Host, appearance};

use super::{SHELL, Ui};

#[derive(Clone, Copy)]
struct Looks(Ui);

impl Host for Looks {
    type Transport = crate::platform::Transport;
    fn transport() -> crate::platform::Transport {
        crate::platform::Transport
    }
    fn catalog_dir(self) -> PathBuf {
        SHELL.with(|s| s.borrow().as_ref().map(|s| s.themes.catalog_dir.clone())).unwrap_or_else(|| std::env::temp_dir().join("tarn-catalog-themes"))
    }
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R {
        SHELL.with(|s| f(&s.borrow().as_ref().expect("the shell opens with the app").core.disk))
    }
    fn changed(self) {
        self.0.act(|s| {
            s.apply_theme();
            Vec::new()
        });
    }
    fn profile(self) -> Option<(Id, String)> {
        SHELL.with(|s| {
            let shell = s.borrow();
            let shell = shell.as_ref()?;
            let space = shell.model.current_space().filter(|s| !s.private)?;
            Some((space.space.profile_id, shell.core.disk.profile(space.space.profile_id).ok()?.name))
        })
    }
    fn safe(self) -> bool {
        false
    }
    fn notice(self) -> Option<String> {
        SHELL.with(|s| s.borrow().as_ref().and_then(|s| s.themes.active.notice.clone()))
    }
    fn system_light(self) -> bool {
        !dark()
    }
    fn themes_dir(self) -> PathBuf {
        std::env::temp_dir().join("tarn-themes")
    }
    fn desktop(self) -> bool {
        false
    }
    fn layout_rows(self) -> Vec<AnyElement> {
        super::panels::layout_rows(self.0)
    }
    fn sidebar_rows(self) -> Vec<AnyElement> {
        super::panels::sidebar_rows(self.0)
    }
}

pub(super) fn pane(ui: Ui) -> AnyElement {
    appearance(Looks(ui))
}
