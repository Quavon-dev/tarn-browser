use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    NewTab,
    OpenLocation,
    OpenFile,
    CommandBar,
    CloseTab,
    ReopenClosedTab,
    CloseAllTabs,
    NextTab,
    PreviousTab,
    LastTab,
    TogglePin,
    NewGroup,
    ToggleSplitView,
    FocusNextPane,
    FocusPreviousPane,
    SwapPanes,
    RotateSplitLayout,
    RemoveFromSplitView,
    Reload,
    ReloadFromOrigin,
    StopLoading,
    Back,
    Forward,
    Find,
    FindNext,
    FindPrevious,
    FindInTabs,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    CopyLink,
    CopyCleanLink,
    CopyMarkdownLink,
    TakeScreenshot,
    ToggleReader,
    PictureInPicture,
    Print,
    BookmarkTab,
    ToggleLibrary,
    ToggleDownloads,
    ManageBookmarks,
    ShowHistory,
    NewWindow,
    CloseWindow,
    ToggleSidebar,
    NewSpace,
    PrivateSpace,
    BurnerTab,
    Minimize,
    ToggleFullScreen,
    HideTarn,
    HideOthers,
    Settings,
    ToggleShortcuts,
    SelectTab(u8),
    SwitchSpace(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keyboard {
    Apple,
    Linux,
    Other,
}

impl Keyboard {
    pub fn apple(self) -> bool {
        self == Keyboard::Apple
    }
}

pub type Binding = (&'static str, Action, &'static str);

pub const SECTIONS: [(&str, &[Binding]); 4] = [
    (
        "Tabs",
        &[
            ("New tab", Action::NewTab, "mod+t"),
            ("Open location", Action::OpenLocation, "mod+l"),
            ("Open file", Action::OpenFile, "mod+o"),
            ("Command bar", Action::CommandBar, "mod+k"),
            ("Close tab", Action::CloseTab, "mod+w"),
            ("Reopen closed tab", Action::ReopenClosedTab, "mod+shift+t"),
            ("Close all tabs", Action::CloseAllTabs, "mod+shift+k"),
            ("Select last tab", Action::LastTab, "mod+9"),
            ("Next tab", Action::NextTab, "ctrl+tab"),
            ("Previous tab", Action::PreviousTab, "ctrl+shift+tab"),
            ("Pin or unpin tab", Action::TogglePin, "mod+d"),
            ("New tab group", Action::NewGroup, "mod+alt+g"),
            ("Open or close split view", Action::ToggleSplitView, "ctrl+mod+n"),
            ("Focus next split pane", Action::FocusNextPane, "ctrl+mod+]"),
            ("Focus previous split pane", Action::FocusPreviousPane, "ctrl+mod+["),
            ("Swap split panes", Action::SwapPanes, "ctrl+mod+s"),
            ("Rotate split layout", Action::RotateSplitLayout, "ctrl+mod+r"),
            ("Remove tab from split view", Action::RemoveFromSplitView, "ctrl+mod+w"),
        ],
    ),
    (
        "Page",
        &[
            ("Reload", Action::Reload, "mod+r"),
            ("Reload without cache", Action::ReloadFromOrigin, "mod+shift+r"),
            ("Stop loading", Action::StopLoading, "mod+."),
            ("Back", Action::Back, "mod+["),
            ("Forward", Action::Forward, "mod+]"),
            ("Find in page", Action::Find, "mod+f"),
            ("Find next", Action::FindNext, "mod+g"),
            ("Find previous", Action::FindPrevious, "mod+shift+g"),
            ("Find in tabs", Action::FindInTabs, "mod+shift+f"),
            ("Zoom in", Action::ZoomIn, "mod+="),
            ("Zoom out", Action::ZoomOut, "mod+-"),
            ("Actual size", Action::ZoomReset, "mod+0"),
            ("Copy link", Action::CopyLink, "mod+shift+c"),
            ("Copy clean link", Action::CopyCleanLink, "ctrl+shift+mod+c"),
            ("Copy link as Markdown", Action::CopyMarkdownLink, "mod+alt+shift+c"),
            ("Take screenshot", Action::TakeScreenshot, "mod+alt+s"),
            ("Reader", Action::ToggleReader, "mod+alt+r"),
            ("Picture in Picture", Action::PictureInPicture, "ctrl+mod+p"),
            ("Print", Action::Print, "mod+p"),
        ],
    ),
    (
        "Library",
        &[
            ("Bookmark this tab", Action::BookmarkTab, "mod+shift+d"),
            ("Show library", Action::ToggleLibrary, "mod+shift+a"),
            ("Show downloads", Action::ToggleDownloads, "mod+shift+j"),
            ("Manage bookmarks", Action::ManageBookmarks, "mod+alt+b"),
            ("Show history", Action::ShowHistory, "mod+y"),
        ],
    ),
    (
        "Window",
        &[
            ("New window", Action::NewWindow, "mod+n"),
            ("Close window", Action::CloseWindow, "mod+shift+w"),
            ("Toggle sidebar", Action::ToggleSidebar, "mod+s"),
            ("New space", Action::NewSpace, "mod+shift+s"),
            ("Private space", Action::PrivateSpace, "mod+shift+n"),
            ("Burner tab", Action::BurnerTab, "mod+alt+shift+n"),
            ("Minimize window", Action::Minimize, "mod+m"),
            ("Enter or exit full screen", Action::ToggleFullScreen, "ctrl+mod+f"),
            ("Hide Tarn", Action::HideTarn, "mod+h"),
            ("Hide others", Action::HideOthers, "mod+alt+h"),
            ("Settings", Action::Settings, "mod+,"),
            ("Keyboard shortcuts", Action::ToggleShortcuts, "mod+/"),
        ],
    ),
];

// On a PC keyboard Ctrl is the command key, so these Apple keys would land on a split view key.
const PC: [(Action, &str); 2] = [(Action::CopyMarkdownLink, "mod+alt+c"), (Action::TakeScreenshot, "mod+alt+shift+s")];
const LINUX: [(Action, &str); 5] = [
    (Action::Back, "alt+left"),
    (Action::Forward, "alt+right"),
    (Action::ToggleReader, "mod+alt+e"),
    (Action::ShowHistory, "mod+h"),
    (Action::ToggleDownloads, "mod+j"),
];

pub const SELECTABLE_TABS: u8 = 8;
pub const SWITCHABLE_SPACES: u8 = 9;
pub const PREFIX: &str = "keymap.";

impl Action {
    pub fn setting_key(self) -> String {
        format!("{PREFIX}tarn::{self:?}")
    }
}

fn rows() -> impl Iterator<Item = Binding> {
    SECTIONS.iter().flat_map(|(_, rows)| rows.iter().copied())
}

pub fn label(action: Action) -> &'static str {
    match action {
        Action::SelectTab(_) => "Select tab 1–8",
        Action::SwitchSpace(_) => "Switch to space 1–9",
        _ => rows().find(|(_, a, _)| *a == action).map_or("", |(l, ..)| l),
    }
}

fn for_keyboard(keys: &str, keyboard: Keyboard) -> String {
    match keyboard {
        Keyboard::Apple => keys.to_owned(),
        _ if keys.contains("mod+") => keys.replace("ctrl+", "alt+"),
        _ => keys.to_owned(),
    }
}

pub fn default_keys(action: Action, keyboard: Keyboard) -> Option<String> {
    let space = if keyboard.apple() { "ctrl" } else { "alt" };
    let keys = match action {
        Action::SelectTab(n) => return (1..=SELECTABLE_TABS).contains(&n).then(|| format!("mod+{n}")),
        Action::SwitchSpace(n) => return (1..=SWITCHABLE_SPACES).contains(&n).then(|| format!("{space}+{n}")),
        _ => rows().find(|(_, a, _)| *a == action)?.2,
    };
    let own = |list: &[(Action, &'static str)]| list.iter().find(|(a, _)| *a == action).map(|(_, k)| *k);
    let keys = match keyboard {
        Keyboard::Apple => keys,
        Keyboard::Linux => own(&LINUX).or(own(&PC)).unwrap_or(keys),
        Keyboard::Other => own(&PC).unwrap_or(keys),
    };
    Some(for_keyboard(keys, keyboard))
}

pub fn bindings(keyboard: Keyboard, offered: impl Fn(Action) -> bool) -> Vec<(String, Action)> {
    rows()
        .map(|(_, action, _)| action)
        .chain((1..=SELECTABLE_TABS).map(Action::SelectTab))
        .chain((1..=SWITCHABLE_SPACES).map(Action::SwitchSpace))
        .filter(|a| offered(*a))
        .filter_map(|a| Some((default_keys(a, keyboard)?, a)))
        .collect()
}

pub fn shown(keys: &str, keyboard: Keyboard) -> String {
    let keys = for_keyboard(keys, keyboard);
    let mut mods: Vec<&str> = keys.split('+').collect();
    let key = mods.pop().unwrap_or_default();
    let has = |m: &str| mods.contains(&m);
    let apple = keyboard.apple();
    let key = match (key, apple) {
        ("tab", true) => "⇥".to_owned(),
        ("tab", false) => "Tab".to_owned(),
        ("escape", _) => "Esc".to_owned(),
        (key, _) => key.to_uppercase(),
    };
    if apple {
        let mods = [("ctrl", "⌃"), ("alt", "⌥"), ("shift", "⇧"), ("mod", "⌘")];
        return mods.iter().filter(|(m, _)| has(m)).map(|(_, g)| *g).collect::<String>() + &key;
    }
    let mods = [(has("mod") || has("ctrl"), "Ctrl"), (has("alt"), "Alt"), (has("shift"), "Shift")];
    mods.iter().filter(|(on, _)| *on).map(|(_, m)| *m).chain(std::iter::once(key.as_str())).collect::<Vec<_>>().join("+")
}

pub fn keys_for(action: Action, keyboard: Keyboard) -> Option<String> {
    rows().any(|(_, a, _)| a == action).then(|| default_keys(action, keyboard)).flatten().map(|k| shown(&k, keyboard))
}

pub fn for_command(command: crate::palette::Command) -> Option<Action> {
    use crate::palette::Command;
    match command {
        Command::OpenFile => Some(Action::OpenFile),
        Command::ReopenClosedTab => Some(Action::ReopenClosedTab),
        Command::TogglePin => Some(Action::TogglePin),
        Command::Reader => Some(Action::ToggleReader),
        Command::BurnerTab => Some(Action::BurnerTab),
        Command::FindInTabs => Some(Action::FindInTabs),
        Command::CopyLink => Some(Action::CopyLink),
        Command::CopyCleanLink => Some(Action::CopyCleanLink),
        Command::NewWindow => Some(Action::NewWindow),
        Command::ToggleSplitView => Some(Action::ToggleSplitView),
        Command::FocusNextPane => Some(Action::FocusNextPane),
        Command::FocusPreviousPane => Some(Action::FocusPreviousPane),
        Command::SwapPanes => Some(Action::SwapPanes),
        Command::RotateSplitLayout => Some(Action::RotateSplitLayout),
        Command::RemoveFromSplitView => Some(Action::RemoveFromSplitView),
        _ => None,
    }
}

pub fn table(keyboard: Keyboard, offered: impl Fn(Action) -> bool) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    let range = |first: Action, last: Action| {
        let keys = |a| default_keys(a, keyboard).map(|k| shown(&k, keyboard)).unwrap_or_default();
        format!("{}–{}", keys(first), keys(last).chars().last().unwrap_or_default())
    };
    SECTIONS
        .iter()
        .map(|(name, rows)| {
            let mut shown_rows = Vec::new();
            for (does, action, _) in rows.iter() {
                match action {
                    Action::LastTab if offered(Action::SelectTab(1)) => {
                        shown_rows.push((label(Action::SelectTab(1)), range(Action::SelectTab(1), Action::SelectTab(SELECTABLE_TABS))))
                    }
                    Action::Settings if offered(Action::SwitchSpace(1)) => {
                        shown_rows.push((label(Action::SwitchSpace(1)), range(Action::SwitchSpace(1), Action::SwitchSpace(SWITCHABLE_SPACES))))
                    }
                    _ => {}
                }
                if offered(*action) {
                    shown_rows.extend(default_keys(*action, keyboard).map(|k| (*does, shown(&k, keyboard))));
                }
            }
            if *name == "Window" {
                shown_rows.push(("Close command bar, popover or Settings", shown("escape", keyboard)));
            }
            (*name, shown_rows)
        })
        .collect()
}

pub trait Keystrokes {
    fn keyboard(&self) -> Keyboard;
    fn offered(&self, action: Action) -> bool;
    fn written(&self, keys: &str) -> String;
    fn parse(&self, raw: &str) -> Result<String, &'static str>;
    fn fixed(&self, keys: &str) -> bool;
    fn same(&self, a: &str, b: &str) -> bool {
        a == b
    }
    fn display(&self, keys: &str) -> String;
}

pub fn rebindable(k: &impl Keystrokes) -> impl Iterator<Item = (&'static str, Action)> + '_ {
    rows().filter(|(_, a, _)| k.offered(*a)).map(|(label, a, _)| (label, a))
}

pub fn default(k: &impl Keystrokes, action: Action) -> String {
    default_keys(action, k.keyboard()).map(|keys| k.written(&keys)).unwrap_or_default()
}

pub fn conflict(k: &impl Keystrokes, keys: &str, action: Action, keymap: &[(String, Action)]) -> Option<&'static str> {
    if k.fixed(keys) {
        return Some("a fixed shortcut");
    }
    keymap.iter().find(|(other, a)| *a != action && k.same(other, keys)).map(|(_, a)| label(*a))
}

pub fn keymap(k: &impl Keystrokes, stored: &HashMap<String, String>) -> Vec<(String, Action)> {
    let chosen = |action: Action| stored.get(&action.setting_key()).and_then(|keys| k.parse(keys).ok());
    let mut keymap: Vec<(String, Action)> = rebindable(k).map(|(_, a)| (chosen(a).unwrap_or_else(|| default(k, a)), a)).collect();
    // A fallback can collide with a row already checked, so repeat until nothing changes.
    let mut changed = true;
    while changed {
        changed = false;
        for ix in 0..keymap.len() {
            let (keys, action) = keymap[ix].clone();
            if keys != default(k, action) && conflict(k, &keys, action, &keymap).is_some() {
                keymap[ix].0 = default(k, action);
                changed = true;
            }
        }
    }
    keymap
}

pub fn stored(disk: &store::Store) -> HashMap<String, String> {
    disk.settings_with_prefix(PREFIX).unwrap_or_default().into_iter().collect()
}

pub fn save(k: &impl Keystrokes, disk: &store::Store, action: Action, raw: &str) -> Result<(), String> {
    let keys = k.parse(raw).map_err(str::to_owned)?;
    if let Some(user) = conflict(k, &keys, action, &keymap(k, &stored(disk))) {
        return Err(format!("{} is already used by {user}.", k.display(&keys)));
    }
    let saved = if keys == default(k, action) { disk.delete_setting(&action.setting_key()) } else { disk.set_setting(&action.setting_key(), &keys) };
    saved.map_err(|e| format!("The shortcut could not be saved ({e})."))
}

pub fn reset(k: &impl Keystrokes, disk: &store::Store, action: Option<Action>) -> Result<(), String> {
    if let Some(action) = action
        && let Some(user) = conflict(k, &default(k, action), action, &keymap(k, &stored(disk)))
    {
        return Err(format!("{} is now used by {user}; change that first.", k.display(&default(k, action))));
    }
    for (_, a) in rebindable(k).filter(|(_, a)| action.is_none_or(|x| x == *a)) {
        disk.delete_setting(&a.setting_key()).map_err(|e| format!("The shortcut could not be reset ({e})."))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYBOARDS: [Keyboard; 3] = [Keyboard::Apple, Keyboard::Linux, Keyboard::Other];

    fn tablet(action: Action) -> bool {
        matches!(
            action,
            Action::NewTab
                | Action::OpenLocation
                | Action::OpenFile
                | Action::CommandBar
                | Action::CloseTab
                | Action::ReopenClosedTab
                | Action::LastTab
                | Action::NextTab
                | Action::PreviousTab
                | Action::TogglePin
                | Action::Reload
                | Action::ReloadFromOrigin
                | Action::Back
                | Action::Forward
                | Action::Find
                | Action::FindInTabs
                | Action::ZoomIn
                | Action::ZoomOut
                | Action::ZoomReset
                | Action::CopyLink
                | Action::CopyCleanLink
                | Action::ToggleReader
                | Action::ToggleDownloads
                | Action::ShowHistory
                | Action::ToggleSidebar
                | Action::NewSpace
                | Action::PrivateSpace
                | Action::BurnerTab
                | Action::Settings
                | Action::SelectTab(_)
                | Action::SwitchSpace(_)
        )
    }

    fn desktop(action: Action) -> bool {
        !matches!(action, Action::CommandBar | Action::HideTarn | Action::SelectTab(_) | Action::SwitchSpace(_))
    }

    fn pressed(keys: &str, keyboard: Keyboard) -> Vec<&str> {
        let mut parts: Vec<&str> = keys.split('+').map(|p| if p == "mod" { if keyboard.apple() { "cmd" } else { "ctrl" } } else { p }).collect();
        parts.sort_unstable();
        parts
    }

    fn unique(keyboard: Keyboard, offered: impl Fn(Action) -> bool) {
        let all = bindings(keyboard, offered);
        for (keys, _) in &all {
            let parts = pressed(keys, keyboard);
            assert!(parts.windows(2).all(|w| w[0] != w[1]), "{keys} repeats a modifier on {keyboard:?}");
        }
        let mut keys: Vec<Vec<&str>> = all.iter().map(|(k, _)| pressed(k, keyboard)).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), all.len(), "{keyboard:?}");
    }

    #[test]
    fn every_key_is_bound_once_on_every_keyboard() {
        unique(Keyboard::Apple, |_| true);
        unique(Keyboard::Linux, desktop);
        unique(Keyboard::Other, tablet);
        let of = |keyboard, action| default_keys(action, keyboard);
        assert_eq!(of(Keyboard::Apple, Action::SwitchSpace(3)).as_deref(), Some("ctrl+3"));
        assert_eq!(of(Keyboard::Other, Action::SwitchSpace(3)).as_deref(), Some("alt+3"), "Ctrl is the command key here");
        assert_eq!(of(Keyboard::Apple, Action::SelectTab(8)).as_deref(), Some("mod+8"));
        assert_eq!(of(Keyboard::Apple, Action::SelectTab(9)), None);
        assert_eq!(of(Keyboard::Apple, Action::CopyCleanLink).as_deref(), Some("ctrl+shift+mod+c"));
        assert_eq!(of(Keyboard::Other, Action::CopyCleanLink).as_deref(), Some("alt+shift+mod+c"));
        assert_eq!(label(Action::ReopenClosedTab), "Reopen closed tab");
        assert_eq!(label(Action::SelectTab(2)), "Select tab 1–8");
        assert_eq!(Action::NewTab.setting_key(), "keymap.tarn::NewTab");
        assert_eq!(Action::ToggleSplitView.setting_key(), "keymap.tarn::ToggleSplitView");
    }

    #[test]
    fn each_keyboard_keeps_its_own_defaults() {
        let of = |keyboard, action| default_keys(action, keyboard).unwrap();
        for keyboard in KEYBOARDS {
            assert_eq!(of(keyboard, Action::NewTab), "mod+t");
            assert_eq!(of(keyboard, Action::NextTab), "ctrl+tab");
        }
        assert_eq!(of(Keyboard::Apple, Action::Back), "mod+[");
        assert_eq!(of(Keyboard::Other, Action::Back), "mod+[");
        assert_eq!(of(Keyboard::Linux, Action::Back), "alt+left");
        assert_eq!(of(Keyboard::Linux, Action::Forward), "alt+right");
        assert_eq!(of(Keyboard::Apple, Action::ShowHistory), "mod+y");
        assert_eq!(of(Keyboard::Linux, Action::ShowHistory), "mod+h");
        assert_eq!(of(Keyboard::Other, Action::ToggleReader), "mod+alt+r");
        assert_eq!(of(Keyboard::Linux, Action::ToggleReader), "mod+alt+e");
        assert_eq!(of(Keyboard::Apple, Action::ToggleSplitView), "ctrl+mod+n");
        assert_eq!(of(Keyboard::Linux, Action::ToggleSplitView), "alt+mod+n");
        assert_eq!(of(Keyboard::Apple, Action::TakeScreenshot), "mod+alt+s");
        assert_eq!(of(Keyboard::Linux, Action::TakeScreenshot), "mod+alt+shift+s");
        assert_eq!(of(Keyboard::Other, Action::CopyMarkdownLink), "mod+alt+c");
    }

    #[test]
    fn keys_read_like_each_platform() {
        let (apple, pc) = (Keyboard::Apple, Keyboard::Other);
        assert_eq!(shown("mod+shift+t", apple), "⇧⌘T");
        assert_eq!(shown("mod+shift+t", pc), "Ctrl+Shift+T");
        assert_eq!(shown("ctrl+shift+mod+c", apple), "⌃⇧⌘C");
        assert_eq!(shown("ctrl+shift+mod+c", pc), "Ctrl+Alt+Shift+C");
        assert_eq!(shown("ctrl+tab", apple), "⌃⇥");
        assert_eq!(shown("ctrl+tab", pc), "Ctrl+Tab");
        assert_eq!(shown("mod+,", apple), "⌘,");
        assert_eq!(shown("mod+=", pc), "Ctrl+=");
        assert_eq!(shown("escape", apple), "Esc");
        assert_eq!(keys_for(Action::TogglePin, apple).as_deref(), Some("⌘D"));
        assert_eq!(keys_for(Action::SelectTab(1), apple), None);
        assert_eq!(for_command(crate::palette::Command::TogglePin), Some(Action::TogglePin));
        assert_eq!(for_command(crate::palette::Command::Memory), None);
    }

    #[test]
    fn the_table_lists_every_offered_action_once() {
        for keyboard in KEYBOARDS {
            let table = table(keyboard, tablet);
            assert_eq!(table.iter().map(|(n, _)| *n).collect::<Vec<_>>(), SECTIONS.map(|(n, _)| n));
            let listed: Vec<_> = table.iter().flat_map(|(_, r)| r.iter()).collect();
            assert_eq!(listed.len(), rows().filter(|(_, a, _)| tablet(*a)).count() + 3);
            assert!(listed.iter().all(|(does, keys)| !does.is_empty() && !keys.is_empty()));
        }
        let names = |rows: &[(&'static str, String)]| rows.iter().map(|(d, _)| *d).collect::<Vec<_>>();
        let tabs = &table(Keyboard::Apple, tablet)[0].1;
        let at = names(tabs).iter().position(|d| *d == "Select tab 1–8").unwrap();
        assert_eq!(tabs[at].1, "⌘1–8");
        assert_eq!(tabs[at + 1].0, "Select last tab", "the range sits where the macOS pane lists it");
        let window = &table(Keyboard::Other, tablet)[3].1;
        assert_eq!(names(window)[names(window).len() - 3..], ["Switch to space 1–9", "Settings", "Close command bar, popover or Settings"]);
        assert_eq!(window[window.len() - 3].1, "Alt+1–9");
        assert_eq!(window[window.len() - 1].1, "Esc");
        assert!(!names(&table(Keyboard::Apple, tablet)[0].1).contains(&"Close all tabs"), "only offered actions are listed");
    }

    struct Plain(Keyboard);

    impl Keystrokes for Plain {
        fn keyboard(&self) -> Keyboard {
            self.0
        }
        fn offered(&self, action: Action) -> bool {
            desktop(action)
        }
        fn written(&self, keys: &str) -> String {
            keys.replace("mod", "ctrl").replace("alt+ctrl", "ctrl+alt")
        }
        fn parse(&self, raw: &str) -> Result<String, &'static str> {
            let raw = raw.trim().to_lowercase();
            if !raw.starts_with("ctrl+") || raw.contains(' ') {
                return Err("Use Ctrl with the key.");
            }
            Ok(raw)
        }
        fn fixed(&self, keys: &str) -> bool {
            matches!(keys, "ctrl+1" | "ctrl+c")
        }
        fn display(&self, keys: &str) -> String {
            keys.to_uppercase()
        }
    }

    #[test]
    fn stored_keys_replace_defaults_unless_taken_or_fixed() {
        let k = Plain(Keyboard::Linux);
        let defaults = keymap(&k, &HashMap::new());
        assert_eq!(defaults.len(), rebindable(&k).count());
        assert!(defaults.iter().all(|(keys, a)| conflict(&k, keys, *a, &defaults).is_none()), "the defaults are distinct");
        assert_eq!(default(&k, Action::Back), "alt+left");
        assert_eq!(default(&k, Action::ToggleSplitView), "ctrl+alt+n");
        let stored = HashMap::from([
            (Action::NewTab.setting_key(), "ctrl+shift+y".to_owned()),
            (Action::Reload.setting_key(), "ctrl+w".to_owned()),
            (Action::Find.setting_key(), "ctrl+1".to_owned()),
            (Action::Print.setting_key(), "nonsense".to_owned()),
        ]);
        let map = keymap(&k, &stored);
        let of = |a: Action| map.iter().find(|(_, x)| *x == a).unwrap().0.clone();
        assert_eq!(of(Action::NewTab), "ctrl+shift+y");
        assert_eq!(of(Action::Reload), "ctrl+r", "taken by Close tab");
        assert_eq!(of(Action::Find), "ctrl+f", "a fixed key");
        assert_eq!(of(Action::Print), "ctrl+p", "unparsable");
        assert_eq!(conflict(&k, "ctrl+w", Action::Reload, &map), Some("Close tab"));
        assert_eq!(conflict(&k, "ctrl+w", Action::CloseTab, &map), None, "an action keeps its own keys");
        assert_eq!(conflict(&k, "ctrl+c", Action::NewTab, &map), Some("a fixed shortcut"));
    }

    #[test]
    fn swapped_keys_hold_and_a_fallback_never_doubles_a_key() {
        let k = Plain(Keyboard::Linux);
        let swapped = HashMap::from([(Action::NewTab.setting_key(), "ctrl+l".to_owned()), (Action::OpenLocation.setting_key(), "ctrl+t".to_owned())]);
        let map = keymap(&k, &swapped);
        assert_eq!((map[0].0.as_str(), map[1].0.as_str()), ("ctrl+l", "ctrl+t"));
        let chained = HashMap::from([(Action::NewTab.setting_key(), "ctrl+l".to_owned()), (Action::OpenLocation.setting_key(), "ctrl+r".to_owned())]);
        let map = keymap(&k, &chained);
        let of = |a: Action| map.iter().find(|(_, x)| *x == a).unwrap().0.clone();
        assert_eq!((of(Action::NewTab), of(Action::OpenLocation), of(Action::Reload)), ("ctrl+t".into(), "ctrl+l".into(), "ctrl+r".into()));
        assert!(map.iter().all(|(keys, a)| conflict(&k, keys, *a, &map).is_none()));
    }

    #[test]
    fn saving_and_resetting_go_through_the_store() {
        let k = Plain(Keyboard::Linux);
        let disk = store::Store::open_in_memory().unwrap();
        assert!(save(&k, &disk, Action::NewTab, "ctrl+w").unwrap_err().contains("Close tab"));
        assert!(save(&k, &disk, Action::NewTab, "t").is_err());
        save(&k, &disk, Action::NewTab, "ctrl+shift+y").unwrap();
        assert_eq!(stored(&disk)["keymap.tarn::NewTab"], "ctrl+shift+y");
        save(&k, &disk, Action::CloseTab, "ctrl+t").unwrap();
        assert!(reset(&k, &disk, Some(Action::NewTab)).unwrap_err().contains("Close tab"));
        save(&k, &disk, Action::CloseTab, "ctrl+w").unwrap();
        assert!(!stored(&disk).contains_key("keymap.tarn::CloseTab"), "saving the default keys forgets the choice");
        reset(&k, &disk, Some(Action::NewTab)).unwrap();
        save(&k, &disk, Action::Reload, "ctrl+shift+y").unwrap();
        save(&k, &disk, Action::Find, "ctrl+r").unwrap();
        reset(&k, &disk, None).unwrap();
        assert!(stored(&disk).is_empty(), "resetting all cannot collide");
    }
}
