use store::{Id, TabKind, WebEngine};

use crate::shortcuts::Action;
use crate::split;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Reload,
    Open,
    Duplicate,
    SwitchEngine,
    CopyLink,
    CopyCleanLink,
    Bookmark,
    Split,
    LeaveSplit,
    RotateSplit,
    CloseSplit,
    TogglePin,
    ToggleTile,
    Rename,
    ResetTitle,
    ReturnToPinned,
    SetPinnedUrl,
    MoveToSpace(usize),
    AddToGroup(Id),
    NewGroup,
    RemoveFromGroup,
    ToggleMute,
    Sleep,
    SleepOthers,
    Unload,
    Close,
    CloseOthers,
    CloseBelow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Command(Command, String, bool, Option<Action>),
    Submenu(&'static str, Vec<Item>),
    Separator,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub kind: TabKind,
    pub active: bool,
    pub has_link: bool,
    pub engine: Option<(WebEngine, bool)>,
    pub split: split::Menu,
    pub split_view: bool,
    pub burner: bool,
    pub custom_title: bool,
    pub drift: Option<String>,
    pub grouped: bool,
    pub muted: Option<bool>,
    pub sleepable: bool,
    pub sleep_others: bool,
    pub others: bool,
    pub below: bool,
    pub spaces: Vec<(usize, String)>,
    pub groups: Vec<(Id, String)>,
}

pub fn pinnable(kind: TabKind, burner: bool) -> bool {
    !burner && kind != TabKind::Tile
}

pub fn items(tab: &Tab, offered: impl Fn(Command) -> bool) -> Vec<Item> {
    let key = |action: Action| tab.active.then_some(action);
    let command = |command: Command, label: &str, enabled: bool, action: Option<Action>| {
        offered(command).then(|| Item::Command(command, label.to_owned(), enabled, action.filter(|_| enabled)))
    };
    let ephemeral = tab.kind == TabKind::Ephemeral;
    let engine = tab.engine.and_then(|(current, switchable)| {
        command(Command::SwitchEngine, if current == WebEngine::Chromium { "Reload in WebKit" } else { "Reload in Chromium" }, switchable, None)
    });
    let split = tab.split_view.then(|| {
        let (menu, member) = (tab.split, tab.split.member);
        Item::Submenu(
            "Split View",
            [
                command(Command::Split, menu.entry.label(), menu.enabled, key(Action::ToggleSplitView)),
                command(Command::LeaveSplit, "Remove from Split View", member, key(Action::RemoveFromSplitView)),
                command(Command::RotateSplit, "Rotate Layout", member, key(Action::RotateSplitLayout)),
                command(Command::CloseSplit, "Close Split View", member, key(Action::ToggleSplitView)),
            ]
            .into_iter()
            .flatten()
            .collect(),
        )
    });
    let drift = tab.drift.as_ref().map(|host| {
        [
            command(Command::ReturnToPinned, &format!("Return to {host}"), true, None),
            command(Command::SetPinnedUrl, "Set Pinned URL to Current Page", true, None),
        ]
    });
    let spaces = (!tab.spaces.is_empty())
        .then(|| Item::Submenu("Move to Space", tab.spaces.iter().filter_map(|(ix, name)| command(Command::MoveToSpace(*ix), name, true, None)).collect()));
    let groups = (ephemeral && offered(Command::NewGroup)).then(|| {
        let mut items: Vec<Item> = tab.groups.iter().filter_map(|(id, name)| command(Command::AddToGroup(*id), name, true, None)).collect();
        if !items.is_empty() {
            items.push(Item::Separator);
        }
        items.extend(command(Command::NewGroup, "New Group", true, None));
        Item::Submenu("Add to Group", items)
    });
    let mut items: Vec<Option<Item>> = vec![
        if tab.active { command(Command::Reload, "Reload", true, Some(Action::Reload)) } else { command(Command::Open, "Open", true, None) },
        command(Command::Duplicate, "Duplicate", tab.has_link, None),
        engine,
        Some(Item::Separator),
        command(Command::CopyLink, "Copy Link", tab.has_link, key(Action::CopyLink)),
        command(Command::CopyCleanLink, "Copy Clean Link", tab.has_link, key(Action::CopyCleanLink)),
        command(Command::Bookmark, "Bookmark Tab…", tab.has_link, key(Action::BookmarkTab)),
        Some(Item::Separator),
        split,
        Some(Item::Separator),
        command(Command::TogglePin, if tab.kind == TabKind::Pinned { "Unpin Tab" } else { "Pin Tab" }, pinnable(tab.kind, tab.burner), key(Action::TogglePin)),
        command(Command::ToggleTile, if tab.kind == TabKind::Tile { "Remove from Tiles" } else { "Add to Tiles" }, !tab.burner, None),
        command(Command::Rename, "Rename", tab.kind != TabKind::Tile, None),
    ];
    if tab.custom_title {
        items.push(command(Command::ResetTitle, "Reset Title", true, None));
    }
    items.extend(drift.into_iter().flatten());
    items.extend([Some(Item::Separator), spaces, groups]);
    if ephemeral && tab.grouped {
        items.push(command(Command::RemoveFromGroup, "Remove from Group", true, None));
    }
    let mute = if tab.muted == Some(true) { "Unmute Site" } else { "Mute Site" };
    items.extend([
        Some(Item::Separator),
        command(Command::ToggleMute, mute, tab.muted.is_some(), None),
        command(Command::Sleep, "Sleep Tab", tab.sleepable, None),
        command(Command::SleepOthers, "Sleep Other Tabs", tab.sleep_others, None),
        Some(Item::Separator),
    ]);
    if ephemeral {
        items.extend([
            command(Command::Close, "Close Tab", true, key(Action::CloseTab)),
            command(Command::CloseOthers, "Close Other Tabs", tab.others, None),
            command(Command::CloseBelow, "Close Tabs Below", tab.below, None),
        ]);
    } else {
        items.push(command(Command::Unload, "Unload Tab", true, key(Action::CloseTab)));
    }
    tidy(items.into_iter().flatten())
}

fn tidy(items: impl Iterator<Item = Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for item in items {
        let item = match item {
            Item::Submenu(_, ref inner) if inner.iter().all(|i| *i == Item::Separator) => continue,
            Item::Separator if out.last().is_none_or(|last| *last == Item::Separator) => continue,
            item => item,
        };
        out.push(item);
    }
    if out.last() == Some(&Item::Separator) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(kind: TabKind) -> Tab {
        Tab {
            kind,
            active: true,
            has_link: true,
            engine: None,
            split: split::Menu { entry: split::Entry::Open, enabled: true, member: false },
            split_view: true,
            burner: false,
            custom_title: false,
            drift: None,
            grouped: false,
            muted: Some(false),
            sleepable: false,
            sleep_others: true,
            others: true,
            below: false,
            spaces: Vec::new(),
            groups: Vec::new(),
        }
    }

    fn labels(items: &[Item]) -> Vec<String> {
        items
            .iter()
            .flat_map(|i| match i {
                Item::Command(_, label, true, _) => vec![label.clone()],
                Item::Command(_, label, false, _) => vec![format!("({label})")],
                Item::Submenu(name, inner) => std::iter::once(format!("# {name}")).chain(labels(inner).into_iter().map(|l| format!("  {l}"))).collect(),
                Item::Separator => vec!["-".to_owned()],
            })
            .collect()
    }

    fn all(_: Command) -> bool {
        true
    }

    #[test]
    fn an_active_loose_tab_follows_the_macos_order() {
        assert_eq!(
            labels(&items(&tab(TabKind::Ephemeral), all)),
            [
                "Reload",
                "Duplicate",
                "-",
                "Copy Link",
                "Copy Clean Link",
                "Bookmark Tab…",
                "-",
                "# Split View",
                "  Open in Split View",
                "  (Remove from Split View)",
                "  (Rotate Layout)",
                "  (Close Split View)",
                "-",
                "Pin Tab",
                "Add to Tiles",
                "Rename",
                "-",
                "# Add to Group",
                "  New Group",
                "-",
                "Mute Site",
                "(Sleep Tab)",
                "Sleep Other Tabs",
                "-",
                "Close Tab",
                "Close Other Tabs",
                "(Close Tabs Below)"
            ]
        );
    }

    #[test]
    fn key_hints_name_core_shortcuts_for_the_active_tab_only() {
        let hints = |tab: &Tab| {
            items(tab, all)
                .into_iter()
                .flat_map(|i| match i {
                    Item::Submenu(_, inner) => inner,
                    i => vec![i],
                })
                .filter_map(|i| match i {
                    Item::Command(c, _, _, Some(a)) => Some((c, a)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            hints(&tab(TabKind::Ephemeral)),
            [
                (Command::Reload, Action::Reload),
                (Command::CopyLink, Action::CopyLink),
                (Command::CopyCleanLink, Action::CopyCleanLink),
                (Command::Bookmark, Action::BookmarkTab),
                (Command::Split, Action::ToggleSplitView),
                (Command::TogglePin, Action::TogglePin),
                (Command::Close, Action::CloseTab)
            ]
        );
        let member = Tab { split: split::Menu { entry: split::Entry::Add, enabled: false, member: true }, ..tab(TabKind::Ephemeral) };
        assert!(hints(&member).contains(&(Command::RotateSplit, Action::RotateSplitLayout)));
        assert!(hints(&Tab { active: false, ..tab(TabKind::Ephemeral) }).is_empty());
    }

    #[test]
    fn a_pinned_tab_unloads_and_a_tile_cannot_be_pinned_or_renamed() {
        let drifted = Tab { spaces: vec![(1, "Work".into())], custom_title: true, drift: Some("mail.example".into()), ..tab(TabKind::Pinned) };
        let pinned = labels(&items(&drifted, all));
        assert!(pinned.ends_with(&["-".into(), "Mute Site".into(), "(Sleep Tab)".into(), "Sleep Other Tabs".into(), "-".into(), "Unload Tab".into()]));
        assert!(pinned.windows(3).any(|w| w == ["Reset Title", "Return to mail.example", "Set Pinned URL to Current Page"]));
        assert!(pinned.windows(2).any(|w| w == ["# Move to Space", "  Work"]));
        assert!(pinned.contains(&"Unpin Tab".to_owned()));
        assert!(!pinned.iter().any(|l| l.contains("Close") && !l.contains("Split") || l.contains("Group")));
        let tile = labels(&items(&tab(TabKind::Tile), all));
        assert!(tile.contains(&"(Pin Tab)".to_owned()) && tile.contains(&"(Rename)".to_owned()) && tile.contains(&"Remove from Tiles".to_owned()));
    }

    #[test]
    fn a_burner_tab_pins_nothing_and_a_split_member_can_leave() {
        let burner = labels(&items(&Tab { burner: true, ..tab(TabKind::Ephemeral) }, all));
        assert!(burner.contains(&"(Pin Tab)".to_owned()) && burner.contains(&"(Add to Tiles)".to_owned()));
        let member = Tab { split: split::Menu { entry: split::Entry::Add, enabled: false, member: true }, ..tab(TabKind::Ephemeral) };
        assert!(labels(&items(&member, all)).contains(&"  Remove from Split View".to_owned()));
        let narrow = labels(&items(&Tab { split_view: false, ..member }, all));
        assert!(!narrow.iter().any(|l| l.contains("Split")));
    }

    #[test]
    fn a_link_less_tab_has_nothing_to_copy_or_duplicate() {
        let empty = Tab { active: false, has_link: false, grouped: true, groups: vec![(7, "Mail".into())], muted: None, ..tab(TabKind::Ephemeral) };
        let empty = labels(&items(&empty, all));
        assert_eq!(empty[..6], ["Open", "(Duplicate)", "-", "(Copy Link)", "(Copy Clean Link)", "(Bookmark Tab…)"]);
        assert!(empty.windows(4).any(|w| w == ["# Add to Group", "  Mail", "  -", "  New Group"]));
        assert!(empty.contains(&"Remove from Group".to_owned()) && empty.contains(&"(Mute Site)".to_owned()));
    }

    #[test]
    fn the_engine_switch_names_the_other_engine_and_sound_follows_the_state() {
        let on_chromium =
            labels(&items(&Tab { engine: Some((WebEngine::Chromium, true)), muted: Some(true), sleepable: true, ..tab(TabKind::Ephemeral) }, all));
        assert_eq!(on_chromium[2], "Reload in WebKit");
        assert!(on_chromium.contains(&"Unmute Site".to_owned()) && on_chromium.contains(&"Sleep Tab".to_owned()));
        let stuck = labels(&items(&Tab { engine: Some((WebEngine::WebKit, false)), ..tab(TabKind::Ephemeral) }, all));
        assert_eq!(stuck[2], "(Reload in Chromium)");
    }

    #[test]
    fn a_client_drops_what_it_does_not_offer_without_stray_separators() {
        let few = |c: Command| matches!(c, Command::Reload | Command::Close | Command::CloseOthers | Command::CloseBelow);
        assert_eq!(labels(&items(&tab(TabKind::Ephemeral), few)), ["Reload", "-", "Close Tab", "Close Other Tabs", "(Close Tabs Below)"]);
        let no_groups = |c: Command| c != Command::NewGroup;
        assert!(!labels(&items(&Tab { groups: vec![(7, "Mail".into())], ..tab(TabKind::Ephemeral) }, no_groups)).iter().any(|l| l.contains("Group")));
    }
}
