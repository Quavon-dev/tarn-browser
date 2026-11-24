//! Snippets: `;name` expands in the address and command bar; Insert Snippet… types one into the
//! page's focused field. Nothing listens for keys on the page.

use std::rc::Rc;

use browser_core::snippets::{self, EMPTY, NO_FIELD, NO_INSERT, PAGE_CHANGED};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{Context, Div, InteractiveElement, Window, actions};
use shield::ShieldLevel;
use store::Snippet;

use crate::browser::{Browser, CommandMode, Suggestion, Target};

actions!(tarn, [InsertSnippet]);

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &InsertSnippet, w, cx| b.insert_snippet(w, cx)))
}

/// The rows Insert Snippet lists for `query`; one row saying there are none when nothing matches.
fn rows(query: &str, all: &[Snippet]) -> Vec<Suggestion> {
    let found: Vec<Suggestion> = snippets::matching(query, all)
        .into_iter()
        .map(|s| Suggestion { section: None, label: s.name.clone(), detail: snippets::preview(&s.body), target: Target::Snippet(s.body.clone()) })
        .collect();
    if found.is_empty() {
        vec![Suggestion { section: None, label: EMPTY.to_owned(), detail: String::new(), target: Target::Snippet(String::new()) }]
    } else {
        found
    }
}

impl Browser {
    fn profile_snippets(&self) -> Rc<Vec<Snippet>> {
        self.core.snippets(self.model.profile())
    }

    /// Replaces a just typed `;name` at the end of the command field with its snippet.
    pub(crate) fn expand_snippet(&mut self, cx: &mut Context<Self>) {
        let input = self.command.read(cx);
        if !input.inserted() || !input.caret_at_end() {
            return;
        }
        let text = input.text().to_owned();
        if let Some(expanded) = self.core.expand_snippet(self.model.profile(), &text) {
            self.command.update(cx, |input, cx| {
                input.set_text(expanded, cx);
                input.append_selected("", cx);
            });
        }
    }

    fn insert_snippet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.aim_insert(NO_INSERT, cx) {
            self.open_command(CommandMode::Snippets, window, cx);
        }
    }

    /// Types an answer into the active page's focused field, like a snippet.
    pub(crate) fn insert_answer(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.aim_insert("Answers cannot be typed into a Lockdown page", cx) {
            self.type_snippet(text, cx);
        }
    }

    /// Remembers the active page as the one to type into; false, with a notice, on a Lockdown page.
    fn aim_insert(&mut self, refused: &'static str, cx: &mut Context<Self>) -> bool {
        let Some(key) = self.model.active else { return false };
        if self.level_for_tab(key, &self.tab_url(key).unwrap_or_default()) == ShieldLevel::Lockdown {
            self.notify_offer(key, refused.into(), None);
            cx.notify();
            return false;
        }
        self.snippet_page = self.tab_url(key).map(|url| (key, url));
        true
    }

    pub(crate) fn snippet_rows(&mut self, query: &str, cx: &mut Context<Self>) {
        self.suggestions = rows(query, &self.profile_snippets());
        self.selected = 0;
        cx.notify();
    }

    /// Types `body` into the focused field of the page Insert Snippet opened on, with a notice when
    /// the tab shows another page or no field has the focus.
    pub(crate) fn type_snippet(&mut self, body: &str, cx: &mut Context<Self>) {
        let Some((key, url)) = self.snippet_page.take().filter(|_| !body.is_empty()) else { return };
        if self.model.active != Some(key) || self.tab_url(key).as_deref() != Some(url.as_str()) {
            return self.show_toast(PAGE_CHANGED, None, cx);
        }
        let Some(view) = self.views.get(&key) else { return };
        let (weak, (tx, mut rx)) = (Rc::downgrade(view), mpsc::unbounded());
        view.insert_text(body, move |answer| drop(tx.unbounded_send(answer)));
        cx.spawn(async move |this, cx| {
            if rx.next().await.is_none_or(|answer| snippets::inserted(&answer)) {
                return;
            }
            this.update(cx, |b, cx| {
                if b.same_view(key, &weak) {
                    b.notify_offer(key, NO_FIELD.into(), None);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snippet(name: &str, body: &str) -> Snippet {
        Snippet { id: 0, profile_id: 1, name: name.into(), body: body.into() }
    }

    #[test]
    fn rows_list_matches_with_previews_or_say_there_are_none() {
        let all = [snippet("sig", "Best,\nAda"), snippet("addr", "1 Reef Road")];
        let found = rows(";si", &all);
        assert_eq!((found.len(), found[0].label.as_str(), found[0].detail.as_str()), (1, "sig", "Best,"));
        assert!(matches!(&found[0].target, Target::Snippet(body) if body == "Best,\nAda"));
        let none = rows("zzz", &all);
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].label, EMPTY);
        assert!(matches!(&none[0].target, Target::Snippet(body) if body.is_empty()));
    }
}
