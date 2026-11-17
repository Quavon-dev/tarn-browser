//! The icon a tab group or folder shows in place of its letter or folder glyph: one of `SYMBOLS`
//! or a single emoji. Typed text and stored text pass the same check.

use store::{Error, Id, Store};

/// Lucide names every client draws (`ui::icon::Icon::named`, `tarn_nsh_shared::icons::named`).
pub const SYMBOLS: [&str; 20] = [
    "briefcase",
    "code",
    "sparkles",
    "bell",
    "heart",
    "play",
    "music",
    "shopping-cart",
    "globe",
    "book-open",
    "plane",
    "wallet",
    "house",
    "graduation-cap",
    "flask-conical",
    "palette",
    "gamepad-2",
    "star",
    "coffee",
    "camera",
];

/// The most pictographs a real ZWJ sequence joins (a family of four).
const MAX_JOINED: usize = 4;

/// Room for the longest emoji sequences in use (a kiss with two skin tones is 35 bytes).
const MAX_EMOJI_BYTES: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon<'a> {
    Symbol(&'static str),
    Emoji(&'a str),
}

impl<'a> Icon<'a> {
    pub fn as_str(self) -> &'a str {
        match self {
            Icon::Symbol(s) => s,
            Icon::Emoji(e) => e,
        }
    }
}

/// The icon `text` names; `None` for empty text or anything that is neither a symbol nor one emoji.
pub fn parse(text: &str) -> Option<Icon<'_>> {
    let text = text.trim();
    match SYMBOLS.iter().find(|s| **s == text) {
        Some(symbol) => Some(Icon::Symbol(symbol)),
        None => is_emoji(text).then_some(Icon::Emoji(text)),
    }
}

/// The emoji of a stored icon, for prefilling an emoji field; empty for a symbol or no icon.
pub fn emoji(stored: &str) -> &str {
    match parse(stored) {
        Some(Icon::Emoji(e)) => e,
        _ => "",
    }
}

/// What to store for `text`: the icon, empty to clear it, or `Err` for text that is no icon.
fn checked(text: &str) -> Result<&str, Error> {
    if text.trim().is_empty() {
        return Ok("");
    }
    parse(text).map(|i| i.as_str()).ok_or(Error::Invalid("icon"))
}

pub fn set_group(store: &Store, id: Id, text: &str) -> Result<(), Error> {
    store.set_group_icon(id, checked(text)?)
}

pub fn set_folder(store: &Store, id: Id, text: &str) -> Result<(), Error> {
    store.set_folder_icon(id, checked(text)?)
}

// shortcut: whole emoji blocks instead of the exact Extended_Pictographic property; enough to keep
// letters, whitespace, controls and bidi characters out. Use a segmentation crate if one is added.
fn pictographic(c: char) -> bool {
    matches!(c as u32,
        0xA9 | 0xAE | 0x203C | 0x2049 | 0x2122 | 0x2139 | 0x2194..=0x2199 | 0x21A9..=0x21AA | 0x231A..=0x23FF | 0x24C2
        | 0x25AA..=0x27BF | 0x2934..=0x2935 | 0x2B05..=0x2B55 | 0x3030 | 0x303D | 0x3297 | 0x3299 | 0x1F000..=0x1F1E5 | 0x1F200..=0x1F3FA | 0x1F400..=0x1FAFF)
}

enum At {
    Start,
    Joined,
    Pictograph,
    Modified,
    Tags,
    End,
}

/// One emoji: a flag, a keycap, or pictographs joined by ZWJ, each with an optional variation
/// selector or skin tone, a tag sequence (subdivision flags) only at the end.
fn is_emoji(text: &str) -> bool {
    if text.is_empty() || text.len() > MAX_EMOJI_BYTES {
        return false;
    }
    let chars: Vec<char> = text.chars().collect();
    let regional = |c: &char| ('\u{1F1E6}'..='\u{1F1FF}').contains(c);
    if chars.iter().any(regional) {
        return chars.len() == 2 && chars.iter().all(regional);
    }
    if let [base, rest @ ..] = chars.as_slice()
        && matches!(base, '0'..='9' | '#' | '*')
    {
        return matches!(rest, ['\u{20E3}'] | ['\u{FE0F}', '\u{20E3}']);
    }
    let mut at = At::Start;
    let mut joined = 0;
    for c in chars {
        at = match (at, c) {
            (At::Start | At::Joined, c) if pictographic(c) && joined < MAX_JOINED => {
                joined += 1;
                At::Pictograph
            }
            (At::Pictograph | At::Modified, '\u{200D}') => At::Joined,
            (At::Pictograph, '\u{FE0F}' | '\u{1F3FB}'..='\u{1F3FF}') => At::Modified,
            (At::Pictograph | At::Tags, '\u{E0020}'..='\u{E007E}') => At::Tags,
            (At::Tags, '\u{E007F}') => At::End,
            _ => return false,
        };
    }
    matches!(at, At::Pictograph | At::Modified | At::End)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_and_single_emoji_are_icons() {
        assert_eq!(parse("code"), Some(Icon::Symbol("code")));
        assert_eq!(parse(" 🚀 "), Some(Icon::Emoji("🚀")));
        for one in ["❤️", "👍🏽", "👩‍💻", "👨‍👩‍👧‍👦", "🇩🇪", "#️⃣", "1⃣", "🏴\u{E0067}\u{E0062}\u{E0073}\u{E0063}\u{E0074}\u{E007F}", "👩🏻‍❤️‍💋‍👨🏼", "❤️‍🔥", "🧑🏽‍💻", "👁️‍🗨️"]
        {
            assert!(parse(one).is_some(), "{one:?}");
        }
    }

    #[test]
    fn text_and_sequences_of_several_emoji_are_not() {
        for bad in [
            "",
            "a",
            "Code",
            "codes",
            "🚀🚀",
            "🚀a",
            "👩‍",
            "\u{200D}👩",
            "🇩",
            "🇩🇪🇫🇷",
            "1",
            "\u{FE0F}",
            "🏻",
            "🏻🏻🏻🏻",
            "👍🏽🏽",
            "🚀\u{200D}🚀\u{200D}🚀\u{200D}🚀\u{200D}🚀",
            "\u{202E}🚀",
            "🚀\u{202E}",
            "🏴\u{E0067}",
            "🏴\u{E007F}🚀",
            &("👩\u{200D}".repeat(10) + "👩"),
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn emoji_prefill_is_only_the_emoji() {
        assert_eq!((emoji("📚"), emoji("code"), emoji(""), emoji("junk")), ("📚", "", "", ""));
    }

    #[test]
    fn icons_round_trip_through_the_store_and_bad_ones_change_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let core = crate::TestCore::open(dir.path(), false);
        let store = &core.disk;
        let space = store.spaces(store.profiles().unwrap()[0].id).unwrap()[0].id;
        let group = store.create_group(space, "Work").unwrap();
        let folder = store.create_folder(space, None, "Docs").unwrap();
        assert_eq!((group.icon.as_str(), folder.icon.as_str()), ("", ""));
        set_group(store, group.id, "briefcase").unwrap();
        set_folder(store, folder.id, " 📚 ").unwrap();
        assert!(set_group(store, group.id, "not an icon").is_err());
        assert!(set_folder(store, folder.id, "📚📚").is_err());
        assert_eq!(store.groups(space).unwrap()[0].icon, "briefcase");
        assert_eq!(store.folders(space).unwrap()[0].icon, "📚");
        set_group(store, group.id, "").unwrap();
        assert_eq!(store.groups(space).unwrap()[0].icon, "");
    }
}
