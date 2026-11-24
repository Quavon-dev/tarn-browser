use std::sync::Arc;

use browser_core::assistant::{Origin, Pill};
use browser_core::tabs::{TabChoice, TabKey};
use nsh_core::prelude::*;
use shield::ShieldLevel;
use tarn_nsh_shared::assistant::{self, Platform};
use tarn_nsh_shared::theme::Theme;
use url::Url;

use super::{Ui, act, read};
use crate::browser::Browser;

pub const WIDTH: f32 = 360.;

fn transport() -> Arc<dyn ai::Transport> {
    Arc::new(ai::UreqTransport)
}

static PLATFORM: Platform = Platform { secrets: &ai::secrets::Keyring, transport, keyring: "the Secret Service keyring", ollama_gap: None, on_device: None };

#[derive(Clone, Copy)]
pub struct Host(pub Ui);

impl assistant::Host for Host {
    fn platform(self) -> &'static Platform {
        &PLATFORM
    }

    fn with_store<R>(self, f: impl FnOnce(&store::Store) -> R) -> R {
        f(&super::core().disk)
    }

    fn origin(self) -> Option<Origin> {
        read(self.0, Browser::assistant_origin).flatten()
    }

    fn level_of(self, url: &Url) -> ShieldLevel {
        read(self.0, |b| b.model.profile().and_then(|(private, id)| b.core.level(private, id, Some(url)))).flatten().unwrap_or(ShieldLevel::Lockdown)
    }

    fn level_in(self, profile: store::Id, url: &Url) -> ShieldLevel {
        read(self.0, |b| b.core.level(false, profile, Some(url))).flatten().unwrap_or(ShieldLevel::Lockdown)
    }

    fn read_page(self) {
        act(self.0, Browser::read_page_for_assistant);
    }

    fn tabs(self) -> Vec<TabChoice> {
        read(self.0, |b| b.model.assistant_tabs(&b.core, |key, t| b.current_url(key).unwrap_or_else(|| t.url.clone()))).unwrap_or_default()
    }

    fn read_tab(self, key: TabKey) {
        act(self.0, |b| b.read_tab_for_assistant(key));
    }

    fn open(self, url: Url) {
        act(self.0, |b| {
            b.new_tab(url.as_str());
        });
    }

    fn changed(self) {
        act(self.0, |_| ());
    }
}

/// Shows the chat beside the page, attaching the page when `page` is set.
pub fn open(ui: Ui, page: bool) {
    let host = Host(ui);
    if !ui.chat.open(host) {
        let text = if assistant::Host::origin(host).is_some_and(browser_core::assistant::allowed) { assistant::OFF } else { assistant::NOT_HERE };
        return act(ui, |b| b.show_toast(text, None));
    }
    ui.assistant.set(true);
    if page {
        assistant::Host::read_page(host);
    }
}

/// Attaches what was read for the chat; a selection opens the chat first and a selection's
/// command sends it.
pub fn attach(ui: Ui, pages: Vec<(Origin, Pill, Option<String>)>) {
    for (origin, pill, command) in pages {
        if matches!(pill, Pill::Selection { .. }) && !ui.assistant.get_untracked() {
            open(ui, false);
        }
        ui.chat.ask_selection(Host(ui), origin, pill, command.as_deref());
    }
}

pub fn panel(ui: Ui) -> View {
    view().w(px(WIDTH)).flex_none().rounded(px(Theme::radius_card())).overflow_hidden().border_1().border_color(Theme::surface_edge()).child(assistant::panel(
        Host(ui),
        ui.chat,
        move || ui.assistant.set(false),
    ))
}

/// Clean Up's Group with Assistant: sends the loose tabs' ids, sites and titles and files them as
/// the checked answer says, after a confirmation.
pub fn group_tabs(ui: Ui) {
    let Some((space, request)) = read(ui, Browser::assistant_grouping).flatten() else {
        return act(ui, |b| b.show_toast("The assistant has nothing to group here", None));
    };
    act(ui, |b| b.show_toast("Asking the assistant to group the tabs…", None));
    assistant::group_tabs(
        Host(ui),
        request,
        move || read(ui, |b| browser_core::sidebar::grouping_here(&b.model, &b.core, space)) == Some(true),
        move |sent, proposal| act(ui, |b| b.apply_assistant_groups(space, &sent, &proposal)),
        move |why| act(ui, |b| b.show_toast_as(browser_core::toast::Tone::Warning, why, None)),
    );
}
