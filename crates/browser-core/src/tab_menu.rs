use crate::split;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Reload,
    Open,
    CopyLink,
    CopyCleanLink,
    Split,
    LeaveSplit,
    MoveToSpace,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Command(Command, &'static str),
    Separator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tab {
    pub active: bool,
    pub has_link: bool,
    pub split: split::Menu,
    pub split_view: bool,
    pub movable: bool,
}

pub fn items(tab: Tab) -> Vec<Item> {
    let open = if tab.active { (Command::Reload, "Reload") } else { (Command::Open, "Open") };
    let link = if tab.has_link { vec![(Command::CopyLink, "Copy Link"), (Command::CopyCleanLink, "Copy Clean Link")] } else { Vec::new() };
    let mut split = Vec::new();
    if tab.split_view && tab.split.enabled {
        split.push((Command::Split, tab.split.entry.label()));
    }
    if tab.split_view && tab.split.member {
        split.push((Command::LeaveSplit, "Remove from Split View"));
    }
    let movable = if tab.movable { vec![(Command::MoveToSpace, "Move to Space…")] } else { Vec::new() };
    let groups = [vec![open], link, split, movable, vec![(Command::Close, "Close Tab")]];
    let mut items = Vec::new();
    for group in groups.into_iter().filter(|g| !g.is_empty()) {
        if !items.is_empty() {
            items.push(Item::Separator);
        }
        items.extend(group.into_iter().map(|(command, label)| Item::Command(command, label)));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(entry: split::Entry, enabled: bool, member: bool) -> split::Menu {
        split::Menu { entry, enabled, member }
    }

    fn labels(items: &[Item]) -> Vec<&'static str> {
        items
            .iter()
            .map(|i| match i {
                Item::Command(_, label) => label,
                Item::Separator => "-",
            })
            .collect()
    }

    #[test]
    fn an_active_tab_reloads_and_offers_its_link_split_and_move() {
        let tab = Tab { active: true, has_link: true, split: menu(split::Entry::Open, true, false), split_view: true, movable: true };
        assert_eq!(labels(&items(tab)), ["Reload", "-", "Copy Link", "Copy Clean Link", "-", "Open in Split View", "-", "Move to Space…", "-", "Close Tab"]);
    }

    #[test]
    fn groups_with_nothing_to_offer_leave_no_separator() {
        let tab = Tab { active: false, has_link: false, split: menu(split::Entry::Add, false, false), split_view: true, movable: false };
        assert_eq!(labels(&items(tab)), ["Open", "-", "Close Tab"]);
    }

    #[test]
    fn a_split_member_can_leave_and_split_view_needs_room() {
        let member = Tab { active: false, has_link: true, split: menu(split::Entry::Add, false, true), split_view: true, movable: false };
        assert_eq!(labels(&items(member)), ["Open", "-", "Copy Link", "Copy Clean Link", "-", "Remove from Split View", "-", "Close Tab"]);
        let narrow = Tab { split_view: false, ..member };
        assert!(!items(narrow).contains(&Item::Command(Command::LeaveSplit, "Remove from Split View")));
    }
}
