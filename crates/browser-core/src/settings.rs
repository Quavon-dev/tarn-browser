use std::time::Duration;

use store::Store;

pub const SUGGESTIONS: &str = "suggestions";
pub const SEARCH_SHORTCUTS: &str = "search_shortcuts";
pub const SEARCH_SHORTCUTS_NOTE: &str = "Type !g, !yt, !w, !gh, !a and others before or after your words to search that site instead of your search engine.";
pub const LIVE_TABS_KEY: &str = "live_tabs";
pub const SLEEP_AFTER_KEY: &str = "sleep_after";

pub const LIVE_TABS: [&str; 4] = ["3", "6", "10", "16"];
pub const LIVE_TABS_DEFAULT: usize = 1;
pub const SLEEP_AFTER: [(&str, Option<Duration>); 4] = [
    ("Never", None),
    ("10 minutes", Some(Duration::from_secs(600))),
    ("30 minutes", Some(Duration::from_secs(1_800))),
    ("2 hours", Some(Duration::from_secs(7_200))),
];
pub const SLEEP_AFTER_DEFAULT: usize = 2;
/// How long a background tab may stay unused while the system is short of memory.
pub const PRESSURE_IDLE: Duration = Duration::from_secs(60);

pub fn option_index(options: &[&str], stored: Option<&str>) -> Option<usize> {
    options.iter().position(|o| Some(*o) == stored)
}

/// The stored option of `key` among `options`, or `default` when none is stored or it is unknown.
pub fn choice(store: &Store, key: &str, options: &[&str], default: usize) -> usize {
    option_index(options, store.setting(key).ok().flatten().as_deref()).unwrap_or(default)
}

pub fn search_shortcuts(store: &Store) -> bool {
    store.setting(SEARCH_SHORTCUTS).ok().flatten().as_deref() == Some("on")
}

pub fn sleep_after_options() -> Vec<&'static str> {
    SLEEP_AFTER.iter().map(|(name, _)| *name).collect()
}

pub fn live_tabs(store: &Store) -> usize {
    LIVE_TABS[choice(store, LIVE_TABS_KEY, &LIVE_TABS, LIVE_TABS_DEFAULT)].parse().unwrap_or(6)
}

/// How long a tab may stay off screen before it sleeps; `None` leaves sleeping to the tab limit.
pub fn sleep_after(store: &Store) -> Option<Duration> {
    let stored = store.setting(SLEEP_AFTER_KEY).ok().flatten();
    SLEEP_AFTER[SLEEP_AFTER.iter().position(|(name, _)| Some(*name) == stored.as_deref()).unwrap_or(SLEEP_AFTER_DEFAULT)].1
}

/// The idle time after which a background tab sleeps: short of memory, at most `PRESSURE_IDLE`.
pub fn max_idle(sleep_after: Option<Duration>, pressure: bool) -> Option<Duration> {
    sleep_after.map(|idle| if pressure { idle.min(PRESSURE_IDLE) } else { idle })
}

/// Whether every word of `query` starts a word of `text`, ignoring case; how Settings search finds a pane.
pub fn words_match(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    query.to_lowercase().split_whitespace().all(|q| text.split_whitespace().any(|w| w.starts_with(q)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_match_from_word_starts_ignoring_case() {
        assert!(words_match("COOKIE ban", "Privacy cookie banners"));
        assert!(words_match("  ", "Privacy"));
        assert!(!words_match("ock", "lockdown"), "words match from their start");
        assert!(!words_match("cookie zoom2", "cookie zoom"));
    }

    #[test]
    fn tab_choices_default_and_ignore_unknown_values() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!((live_tabs(&store), sleep_after(&store)), (6, Some(Duration::from_secs(1_800))));
        store.set_setting(LIVE_TABS_KEY, "16").unwrap();
        store.set_setting(SLEEP_AFTER_KEY, "Never").unwrap();
        assert_eq!((live_tabs(&store), sleep_after(&store)), (16, None));
        store.set_setting(LIVE_TABS_KEY, "999").unwrap();
        store.set_setting(SLEEP_AFTER_KEY, "1 week").unwrap();
        assert_eq!((live_tabs(&store), sleep_after(&store)), (6, Some(Duration::from_secs(1_800))));
    }

    #[test]
    fn pressure_shortens_sleep_but_never_turns_it_on() {
        let ten = Some(Duration::from_secs(600));
        assert_eq!(max_idle(ten, false), ten);
        assert_eq!(max_idle(ten, true), Some(PRESSURE_IDLE));
        assert_eq!(max_idle(None, true), None);
    }
}
