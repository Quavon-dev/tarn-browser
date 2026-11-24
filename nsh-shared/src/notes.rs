use browser_core::notes::{Draft, EMPTY, INTRO, NO_MATCH, PRIVATE, Sort, markdown, sorted};
use nsh_core::prelude::*;
use store::{Id, Note, Store};

use crate::controls::{Segment, accent_pill, confirm, empty_state, field, group, hint, list_row, pill, section, segmented, signal_field};
use crate::theme::Theme;

/// What the Notes pane edits: the current profile's notes in the disk store.
pub trait Host: Copy + 'static {
    /// The current profile, or none in a private space or burner tab.
    fn profile(self) -> Option<Id>;
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R;
    /// Opens the assistant's chat with `note` attached, under the chat's own rules.
    fn ask(self, note: &Note);
    /// Hands `markdown`, every note as one file named `notes::EXPORT_NAME`, to the system to
    /// save or share, and says when it could not.
    fn export(self, markdown: String);
}

/// Settings › Notes: the current profile's notes with a search field, each with its title and a
/// line of text, and a form to write or edit one.
pub fn pane<H: Host>(host: H) -> AnyElement {
    let rev = signal(0u64);
    let query = signal(String::new());
    let draft: Signal<Option<Draft>> = signal(None);
    let error: Signal<Option<String>> = signal(None);
    let editing = computed(move || draft.with(Option::is_some));
    let edit = move |next: Option<Draft>| {
        error.set(None);
        draft.set(next);
    };
    dynamic(move || {
        rev.get();
        let intro = hint(INTRO);
        let Some(profile) = host.profile() else {
            return Some(view().gap_2().child(intro).child(empty_state(PRIVATE)));
        };
        let saved = host.with_store(|s| s.notes(profile).unwrap_or_default());
        let sort = host.with_store(Sort::load);
        let sorts: Vec<Segment> = Sort::ALL
            .into_iter()
            .map(|choice| {
                let pick = move || {
                    host.with_store(|s| choice.save(s));
                    rev.update(|r| *r += 1);
                };
                (choice.name().to_owned(), Box::new(move || choice == sort) as Box<dyn Fn() -> bool>, Box::new(pick) as Box<dyn Fn()>)
            })
            .collect();
        let all = markdown(&saved);
        let has_notes = !saved.is_empty();
        let list = dynamic(move || {
            let shown = sorted(&saved, &query.get(), sort);
            if shown.is_empty() {
                return Some(view().child(empty_state(if saved.is_empty() { EMPTY } else { NO_MATCH })));
            }
            let rows: Vec<AnyElement> = shown
                .into_iter()
                .enumerate()
                .map(|(ix, r)| {
                    let (id, title) = (r.id, r.title.clone());
                    let note = saved.iter().find(|n| n.id == id).cloned();
                    let open = note.as_ref().map(Draft::of);
                    let (gone, named, asked) = (title.clone(), title.clone(), title.clone());
                    list_row(ix, r.title, r.preview)
                        .children(note.map(|n| pill("Ask", move || host.ask(&n)).accessibility_label(format!("Ask the assistant about {asked}"))))
                        .child(pill("Edit", move || edit(open.clone())).accessibility_label(format!("Edit {title}")))
                        .child(
                            pill("Delete", move || {
                                confirm(format!("Delete {gone}?"), "The note is removed from this profile.", "Delete", move || {
                                    host.with_store(|s| s.delete_note(profile, id).ok());
                                    rev.update(|r| *r += 1);
                                })
                            })
                            .accessibility_label(format!("Delete {named}")),
                        )
                        .into_element()
                })
                .collect();
            Some(group().children(rows))
        });
        let editor = dynamic(move || {
            editing.get().then(|| {
                let save = move || {
                    let Some(d) = draft.get_untracked() else { return };
                    match host.with_store(|s| d.save(s, profile)) {
                        Ok(_) => {
                            edit(None);
                            rev.update(|r| *r += 1);
                        }
                        Err(message) => error.set(Some(message)),
                    }
                };
                let set = move |f: &dyn Fn(&mut Draft)| draft.update(|d| d.iter_mut().for_each(f));
                group()
                    .p_3()
                    .gap_2()
                    .child(field(
                        "Title",
                        move || draft.with(|d| d.as_ref().map(|d| d.title.clone()).unwrap_or_default()),
                        move |v| set(&|d| d.title = v.to_owned()),
                    ))
                    .child(
                        field("Note", move || draft.with(|d| d.as_ref().map(|d| d.body.clone()).unwrap_or_default()), move |v| set(&|d| d.body = v.to_owned()))
                            .multiline(true)
                            .min_h(px(120.)),
                    )
                    .child(dynamic(move || error.get().map(|e| hint(e).text_color(Theme::accent_text))))
                    .child(view().flex_row().gap_2().child(accent_pill("Save", save)).child(pill("Cancel", move || edit(None))))
            })
        });
        Some(
            view()
                .gap_2()
                .child(intro)
                .child(section("Notes"))
                .child(signal_field("Search notes", query))
                .when(has_notes, |v| v.child(segmented(sorts)))
                .child(list)
                .child(dynamic(move || {
                    (!editing.get()).then(|| {
                        let all = all.clone();
                        view()
                            .flex_row()
                            .flex_wrap()
                            .gap_2()
                            .child(pill("New Note", move || edit(Some(Draft::default()))))
                            .when(has_notes, |v| v.child(pill("Export", move || host.export(all.clone())).accessibility_label("Export Notes as Markdown")))
                    })
                }))
                .child(editor),
        )
    })
    .into_element()
}
