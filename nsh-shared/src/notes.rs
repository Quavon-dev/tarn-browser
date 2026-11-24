use browser_core::notes::{Draft, EMPTY, INTRO, NO_MATCH, PRIVATE, Sort, markdown, sorted};
use nsh_core::prelude::*;
use store::{Id, Note, Store};

use crate::controls::{
    Segment, add_button, confirm, editor, editor_actions, empty, field, group, hint, open_row, row_action, section_with, segmented, signal_field,
};
use crate::icons;
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
    let heading = computed(move || draft.with(|d| d.as_ref().map(|d| if d.id.is_none() { "New Note" } else { "Edit Note" })));
    let edit = move |next: Option<Draft>| {
        error.set(None);
        draft.set(next);
    };
    let add = move || add_button("New Note", move || edit(Some(Draft::default())));
    dynamic(move || {
        rev.get();
        let Some(profile) = host.profile() else {
            return Some(view().gap_2().child(section_with("Notes", INTRO)).child(group().child(empty(icons::LOCK, PRIVATE, None))));
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
        let has_notes = !saved.is_empty();
        let list = dynamic(move || {
            if saved.is_empty() {
                return Some(group().child(empty(icons::PENCIL, EMPTY, (!editing.get()).then(add))));
            }
            let shown = sorted(&saved, &query.get(), sort);
            if shown.is_empty() {
                return Some(group().child(empty(icons::SEARCH, NO_MATCH, None)));
            }
            let rows: Vec<AnyElement> = shown
                .into_iter()
                .enumerate()
                .map(|(ix, r)| {
                    let (id, title) = (r.id, r.title.clone());
                    let note = saved.iter().find(|n| n.id == id).cloned();
                    let open = note.as_ref().map(Draft::of);
                    let gone = title.clone();
                    open_row(ix, r.title, r.preview, format!("Edit {title}"), move || edit(open.clone()))
                        .children(note.map(|n| row_action(icons::SPARKLES, format!("Ask the assistant about {title}"), move || host.ask(&n))))
                        .child(row_action(icons::TRASH, format!("Delete {gone}"), move || {
                            confirm(format!("Delete {gone}?"), "The note is removed from this profile.", "Delete", move || {
                                host.with_store(|s| s.delete_note(profile, id).ok());
                                rev.update(|r| *r += 1);
                            })
                        }))
                        .into_element()
                })
                .collect();
            Some(group().children(rows))
        });
        let editor = dynamic(move || {
            let title = heading.get()?;
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
            Some(
                editor(title)
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
                    .child(editor_actions(save, move || edit(None))),
            )
        });
        let top = view().flex_row().items_end().gap_2().child(section_with("Notes", INTRO).flex_1()).when(has_notes, |v| {
            v.child(row_action(icons::FILE_DOWN, "Export Notes as Markdown", move || {
                host.export(markdown(&host.with_store(|s| s.notes(profile).unwrap_or_default())))
            }))
        });
        Some(
            view()
                .gap_2()
                .child(top)
                .when(has_notes, |v| v.child(signal_field("Search notes", query)).child(segmented(sorts)))
                .child(list)
                .child(dynamic(move || (has_notes && !editing.get()).then(|| view().flex_row().child(add()))))
                .child(editor),
        )
    })
    .into_element()
}
