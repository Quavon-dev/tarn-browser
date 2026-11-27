use std::time::Duration;

use browser_core::switcher::Switcher;
use nsh_core::prelude::*;
use tarn_nsh_shared::switcher::{Card, strip};

use super::{Ui, act, peek, read};
use crate::browser::Browser;
use crate::theme::themed;

/// How often a held Ctrl is checked; NSH reports no key release, GDK reports what is held.
const POLL: Duration = Duration::from_millis(50);

pub(super) fn press(ui: Ui, forward: bool) {
    act(ui, |b| Switcher::press(&mut b.switcher, &b.model, forward));
    if let Some(at) = presses(ui) {
        watch(ui, at);
    }
}

fn presses(ui: Ui) -> Option<u64> {
    read(ui, |b| b.switcher.as_ref().map(|s| s.presses)).flatten()
}

fn watch(ui: Ui, at: u64) {
    peek::later(ui, POLL, move || {
        if presses(ui) != Some(at) {
            return;
        }
        if peek::held(webkit6::gdk::ModifierType::CONTROL_MASK) {
            watch(ui, at);
        } else {
            act(ui, Browser::commit_switcher);
        }
    });
}

pub(super) fn layer(ui: Ui) -> AnyElement {
    let open = computed(move || ui.snap.with(|s| s.switcher.clone()));
    themed(move || {
        let (cards, selected) = open.get()?;
        let cards = cards
            .into_iter()
            .map(|(host, title)| Card { icon: ui.icons.with_untracked(|i| i.find(&host)).map(|icon| icon.to_string()), host, title })
            .collect();
        let pick = move |ix| {
            act(ui, |b| {
                b.switcher.iter_mut().for_each(|s| s.index = ix);
                b.commit_switcher();
            })
        };
        Some(strip(cards, selected, pick, move || act(ui, Browser::commit_switcher), move || act(ui, |b| b.switcher = None)))
    })
}
