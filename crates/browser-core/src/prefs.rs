use store::Store;

use crate::theme::Tokens;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switch {
    pub key: &'static str,
    pub default: bool,
}

pub const GESTURES: Switch = Switch { key: "gestures", default: true };
pub const REDUCE_MOTION: Switch = Switch { key: "reduce_motion", default: false };
pub const NEW_TAB_START_PAGE: Switch = Switch { key: "new_tab_start_page", default: false };
pub const CLOSE_SELECTS_PREVIOUS: Switch = Switch { key: "close_selects_previous", default: false };
pub const TEAR_OFF_TEMPORARY: Switch = Switch { key: "tear_off_temporary", default: true };
/// A playing video moves into Picture in Picture when its tab leaves the screen, and back after.
pub const AUTO_PICTURE_IN_PICTURE: Switch = Switch { key: "auto_picture_in_picture", default: true };
/// Settings' switch for the toasts that name a command's shortcut (`hints`), on every client.
pub const FEATURE_HINTS: Switch = Switch { key: "feature_hints", default: true };
/// Settings' Appearance switch for compact density: tighter sidebar and tab strip rows on every client.
pub const COMPACT: Switch = Switch { key: "compact_density", default: false };
/// A card with the tab's title while the pointer rests on a tab, on every client.
pub const HOVER_CARDS: Switch = Switch { key: "tab_hover_cards", default: true };
/// The card also shows a snapshot of an awake tab's page, kept in memory while the pointer rests.
pub const PAGE_PREVIEWS: Switch = Switch { key: "tab_page_previews", default: true };
pub const COMPACT_NOTE: &str = "Shorter rows and gaps in the sidebar and the tab strip.";

/// The address field in a slimmer bar, for the Classic and Compact toolbar choice.
pub const COMPACT_TOOLBAR: Switch = Switch { key: "compact_toolbar", default: false };
/// Back, Forward, Reload and the address field at the top of the sidebar.
pub const SIDEBAR_TOOLBAR: Switch = Switch { key: "sidebar_toolbar", default: true };
/// The profile's latest assistant chats under the pinned sites.
pub const SIDEBAR_CHATS: Switch = Switch { key: "sidebar_chats", default: true };
/// The compact row of pinned sites in place of large tiles.
pub const SMALL_TILES: Switch = Switch { key: "sidebar_small_tiles", default: false };
/// Groups and folders as cards; off keeps the classic coloured badges.
pub const SIDEBAR_CARDS: Switch = Switch { key: "sidebar_cards", default: true };
/// Sleeping tabs greyed out with a moon.
pub const DIM_SLEEPING: Switch = Switch { key: "sidebar_dim_sleeping", default: false };
/// A dot on a tab that retitled itself in the background.
pub const UNREAD_DOTS: Switch = Switch { key: "sidebar_unread_dots", default: true };

/// A layout that sets the toolbar and sidebar switches together: Modern is the toolbar in the
/// sidebar with cards, Classic Tarn's earlier layout; Custom when the switches match neither.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Modern,
    Classic,
    Custom,
}

/// Each switch a style sets, with its value in Modern and in Classic.
pub const STYLE: [(Switch, bool, bool); 9] = [
    (SIDEBAR_TOOLBAR, true, false),
    (COMPACT_TOOLBAR, false, false),
    (SIDEBAR_CARDS, true, false),
    (SIDEBAR_CHATS, true, false),
    (UNREAD_DOTS, true, false),
    (DIM_SLEEPING, false, true),
    (SMALL_TILES, false, false),
    (HOVER_CARDS, true, true),
    (PAGE_PREVIEWS, true, false),
];

/// The style the switches as `on` reads them make up.
pub fn style(on: impl Fn(Switch) -> bool) -> Style {
    if STYLE.iter().all(|(s, modern, _)| on(*s) == *modern) {
        Style::Modern
    } else if STYLE.iter().all(|(s, _, classic)| on(*s) == *classic) {
        Style::Classic
    } else {
        Style::Custom
    }
}

/// What a style's switch reads with nothing stored: someone who already picked the Classic or
/// Compact toolbar keeps the Classic layout, everyone else starts Modern.
pub fn style_default(store: &Store, switch: Switch) -> bool {
    let upgraded = store.setting(COMPACT_TOOLBAR.key).ok().flatten().is_some();
    match STYLE.iter().find(|(s, ..)| *s == switch) {
        Some((_, _, classic)) if upgraded => *classic,
        _ => switch.default,
    }
}

/// Sets every switch of the Modern style, or of the Classic one.
pub fn set_style(store: &Store, modern: bool) -> bool {
    STYLE.iter().all(|(s, in_modern, in_classic)| s.set(store, if modern { *in_modern } else { *in_classic }))
}

impl Switch {
    /// A style's switch with nothing stored reads `style_default`.
    pub fn on(self, store: &Store) -> bool {
        match store.setting(self.key).ok().flatten().as_deref() {
            Some("on") => true,
            Some("off") => false,
            _ => style_default(store, self),
        }
    }

    pub fn set(self, store: &Store, on: bool) -> bool {
        store.set_setting(self.key, if on { "on" } else { "off" }).is_ok()
    }
}

pub const ACCENT: &str = "accent";
pub const THEME_ACCENT: &str = "Theme";

pub const ACCENTS: [(&str, u32); 9] = [
    ("Tarn", 0x1f75b0),
    ("Blue", 0x0a7aff),
    ("Purple", 0xa550a7),
    ("Pink", 0xf74f9e),
    ("Red", 0xff5257),
    ("Orange", 0xf7821b),
    ("Yellow", 0xd9a400),
    ("Green", 0x62ba46),
    ("Graphite", 0x8c8c8c),
];

pub fn accent_choice(store: &Store) -> usize {
    let stored = store.setting(ACCENT).ok().flatten();
    ACCENTS.iter().position(|(name, _)| Some(*name) == stored.as_deref()).map_or(0, |ix| ix + 1)
}

pub fn set_accent_choice(store: &Store, choice: usize) -> bool {
    let name = match choice.checked_sub(1) {
        None => THEME_ACCENT,
        Some(ix) => match ACCENTS.get(ix) {
            Some((name, _)) => name,
            None => return false,
        },
    };
    store.set_setting(ACCENT, name).is_ok()
}

pub fn accent(store: &Store) -> Option<u32> {
    accent_choice(store).checked_sub(1).map(|ix| ACCENTS[ix].1)
}

pub fn accented(tokens: Tokens, store: &Store) -> Tokens {
    match accent(store) {
        Some(color) => Tokens { accent: color << 8 | 0xff, ..tokens },
        None => tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_install_starts_modern_and_an_earlier_toolbar_choice_keeps_classic() {
        let store = Store::open_in_memory().unwrap();
        let read = |store: &Store| style(|s| store.setting(s.key).ok().flatten().map_or_else(|| style_default(store, s), |v| v == "on"));
        assert_eq!(read(&store), Style::Modern);
        store.set_setting(COMPACT_TOOLBAR.key, "off").unwrap();
        assert_eq!(read(&store), Style::Classic);
        store.set_setting(SIDEBAR_CHATS.key, "on").unwrap();
        assert_eq!(read(&store), Style::Custom);
        assert_eq!(style_default(&store, GESTURES), GESTURES.default, "switches outside the style keep their own default");
        assert!(!style_default(&store, SIDEBAR_CARDS), "a style switch takes the Classic value after an earlier toolbar choice");
        assert!(!SIDEBAR_CARDS.on(&store), "an unset style switch reads its style default");
        assert!(set_style(&store, true));
        assert_eq!(style(|s| s.on(&store)), Style::Modern);
        assert!(set_style(&store, false));
        assert_eq!(style(|s| s.on(&store)), Style::Classic);
    }

    #[test]
    fn switches_read_their_default_until_set() {
        let store = Store::open_in_memory().unwrap();
        assert!(GESTURES.on(&store) && !REDUCE_MOTION.on(&store) && !NEW_TAB_START_PAGE.on(&store) && !CLOSE_SELECTS_PREVIOUS.on(&store));
        assert!(TEAR_OFF_TEMPORARY.on(&store) && !COMPACT.on(&store));
        assert!(GESTURES.set(&store, false) && CLOSE_SELECTS_PREVIOUS.set(&store, true));
        assert!(!GESTURES.on(&store) && CLOSE_SELECTS_PREVIOUS.on(&store));
        assert_eq!(store.setting("close_selects_previous").unwrap().as_deref(), Some("on"));
        store.set_setting(REDUCE_MOTION.key, "maybe").unwrap();
        assert!(!REDUCE_MOTION.on(&store), "an unknown value reads as the default");
    }

    #[test]
    fn the_accent_is_stored_by_name_and_tints_only_the_accent() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!((accent_choice(&store), accent(&store)), (0, None));
        assert_eq!(accented(Tokens::DARK, &store), Tokens::DARK);
        assert!(set_accent_choice(&store, 2));
        assert_eq!(store.setting(ACCENT).unwrap().as_deref(), Some("Blue"));
        assert_eq!((accent_choice(&store), accent(&store)), (2, Some(0x0a7aff)));
        let tinted = accented(Tokens::DARK, &store);
        assert_eq!(tinted.accent, 0x0a7affff);
        assert_eq!(Tokens { accent: Tokens::DARK.accent, ..tinted }, Tokens::DARK);
        assert!(!set_accent_choice(&store, ACCENTS.len() + 1), "past the last accent");
        assert_eq!(accent_choice(&store), 2);
        assert!(set_accent_choice(&store, 0));
        assert_eq!((store.setting(ACCENT).unwrap().as_deref(), accent(&store)), (Some(THEME_ACCENT), None));
        store.set_setting(ACCENT, "Teal").unwrap();
        assert_eq!(accent_choice(&store), 0, "an unknown name keeps the theme's");
    }
}
