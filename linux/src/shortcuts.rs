use std::collections::HashMap;

use nsh_core::keymap::Keystroke;
use nsh_core::platform::Os;

pub const PREFIX: &str = "keymap.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    NewTab,
    OpenLocation,
    CloseTab,
    ReopenClosedTab,
    NextTab,
    PreviousTab,
    TogglePin,
    BurnerTab,
    Reload,
    Back,
    Forward,
    Find,
    FindInTabs,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    CopyLink,
    CopyCleanLink,
    ToggleReader,
    ShowHistory,
    ToggleDownloads,
    ToggleSidebar,
    NewSpace,
    PrivateSpace,
    Settings,
    NewWindow,
    ToggleSplitView,
    FocusNextPane,
    FocusPreviousPane,
    SwapPanes,
    RotateSplitLayout,
    RemoveFromSplitView,
    CloseAllTabs,
    LastTab,
    CopyMarkdownLink,
    ToggleShortcuts,
}

pub type Binding = (&'static str, Action, &'static str);

pub const SECTIONS: [(&str, &[Binding]); 5] = [
    (
        "Tabs",
        &[
            ("New tab", Action::NewTab, "ctrl+t"),
            ("Open location", Action::OpenLocation, "ctrl+l"),
            ("Close tab", Action::CloseTab, "ctrl+w"),
            ("Reopen closed tab", Action::ReopenClosedTab, "ctrl+shift+t"),
            ("Close all tabs", Action::CloseAllTabs, "ctrl+shift+k"),
            ("Select last tab", Action::LastTab, "ctrl+9"),
            ("Next tab", Action::NextTab, "ctrl+tab"),
            ("Previous tab", Action::PreviousTab, "ctrl+shift+tab"),
            ("Pin or unpin tab", Action::TogglePin, "ctrl+d"),
            ("Burner tab", Action::BurnerTab, "ctrl+alt+shift+n"),
        ],
    ),
    (
        "Split View",
        &[
            ("Open or close split view", Action::ToggleSplitView, "ctrl+alt+n"),
            ("Focus next split pane", Action::FocusNextPane, "ctrl+alt+]"),
            ("Focus previous split pane", Action::FocusPreviousPane, "ctrl+alt+["),
            ("Swap split panes", Action::SwapPanes, "ctrl+alt+s"),
            ("Rotate split layout", Action::RotateSplitLayout, "ctrl+alt+r"),
            ("Remove tab from split view", Action::RemoveFromSplitView, "ctrl+alt+w"),
        ],
    ),
    (
        "Page",
        &[
            ("Reload", Action::Reload, "ctrl+r"),
            ("Back", Action::Back, "alt+left"),
            ("Forward", Action::Forward, "alt+right"),
            ("Find in page", Action::Find, "ctrl+f"),
            ("Find in tabs", Action::FindInTabs, "ctrl+shift+f"),
            ("Zoom in", Action::ZoomIn, "ctrl+="),
            ("Zoom out", Action::ZoomOut, "ctrl+-"),
            ("Actual size", Action::ZoomReset, "ctrl+0"),
            ("Copy link", Action::CopyLink, "ctrl+shift+c"),
            ("Copy clean link", Action::CopyCleanLink, "ctrl+alt+shift+c"),
            ("Copy link as Markdown", Action::CopyMarkdownLink, "ctrl+alt+c"),
            ("Reader", Action::ToggleReader, "ctrl+shift+r"),
        ],
    ),
    ("Library", &[("Show history", Action::ShowHistory, "ctrl+h"), ("Show downloads", Action::ToggleDownloads, "ctrl+j")]),
    (
        "Window",
        &[
            ("New window", Action::NewWindow, "ctrl+n"),
            ("Toggle sidebar", Action::ToggleSidebar, "ctrl+s"),
            ("New space", Action::NewSpace, "ctrl+shift+s"),
            ("Private space", Action::PrivateSpace, "ctrl+shift+n"),
            ("Settings", Action::Settings, "ctrl+,"),
            ("Keyboard shortcuts", Action::ToggleShortcuts, "ctrl+/"),
        ],
    ),
];

pub const ZOOM_IN_SHIFTED: &str = "ctrl+shift+=";
pub const FIND_NEXT: &str = "ctrl+g";
pub const FIND_PREVIOUS: &str = "ctrl+shift+g";

pub const FIXED: [(&str, &str, &str); 8] = [
    ("Tabs", "Select tab 1–8", "Ctrl+1 … Ctrl+8"),
    ("Page", "Find next", "Ctrl+G"),
    ("Page", "Find previous", "Ctrl+Shift+G"),
    ("Window", "Switch to space 1–9", "Alt+1 … Alt+9"),
    ("Window", "Enter or exit full screen", "F11"),
    ("Window", "Close command bar, panel or pop-up", "Esc"),
    ("Window", "Undo / Redo", "Ctrl+Z  Ctrl+Shift+Z"),
    ("Window", "Quit", "Ctrl+Q"),
];

const RESERVED: [&str; 18] = [
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
];

impl Action {
    pub fn name(self) -> String {
        format!("tarn::{self:?}")
    }

    pub fn setting_key(self) -> String {
        format!("{PREFIX}{}", self.name())
    }
}

pub fn bindable() -> impl Iterator<Item = (&'static str, Action, &'static str)> {
    SECTIONS.iter().flat_map(|(_, rows)| rows.iter().copied())
}

pub fn label(action: Action) -> &'static str {
    bindable().find(|(_, a, _)| *a == action).map_or("", |(l, ..)| l)
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

pub fn conflict(keys: &str, action: Action, keymap: &[(String, Action)]) -> Option<&'static str> {
    if RESERVED.iter().filter_map(|r| canonical(r)).any(|r| r.to_string() == keys) {
        return Some("a fixed shortcut");
    }
    keymap.iter().find(|(k, a)| k == keys && *a != action).map(|(_, other)| label(*other))
}

fn default(keys: &str) -> String {
    canonical(keys).map_or_else(|| keys.to_owned(), |k| k.to_string())
}

pub fn keymap(stored: &HashMap<String, String>) -> Vec<(String, Action)> {
    let chosen = |action: Action| stored.get(&action.setting_key()).and_then(|k| parse_keys(k).ok());
    let mut keymap: Vec<(String, Action)> = bindable().map(|(_, a, keys)| (chosen(a).unwrap_or_else(|| default(keys)), a)).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for (ix, (_, action, keys)) in bindable().enumerate() {
            if keymap[ix].0 != default(keys) && conflict(&keymap[ix].0, action, &keymap).is_some() {
                keymap[ix].0 = default(keys);
                changed = true;
            }
        }
    }
    keymap
}

pub fn stored(disk: &store::Store) -> HashMap<String, String> {
    disk.settings_with_prefix(PREFIX).unwrap_or_default().into_iter().collect()
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
    let keys = parse_keys(raw).map_err(str::to_owned)?;
    let map = keymap(&stored(disk));
    if let Some(user) = conflict(&keys, action, &map) {
        return Err(format!("{} is already used by {user}.", display(&keys)));
    }
    disk.set_setting(&action.setting_key(), &keys).map_err(|e| format!("The shortcut could not be saved ({e})."))
}

pub fn reset(disk: &store::Store, action: Option<Action>) -> Result<(), String> {
    let map = keymap(&stored(disk));
    for (_, a, keys) in bindable().filter(|(_, a, _)| action.is_none_or(|x| x == *a)) {
        if let Some(user) = conflict(&default(keys), a, &map) {
            return Err(format!("{} is now used by {user}; change that first.", display(&default(keys))));
        }
        disk.delete_setting(&a.setting_key()).map_err(|e| format!("The shortcut could not be reset ({e})."))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for (_, _, keys) in SECTIONS.iter().find(|(name, _)| *name == "Split View").unwrap().1 {
            assert_eq!(parse_keys(keys).as_deref(), Ok(default(keys).as_str()), "{keys} can be typed back in");
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
    fn rows_ported_from_macos_have_their_keys_and_fixed_rows_name_a_section() {
        let map = keymap(&HashMap::new());
        let of = |a: Action| map.iter().find(|(_, x)| *x == a).unwrap().0.clone();
        assert_eq!(of(Action::CloseAllTabs), "ctrl+shift+k");
        assert_eq!(of(Action::LastTab), "ctrl+9");
        assert_eq!(of(Action::CopyMarkdownLink), "ctrl+alt+c");
        assert_eq!(of(Action::ToggleShortcuts), "ctrl+/");
        assert_eq!(conflict("ctrl+9", Action::NewTab, &map), Some("Select last tab"));
        assert!(FIXED.iter().all(|(section, ..)| SECTIONS.iter().any(|(name, _)| name == section)));
    }

    #[test]
    fn saving_and_resetting_go_through_the_store() {
        let disk = store::Store::open_in_memory().unwrap();
        assert!(save(&disk, Action::NewTab, "ctrl+w").unwrap_err().contains("Close tab"));
        save(&disk, Action::NewTab, "ctrl+shift+y").unwrap();
        assert_eq!(stored(&disk)["keymap.tarn::NewTab"], "ctrl+shift+y");
        save(&disk, Action::CloseTab, "ctrl+t").unwrap();
        assert!(reset(&disk, Some(Action::NewTab)).unwrap_err().contains("Close tab"));
        reset(&disk, None).unwrap_err();
        save(&disk, Action::CloseTab, "ctrl+w").unwrap();
        reset(&disk, None).unwrap();
        assert!(stored(&disk).is_empty());
    }
}
