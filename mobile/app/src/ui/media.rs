use nsh_core::prelude::*;
use tarn_nsh_shared::mini_player::{every, mini_player};

use super::{Screen, Ui};

/// Asks the shown tab for its media every `media::POLL` while it plays; registered once per app.
pub fn poll(ui: Ui) {
    let polls = computed(move || ui.read(|s| s.player.polls()));
    every(browser_core::media::POLL, move || polls.get(), move || ui.act_on_screen(|s| s.poll_media()));
}

/// The mini-player; `in_sidebar` places it in the iPad sidebar, else above the bottom bar.
pub fn bar(ui: Ui, in_sidebar: bool) -> impl IntoElement {
    let here = move || in_sidebar == (ui.mac.get() && ui.read_data(|s| s.sidebar_open)) && (in_sidebar || ui.browsing());
    let media = move || here().then(|| ui.read(|s| s.mini_player().map(|(_, title, playing)| (title, playing)))).flatten();
    let open = move || {
        if let Some(key) = ui.read(|s| s.mini_player().map(|m| m.0)) {
            ui.show(Screen::Browse);
            ui.act(|s| {
                s.activate(key);
                Vec::new()
            });
        }
    };
    let player = mini_player(media, open, move |control| ui.act_on_screen(|s| s.press_media(control)));
    if in_sidebar {
        view().px_2().pb_2().child(player)
    } else {
        view().absolute().left(px(0.)).right(px(0.)).bottom(px(0.)).z_index(1).px_3().pb_2().pointer_events(PointerEvents::BoxNone).child(player)
    }
}
