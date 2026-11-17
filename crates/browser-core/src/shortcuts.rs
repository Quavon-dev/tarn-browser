#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    NewTab,
    OpenLocation,
    CommandBar,
    CloseTab,
    ReopenClosedTab,
    NextTab,
    PreviousTab,
    LastTab,
    TogglePin,
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
            ("Back", Action::Back, "mod+["),
            ("Forward", Action::Forward, "mod+]"),
            ("Find in page", Action::Find, "mod+f"),
            ("Find in tabs", Action::FindInTabs, "mod+shift+f"),
            ("Zoom in", Action::ZoomIn, "mod+="),
            ("Zoom out", Action::ZoomOut, "mod+-"),
            ("Actual size", Action::ZoomReset, "mod+0"),
            ("Copy link", Action::CopyLink, "mod+shift+c"),
            ("Copy clean link", Action::CopyCleanLink, "ctrl+shift+mod+c"),
            ("Reader", Action::ToggleReader, "mod+shift+r"),
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

pub fn tab_to_select(action: Action, count: usize, active: Option<usize>) -> Option<usize> {
    let last = count.checked_sub(1)?;
    match action {
        Action::SelectTab(n) => Some(usize::from(n).checked_sub(1)?).filter(|ix| *ix <= last),
        Action::LastTab => Some(last),
        Action::NextTab => Some(active.map_or(0, |ix| if ix >= last { 0 } else { ix + 1 })),
        Action::PreviousTab => Some(active.map_or(last, |ix| if ix == 0 { last } else { ix - 1 })),
        _ => None,
    }
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
    fn tab_keys_pick_an_index_that_exists() {
        assert_eq!(tab_to_select(Action::SelectTab(1), 3, Some(2)), Some(0));
        assert_eq!(tab_to_select(Action::SelectTab(4), 3, Some(0)), None);
        assert_eq!(tab_to_select(Action::LastTab, 3, Some(0)), Some(2));
        assert_eq!(tab_to_select(Action::NextTab, 3, Some(2)), Some(0));
        assert_eq!(tab_to_select(Action::NextTab, 3, None), Some(0));
        assert_eq!(tab_to_select(Action::PreviousTab, 3, Some(0)), Some(2));
        assert_eq!(tab_to_select(Action::PreviousTab, 3, Some(2)), Some(1));
        assert_eq!(tab_to_select(Action::NextTab, 0, None), None);
        assert_eq!(tab_to_select(Action::Reload, 3, Some(0)), None);
    }
}
