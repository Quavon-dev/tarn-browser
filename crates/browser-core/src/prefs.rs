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

impl Switch {
    pub fn on(self, store: &Store) -> bool {
        match store.setting(self.key).ok().flatten().as_deref() {
            Some("on") => true,
            Some("off") => false,
            _ => self.default,
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
    fn switches_read_their_default_until_set() {
        let store = Store::open_in_memory().unwrap();
        assert!(GESTURES.on(&store) && !REDUCE_MOTION.on(&store) && !NEW_TAB_START_PAGE.on(&store) && !CLOSE_SELECTS_PREVIOUS.on(&store));
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
