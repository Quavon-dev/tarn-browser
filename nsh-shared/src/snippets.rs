use browser_core::snippets::{EXPAND, EXPAND_NOTE, MAX_SNIPPET, NAME_RULE, NO_MATCH, NOTE, matching, preview, valid_name};
use nsh_core::prelude::*;
use store::{Id, Snippet, Store};

use crate::controls::{
    add_button, confirm, described, editor, editor_actions, empty, field, group, hint, list_row, row_action, section_with, signal_field, switch_row,
};
use crate::icons;
use crate::theme::Theme;

/// What the Snippets pane edits: the current profile's snippets in the disk store.
pub trait Host: Copy + 'static {
    /// The current profile, or none in a private space.
    fn profile(self) -> Option<Id>;
    fn with_store<R>(self, f: impl FnOnce(&Store) -> R) -> R;
    /// The profile's snippets changed; shells reload what their fields expand.
    fn changed(self);
}

/// A snippet being added (`was` none) or edited.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub was: Option<(Id, String)>,
    pub name: String,
    pub body: String,
}

impl Draft {
    /// Saves the draft for `profile` among `existing`; the message to show when it cannot.
    pub fn save(&self, store: &Store, profile: Id, existing: &[Snippet]) -> Result<(), String> {
        let name = self.name.trim();
        if !valid_name(name) {
            return Err(NAME_RULE.into());
        }
        if self.body.trim().is_empty() {
            return Err("A snippet needs some text.".into());
        }
        if self.body.len() > MAX_SNIPPET {
            return Err(format!("A snippet holds up to {} KB.", MAX_SNIPPET / 1024));
        }
        let renamed = self.was.as_ref().filter(|(_, old)| old != name);
        if (self.was.is_none() || renamed.is_some()) && existing.iter().any(|s| s.name == name) {
            return Err(format!("A snippet named {name} already exists."));
        }
        store.save_snippet(profile, name, &self.body).map_err(|_| "The snippet could not be saved.".to_owned())?;
        if let Some((id, _)) = renamed {
            store.delete_snippet(profile, *id).ok();
        }
        Ok(())
    }
}

/// Settings › Snippets: the current profile's snippets, each with its name and first line, and
/// a form to add or edit one.
pub fn pane<H: Host>(host: H) -> AnyElement {
    let rev = signal(0u64);
    let query = signal(String::new());
    let draft: Signal<Option<Draft>> = signal(None);
    let error: Signal<Option<String>> = signal(None);
    let editing = computed(move || draft.with(Option::is_some));
    let heading =
        computed(move || draft.with(|d| d.as_ref().map(|d| d.was.as_ref().map_or_else(|| "New Snippet".to_owned(), |(_, name)| format!("Edit ;{name}")))));
    let edit = move |next: Option<Draft>| {
        error.set(None);
        draft.set(next);
    };
    let add = move || add_button("Add Snippet", move || edit(Some(Draft::default())));
    dynamic(move || {
        rev.get();
        let Some(profile) = host.profile() else {
            return Some(view().gap_2().child(section_with("Snippets", NOTE)).child(group().child(empty(
                icons::LOCK,
                "Snippets belong to a profile. Edit them outside a private space.",
                None,
            ))));
        };
        let list = host.with_store(|s| s.snippets(profile).unwrap_or_default());
        let has_snippets = !list.is_empty();
        let expands = host.with_store(|s| EXPAND.on(s));
        let toggle = move || {
            host.with_store(|s| EXPAND.set(s, !expands));
            host.changed();
            rev.update(|r| *r += 1);
        };
        let rows = dynamic(move || {
            if list.is_empty() {
                return Some(group().child(empty(icons::FILE_TEXT, "No snippets yet.", (!editing.get()).then(add))));
            }
            let shown = matching(&query.get(), &list);
            if shown.is_empty() {
                return Some(group().child(empty(icons::SEARCH, NO_MATCH, None)));
            }
            Some(group().children(shown.into_iter().enumerate().map(|(ix, s)| {
                let (id, name, body) = (s.id, s.name.clone(), s.body.clone());
                let open = Draft { was: Some((id, name.clone())), name: name.clone(), body: body.clone() };
                let gone = name.clone();
                list_row(ix, format!(";{name}"), preview(&body))
                    .child(row_action(icons::PENCIL, format!("Edit {name}"), move || edit(Some(open.clone()))))
                    .child(row_action(icons::TRASH, format!("Delete {name}"), move || {
                        confirm(format!("Delete ;{gone}?"), "The snippet is removed from this profile.", "Delete", move || {
                            host.with_store(|s| s.delete_snippet(profile, id).ok());
                            host.changed();
                            rev.update(|r| *r += 1);
                        })
                    }))
                    .into_element()
            })))
        });
        let editor = dynamic(move || {
            let title = heading.get()?;
            let save = move || {
                let Some(d) = draft.get_untracked() else { return };
                let existing = host.with_store(|s| s.snippets(profile).unwrap_or_default());
                match host.with_store(|s| d.save(s, profile, &existing)) {
                    Ok(()) => {
                        edit(None);
                        host.changed();
                        rev.update(|r| *r += 1);
                    }
                    Err(message) => error.set(Some(message)),
                }
            };
            let set = move |f: &dyn Fn(&mut Draft)| draft.update(|d| d.iter_mut().for_each(f));
            Some(
                editor(title)
                    .child(field(
                        "Name",
                        move || draft.with(|d| d.as_ref().map(|d| d.name.clone()).unwrap_or_default()),
                        move |v| set(&|d| d.name = v.to_owned()),
                    ))
                    .child(hint(NAME_RULE))
                    .child(
                        field("Text", move || draft.with(|d| d.as_ref().map(|d| d.body.clone()).unwrap_or_default()), move |v| set(&|d| d.body = v.to_owned()))
                            .multiline(true)
                            .min_h(px(88.)),
                    )
                    .child(dynamic(move || error.get().map(|e| hint(e).text_color(Theme::accent_text))))
                    .child(editor_actions(save, move || edit(None))),
            )
        });
        Some(
            view()
                .gap_2()
                .child(group().child(described(switch_row("Expand ;name in the address bar", move || expands, toggle), EXPAND_NOTE)))
                .child(section_with("Snippets", NOTE))
                .when(has_snippets, |v| v.child(signal_field("Search snippets", query)))
                .child(rows)
                .child(dynamic(move || (has_snippets && !editing.get()).then(|| view().flex_row().child(add()))))
                .child(editor),
        )
    })
    .into_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_save_checked_and_a_rename_replaces_the_old_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let profile = store.create_profile("P", "u1", false).unwrap().id;
        let draft = |was: Option<(Id, String)>, name: &str, body: &str| Draft { was, name: name.into(), body: body.into() };
        assert_eq!(draft(None, ";sig", "x").save(&store, profile, &[]), Err(NAME_RULE.into()));
        assert!(draft(None, "sig", "  ").save(&store, profile, &[]).is_err());
        assert!(draft(None, "sig", &"a".repeat(MAX_SNIPPET + 1)).save(&store, profile, &[]).is_err());
        assert_eq!(draft(None, "sig", "Best, Ada").save(&store, profile, &[]), Ok(()));
        let saved = store.snippets(profile).unwrap();
        assert!(draft(None, "sig", "again").save(&store, profile, &saved).is_err(), "a new snippet does not replace one");
        let id = saved[0].id;
        assert_eq!(draft(Some((id, "sig".into())), "sign", "Best, Ada L").save(&store, profile, &saved), Ok(()));
        let after = store.snippets(profile).unwrap();
        assert_eq!(after.iter().map(|s| (s.name.as_str(), s.body.as_str())).collect::<Vec<_>>(), [("sign", "Best, Ada L")]);
        assert_eq!(draft(Some((after[0].id, "sign".into())), "sign", "Edited").save(&store, profile, &after), Ok(()));
        assert_eq!(store.snippets(profile).unwrap()[0].body, "Edited");
    }
}
