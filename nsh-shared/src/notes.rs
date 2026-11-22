use browser_core::notes::{Draft, EMPTY, INTRO, NO_MATCH, PRIVATE, rows};
use nsh_base::{Input, input};
use nsh_core::prelude::*;
use store::{Id, Store};

use crate::controls::{accent_pill, confirm, empty_state, group, label, pill, section};
use crate::theme::{TEXT_SMALL, Theme};

/// What the Notes pane edits: the current profile's notes in the disk store.
pub trait Host: Copy + 'static {
    /// The current profile, or none in a private space or burner tab.
    fn profile(self) -> Option<Id>;
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R;
}

fn field(name: &'static str, value: impl Fn() -> String + 'static, set: impl Fn(&str) + 'static) -> Input {
    input()
        .value(value)
        .on_change(move |v: &str| set(v))
        .label(name)
        .placeholder(name)
        .w_full()
        .px(px(8.))
        .py(px(6.))
        .rounded(px(Theme::radius_row()))
        .bg(Theme::raised())
        .border_1()
        .border_color(Theme::hairline())
        .text_color(Theme::text())
}

fn hint(content: impl IntoReactive<SharedString>) -> Text {
    text(content).text_size(px(TEXT_SMALL)).text_color(Theme::muted).px_1()
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
        let list = dynamic(move || {
            let shown = rows(&saved, &query.get());
            if shown.is_empty() {
                return Some(view().child(empty_state(if saved.is_empty() { EMPTY } else { NO_MATCH })));
            }
            let rows: Vec<AnyElement> = shown
                .into_iter()
                .enumerate()
                .map(|(ix, r)| {
                    let (id, title) = (r.id, r.title.clone());
                    let open = saved.iter().find(|n| n.id == id).map(Draft::of);
                    let (gone, named) = (title.clone(), title.clone());
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .when(ix > 0, |v| v.border_t_1().border_color(Theme::separator))
                        .child(
                            view()
                                .flex_1()
                                .min_w(px(0.))
                                .child(label(r.title).font_weight(FontWeight::MEDIUM).number_of_lines(1_u32))
                                .child(hint(r.preview).number_of_lines(1_u32)),
                        )
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
                .child(field("Search notes", move || query.get(), move |v| query.set(v.to_owned())))
                .child(list)
                .child(dynamic(move || (!editing.get()).then(|| view().flex_row().child(pill("New Note", move || edit(Some(Draft::default())))))))
                .child(editor),
        )
    })
    .into_element()
}
