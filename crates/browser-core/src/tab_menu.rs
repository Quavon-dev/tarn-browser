use store::{Id, TabKind};

use crate::split;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Reload,
    Open,
    Duplicate,
    CopyLink,
    CopyCleanLink,
    Split,
    LeaveSplit,
    TogglePin,
    ToggleTile,
    Rename,
    ResetTitle,
    MoveToSpace(usize),
    AddToGroup(Id),
    NewGroup,
    RemoveFromGroup,
    Unload,
    Close,
    CloseOthers,
    CloseBelow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Command(Command, String, bool),
    Heading(&'static str),
    Separator,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub kind: TabKind,
    pub active: bool,
    pub has_link: bool,
    pub split: split::Menu,
    pub split_view: bool,
    pub burner: bool,
    pub custom_title: bool,
    pub grouped: bool,
    pub others: bool,
    pub below: bool,
    pub spaces: Vec<(usize, String)>,
    pub groups: Vec<(Id, String)>,
}

fn command(command: Command, label: impl Into<String>, enabled: bool) -> Item {
    Item::Command(command, label.into(), enabled)
}

pub fn pinnable(kind: TabKind, burner: bool) -> bool {
    !burner && kind != TabKind::Tile
}

pub fn items(tab: &Tab) -> Vec<Item> {
    let ephemeral = tab.kind == TabKind::Ephemeral;
    let mut items = vec![
        if tab.active { command(Command::Reload, "Reload", true) } else { command(Command::Open, "Open", true) },
        command(Command::Duplicate, "Duplicate", tab.has_link),
        Item::Separator,
        command(Command::CopyLink, "Copy Link", tab.has_link),
        command(Command::CopyCleanLink, "Copy Clean Link", tab.has_link),
    ];
    if tab.split_view {
        items.push(Item::Separator);
        items.push(command(Command::Split, tab.split.entry.label(), tab.split.enabled));
        if tab.split.member {
            items.push(command(Command::LeaveSplit, "Remove from Split View", true));
        }
    }
    items.extend([
        Item::Separator,
        command(Command::TogglePin, if tab.kind == TabKind::Pinned { "Unpin Tab" } else { "Pin Tab" }, pinnable(tab.kind, tab.burner)),
        command(Command::ToggleTile, if tab.kind == TabKind::Tile { "Remove from Tiles" } else { "Add to Tiles" }, !tab.burner),
        command(Command::Rename, "Rename", tab.kind != TabKind::Tile),
    ]);
    if tab.custom_title {
        items.push(command(Command::ResetTitle, "Reset Title", true));
    }
    if !tab.spaces.is_empty() {
        items.extend([Item::Separator, Item::Heading("Move to Space")]);
        items.extend(tab.spaces.iter().map(|(ix, name)| command(Command::MoveToSpace(*ix), name.clone(), true)));
    }
    if ephemeral {
        items.extend([Item::Separator, Item::Heading("Add to Group")]);
        items.extend(tab.groups.iter().map(|(id, name)| command(Command::AddToGroup(*id), name.clone(), true)));
        items.push(command(Command::NewGroup, "New Group", true));
        if tab.grouped {
            items.push(command(Command::RemoveFromGroup, "Remove from Group", true));
        }
    }
    items.push(Item::Separator);
    if !ephemeral {
        items.push(command(Command::Unload, "Unload Tab", true));
        return items;
    }
    items.extend([
        command(Command::Close, "Close Tab", true),
        command(Command::CloseOthers, "Close Other Tabs", tab.others),
        command(Command::CloseBelow, "Close Tabs Below", tab.below),
    ]);
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(kind: TabKind) -> Tab {
        Tab {
            kind,
            active: true,
            has_link: true,
            split: split::Menu { entry: split::Entry::Open, enabled: true, member: false },
            split_view: true,
            burner: false,
            custom_title: false,
            grouped: false,
            others: true,
            below: false,
            spaces: Vec::new(),
            groups: Vec::new(),
        }
    }

    fn labels(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                Item::Command(_, label, true) => label.clone(),
                Item::Command(_, label, false) => format!("({label})"),
                Item::Heading(h) => format!("# {h}"),
                Item::Separator => "-".to_owned(),
            })
            .collect()
    }

    #[test]
    fn an_active_loose_tab_follows_the_macos_order() {
        assert_eq!(
            labels(&items(&tab(TabKind::Ephemeral))),
            [
                "Reload",
                "Duplicate",
                "-",
                "Copy Link",
                "Copy Clean Link",
                "-",
                "Open in Split View",
                "-",
                "Pin Tab",
                "Add to Tiles",
                "Rename",
                "-",
                "# Add to Group",
                "New Group",
                "-",
                "Close Tab",
                "Close Other Tabs",
                "(Close Tabs Below)"
            ]
        );
    }

    #[test]
    fn a_pinned_tab_unloads_and_a_tile_cannot_be_pinned_or_renamed() {
        let pinned = labels(&items(&Tab { spaces: vec![(1, "Work".into())], custom_title: true, ..tab(TabKind::Pinned) }));
        assert!(pinned.ends_with(&["-".into(), "# Move to Space".into(), "Work".into(), "-".into(), "Unload Tab".into()]));
        assert!(pinned.contains(&"Unpin Tab".to_owned()) && pinned.contains(&"Reset Title".to_owned()));
        assert!(!pinned.iter().any(|l| l.contains("Close") || l.contains("Group")));
        let tile = labels(&items(&tab(TabKind::Tile)));
        assert!(tile.contains(&"(Pin Tab)".to_owned()) && tile.contains(&"(Rename)".to_owned()) && tile.contains(&"Remove from Tiles".to_owned()));
    }

    #[test]
    fn a_burner_tab_pins_nothing_and_a_split_member_can_leave() {
        let burner = labels(&items(&Tab { burner: true, ..tab(TabKind::Ephemeral) }));
        assert!(burner.contains(&"(Pin Tab)".to_owned()) && burner.contains(&"(Add to Tiles)".to_owned()));
        let member = Tab { split: split::Menu { entry: split::Entry::Add, enabled: false, member: true }, ..tab(TabKind::Ephemeral) };
        assert!(labels(&items(&member)).contains(&"Remove from Split View".to_owned()));
        let narrow = labels(&items(&Tab { split_view: false, ..member }));
        assert!(!narrow.iter().any(|l| l.contains("Split")));
    }

    #[test]
    fn a_link_less_tab_has_nothing_to_copy_or_duplicate() {
        let empty = labels(&items(&Tab { active: false, has_link: false, grouped: true, groups: vec![(7, "Mail".into())], ..tab(TabKind::Ephemeral) }));
        assert_eq!(empty[..5], ["Open", "(Duplicate)", "-", "(Copy Link)", "(Copy Clean Link)"]);
        assert!(empty.contains(&"Mail".to_owned()) && empty.contains(&"Remove from Group".to_owned()));
    }
}
