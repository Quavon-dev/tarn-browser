use crate::command::rank_commands;

/// Command rows the field lists for plain text; a `>` list shows every match.
pub const PLAIN_ROOM: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    ReopenClosedTab,
    TogglePin,
    CloseDuplicates,
    ZapElement,
    Reader,
    SaveForLater,
    BurnerTab,
    FindInTabs,
    CopyLink,
    CopyCleanLink,
    ShowQrCode,
    SaveSnapshot,
    Snapshots,
    Memory,
    ReadingList,
    NewWindow,
    ToggleSplitView,
    FocusNextPane,
    FocusPreviousPane,
    SwapPanes,
    RotateSplitLayout,
    RemoveFromSplitView,
    CloseSplit,
}

pub const COMMANDS: [(&str, &str, Command); 23] = [
    ("Reopen Closed Tab", "undo restore", Command::ReopenClosedTab),
    ("Pin or Unpin Tab", "", Command::TogglePin),
    ("Close Duplicate Tabs", "same page dedupe clean up", Command::CloseDuplicates),
    ("Zap Element", "hide remove element block", Command::ZapElement),
    ("Reader", "article reading view text", Command::Reader),
    ("Save for Later", "reading list read later bookmark", Command::SaveForLater),
    ("Burner Tab", "throwaway incognito temporary private", Command::BurnerTab),
    ("Find in Tabs", "search text all pages", Command::FindInTabs),
    ("Copy Link", "url address", Command::CopyLink),
    ("Copy Clean Link", "url tracking parameters utm strip", Command::CopyCleanLink),
    ("Show QR Code", "phone scan link", Command::ShowQrCode),
    ("Save Space as Snapshot", "session tabs save", Command::SaveSnapshot),
    ("Restore Snapshot…", "session tabs open", Command::Snapshots),
    ("Memory", "process cpu ram usage sleep", Command::Memory),
    ("Reading List", "read later saved pages", Command::ReadingList),
    ("New Window", "open another browser window", Command::NewWindow),
    ("Open or Close Split View", "side by side tabs grid four", Command::ToggleSplitView),
    ("Focus Next Pane", "split view switch", Command::FocusNextPane),
    ("Focus Previous Pane", "split view switch", Command::FocusPreviousPane),
    ("Swap Split Panes", "split view exchange move", Command::SwapPanes),
    ("Rotate Split Layout", "split view stacked side by side rows columns", Command::RotateSplitLayout),
    ("Remove Tab from Split View", "split pane leave", Command::RemoveFromSplitView),
    ("Close Split View", "split panes end separate", Command::CloseSplit),
];

/// What the active tab offers, which decides the rows shown and how toggles are named.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub pinned: bool,
    pub pin_blocked: bool,
    pub split: bool,
    pub reopenable: bool,
    pub duplicates: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Command(Command),
    Extra(usize),
}

/// Whether `query` asks for the command list: every command it names, and nothing else.
pub fn listing(query: &str) -> bool {
    query.trim_start().starts_with('>')
}

impl State {
    fn shows(self, command: Command) -> bool {
        match command {
            Command::CloseDuplicates => self.duplicates,
            Command::ReopenClosedTab => self.reopenable,
            Command::TogglePin => !self.pin_blocked,
            Command::CloseSplit
            | Command::FocusNextPane
            | Command::FocusPreviousPane
            | Command::SwapPanes
            | Command::RotateSplitLayout
            | Command::RemoveFromSplitView => self.split,
            _ => true,
        }
    }

    fn name(self, label: &str, command: Command) -> String {
        match command {
            Command::TogglePin => (if self.pinned { "Unpin Tab" } else { "Pin Tab" }).to_owned(),
            Command::ToggleSplitView => (if self.split { "Close Split View" } else { "Open in Split View" }).to_owned(),
            _ => label.to_owned(),
        }
    }
}

/// The rows `query` names, best first: the client's `offered` commands that have something to act
/// on, then its `extra` `(label, keywords)` rows, named by what a toggle does now.
pub fn rows(query: &str, state: State, offered: impl Fn(Command) -> bool, extra: &[(String, &str)]) -> Vec<(String, Row)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut all: Vec<(String, &str, &str, Row)> =
        COMMANDS.iter().filter(|(_, _, c)| offered(*c) && state.shows(*c)).map(|(l, k, c)| (state.name(l, *c), *l, *k, Row::Command(*c))).collect();
    all.extend(extra.iter().enumerate().map(|(ix, (l, k))| (l.clone(), l.as_str(), *k, Row::Extra(ix))));
    let ranked = rank_commands(query, all.iter().map(|(_, l, k, _)| (*l, *k)));
    let room = if listing(query) { all.len() } else { PLAIN_ROOM };
    ranked.into_iter().take(room).map(|ix| (all[ix].0.clone(), all[ix].3)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(query: &str, state: State) -> Vec<String> {
        rows(query, state, |_| true, &[]).into_iter().map(|(n, _)| n).collect()
    }

    #[test]
    fn an_empty_query_lists_nothing() {
        assert!(names("", State::default()).is_empty());
        assert!(names("   ", State::default()).is_empty());
    }

    #[test]
    fn plain_text_lists_at_most_four_commands() {
        assert!(names("split", State { split: true, ..State::default() }).len() <= PLAIN_ROOM);
        assert!(names(">", State { split: true, ..State::default() }).len() > PLAIN_ROOM);
    }

    #[test]
    fn toggles_are_named_by_what_they_do_now() {
        assert_eq!(names("pin tab", State::default()), ["Pin Tab"]);
        assert_eq!(names("pin tab", State { pinned: true, ..State::default() }), ["Unpin Tab"]);
        assert!(names("pin tab", State { pin_blocked: true, ..State::default() }).is_empty(), "a tile or a burner tab cannot be pinned");
        assert!(names("split view", State::default()).contains(&"Open in Split View".to_owned()));
        assert!(names("split view", State { split: true, ..State::default() }).contains(&"Close Split View".to_owned()));
    }

    #[test]
    fn commands_without_anything_to_act_on_are_left_out() {
        assert!(names("reopen", State::default()).is_empty());
        assert_eq!(names("reopen", State { reopenable: true, ..State::default() }), ["Reopen Closed Tab"]);
        assert!(names("swap panes", State::default()).is_empty());
        assert!(names("close duplicate", State::default()).is_empty());
    }

    #[test]
    fn a_client_lists_only_what_it_offers_and_its_extra_rows() {
        let extra = [("Switch to Work".to_owned(), "space workspace")];
        let found = rows(">", State::default(), |c| c == Command::Reader, &extra);
        assert_eq!(found, [("Reader".to_owned(), Row::Command(Command::Reader)), ("Switch to Work".to_owned(), Row::Extra(0))]);
    }

    #[test]
    fn listing_needs_a_leading_angle_bracket() {
        assert!(listing("  > copy"));
        assert!(!listing("copy >"));
    }
}
