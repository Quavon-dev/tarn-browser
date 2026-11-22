use crate::command::rank_commands;

pub mod calc;
pub mod emoji;
pub mod units;

pub const PLAIN_ROOM: usize = 4;

/// A row the command bar answers itself, offline: a sum, a unit conversion or an emoji. Enter
/// copies `copy`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub label: String,
    pub copy: String,
}

pub const ANSWER_DETAIL: &str = "Copy";

/// The answer rows for `query`, first in the list: a result of `calc` or `units`, else the emoji
/// `emoji::search` finds.
pub fn answers(query: &str) -> Vec<Answer> {
    let value = calc::evaluate(query).map(|v| (calc::format(v), String::new())).or_else(|| units::convert(query).map(|(v, unit)| (calc::format(v), unit)));
    if let Some((number, unit)) = value {
        let label = if unit.is_empty() { format!("= {number}") } else { format!("= {number} {unit}") };
        return vec![Answer { label, copy: number }];
    }
    emoji::search(query).into_iter().map(|(emoji, name)| Answer { label: format!("{emoji}  {name}"), copy: emoji.to_owned() }).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    Settings,
    OpenFile,
    NewGroup,
    BookmarkTab,
    BookmarkAllTabs,
    ManageBookmarks,
    NewSpace,
    EditSpace,
    PrivateSpace,
    BurnerTab,
    NewProfile,
    EditProfile,
    ReopenClosedTab,
    ToastAction,
    ToggleSidebar,
    AutoHideSidebar,
    CompactToolbar,
    CloseAllTabs,
    ShowLibrary,
    SaveSnapshot,
    Snapshots,
    ShowDownloads,
    DuplicateTab,
    ReloadInChromium,
    ReloadInWebKit,
    CopyLink,
    CopyCleanLink,
    TogglePin,
    ToggleMute,
    ShowHistory,
    RenameTab,
    ResetTitle,
    ReturnToPinned,
    SetPinnedUrl,
    SleepOthers,
    Memory,
    CloseDuplicates,
    SiteSettings,
    ZapElement,
    ToggleSplitView,
    FocusNextPane,
    FocusPreviousPane,
    SwapPanes,
    RotateSplitLayout,
    RemoveFromSplitView,
    CloseSplit,
    Reader,
    SaveForLater,
    ReadingList,
    Archived,
    PictureInPicture,
    TakeScreenshot,
    SavePdf,
    CopyMarkdownLink,
    CopyTitle,
    Share,
    ShowQrCode,
    FindInTabs,
    ImportBrowserData,
    NewWindow,
    FullScreen,
    StopLoading,
    ReloadFromOrigin,
    CloseWindow,
    KeyboardShortcuts,
    Ask,
    AskPage,
}

pub const COMMANDS: [(&str, &str, Command); 67] = [
    ("Settings", "preferences options", Command::Settings),
    ("Open File\u{2026}", "local disk html pdf image text document view", Command::OpenFile),
    ("New Tab Group", "group", Command::NewGroup),
    ("Bookmark Tab\u{2026}", "favorite save star", Command::BookmarkTab),
    ("Bookmark All Tabs", "favorite save folder", Command::BookmarkAllTabs),
    ("Manage Bookmarks", "favorites library folders edit", Command::ManageBookmarks),
    ("New Space", "workspace profile", Command::NewSpace),
    ("Edit Space", "workspace rename color icon", Command::EditSpace),
    ("Private Space", "incognito", Command::PrivateSpace),
    ("Burner Tab", "throwaway incognito temporary private", Command::BurnerTab),
    ("New Profile", "account", Command::NewProfile),
    ("Edit Profile", "account", Command::EditProfile),
    ("Reopen Closed Tab", "undo restore", Command::ReopenClosedTab),
    ("Notification Action", "toast undo reveal restart", Command::ToastAction),
    ("Toggle Sidebar", "show hide", Command::ToggleSidebar),
    ("Auto-Hide Sidebar", "floating", Command::AutoHideSidebar),
    ("Compact Toolbar", "toggle address", Command::CompactToolbar),
    ("Close All Tabs", "clear", Command::CloseAllTabs),
    ("Show Library", "bookmarks history", Command::ShowLibrary),
    ("Save Space as Snapshot", "session tabs save", Command::SaveSnapshot),
    ("Restore Snapshot\u{2026}", "session tabs open", Command::Snapshots),
    ("Show Downloads", "files", Command::ShowDownloads),
    ("Duplicate Tab", "copy", Command::DuplicateTab),
    ("Reload in Chromium", "engine chrome switch", Command::ReloadInChromium),
    ("Reload in WebKit", "engine safari switch", Command::ReloadInWebKit),
    ("Copy Link", "url address", Command::CopyLink),
    ("Copy Clean Link", "url tracking parameters utm strip", Command::CopyCleanLink),
    ("Pin or Unpin Tab", "", Command::TogglePin),
    ("Mute or Unmute Site", "sound audio silence", Command::ToggleMute),
    ("Show History", "visited", Command::ShowHistory),
    ("Rename Tab", "title name", Command::RenameTab),
    ("Reset Tab Title", "rename name", Command::ResetTitle),
    ("Return to Pinned URL", "home back pinned", Command::ReturnToPinned),
    ("Set Pinned URL to Current Page", "pinned home", Command::SetPinnedUrl),
    ("Sleep Other Tabs", "hibernate unload memory", Command::SleepOthers),
    ("Show Memory Use", "memory ram cpu energy process task manager usage sleep", Command::Memory),
    ("Close Duplicate Tabs", "same page dedupe clean up", Command::CloseDuplicates),
    ("Site Settings", "shield permissions zoom mute clear data", Command::SiteSettings),
    ("Zap Element", "hide remove element block", Command::ZapElement),
    ("Open in Split View", "open or close side by side tabs grid four", Command::ToggleSplitView),
    ("Focus Next Pane", "split view switch", Command::FocusNextPane),
    ("Focus Previous Pane", "split view switch", Command::FocusPreviousPane),
    ("Swap Split Panes", "split view exchange move", Command::SwapPanes),
    ("Rotate Split Layout", "split view stacked side by side rows columns", Command::RotateSplitLayout),
    ("Remove Tab from Split View", "split pane leave", Command::RemoveFromSplitView),
    ("Close Split View", "split panes end separate", Command::CloseSplit),
    ("Reader", "article reading view text", Command::Reader),
    ("Save for Later", "reading list read later bookmark", Command::SaveForLater),
    ("Reading List", "read later saved pages", Command::ReadingList),
    ("Archived Tabs", "unused closed old bookmarks", Command::Archived),
    ("Picture in Picture", "video pip float", Command::PictureInPicture),
    ("Take Screenshot", "capture image png", Command::TakeScreenshot),
    ("Save Page as PDF", "export print", Command::SavePdf),
    ("Copy Link as Markdown", "url markdown", Command::CopyMarkdownLink),
    ("Copy Title", "page name", Command::CopyTitle),
    ("Share", "send airdrop mail messages link", Command::Share),
    ("Show QR Code", "phone scan link", Command::ShowQrCode),
    ("Find in Tabs", "search text all pages", Command::FindInTabs),
    ("Import Browser Data\u{2026}", "chrome edge brave chromium firefox safari history bookmarks cookies", Command::ImportBrowserData),
    ("New Window", "open another browser window", Command::NewWindow),
    ("Enter or Exit Full Screen", "fullscreen window video f11", Command::FullScreen),
    ("Stop Loading", "cancel halt", Command::StopLoading),
    ("Reload Without Cache", "refresh hard force", Command::ReloadFromOrigin),
    ("Close Window", "quit", Command::CloseWindow),
    ("Keyboard Shortcuts", "help keys cheat sheet", Command::KeyboardShortcuts),
    ("Ask Assistant", "ai chat question help", Command::Ask),
    ("Ask About This Page", "ai chat summarize explain page", Command::AskPage),
];

impl Command {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        COMMANDS[self.index()].0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub pinned: bool,
    pub pin_blocked: bool,
    pub split: bool,
    pub reopenable: bool,
    pub duplicates: bool,
    pub no_tab: bool,
    pub no_page: bool,
    pub private: bool,
    pub muted: Option<bool>,
    pub reader: Option<bool>,
    pub custom_title: bool,
    pub renamable: bool,
    pub drifted: bool,
    pub sleep_others: bool,
    pub unpinned: bool,
    pub loading: bool,
    pub toast: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Command(Command),
    Extra(usize),
}

pub fn listing(query: &str) -> bool {
    query.trim_start().starts_with('>')
}

impl State {
    fn shows(&self, command: Command) -> bool {
        use Command::*;
        let tab = !self.no_tab;
        match command {
            CloseDuplicates => self.duplicates,
            ReopenClosedTab => self.reopenable,
            TogglePin => tab && !self.pin_blocked,
            ToggleSplitView => tab && !self.split,
            CloseSplit | FocusNextPane | FocusPreviousPane | SwapPanes | RotateSplitLayout | RemoveFromSplitView => self.split,
            SaveSnapshot | Snapshots => !self.private,
            ToggleMute => tab && self.muted.is_some(),
            ResetTitle => tab && self.custom_title,
            RenameTab => tab && self.renamable,
            ReturnToPinned | SetPinnedUrl => tab && self.drifted,
            SleepOthers => tab && self.sleep_others,
            CloseAllTabs => self.unpinned,
            StopLoading => tab && self.loading,
            ToastAction => self.toast.is_some(),
            SiteSettings | ReloadFromOrigin => tab && !self.no_page,
            ReloadInChromium | ReloadInWebKit => tab,
            Reader | SaveForLater | CopyLink | CopyCleanLink | CopyMarkdownLink | CopyTitle | ShowQrCode | Share | SavePdf | ZapElement | BookmarkTab
            | TakeScreenshot | DuplicateTab | PictureInPicture => tab,
            _ => true,
        }
    }

    fn name(&self, command: Command) -> String {
        let pick = |on: bool, yes: &str, no: &str| (if on { yes } else { no }).to_owned();
        match command {
            Command::TogglePin => pick(self.pinned, "Unpin Tab", "Pin Tab"),
            Command::ToggleMute => pick(self.muted == Some(true), "Unmute Site", "Mute Site"),
            Command::Reader if self.reader.is_some() => pick(self.reader == Some(true), "Hide Reader", "Show Reader"),
            Command::ToastAction => self.toast.clone().unwrap_or_default(),
            _ => command.label().to_owned(),
        }
    }
}

pub fn ranked(query: &str, state: &State, offered: impl Fn(Command) -> bool, extra: &[(String, &str)]) -> Vec<(String, Row)> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut all: Vec<(String, &str, &str, Row)> =
        COMMANDS.iter().filter(|(_, _, c)| state.shows(*c) && offered(*c)).map(|(l, k, c)| (state.name(*c), *l, *k, Row::Command(*c))).collect();
    all.extend(extra.iter().enumerate().map(|(ix, (l, k))| (l.clone(), l.as_str(), *k, Row::Extra(ix))));
    let ranked = rank_commands(query, all.iter().map(|(_, l, k, _)| (*l, *k)));
    ranked.into_iter().map(|ix| (all[ix].0.clone(), all[ix].3)).collect()
}

pub fn rows(query: &str, state: State, offered: impl Fn(Command) -> bool, extra: &[(String, &str)]) -> Vec<(String, Row)> {
    let room = if listing(query) { usize::MAX } else { PLAIN_ROOM };
    ranked(query, &state, offered, extra).into_iter().take(room).collect()
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
    fn the_split_command_is_found_by_the_name_it_shows() {
        assert_eq!(names("open in split view", State::default()), ["Open in Split View"]);
        assert!(names("close split", State::default()).contains(&"Open in Split View".to_owned()), "the old wording still finds it");
        assert!(names("open or close split view", State::default()).contains(&"Open in Split View".to_owned()), "the old name finds it whole");
    }

    #[test]
    fn an_open_split_lists_close_split_view_once() {
        let listed = names("> split", State { split: true, ..State::default() });
        assert_eq!(listed.iter().filter(|n| *n == "Close Split View").count(), 1, "{listed:?}");
        assert!(names("> split", State::default()).contains(&"Open in Split View".to_owned()));
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
    fn commands_are_listed_in_their_enum_order() {
        for (ix, (_, _, c)) in COMMANDS.iter().enumerate() {
            assert_eq!(c.index(), ix, "{c:?}");
        }
        assert_eq!(crate::command::glyph("Ask Assistant"), "sparkles");
        assert_eq!(crate::command::glyph("Ask About This Page"), "sparkles");
    }

    #[test]
    fn tab_commands_need_a_tab_and_page_commands_a_page() {
        let none = State { no_tab: true, ..State::default() };
        assert!(names("copy link", none.clone()).is_empty());
        assert!(names("site settings", State { no_page: true, ..State::default() }).is_empty());
        assert_eq!(names("site settings", State::default()), ["Site Settings"]);
        assert_eq!(
            names("reload in webkit", State { no_page: true, ..State::default() }),
            ["Reload in WebKit"],
            "a file or blank page can still switch engines"
        );
    }

    #[test]
    fn a_private_window_offers_no_snapshots() {
        assert!(names("> snapshot", State { private: true, ..State::default() }).is_empty());
        assert!(!names("> snapshot", State::default()).is_empty());
    }

    #[test]
    fn mute_reader_and_notice_are_named_by_their_state() {
        assert!(!names("mute site", State::default()).iter().any(|n| n.contains("Mute")));
        assert_eq!(names("mute site", State { muted: Some(true), ..State::default() })[0], "Unmute Site");
        assert_eq!(names("> reader", State { reader: Some(true), ..State::default() })[0], "Hide Reader");
        assert_eq!(names("> reader", State::default())[0], "Reader");
        let toast = State { toast: Some("Undo \u{2014} Tab closed".into()), ..State::default() };
        assert_eq!(names("notification action", toast), ["Undo \u{2014} Tab closed"]);
        assert!(names("notification action", State::default()).is_empty());
    }

    #[test]
    fn answers_copy_a_result_a_conversion_or_an_emoji() {
        assert_eq!(answers("6 * 7"), [Answer { label: "= 42".into(), copy: "42".into() }]);
        assert_eq!(answers("2 km in m")[0].label, "= 2000 m");
        assert_eq!(answers("2 km in m")[0].copy, "2000");
        assert_eq!(answers(":taco")[0].copy, "\u{1F32E}");
        assert!(answers("42").is_empty() && answers("rust language").is_empty() && answers("> copy").is_empty());
    }

    #[test]
    fn listing_needs_a_leading_angle_bracket() {
        assert!(listing("  > copy"));
        assert!(!listing("copy >"));
    }
}
