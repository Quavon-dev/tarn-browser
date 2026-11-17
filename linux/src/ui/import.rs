use std::cell::Cell;
use std::time::Duration;

use browser_core::import::{self as data, Kinds, Report, Source};
use browser_core::text::bounded_title;
use futures_channel::oneshot;
use futures_util::future::{Either, select};
use nsh_core::prelude::*;

use super::panels::{heading, popover};
use super::tools::note;
use super::{Panel, Ui, act, core, read, show};
use crate::controls::{accent_pill, label, pill, small, title};
use crate::theme::Theme;

const IMPORT_TIMEOUT: Duration = Duration::from_secs(120);
const THIS_DEVICE: &str = "this computer";

thread_local! {
    static IMPORTING: Cell<bool> = const { Cell::new(false) };
}

struct Importing;

impl Drop for Importing {
    fn drop(&mut self) {
        IMPORTING.with(|i| i.set(false));
    }
}

#[derive(Clone, Default, PartialEq)]
pub enum Step {
    #[default]
    Sources,
    Confirm(usize),
    Refused,
    Running(&'static str),
    Done(&'static str, String),
}

#[derive(Clone, Default, PartialEq)]
pub struct State {
    pub sources: Vec<Source>,
    pub step: Step,
}

pub fn open(ui: Ui) {
    let sources = std::env::home_dir().map(|home| data::sources(&home)).unwrap_or_default();
    ui.import.set(State { sources, step: Step::Sources });
    if ui.panel.get_untracked() != Panel::Import {
        show(ui, Panel::Import);
    }
}

fn target(ui: Ui) -> Option<(bool, store::Id)> {
    read(ui, |b| b.model.current_space().map(|s| (s.private, s.space.profile_id))).flatten()
}

fn choose(ui: Ui, ix: usize) {
    let refused = target(ui).is_none_or(|(private, id)| core().import_target(private, id).is_err());
    ui.import.update(|s| s.step = if refused { Step::Refused } else { Step::Confirm(ix) });
}

fn start(ui: Ui, source: Source, cookies: bool) {
    let Some((false, profile_id)) = target(ui) else { return ui.import.update(|s| s.step = Step::Refused) };
    if IMPORTING.with(|i| i.replace(true)) {
        return ui.import.update(|s| s.step = Step::Done(source.browser.name(), "An import is already running.".into()));
    }
    let running = Importing;
    let browser = source.browser;
    ui.import.update(|s| s.step = Step::Running(browser.name()));
    let kinds = Kinds { cookies, history: true, bookmarks: true };
    ui.root.run(|| {
        drop(spawn(async move {
            let read = blocking(move || data::read(&source, kinds)).await.unwrap_or_default();
            let (tx, rx) = oneshot::channel::<Report>();
            let summary = match core().import(profile_id, browser, read, move |r| drop(tx.send(r))) {
                Ok(()) => match select(rx, Box::pin(timer(IMPORT_TIMEOUT))).await {
                    Either::Left((report, _)) => data::summary(browser.name(), report.ok().as_ref()),
                    Either::Right(_) => data::summary(browser.name(), None),
                },
                Err(_) => "Nothing was imported: this profile can no longer take imported data.".into(),
            };
            drop(running);
            ui.history.set(Default::default());
            ui.import.update(|s| s.step = Step::Done(browser.name(), summary));
            act(ui, |_| {});
        }))
    });
}

fn confirm_rows(ui: Ui, source: &Source) -> View {
    let profile = target(ui).and_then(|(_, id)| core().disk.profile(id).ok()).map(|p| p.name).unwrap_or_default();
    let data::Confirm { message, detail, cookies } = data::confirm(source, &profile, THIS_DEVICE);
    let (all, some) = (source.clone(), source.clone());
    let buttons = view().flex_row().flex_wrap().gap_2();
    let buttons = if cookies {
        buttons
            .child(accent_pill("Import All", move || start(ui, all.clone(), true)))
            .child(pill("History and Bookmarks Only", move || start(ui, some.clone(), false)))
    } else {
        buttons.child(accent_pill("Import", move || start(ui, all.clone(), false)))
    };
    view()
        .flex_col()
        .gap_2()
        .child(label(message))
        .child(note(detail))
        .child(buttons.child(pill("Cancel", move || ui.import.update(|s| s.step = Step::Sources))))
}

fn source_rows(ui: Ui, sources: &[Source]) -> View {
    if sources.is_empty() {
        return view().child(note("No supported browser found"));
    }
    view().flex_col().gap(px(1.)).children(sources.iter().enumerate().map(|(ix, s)| {
        crate::controls::row(Some(SharedString::from(format!("{} {}", s.browser.name(), bounded_title(&s.label)))), move || choose(ui, ix))
            .h(px(40.))
            .child(view().flex_col().flex_1().min_w(px(0.)).child(label(s.browser.name().to_owned())).child(small(bounded_title(&s.label)).truncate()))
            .into_element()
    }))
}

pub fn panel(ui: Ui) -> AnyElement {
    let State { sources, step } = ui.import.get();
    let content = match &step {
        Step::Sources => source_rows(ui, &sources),
        Step::Confirm(ix) => match sources.get(*ix) {
            Some(source) => confirm_rows(ui, source),
            None => source_rows(ui, &sources),
        },
        Step::Refused => view()
            .flex_col()
            .gap_2()
            .child(label("Switch to a space of a normal profile to import into it."))
            .child(note("Private and Lockdown profiles keep website data in memory only, so imported sign-ins would not last."))
            .child(view().flex_row().child(pill("OK", move || ui.import.update(|s| s.step = Step::Sources)))),
        Step::Running(name) => view().child(note(format!("Importing from {name}\u{2026}"))),
        Step::Done(name, summary) => view()
            .flex_col()
            .gap_2()
            .child(label(format!("Import from {name}")))
            .children(summary.lines().map(|l| small(l.to_owned()).text_color(Theme::text()).into_element()))
            .child(view().flex_row().child(pill("Done", move || ui.import.update(|s| s.step = Step::Sources)))),
    };
    popover().child(heading(ui, title("Import Browser Data"))).child(content).into_element()
}
