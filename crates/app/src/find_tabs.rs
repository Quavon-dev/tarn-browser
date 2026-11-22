use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{Context, Div, InteractiveElement, Window, actions};

use crate::browser::{Browser, CommandMode, Suggestion, TabKey, Target};

actions!(tarn, [FindInTabs]);

/// Typing pauses this long before the awake tabs are searched again.
const DEBOUNCE: Duration = Duration::from_millis(150);

pub fn listen(root: Div, cx: &mut Context<Browser>) -> Div {
    root.on_action(cx.listener(|b, _: &FindInTabs, w, cx| b.open_command(CommandMode::FindInTabs, w, cx)))
}

impl Browser {
    /// Lists the current space's tabs that contain `query`: any tab whose shown title or address
    /// matches at once, then snippets of an awake tab's text, read in the engine's isolated world.
    /// Rows of the last query never stay, so Return cannot act on a stale one.
    pub(crate) fn find_in_tabs(&mut self, query: String, cx: &mut Context<Self>) {
        self.suggestions = browser_core::find::named(&self.model.tabs, &query, crate::tabs::display_title)
            .into_iter()
            .map(|t| Suggestion {
                section: None,
                label: crate::tabs::display_title(t),
                detail: "Title or address".to_owned(),
                target: Target::Tab(self.model.key(t)),
            })
            .collect();
        self.selected = 0;
        cx.notify();
        if query.is_empty() {
            self.find_tabs = None;
            return;
        }
        // No script of Tarn's runs on a Lockdown page; such a tab is found by title and address only.
        let unlocked =
            |b: &Self, k: &TabKey| b.views.get(k).and_then(|v| v.url()).is_some_and(|u| b.level_for_tab(*k, u.as_str()) != shield::ShieldLevel::Lockdown);
        let awake: Vec<TabKey> = self.model.tabs.iter().map(|t| self.model.key(t)).filter(|k| unlocked(self, k)).collect();
        self.find_tabs = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            let (tx, mut rx) = mpsc::unbounded();
            let asked = this.update(cx, |b, _| {
                for key in awake {
                    let tx = tx.clone();
                    if let Some(view) = b.views.get(&key) {
                        view.find_text(&query, move |found| drop(tx.unbounded_send((key, found))));
                    }
                }
            });
            drop(tx);
            let mut text_rows = 0;
            while asked.is_ok()
                && let Some((key, found)) = rx.next().await
            {
                if this.update(cx, |b, cx| b.add_text_matches(key, found, &query, &mut text_rows, cx)).is_err() {
                    return;
                }
            }
        }));
    }

    /// Text matches go above the title and address matches, in the order the tabs answer.
    fn add_text_matches(&mut self, key: TabKey, found: Vec<String>, query: &str, text_rows: &mut usize, cx: &mut Context<Self>) {
        if self.command_mode != Some(CommandMode::FindInTabs) {
            return;
        }
        if let Some(title) = self.model.tab(key).map(crate::tabs::display_title) {
            let rows = found.iter().map(|snippet| Suggestion {
                section: None,
                label: ui::text::bounded_title(snippet),
                detail: title.clone(),
                target: Target::Find(key, query.to_owned()),
            });
            browser_core::find::insert_text_rows(&mut self.suggestions, &mut self.selected, text_rows, rows.collect());
        }
        cx.notify();
    }

    /// Opens the tab with the find bar running `query`; a tab that was asleep is searched once its page has loaded.
    pub(crate) fn open_text_match(&mut self, key: TabKey, query: String, window: &mut Window, cx: &mut Context<Self>) {
        let asleep = self.sleeping(key);
        self.activate(key, window, cx);
        self.find_input.update(cx, |input, cx| input.set_text(query, cx));
        self.open_find(window, cx);
        if asleep {
            self.find_after_load = Some(key);
        } else {
            self.find(false, cx);
        }
    }
}
