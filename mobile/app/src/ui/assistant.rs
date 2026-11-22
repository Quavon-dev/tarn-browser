use browser_core::assistant::{Origin, allowed};
use nsh_core::prelude::*;
use shield::ShieldLevel;
use tarn_nsh_shared::assistant::{self, Platform};
use url::Url;

use super::{Screen, Ui};
use crate::theme::Theme;

const COLUMN: f32 = 340.;

#[derive(Clone, Copy)]
pub struct Host(pub Ui);

impl assistant::Host for Host {
    fn platform(self) -> &'static Platform {
        &crate::platform::ASSISTANT
    }

    fn with_store<R>(self, f: impl FnOnce(&store::Store) -> R) -> R {
        untracked(|| self.0.read(|s| f(&s.core.disk)))
    }

    fn origin(self) -> Option<Origin> {
        untracked(|| self.0.read(|s| s.assistant_origin()))
    }

    fn level_of(self, url: &Url) -> ShieldLevel {
        untracked(|| self.0.read(|s| s.model.profile().and_then(|(private, id)| s.core.level(private, id, Some(url))))).unwrap_or(ShieldLevel::Lockdown)
    }

    fn read_page(self) {
        self.0.act_on_screen(|s| s.read_page_for_assistant());
    }

    fn open(self, url: Url) {
        self.0.act(|s| {
            s.new_tab(Some(url.as_str()));
            Vec::new()
        });
        self.0.show(Screen::Browse);
    }
}

/// Shows the chat (a sheet on a phone, a column beside the page in the Mac layout), attaching the
/// page when `page` is set.
pub fn open(ui: Ui, page: bool) {
    let host = Host(ui);
    if !ui.chat.open(host) {
        let text = if assistant::Host::origin(host).is_some_and(allowed) { assistant::OFF } else { assistant::NOT_HERE };
        return ui.act_on_screen(|s| {
            s.notice = Some(text.to_owned());
            Vec::new()
        });
    }
    ui.show(Screen::Assistant);
    if page {
        assistant::Host::read_page(host);
    }
}

pub fn attach(ui: Ui, pages: Vec<(Origin, browser_core::assistant::Pill)>) {
    for (origin, pill) in pages {
        ui.chat.attach(Host(ui), origin, pill);
    }
}

pub fn panel(ui: Ui) -> AnyElement {
    assistant::panel(Host(ui), ui.chat, move || ui.show(Screen::Browse))
}

pub fn column(ui: Ui) -> View {
    view().w(px(COLUMN)).flex_none().border_l_1().border_color(Theme::surface_edge()).child(panel(ui))
}

/// Clean Up's Group with Assistant: sends the loose tabs' ids, sites and titles and files them as
/// the checked answer says, after a confirmation.
pub fn group_tabs(ui: Ui) {
    let notice = move |text: String| {
        ui.act_on_screen(move |s| {
            s.notice = Some(text);
            Vec::new()
        })
    };
    let Some((space, request)) = ui.read(|s| s.assistant_grouping()) else {
        return notice("The assistant has nothing to group here.".to_owned());
    };
    notice("Asking the assistant to group the tabs…".to_owned());
    assistant::group_tabs(
        Host(ui),
        request,
        move |sent, proposal| {
            ui.act(|s| {
                s.group_as_proposed(space, &sent, &proposal);
                Vec::new()
            })
        },
        notice,
    );
}
