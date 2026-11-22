use std::collections::HashMap;

use browser_core::shortcuts::{self as core, Keyboard, Keystrokes};
pub use browser_core::shortcuts::{Action, stored};
use nsh_core::keymap::Keystroke;
use nsh_core::platform::Os;

pub const ZOOM_IN_SHIFTED: &str = "ctrl+shift+=";
pub const FIND_NEXT: &str = "ctrl+g";
pub const FIND_PREVIOUS: &str = "ctrl+shift+g";

pub const NEXT_TAB: &str = "ctrl+pagedown";
pub const PREVIOUS_TAB: &str = "ctrl+pageup";

pub const FIXED: [(&str, &str, &str); 11] = [
    ("Tabs", "Select tab 1–8", "Ctrl+1 … Ctrl+8"),
    ("Tabs", "Next / previous tab", "Ctrl+Page Down  Ctrl+Page Up"),
    ("Page", "Stop loading", "Esc"),
    ("Page", "Find next", "Ctrl+G"),
    ("Page", "Find previous", "Ctrl+Shift+G"),
    ("Window", "Switch to space 1–9", "Alt+1 … Alt+9"),
    ("Window", "Enter or exit full screen", "F11"),
    ("Window", "Move between page, address field and sidebar", "F6  Shift+F6"),
    ("Window", "Close command bar, panel or pop-up", "Esc"),
    ("Window", "Undo / Redo", "Ctrl+Z  Ctrl+Shift+Z"),
    ("Window", "Quit", "Ctrl+Q"),
];

const RESERVED: [&str; 20] = [
    "ctrl+1",
    "ctrl+2",
    "ctrl+3",
    "ctrl+4",
    "ctrl+5",
    "ctrl+6",
    "ctrl+7",
    "ctrl+8",
    "ctrl+z",
    "ctrl+shift+z",
    "ctrl+x",
    "ctrl+c",
    "ctrl+v",
    "ctrl+a",
    "escape",
    ZOOM_IN_SHIFTED,
    FIND_NEXT,
    FIND_PREVIOUS,
    NEXT_TAB,
    PREVIOUS_TAB,
];

struct Linux;

impl Keystrokes for Linux {
    fn keyboard(&self) -> Keyboard {
        Keyboard::Linux
    }

    fn offered(&self, action: Action) -> bool {
        offered(action)
    }

    fn written(&self, keys: &str) -> String {
        let keys = keys.replace("mod", "ctrl");
        canonical(&keys).map_or(keys, |k| k.to_string())
    }

    fn parse(&self, raw: &str) -> Result<String, &'static str> {
        parse_keys(raw)
    }

    fn fixed(&self, keys: &str) -> bool {
        RESERVED.iter().filter_map(|r| canonical(r)).any(|r| r.to_string() == keys)
    }

    fn display(&self, keys: &str) -> String {
        display(keys)
    }
}

pub fn offered(action: Action) -> bool {
    !matches!(
        action,
        Action::CommandBar
            | Action::StopLoading
            | Action::FindNext
            | Action::FindPrevious
            | Action::PictureInPicture
            | Action::ToggleLibrary
            | Action::Minimize
            | Action::ToggleFullScreen
            | Action::HideTarn
            | Action::HideOthers
            | Action::SelectTab(_)
            | Action::SwitchSpace(_)
    )
}

pub fn sections() -> impl Iterator<Item = (&'static str, Vec<(&'static str, Action)>)> {
    core::SECTIONS.iter().map(|(name, rows)| (*name, rows.iter().filter(|(_, a, _)| offered(*a)).map(|(l, a, _)| (*l, *a)).collect()))
}

fn canonical(text: &str) -> Option<Keystroke> {
    Keystroke::parse_with(text.trim(), false).ok().filter(|_| !text.trim().contains(char::is_whitespace))
}

pub fn parse_keys(raw: &str) -> Result<String, &'static str> {
    let keystroke = canonical(raw).ok_or("Not a key combination.")?;
    if !keystroke.modifiers.ctrl {
        return Err("Use Ctrl with the key.");
    }
    if keystroke.reserved(Os::Linux).is_some() {
        return Err("The desktop keeps this key.");
    }
    Ok(keystroke.to_string())
}

pub fn keymap(stored: &HashMap<String, String>) -> Vec<(String, Action)> {
    core::keymap(&Linux, stored)
}

pub fn display(keys: &str) -> String {
    keys.split('+')
        .map(|part| match part {
            "ctrl" => "Ctrl".to_owned(),
            "alt" => "Alt".to_owned(),
            "shift" => "Shift".to_owned(),
            "" => "+".to_owned(),
            key if key.chars().count() == 1 => key.to_uppercase(),
            key => {
                let mut c = key.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join("+")
        .replace("++", "+")
}

pub fn save(disk: &store::Store, action: Action, raw: &str) -> Result<(), String> {
    core::save(&Linux, disk, action, raw)
}

pub fn reset(disk: &store::Store, action: Option<Action>) -> Result<(), String> {
    core::reset(&Linux, disk, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflict(keys: &str, action: Action, keymap: &[(String, Action)]) -> Option<&'static str> {
        core::conflict(&Linux, keys, action, keymap)
    }

    fn default(action: Action) -> String {
        core::default(&Linux, action)
    }

    #[test]
    fn keys_parse_to_one_canonical_keystroke_with_ctrl() {
        assert_eq!(parse_keys("Shift+Ctrl+T"), Ok("ctrl+shift+t".to_owned()));
        assert_eq!(parse_keys(" ctrl+alt+k "), Ok("ctrl+alt+k".to_owned()));
        assert_eq!(parse_keys("ctrl+-"), Ok("ctrl+-".to_owned()));
        assert!(parse_keys("t").is_err() && parse_keys("alt+t").is_err() && parse_keys("ctrl+k ctrl+s").is_err());
    }

    #[test]
    fn defaults_are_distinct_and_stored_keys_replace_them_unless_taken() {
        let defaults = keymap(&HashMap::new());
        let mut keys: Vec<&String> = defaults.iter().map(|(k, _)| k).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), defaults.len());
        for action in [
            Action::ToggleSplitView,
            Action::FocusNextPane,
            Action::FocusPreviousPane,
            Action::SwapPanes,
            Action::RotateSplitLayout,
            Action::RemoveFromSplitView,
            Action::TakeScreenshot,
        ] {
            assert_eq!(parse_keys(&default(action)).as_deref(), Ok(default(action).as_str()), "{action:?} can be typed back in");
        }
        assert_eq!(Action::NewTab.setting_key(), "keymap.tarn::NewTab");
        let stored = HashMap::from([
            (Action::NewTab.setting_key(), "ctrl+shift+y".to_owned()),
            (Action::Reload.setting_key(), "ctrl+w".to_owned()),
            (Action::Find.setting_key(), "ctrl+1".to_owned()),
        ]);
        let map = keymap(&stored);
        let of = |a: Action| map.iter().find(|(_, x)| *x == a).unwrap().0.clone();
        assert_eq!(of(Action::NewTab), "ctrl+shift+y");
        assert_eq!(of(Action::Reload), "ctrl+r", "taken by Close tab");
        assert_eq!(of(Action::Find), "ctrl+f", "a fixed key");
        assert_eq!(conflict("ctrl+w", Action::Reload, &map), Some("Close tab"));
        assert_eq!(conflict(&parse_keys(ZOOM_IN_SHIFTED).unwrap(), Action::NewTab, &map), Some("a fixed shortcut"));
        assert_eq!(conflict(FIND_PREVIOUS, Action::NewTab, &map), Some("a fixed shortcut"));
        assert_eq!(display("ctrl+shift+t"), "Ctrl+Shift+T");
        assert_eq!(display("alt+left"), "Alt+Left");
    }

    #[test]
    fn every_linux_default_stays_where_it_was() {
        let map = keymap(&HashMap::new());
        let of = |a: Action| map.iter().find(|(_, x)| *x == a).unwrap().0.clone();
        for (action, keys) in [
            (Action::CloseAllTabs, "ctrl+shift+k"),
            (Action::LastTab, "ctrl+9"),
            (Action::CopyMarkdownLink, "ctrl+alt+c"),
            (Action::ToggleShortcuts, "ctrl+/"),
            (Action::Back, "alt+left"),
            (Action::Forward, "alt+right"),
            (Action::ToggleReader, "ctrl+alt+e"),
            (Action::ShowHistory, "ctrl+h"),
            (Action::ToggleDownloads, "ctrl+j"),
            (Action::TakeScreenshot, "ctrl+alt+shift+s"),
            (Action::ToggleSplitView, "ctrl+alt+n"),
            (Action::RemoveFromSplitView, "ctrl+alt+w"),
            (Action::NewGroup, "ctrl+alt+g"),
            (Action::BurnerTab, "ctrl+alt+shift+n"),
            (Action::CopyCleanLink, "ctrl+alt+shift+c"),
            (Action::ManageBookmarks, "ctrl+alt+b"),
            (Action::NewWindow, "ctrl+n"),
        ] {
            assert_eq!(of(action), keys, "{action:?}");
        }
        assert_eq!(conflict("ctrl+9", Action::NewTab, &map), Some("Select last tab"));
        assert!(FIXED.iter().all(|(section, ..)| core::SECTIONS.iter().any(|(name, _)| name == section)));
        assert!(!map.iter().any(|(_, a)| *a == Action::HideTarn), "macOS-only actions take no key here");
    }

    #[test]
    fn saving_and_resetting_go_through_the_store() {
        let disk = store::Store::open_in_memory().unwrap();
        assert!(save(&disk, Action::NewTab, "ctrl+w").unwrap_err().contains("Close tab"));
        save(&disk, Action::NewTab, "ctrl+shift+y").unwrap();
        assert_eq!(stored(&disk)["keymap.tarn::NewTab"], "ctrl+shift+y");
        save(&disk, Action::CloseTab, "ctrl+t").unwrap();
        assert!(reset(&disk, Some(Action::NewTab)).unwrap_err().contains("Close tab"));
        reset(&disk, None).unwrap();
        assert!(stored(&disk).is_empty());
    }
}
