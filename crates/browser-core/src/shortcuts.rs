#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    NewTab,
    OpenLocation,
    OpenFile,
    CommandBar,
    CloseTab,
    ReopenClosedTab,
    NextTab,
    PreviousTab,
    LastTab,
    TogglePin,
    Reload,
    ReloadFromOrigin,
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
    BurnerTab,
    Settings,
    SelectTab(u8),
    SwitchSpace(u8),
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
            ("Select last tab", Action::LastTab, "mod+9"),
            ("Next tab", Action::NextTab, "ctrl+tab"),
            ("Previous tab", Action::PreviousTab, "ctrl+shift+tab"),
            ("Pin or unpin tab", Action::TogglePin, "mod+d"),
        ],
    ),
    (
        "Page",
        &[
            ("Reload", Action::Reload, "mod+r"),
            ("Reload without cache", Action::ReloadFromOrigin, "mod+shift+r"),
            ("Back", Action::Back, "mod+["),
            ("Forward", Action::Forward, "mod+]"),
            ("Find in page", Action::Find, "mod+f"),
            ("Find in tabs", Action::FindInTabs, "mod+shift+f"),
            ("Zoom in", Action::ZoomIn, "mod+="),
            ("Zoom out", Action::ZoomOut, "mod+-"),
            ("Actual size", Action::ZoomReset, "mod+0"),
            ("Copy link", Action::CopyLink, "mod+shift+c"),
            ("Copy clean link", Action::CopyCleanLink, "ctrl+shift+mod+c"),
            ("Reader", Action::ToggleReader, "mod+alt+r"),
        ],
    ),
    ("Library", &[("Show downloads", Action::ToggleDownloads, "mod+shift+j"), ("Show history", Action::ShowHistory, "mod+y")]),
    (
        "Window",
        &[
            ("Toggle sidebar", Action::ToggleSidebar, "mod+s"),
            ("New space", Action::NewSpace, "mod+shift+s"),
            ("Private space", Action::PrivateSpace, "mod+shift+n"),
            ("Burner tab", Action::BurnerTab, "mod+alt+shift+n"),
            ("Settings", Action::Settings, "mod+,"),
        ],
    ),
];

pub const SELECTABLE_TABS: u8 = 8;
pub const SWITCHABLE_SPACES: u8 = 9;

pub fn label(action: Action) -> &'static str {
    match action {
        Action::SelectTab(_) => "Select tab 1–8",
        Action::SwitchSpace(_) => "Switch to space 1–9",
        _ => SECTIONS.iter().flat_map(|(_, rows)| rows.iter()).find(|(_, a, _)| *a == action).map_or("", |(l, ..)| l),
    }
}

fn for_keyboard(keys: &str, apple: bool) -> String {
    match apple {
        true => keys.to_owned(),
        false if keys.contains("mod+") => keys.replace("ctrl+", "alt+"),
        false => keys.to_owned(),
    }
}

pub fn bindings(apple: bool) -> Vec<(String, Action)> {
    let space = if apple { "ctrl" } else { "alt" };
    SECTIONS
        .iter()
        .flat_map(|(_, rows)| rows.iter().map(move |(_, action, keys)| (for_keyboard(keys, apple), *action)))
        .chain((1..=SELECTABLE_TABS).map(|n| (format!("mod+{n}"), Action::SelectTab(n))))
        .chain((1..=SWITCHABLE_SPACES).map(|n| (format!("{space}+{n}"), Action::SwitchSpace(n))))
        .collect()
}

pub fn shown(keys: &str, apple: bool) -> String {
    let keys = for_keyboard(keys, apple);
    let mut mods: Vec<&str> = keys.split('+').collect();
    let key = mods.pop().unwrap_or_default();
    let has = |m: &str| mods.contains(&m);
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

pub fn keys_for(action: Action, apple: bool) -> Option<String> {
    SECTIONS.iter().flat_map(|(_, rows)| rows.iter()).find(|(_, a, _)| *a == action).map(|(_, _, keys)| shown(keys, apple))
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
        _ => None,
    }
}

pub fn table(apple: bool) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    let range = |first: &str, last: &str| format!("{}–{}", shown(first, apple), shown(last, apple).chars().last().unwrap_or_default());
    let space = if apple { "ctrl" } else { "alt" };
    SECTIONS
        .iter()
        .map(|(name, rows)| {
            let mut shown_rows = Vec::new();
            for (does, action, keys) in rows.iter() {
                match action {
                    Action::LastTab => shown_rows.push((label(Action::SelectTab(1)), range("mod+1", &format!("mod+{SELECTABLE_TABS}")))),
                    Action::Settings => shown_rows.push((label(Action::SwitchSpace(1)), range(&format!("{space}+1"), &format!("{space}+{SWITCHABLE_SPACES}")))),
                    _ => {}
                }
                shown_rows.push((*does, shown(keys, apple)));
            }
            if *name == "Window" {
                shown_rows.push(("Close command bar, popover or Settings", shown("escape", apple)));
            }
            (*name, shown_rows)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressed(keys: &str, apple: bool) -> Vec<&str> {
        let mut parts: Vec<&str> = keys.split('+').map(|p| if p == "mod" { if apple { "cmd" } else { "ctrl" } } else { p }).collect();
        parts.sort_unstable();
        parts
    }

    #[test]
    fn every_key_is_bound_once_on_both_keyboards() {
        for apple in [true, false] {
            let all = bindings(apple);
            for (keys, _) in &all {
                let parts = pressed(keys, apple);
                assert!(parts.windows(2).all(|w| w[0] != w[1]), "{keys} repeats a modifier, apple: {apple}");
            }
            let mut keys: Vec<Vec<&str>> = all.iter().map(|(k, _)| pressed(k, apple)).collect();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), all.len(), "apple: {apple}");
        }
        let of = |apple: bool, action: Action| bindings(apple).into_iter().find(|(_, a)| *a == action).map(|(k, _)| k);
        assert_eq!(of(true, Action::SwitchSpace(3)).as_deref(), Some("ctrl+3"));
        assert_eq!(of(false, Action::SwitchSpace(3)).as_deref(), Some("alt+3"), "Ctrl is the command key here");
        assert_eq!(of(true, Action::SelectTab(8)).as_deref(), Some("mod+8"));
        assert_eq!(of(true, Action::SelectTab(9)), None);
        assert_eq!(of(true, Action::CopyCleanLink).as_deref(), Some("ctrl+shift+mod+c"));
        assert_eq!(of(false, Action::CopyCleanLink).as_deref(), Some("alt+shift+mod+c"));
        assert_eq!(label(Action::ReopenClosedTab), "Reopen closed tab");
        assert_eq!(label(Action::SelectTab(2)), "Select tab 1–8");
    }

    #[test]
    fn keys_read_like_each_platform() {
        assert_eq!(shown("mod+shift+t", true), "⇧⌘T");
        assert_eq!(shown("mod+shift+t", false), "Ctrl+Shift+T");
        assert_eq!(shown("ctrl+shift+mod+c", true), "⌃⇧⌘C");
        assert_eq!(shown("ctrl+shift+mod+c", false), "Ctrl+Alt+Shift+C");
        assert_eq!(shown("ctrl+tab", true), "⌃⇥");
        assert_eq!(shown("ctrl+tab", false), "Ctrl+Tab");
        assert_eq!(shown("mod+,", true), "⌘,");
        assert_eq!(shown("mod+=", false), "Ctrl+=");
        assert_eq!(shown("escape", true), "Esc");
        assert_eq!(keys_for(Action::TogglePin, true).as_deref(), Some("⌘D"));
        assert_eq!(keys_for(Action::SelectTab(1), true), None);
        assert_eq!(for_command(crate::palette::Command::TogglePin), Some(Action::TogglePin));
        assert_eq!(for_command(crate::palette::Command::Memory), None);
    }

    #[test]
    fn the_table_lists_every_bound_action_once() {
        for apple in [true, false] {
            let table = table(apple);
            assert_eq!(table.iter().map(|(n, _)| *n).collect::<Vec<_>>(), SECTIONS.map(|(n, _)| n));
            let rows: Vec<_> = table.iter().flat_map(|(_, r)| r.iter()).collect();
            assert_eq!(rows.len(), SECTIONS.iter().map(|(_, r)| r.len()).sum::<usize>() + 3);
            assert!(rows.iter().all(|(does, keys)| !does.is_empty() && !keys.is_empty()));
        }
        let names = |rows: &[(&'static str, String)]| rows.iter().map(|(d, _)| *d).collect::<Vec<_>>();
        let tabs = &table(true)[0].1;
        let at = names(tabs).iter().position(|d| *d == "Select tab 1–8").unwrap();
        assert_eq!(tabs[at].1, "⌘1–8");
        assert_eq!(tabs[at + 1].0, "Select last tab", "the range sits where the macOS pane lists it");
        let window = &table(false)[3].1;
        assert_eq!(names(window)[names(window).len() - 3..], ["Switch to space 1–9", "Settings", "Close command bar, popover or Settings"]);
        assert_eq!(window[window.len() - 3].1, "Alt+1–9");
        assert_eq!(window[window.len() - 1].1, "Esc");
    }
}
