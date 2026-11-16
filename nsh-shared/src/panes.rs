#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Profile,
    General,
    Appearance,
    Search,
    Privacy,
    Permissions,
    Profiles,
    Shortcuts,
}

pub const PANES: [(Pane, &str); 8] = [
    (Pane::Profile, "Profile"),
    (Pane::General, "General"),
    (Pane::Appearance, "Appearance"),
    (Pane::Search, "Search"),
    (Pane::Privacy, "Privacy"),
    (Pane::Permissions, "Permissions"),
    (Pane::Profiles, "Profiles"),
    (Pane::Shortcuts, "Shortcuts"),
];

pub const GROUPS: [(&str, &[Pane]); 2] = [
    ("You", &[Pane::Profile, Pane::Profiles]),
    ("Browser", &[Pane::General, Pane::Appearance, Pane::Search, Pane::Privacy, Pane::Permissions, Pane::Shortcuts]),
];

pub fn name(pane: Pane) -> &'static str {
    PANES.iter().find(|(p, _)| *p == pane).map_or("", |(_, n)| n)
}

pub fn icon(pane: Pane) -> &'static str {
    match pane {
        Pane::Profile => crate::icons::SHIELD,
        Pane::General => crate::icons::GEAR,
        Pane::Appearance => crate::icons::PALETTE,
        Pane::Search => crate::icons::SEARCH,
        Pane::Privacy => crate::icons::HAND,
        Pane::Permissions => crate::icons::CAMERA,
        Pane::Profiles => crate::icons::PERSON,
        Pane::Shortcuts => crate::icons::KEYBOARD,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_follow_the_mac_order_and_have_names() {
        assert_eq!(PANES[0].0, Pane::Profile);
        assert!(PANES.iter().all(|(p, n)| name(*p) == *n && !n.is_empty() && !icon(*p).is_empty()));
        let grouped: Vec<Pane> = GROUPS.iter().flat_map(|(_, panes)| panes.iter().copied()).collect();
        assert_eq!(grouped.len(), PANES.len());
        assert!(PANES.iter().all(|(p, _)| grouped.contains(p)), "every pane is in one group");
    }
}
