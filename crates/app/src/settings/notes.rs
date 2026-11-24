use browser_core::notes::{Draft, EMPTY, EXPORT_NAME, INTRO, NO_MATCH, PRIVATE, Sort, markdown, sorted};
use gpui::{AnyElement, AppContext, Context, Entity, Focusable, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Subscription, Window, div, px};
use shield::ShieldLevel;
use store::{Id, Note};
use ui::controls::{button, field, group, note, row, section, segment, segmented, tab_stop};
use ui::text_input::{TextInput, TextInputEvent};

use super::{SettingsWindow, Snapshot, editor_actions, heading, label, search_field, value, warning};

pub(super) struct Pane {
    search: Entity<TextInput>,
    title: Entity<TextInput>,
    body: Entity<TextInput>,
    editing: Option<Id>,
    error: Option<String>,
    exported: Option<String>,
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
            exported: None,
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

    /// Writes every note as one Markdown file where the person picks; nothing leaves this computer.
    fn export_notes(&mut self, notes: Vec<Note>, cx: &mut Context<Self>) {
        let dir = std::env::home_dir().map(|h| h.join("Downloads")).unwrap_or_default();
        let path = cx.prompt_for_new_path(&dir, Some(EXPORT_NAME));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else { return };
            let written = std::fs::write(&path, markdown(&notes));
            this.update(cx, |this, cx| {
                (this.notes.exported, this.notes.error) = match written {
                    Ok(()) => (Some(format!("Saved to {}.", path.display())), None),
                    Err(e) => (None, Some(format!("The notes could not be saved: {e}"))),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
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
        let sort = Sort::load(&crate::shared::core(cx).disk);
        let shown = sorted(saved, &query, sort);
        let ask = s.profile.as_ref().is_some_and(|p| p.ai_enabled && p.shield_level != ShieldLevel::Lockdown && !s.burner);
        let list = if shown.is_empty() {
            note(if saved.is_empty() { EMPTY } else { NO_MATCH }).into_any_element()
        } else {
            let mut list = group();
            for (ix, r) in shown.into_iter().enumerate() {
                let (id, edit) = (r.id, saved.iter().find(|n| n.id == r.id).cloned());
                let asked = edit.clone().filter(|_| ask);
                list = list.child(
                    row(ix == 0)
                        .child(div().w(px(180.)).flex_none().child(label(r.title)))
                        .child(value(r.preview).flex_1().min_w_0().overflow_hidden())
                        .children(asked.map(|note| {
                            tab_stop(button(("note-ask", ix), "Ask", false)).on_click(cx.listener(move |this, _, _, cx| {
                                let note = note.clone();
                                this.with_browser(cx, move |b, window, cx| b.ask_note(&note, window, cx));
                            }))
                        }))
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
        let actions = editor_actions(
            "note-editor",
            editing,
            Box::new(cx.listener(|this, _, _, cx| this.clear_note_editor(cx))),
            Box::new(cx.listener(|this, _, _, cx| this.save_note(cx))),
        );
        let order = segmented().children(Sort::ALL.into_iter().enumerate().map(|(ix, choice)| {
            tab_stop(segment(("note-sort", ix), choice.name(), choice == sort)).on_click(cx.listener(move |_, _, _, cx| {
                choice.save(&crate::shared::core(cx).disk);
                cx.notify();
            }))
        }));
        let all = saved.clone();
        let export = tab_stop(button("note-export", "Export\u{2026}", false)).on_click(cx.listener(move |this, _, _, cx| this.export_notes(all.clone(), cx)));
        page.child(heading("Your notes", if saved.is_empty() { div().into_any_element() } else { export.into_any_element() }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .flex_wrap()
                    .child(search_field(&pane.search, focused(&pane.search)))
                    .child(div().pt_2().child(order)),
            )
            .child(div().pt_2().child(list))
            .children(pane.exported.clone().map(note))
            .child(section(if editing { "Edit note" } else { "New note" }))
            .child(editor)
            .children(pane.error.clone().map(warning))
            .child(actions)
            .into_any_element()
    }
}
