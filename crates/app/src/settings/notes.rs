use browser_core::notes::{Draft, EMPTY, INTRO, NO_MATCH, PRIVATE, rows};
use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px,
};
use store::{Id, Note};
use ui::controls::{button, field, group, note, row, section, tab_stop};
use ui::text_input::{TextInput, TextInputEvent};
use ui::theme::Theme;

use super::{SettingsWindow, Snapshot, label, value};

pub(super) struct Pane {
    search: Entity<TextInput>,
    title: Entity<TextInput>,
    body: Entity<TextInput>,
    editing: Option<Id>,
    error: Option<String>,
    _inputs: [Subscription; 3],
}

impl Pane {
    pub(super) fn new(cx: &mut Context<SettingsWindow>) -> Pane {
        let search = cx.new(|cx| TextInput::new("Search notes", cx));
        let title = cx.new(|cx| TextInput::new("Title", cx));
        let body = cx.new(|cx| TextInput::new("Note", cx));
        let submit = |this: &mut SettingsWindow, _: Entity<TextInput>, event: &TextInputEvent, cx: &mut Context<SettingsWindow>| {
            if matches!(event, TextInputEvent::Submit) {
                this.save_note(cx);
            }
        };
        let filter = |_: &mut SettingsWindow, _: Entity<TextInput>, event: &TextInputEvent, cx: &mut Context<SettingsWindow>| {
            if matches!(event, TextInputEvent::Changed) {
                cx.notify();
            }
        };
        Pane {
            _inputs: [cx.subscribe(&search, filter), cx.subscribe(&title, submit), cx.subscribe(&body, submit)],
            search,
            title,
            body,
            editing: None,
            error: None,
        }
    }
}

impl SettingsWindow {
    /// The saved profile Settings edits notes and snippets for; none in a private space.
    pub(super) fn saved_profile(&self, cx: &Context<Self>) -> Option<Id> {
        match self.browser.upgrade()?.read(cx).model.profile()? {
            (false, id) => Some(id),
            (true, _) => None,
        }
    }

    fn save_note(&mut self, cx: &mut Context<Self>) {
        let Some(profile) = self.saved_profile(cx) else { return };
        let pane = &self.notes;
        let draft = Draft { id: pane.editing, title: pane.title.read(cx).text().to_owned(), body: pane.body.read(cx).text().to_owned() };
        match draft.save(&crate::shared::core(cx).disk, profile) {
            Ok(_) => self.clear_note_editor(cx),
            Err(e) => self.notes.error = Some(e),
        }
        cx.notify();
    }

    fn clear_note_editor(&mut self, cx: &mut Context<Self>) {
        self.notes.editing = None;
        self.notes.error = None;
        self.notes.title.update(cx, |i, cx| i.set_text("", cx));
        self.notes.body.update(cx, |i, cx| i.set_text("", cx));
    }

    fn edit_note(&mut self, note: Note, window: &mut Window, cx: &mut Context<Self>) {
        self.notes.title.update(cx, |i, cx| i.set_text(note.title.clone(), cx));
        self.notes.body.update(cx, |i, cx| i.set_text(note.body.clone(), cx));
        window.focus(&self.notes.body.focus_handle(cx));
        self.notes.editing = Some(note.id);
        self.notes.error = None;
        cx.notify();
    }

    fn delete_note(&mut self, profile: Id, id: Id, cx: &mut Context<Self>) {
        if crate::shared::core(cx).disk.delete_note(profile, id).is_ok() && self.notes.editing == Some(id) {
            self.clear_note_editor(cx);
        }
        cx.notify();
    }

    pub(super) fn notes(&self, s: &Snapshot, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let page = div().pt_3().child(note(INTRO));
        let Some(profile) = s.profile.as_ref().filter(|_| !s.private).map(|p| p.id) else {
            return page.child(note(PRIVATE)).into_any_element();
        };
        let pane = &self.notes;
        let focused = |input: &Entity<TextInput>| input.focus_handle(cx).is_focused(window);
        let saved = &s.notes;
        let query = pane.search.read(cx).text().to_owned();
        let shown = rows(saved, &query);
        let list = if shown.is_empty() {
            note(if saved.is_empty() { EMPTY } else { NO_MATCH }).into_any_element()
        } else {
            let mut list = group();
            for (ix, r) in shown.into_iter().enumerate() {
                let (id, edit) = (r.id, saved.iter().find(|n| n.id == r.id).cloned());
                list = list.child(
                    row(ix == 0)
                        .child(div().w(px(180.)).flex_none().child(label(r.title)))
                        .child(value(r.preview).flex_1().min_w_0().overflow_hidden())
                        .child(tab_stop(button(("note-edit", ix), "Edit", false)).on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(note) = edit.clone() {
                                this.edit_note(note, window, cx);
                            }
                        })))
                        .child(
                            tab_stop(button(("note-delete", ix), "Delete", false))
                                .on_click(cx.listener(move |this, _, _, cx| this.delete_note(profile, id, cx))),
                        ),
                );
            }
            list.into_any_element()
        };
        let editing = pane.editing.is_some();
        let editor = group()
            .child(row(true).child(label("Title")).child(field(focused(&pane.title)).w(px(420.)).py_0p5().child(pane.title.clone())))
            .child(row(false).child(label("Note")).child(field(focused(&pane.body)).w(px(420.)).py_0p5().child(pane.body.clone())));
        let actions = div()
            .pt_2()
            .flex()
            .justify_end()
            .gap_2()
            .when(editing, |d| d.child(tab_stop(button("note-cancel", "Cancel", false)).on_click(cx.listener(|this, _, _, cx| this.clear_note_editor(cx)))))
            .child(tab_stop(button("note-save", if editing { "Save" } else { "Add" }, true)).on_click(cx.listener(|this, _, _, cx| this.save_note(cx))));
        page.child(div().pt_2().child(field(focused(&pane.search)).w(px(320.)).py_0p5().child(pane.search.clone())))
            .child(div().pt_2().child(list))
            .child(section(if editing { "Edit note" } else { "New note" }))
            .child(editor)
            .children(pane.error.clone().map(|e| note(e).text_color(Theme::warning())))
            .child(actions)
            .into_any_element()
    }
}
