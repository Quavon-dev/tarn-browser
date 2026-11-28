use browser_core::prefs::{self, Style, Switch};
use browser_core::sidebar::ChatRow;
use store::Id;

use super::Browser;

/// The listed chats, read again when the profile, its chats or the minute changes.
pub(super) type ChatCache = ((Id, u64, i64), Vec<ChatRow>);

impl Browser {
    /// Whether the sidebar holds the navigation buttons and the address field.
    pub fn sidebar_chrome(&self) -> bool {
        self.pref(prefs::SIDEBAR_TOOLBAR) && !self.tabs_on_top()
    }

    /// A sidebar switch, which applies only while the sidebar holds the toolbar.
    pub fn chrome_pref(&self, switch: Switch) -> bool {
        self.sidebar_chrome() && self.pref(switch)
    }

    /// The layout the switches make up; Modern also needs the tabs at the side.
    pub fn style(&self) -> Style {
        match prefs::style(|s| self.pref(s)) {
            Style::Modern if self.tabs_on_top() => Style::Custom,
            style => style,
        }
    }

    pub fn set_style(&mut self, modern: bool) {
        for (switch, in_modern, in_classic) in prefs::STYLE {
            self.set_pref(switch, if modern { in_modern } else { in_classic });
        }
    }

    /// The profile's latest chats, while its AI is on, it is not private and the sidebar shows them.
    pub fn sidebar_chats(&self) -> Option<Vec<ChatRow>> {
        // Listing titles sends nothing, so the profile's level decides, not the page's.
        if !self.chrome_pref(prefs::SIDEBAR_CHATS) || !self.model.assistant_offered(&self.core, None) {
            return None;
        }
        let profile = self.model.assistant_origin(&self.core, None)?.profile;
        let now = browser_core::now();
        let key = (profile, self.core.disk.chat_changes(), now / 60_000);
        let mut cache = self.chat_rows.borrow_mut();
        if cache.as_ref().is_none_or(|(k, _)| *k != key) {
            *cache = Some((key, browser_core::sidebar::chat_rows(&self.core.disk, profile, now)));
        }
        cache.as_ref().map(|(_, rows)| rows.clone())
    }
}

#[cfg(test)]
mod tests {
    use browser_core::prefs::{self, Style};
    use serde_json::json;

    use crate::browser::Browser;

    #[test]
    fn a_new_window_starts_modern_and_classic_takes_the_toolbar_out_of_the_sidebar() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::open(dir.path());
        assert!(b.sidebar_chrome() && b.chrome_pref(prefs::SIDEBAR_CARDS));
        assert_eq!(b.style(), Style::Modern);
        b.set_style(false);
        assert_eq!(b.style(), Style::Classic);
        assert!(!b.sidebar_chrome() && !b.chrome_pref(prefs::SIDEBAR_CARDS));
        b.set_style(true);
        b.toggle_tabs_on_top();
        assert!(!b.sidebar_chrome(), "tabs on top keep the toolbar above the page");
        assert_eq!(b.style(), Style::Custom);
    }

    #[test]
    fn an_earlier_compact_toolbar_choice_keeps_the_classic_layout() {
        let dir = tempfile::tempdir().unwrap();
        let b = Browser::open(dir.path());
        b.core.disk.set_setting(prefs::COMPACT_TOOLBAR.key, "on").unwrap();
        assert!(!b.sidebar_chrome());
        assert_eq!(b.style(), Style::Custom, "Compact is on, which neither style sets");
    }

    #[test]
    fn chats_list_only_while_the_profile_has_ai_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::open(dir.path());
        let profile = b.current_profile().unwrap();
        let turns = [store::ChatTurn { you: true, text: "hi".into(), by: None, interrupted: false, command: false }];
        b.core.disk.save_chat(profile, None, Some("Trip"), &turns, &[]).unwrap();
        assert_eq!(b.sidebar_chats(), None, "AI is off until the user turns it on");
        b.core.disk.set_profile_ai(profile, true).unwrap();
        assert_eq!(b.sidebar_chats().unwrap()[0].title, "Trip");
        b.set_pref(prefs::SIDEBAR_CHATS, false);
        assert_eq!(b.sidebar_chats(), None);
    }

    #[test]
    fn a_tab_that_retitles_itself_in_the_background_is_marked_until_shown() {
        let dir = tempfile::tempdir().unwrap();
        let mut b = Browser::open(dir.path());
        let first = b.model.active.unwrap();
        b.on_event(first, "finished", &json!({ "url": "https://mail.test/", "title": "Inbox" }));
        b.on_event(first, "title", &json!({ "title": "Inbox" }));
        b.new_tab("https://b.test/");
        b.on_event(first, "title", &json!({ "title": "Inbox (1)" }));
        assert!(b.model.unread(first));
        b.activate(first);
        b.model.seen();
        assert!(!b.model.unread(first));
    }
}
