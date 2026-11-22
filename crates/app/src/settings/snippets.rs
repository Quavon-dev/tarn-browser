use browser_core::snippets::{NAME_RULE, NOTE, preview, valid_name};
use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};
use store::{Id, Snippet};
use ui::controls::{button, field, group, note, row, section, tab_stop};
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::Theme;

use super::{SettingsWindow, Snapshot, label, value};

pub(super) struct Pane {
    name: Entity<TextInput>,
    body: Entity<TextInput>,
    editing: Option<Snippet>,
    error: Option<String>,
    _submit: [Subscription; 2],
}

impl Pane {
    pub(super) fn new(cx: &mut Context<SettingsWindow>) -> Pane {
        let name = cx.new(|cx| TextInput::new("name", cx));
        let body = cx.new(|cx| TextInput::new("Text to insert", cx));
        let submit = |this: &mut SettingsWindow, _: Entity<TextInput>, event: &TextInputEvent, cx: &mut Context<SettingsWindow>| {
            if matches!(event, TextInputEvent::Submit) {
                this.save_snippet(cx);
            }
        };
        Pane { _submit: [cx.subscribe(&name, submit), cx.subscribe(&body, submit)], name, body, editing: None, error: None }
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

    pub(super) fn snippets(&self, s: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let page = div().pt_3().child(note(NOTE));
        let Some(profile) = s.profile.as_ref().filter(|_| !s.private).map(|p| p.id) else {
            return page.child(note("Snippets belong to a saved profile. Open Settings from one of its spaces to edit them.")).into_any_element();
        };
        let list = if s.snippets.is_empty() {
            note("No snippets yet.").into_any_element()
        } else {
            let mut list = group();
            for (ix, snippet) in s.snippets.iter().enumerate() {
                let (id, edit) = (snippet.id, snippet.clone());
                list = list.child(
                    row(ix == 0)
                        .child(div().w(px(140.)).flex_none().child(label(snippet.name.clone())))
                        .child(value(preview(&snippet.body)).flex_1().min_w_0().overflow_hidden())
                        .child(
                            tab_stop(button(("snippet-edit", ix), "Edit", false))
                                .on_click(cx.listener(move |this, _, window, cx| this.edit_snippet(edit.clone(), window, cx))),
                        )
                        .child(
                            tab_stop(button(("snippet-delete", ix), "Delete", false))
                                .on_click(cx.listener(move |this, _, _, cx| this.delete_snippet(profile, id, cx))),
                        ),
                );
            }
            list.into_any_element()
        };
        let pane = &self.snippets;
        let focused = |input: &Entity<TextInput>| input.focus_handle(cx).is_focused(window);
        let editing = pane.editing.is_some();
        let editor = group()
            .child(row(true).child(label("Name")).child(field(focused(&pane.name)).w(px(260.)).py_0p5().child(pane.name.clone())))
            .child(row(false).child(label("Text")).child(field(focused(&pane.body)).w(px(420.)).py_0p5().child(pane.body.clone())));
        let actions = div()
            .pt_2()
            .flex()
            .justify_end()
            .gap_2()
            .when(editing, |d| {
                d.child(tab_stop(button("snippet-cancel", "Cancel", false)).on_click(cx.listener(|this, _, _, cx| this.clear_snippet_editor(cx))))
            })
            .child(tab_stop(button("snippet-save", if editing { "Save" } else { "Add" }, true)).on_click(cx.listener(|this, _, _, cx| this.save_snippet(cx))));
        page.child(list)
            .child(section(if editing { "Edit snippet" } else { "New snippet" }))
            .child(editor)
            .child(note(NAME_RULE))
            .children(pane.error.clone().map(|e| note(e).text_color(Theme::warning())))
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
