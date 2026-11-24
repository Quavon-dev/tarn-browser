use browser_core::snippets::{EXPAND, EXPAND_NOTE, NAME_RULE, NO_MATCH, NOTE, matching, preview, valid_name};
use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder,
};
use store::{Id, Snippet};
use ui::controls::{empty_state, group, icon_text_button, note, row, row_action, section, switch_row, text_block, text_lines};
use ui::icon::Icon;
use ui::text_input::{TextInput, TextInputEvent};

use super::{SettingsWindow, Snapshot, editor_actions, editor_row, list_heading, search_field, warning};

pub(super) struct Pane {
    search: Entity<TextInput>,
    name: Entity<TextInput>,
    body: Entity<TextInput>,
    editing: Option<Snippet>,
    adding: bool,
    error: Option<String>,
    _inputs: [Subscription; 3],
}

impl Pane {
    pub(super) fn new(cx: &mut Context<SettingsWindow>) -> Pane {
        let search = cx.new(|cx| TextInput::new("Search snippets", cx));
        let name = cx.new(|cx| TextInput::new("name", cx));
        let body = cx.new(|cx| TextInput::new("Text to insert", cx));
        let submit = |this: &mut SettingsWindow, _: Entity<TextInput>, event: &TextInputEvent, cx: &mut Context<SettingsWindow>| match event {
            TextInputEvent::Submit => this.save_snippet(cx),
            TextInputEvent::Cancel => {
                this.clear_snippet_editor(cx);
                cx.notify();
            }
            _ => {}
        };
        let filter = |_: &mut SettingsWindow, _: Entity<TextInput>, event: &TextInputEvent, cx: &mut Context<SettingsWindow>| {
            if matches!(event, TextInputEvent::Changed) {
                cx.notify();
            }
        };
        Pane {
            _inputs: [cx.subscribe(&search, filter), cx.subscribe(&name, submit), cx.subscribe(&body, submit)],
            search,
            name,
            body,
            editing: None,
            adding: false,
            error: None,
        }
    }
}

/// Why a snippet cannot be saved, before the store is asked.
fn problem(name: &str, body: &str) -> Option<&'static str> {
    if !valid_name(name) {
        Some(NAME_RULE)
    } else if body.trim().is_empty() {
        Some("Write the text the snippet inserts.")
    } else {
        None
    }
}

impl SettingsWindow {
    fn save_snippet(&mut self, cx: &mut Context<Self>) {
        let Some(profile) = self.saved_profile(cx) else { return };
        let pane = &self.snippets;
        let (name, body) = (pane.name.read(cx).text().trim().to_owned(), pane.body.read(cx).text().to_owned());
        if let Some(problem) = problem(&name, &body) {
            self.snippets.error = Some(problem.to_owned());
            return cx.notify();
        }
        let core = crate::shared::core(cx);
        let disk = &core.disk;
        match disk.save_snippet(profile, &name, &body) {
            Ok(_) => {
                if let Some(old) = self.snippets.editing.take().filter(|old| old.name != name) {
                    disk.delete_snippet(profile, old.id).ok();
                }
                self.clear_snippet_editor(cx);
                snippets_changed(cx);
            }
            Err(e) => self.snippets.error = Some(e.to_string()),
        }
        cx.notify();
    }

    fn clear_snippet_editor(&mut self, cx: &mut Context<Self>) {
        self.snippets.editing = None;
        self.snippets.adding = false;
        self.snippets.error = None;
        self.snippets.name.update(cx, |i, cx| i.set_text("", cx));
        self.snippets.body.update(cx, |i, cx| i.set_text("", cx));
    }

    fn edit_snippet(&mut self, snippet: Snippet, window: &mut Window, cx: &mut Context<Self>) {
        self.snippets.name.update(cx, |i, cx| i.set_text(snippet.name.clone(), cx));
        self.snippets.body.update(cx, |i, cx| i.set_text(snippet.body.clone(), cx));
        window.focus(&self.snippets.body.focus_handle(cx));
        self.snippets.editing = Some(snippet);
        self.snippets.error = None;
        cx.notify();
    }

    fn delete_snippet(&mut self, profile: Id, id: Id, cx: &mut Context<Self>) {
        if crate::shared::core(cx).disk.delete_snippet(profile, id).is_ok() {
            if self.snippets.editing.as_ref().is_some_and(|s| s.id == id) {
                self.clear_snippet_editor(cx);
            }
            snippets_changed(cx);
        }
        cx.notify();
    }

    fn add_snippet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_snippet_editor(cx);
        self.snippets.adding = true;
        window.focus(&self.snippets.name.focus_handle(cx));
        cx.notify();
    }

    pub(super) fn snippets(&self, s: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(profile) = s.profile.as_ref().filter(|_| !s.private).map(|p| p.id) else {
            return div()
                .pt_6()
                .child(empty_state(Icon::Lock, "Snippets belong to a saved profile. Open Settings from one of its spaces to edit them."))
                .into_any_element();
        };
        let pane = &self.snippets;
        let focused = |input: &Entity<TextInput>| input.focus_handle(cx).is_focused(window);
        let expand = EXPAND.on(&crate::shared::core(cx).disk);
        let toggle =
            group().child(switch_row("snippet-expand", true, text_block("Expand ;name in the address and command bar", EXPAND_NOTE), expand).on_click(
                cx.listener(move |_, _, _, cx| {
                    EXPAND.set(&crate::shared::core(cx).disk, !expand);
                    cx.notify();
                }),
            ));
        let editing = pane.editing.is_some();
        let open = editing || pane.adding;
        let add = || icon_text_button("snippet-add", Icon::Plus, "Add Snippet").on_click(cx.listener(|this, _, window, cx| this.add_snippet(window, cx)));
        let shown = matching(pane.search.read(cx).text(), &s.snippets);
        let list = if s.snippets.is_empty() {
            empty_state(Icon::FileText, "No snippets yet.").child(div().pt_1().children((!open).then(add))).into_any_element()
        } else if shown.is_empty() {
            empty_state(Icon::Search, NO_MATCH).into_any_element()
        } else {
            group()
                .children(shown.into_iter().enumerate().map(|(ix, snippet)| {
                    let (id, edit) = (snippet.id, snippet.clone());
                    row(ix == 0)
                        .child(text_lines(snippet.name.clone(), preview(&snippet.body)))
                        .child(
                            row_action(("snippet-edit", ix), Icon::Pencil, "Edit")
                                .on_click(cx.listener(move |this, _, window, cx| this.edit_snippet(edit.clone(), window, cx))),
                        )
                        .child(
                            row_action(("snippet-delete", ix), Icon::Trash, "Delete")
                                .on_click(cx.listener(move |this, _, _, cx| this.delete_snippet(profile, id, cx))),
                        )
                }))
                .into_any_element()
        };
        let page = div()
            .child(section("Expansion"))
            .child(toggle)
            .child(note(NOTE))
            .child(list_heading("Your snippets", (!open && !s.snippets.is_empty()).then(add)))
            .when(!s.snippets.is_empty(), |d| d.child(div().pb_2().child(search_field(&pane.search, focused(&pane.search)))))
            .child(list);
        if !open {
            return page.into_any_element();
        }
        let actions = editor_actions(
            "snippet-editor",
            editing,
            Box::new(cx.listener(|this, _, _, cx| {
                this.clear_snippet_editor(cx);
                cx.notify();
            })),
            Box::new(cx.listener(|this, _, _, cx| this.save_snippet(cx))),
        );
        page.child(section(if editing { "Edit snippet" } else { "New snippet" }))
            .child(group().child(editor_row(true, "Name", &pane.name, focused(&pane.name))).child(editor_row(false, "Text", &pane.body, focused(&pane.body))))
            .child(note(NAME_RULE))
            .children(pane.error.clone().map(warning))
            .child(actions)
            .into_any_element()
    }
}

/// Every window reads the profile's snippets again on the next field edit.
fn snippets_changed(cx: &mut gpui::App) {
    crate::shared::each(cx, |b, _, _| b.core.snippets_changed());
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_snippet_needs_a_valid_name_and_some_text() {
        assert_eq!(super::problem("Sig", "x"), Some(super::NAME_RULE));
        assert!(super::problem("sig", "  \n").is_some());
        assert_eq!(super::problem("sig", "Best,\nAda"), None);
    }
}
