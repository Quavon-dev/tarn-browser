use std::collections::HashSet;

use browser_core::command::{ENGINES, Input, classify_input};
use browser_core::incoming::{self, LOCALHOST_FROM_APPS};
use browser_core::suggest::{self, Page};
use browser_core::tabs::TabKey;
use browser_core::text::bounded_title;

use super::Browser;

#[derive(Clone, Debug, PartialEq)]
pub struct TabRow {
    pub key: TabKey,
    pub title: String,
    pub url: String,
    pub space: Option<String>,
}

impl TabRow {
    pub fn detail(&self) -> String {
        browser_core::tabs::switch_detail(self.space.as_deref())
    }
}

impl Browser {
    pub fn open_app_links(&mut self, urls: &[String]) {
        let localhost = self.setting_on(LOCALHOST_FROM_APPS);
        for arg in urls {
            let Some((url, typed)) = incoming::app_link(arg, localhost) else { continue };
            let url = self.core.clean_counted_url(self.model.profile(), url);
            if let Some(ix) = self.routed_space(&url) {
                self.switch_space(ix);
            }
            let Some(key) = self.new_tab(url.as_str()) else { continue };
            if !typed {
                self.pages.entry(key).or_default().link = true;
            }
        }
    }

    pub fn switch_to_tab(&mut self, key: TabKey) {
        if self.model.tab(key).is_none()
            && let Ok(tab) = self.core.store(key.private).tab(key.id)
            && let Some(ix) = self.model.spaces.iter().position(|s| s.private == key.private && s.space.id == tab.space_id)
        {
            self.switch_space(ix);
        }
        if self.model.tab(key).is_some() {
            self.activate(key);
        }
    }

    pub fn tab_suggestions(&self, query: &str) -> Vec<TabRow> {
        let Some(private) = self.model.current_space().map(|s| s.private).filter(|_| !query.trim().is_empty()) else { return Vec::new() };
        let row = |key: TabKey, title: &str, url: &str, space: Option<String>| TabRow {
            key,
            title: bounded_title(if title.is_empty() { url } else { title }),
            url: url.to_owned(),
            space,
        };
        let mut rows: Vec<TabRow> = match classify_input(query, &ENGINES[self.search_engine()]) {
            Some(Input::Navigate(url)) => {
                let url = self.core.clean_user_url(self.model.profile(), url);
                self.model
                    .tabs_showing(&self.core, &url)
                    .into_iter()
                    .filter(|(key, ..)| self.model.active != Some(*key))
                    .map(|(key, t, space)| row(key, &t.title, &t.url, space))
                    .collect()
            }
            _ => Vec::new(),
        };
        let tabs = &self.model.tabs;
        let listed: HashSet<TabKey> = rows.iter().map(|r| r.key).collect();
        let key = |ix: usize| TabKey { private, id: tabs[ix].id };
        let pages: Vec<Page> = tabs.iter().map(Page::from).collect();
        let shown = |ix: usize| listed.contains(&key(ix)) || self.model.active == Some(key(ix));
        let ranked = suggest::rank_rows(query, &pages, tabs.len(), shown, browser_core::now());
        rows.extend(ranked.into_iter().map(|ix| row(key(ix), &tabs[ix].title, &tabs[ix].url, None)));
        rows
    }
}
