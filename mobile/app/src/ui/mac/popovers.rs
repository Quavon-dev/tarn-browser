use nsh_core::prelude::*;

use super::super::{Screen, Ui};

pub(super) fn owns(_screen: Screen) -> bool {
    false
}

pub(super) fn popovers(ui: Ui) -> impl IntoElement {
    dynamic(move || owns(ui.screen.get()).then(view))
}
