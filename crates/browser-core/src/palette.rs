use std::collections::HashSet;
use std::time::Duration;

use store::{Bookmark, HistoryEntry, Id, Tab, TabKind};

use crate::command::{Section, display_title, rank_commands};
use crate::suggest::frecency;

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
/// The longest answer label, in characters.
pub const MAX_ANSWER_LABEL: usize = 64;

/// The answer rows for `query`, first in the list: a result of `calc` or `units`, else the emoji
/// `emoji::search` finds. Labels are cut to `MAX_ANSWER_LABEL`.
pub fn answers(query: &str) -> Vec<Answer> {
    let value = calc::evaluate(query).map(|v| (calc::format(v), String::new())).or_else(|| units::convert(query).map(|(v, unit)| (calc::format(v), unit)));
    let answer = |label: String, copy: String| {
        let label = if label.chars().count() > MAX_ANSWER_LABEL { label.chars().take(MAX_ANSWER_LABEL - 1).collect::<String>() + "…" } else { label };
        Answer { label, copy }
    };
    if let Some((number, unit)) = value {
        let label = if unit.is_empty() { format!("= {number}") } else { format!("= {number} {unit}") };
        return vec![answer(label, number)];
    }
    emoji::search(query).into_iter().map(|(emoji, name)| answer(format!("{emoji}  {name}"), emoji.to_owned())).collect()
}

/// The row Enter takes before the user picks one, of `rows` listed with `answers` answer rows
/// first: the first row after them, so Enter runs what was typed and an answer is copied only
/// once picked; `None` when only answers are listed.
pub fn default_row(answers: usize, rows: usize) -> Option<usize> {
    (answers < rows).then_some(answers)
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
    TabsOnTop,
    AutoHideSidebar,
    CompactToolbar,
    SidebarToolbar,
    SidebarChats,
    GlassDesign,
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
    CaptureArea,
    CaptureElement,
    CaptureFullPage,
    Measure,
    ToggleInspector,
    MoveInspector,
    InsertSnippet,
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

pub const COMMANDS: [(&str, &str, Command); 78] = [
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
    ("Tabs on Top", "layout strip horizontal sidebar", Command::TabsOnTop),
    ("Auto-Hide Sidebar", "floating", Command::AutoHideSidebar),
    ("Compact Toolbar", "toggle address", Command::CompactToolbar),
    ("Toolbar in Sidebar", "address navigation back forward layout", Command::SidebarToolbar),
    ("Chats in Sidebar", "assistant ai conversations show hide", Command::SidebarChats),
    ("Glass Design", "liquid classic translucent transparent blur appearance look", Command::GlassDesign),
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
    ("Take Screenshot", "capture image png visible", Command::TakeScreenshot),
    ("Capture Area", "screenshot region rectangle crop image png", Command::CaptureArea),
    ("Capture Element", "screenshot pick part image png", Command::CaptureElement),
    ("Capture Full Page", "screenshot whole long scrolling image png", Command::CaptureFullPage),
    ("Measure", "ruler pixels size distance developer", Command::Measure),
    ("Show or Hide Inspector", "devtools developer tools web inspector console elements", Command::ToggleInspector),
    ("Move Inspector to Other Side", "devtools developer tools dock left right swap", Command::MoveInspector),
    ("Insert Snippet\u{2026}", "text expand template paste type", Command::InsertSnippet),
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
    /// `assistant::allowed` for the active page and the profile's AI switch both say yes.
    pub assistant: bool,
    pub muted: Option<bool>,
    pub reader: Option<bool>,
    pub custom_title: bool,
    pub renamable: bool,
    pub drifted: bool,
    pub sleep_others: bool,
    pub unpinned: bool,
    pub loading: bool,
    pub toast: Option<String>,
    pub developer: bool,
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
            Ask => self.assistant,
            AskPage => tab && !self.no_page && self.assistant,
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
            CaptureArea | CaptureElement | CaptureFullPage | InsertSnippet => tab && !self.no_page,
            Measure | ToggleInspector | MoveInspector => tab && !self.no_page && self.developer,
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

/// Whether the command bar shows `sections` instead of ranked rows: nothing typed yet.
pub fn sectioned(query: &str) -> bool {
    query.trim().is_empty()
}

/// A row of the command bar that has an actions page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Command(Command),
    Tab(Id),
    Bookmark(String),
    History(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub item: Item,
    pub label: String,
    pub detail: String,
}

/// What the empty command bar lists from: the current space's tabs, the profile's bookmarks and
/// history.
pub struct Data<'a> {
    pub tabs: &'a [Tab],
    pub active: Option<Id>,
    pub bookmarks: &'a [Bookmark],
    pub history: &'a [HistoryEntry],
    pub now: i64,
}

pub const SECTION_ROWS: usize = 5;
const NOW: [Command; 6] = [Command::CopyLink, Command::BookmarkTab, Command::TogglePin, Command::Reader, Command::ToggleSplitView, Command::Share];
const NOW_ROWS: usize = 4;
const QUICK: [Command; 7] =
    [Command::ReopenClosedTab, Command::ShowHistory, Command::ShowDownloads, Command::ManageBookmarks, Command::NewWindow, Command::Ask, Command::Settings];

fn page_detail(url: &str) -> String {
    crate::text::bounded_url(url.trim_start_matches("https://").trim_end_matches('/'))
}

/// The sections the empty command bar lists, each non-empty and in order: Now (commands for the
/// current tab), Favorites (pinned tabs, then top-level bookmarks), Recent Tabs (the most recently used other
/// tabs), Frequent (history by frecency) and Quick Actions. A private space lists no Recent Tabs
/// and no Frequent rows. A page shows once, in its first section.
pub fn sections(state: &State, offered: impl Fn(Command) -> bool, data: &Data) -> Vec<(Section, Vec<Entry>)> {
    let command = |c: Command| Entry { item: Item::Command(c), label: state.name(c), detail: String::new() };
    let usable = |c: &&Command| state.shows(**c) && offered(**c);
    let now: Vec<Entry> = NOW.iter().filter(usable).take(NOW_ROWS).map(|c| command(*c)).collect();
    let tab = |t: &Tab| Entry { item: Item::Tab(t.id), label: display_title(&t.custom_title, &t.title, &t.url), detail: "Switch to Tab".to_owned() };
    let mut shown: HashSet<&str> = HashSet::new();
    let mut favorites: Vec<Entry> = Vec::new();
    for t in data.tabs.iter().filter(|t| t.kind != TabKind::Ephemeral && Some(t.id) != data.active) {
        shown.insert(&t.url);
        favorites.push(tab(t));
    }
    for b in data.bookmarks.iter().filter(|b| b.parent_id.is_none()) {
        let Some(url) = b.url.as_deref().filter(|u| shown.insert(u)) else { continue };
        let label = display_title("", &b.title, url);
        favorites.push(Entry { item: Item::Bookmark(url.to_owned()), label, detail: page_detail(url) });
    }
    favorites.truncate(SECTION_ROWS);
    let (mut recent, mut frequent) = (Vec::new(), Vec::new());
    if !state.private {
        let mut tabs: Vec<&Tab> = data.tabs.iter().filter(|t| t.kind == TabKind::Ephemeral && Some(t.id) != data.active).collect();
        tabs.sort_by_key(|t| std::cmp::Reverse(t.last_active));
        recent = tabs.into_iter().take(SECTION_ROWS).map(tab).collect();
        shown.extend(data.tabs.iter().map(|t| t.url.as_str()));
        let mut pages: Vec<&HistoryEntry> = data.history.iter().filter(|h| !shown.contains(h.url.as_str())).collect();
        pages.sort_by_key(|h| (std::cmp::Reverse(frecency(h.visits, h.visited_at, data.now)), std::cmp::Reverse(h.visited_at)));
        let mut seen = HashSet::new();
        frequent = pages
            .into_iter()
            .filter(|h| seen.insert(h.url.as_str()))
            .take(SECTION_ROWS)
            .map(|h| Entry { item: Item::History(h.url.clone()), label: display_title("", &h.title, &h.url), detail: page_detail(&h.url) })
            .collect();
    }
    let ran: Vec<Command> = now.iter().filter_map(|e| if let Item::Command(c) = e.item { Some(c) } else { None }).collect();
    let quick: Vec<Entry> = QUICK.iter().filter(usable).filter(|c| !ran.contains(c)).take(SECTION_ROWS).map(|c| command(*c)).collect();
    [(Section::Now, now), (Section::Favorites, favorites), (Section::RecentTabs, recent), (Section::Frequent, frequent), (Section::QuickActions, quick)]
        .into_iter()
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

/// What a row's actions page offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Run,
    Switch,
    Close,
    CopyLink,
    /// Moves the tab to the client's `ix`-th move target.
    MoveToSpace(usize),
    Open,
    OpenInNewTab,
    RemoveFromHistory,
}

/// The actions page of `item`, named; `spaces` names the spaces a tab can move to, in the client's
/// order. A command's page holds Run, beside which the client shows its shortcut.
pub fn actions(item: &Item, spaces: &[&str]) -> Vec<(String, Action)> {
    let named = |list: &[(&str, Action)]| list.iter().map(|(l, a)| ((*l).to_owned(), *a)).collect::<Vec<_>>();
    match item {
        Item::Command(_) => named(&[("Run", Action::Run)]),
        Item::Tab(_) => {
            let mut all = named(&[("Switch to Tab", Action::Switch), ("Close Tab", Action::Close), ("Copy Link", Action::CopyLink)]);
            all.extend(spaces.iter().enumerate().map(|(ix, name)| (format!("Move to {}", crate::text::bounded_title(name)), Action::MoveToSpace(ix))));
            all
        }
        Item::Bookmark(_) => named(&[("Open", Action::Open), ("Open in New Tab", Action::OpenInNewTab), ("Copy Link", Action::CopyLink)]),
        Item::History(_) => named(&[
            ("Open", Action::Open),
            ("Open in New Tab", Action::OpenInNewTab),
            ("Copy Link", Action::CopyLink),
            ("Remove from History", Action::RemoveFromHistory),
        ]),
    }
}

/// How long the address field shakes when Enter meets an address Tarn cannot open.
pub const SHAKE: Duration = Duration::from_millis(360);
/// What the field says instead of shaking when motion is reduced.
pub const INVALID_ADDRESS: &str = "Can\u{2019}t open this address";

/// The field's horizontal offset in points at progress `t` (0 to 1) of the shake: three decaying
/// swings that end at rest.
pub fn shake(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    6. * (1. - t) * (t * 3. * std::f32::consts::TAU).sin()
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
        assert_eq!(names("capture area", State::default())[0], "Capture Area");
        assert!(!names("measure", State::default()).contains(&"Measure".to_owned()), "only with the developer tools on");
        assert_eq!(names("measure", State { developer: true, ..State::default() })[0], "Measure");
        assert!(names("inspector", State::default()).is_empty(), "the inspector only with the developer tools on");
        assert_eq!(names("show inspector", State { developer: true, ..State::default() })[0], "Show or Hide Inspector");
        assert!(
            !names("capture", State { no_page: true, ..State::default() }).iter().any(|n| n.starts_with("Capture")),
            "a tab without a page has nothing to capture"
        );
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
    fn the_assistant_is_offered_only_where_it_is_allowed_and_about_a_page_only_with_one() {
        let on = State { assistant: true, ..State::default() };
        assert_eq!(names("ask a", on.clone()), ["Ask Assistant", "Ask About This Page"]);
        assert!(names("ask a", State::default()).is_empty(), "private, burner, Lockdown or AI off");
        for state in [State { no_page: true, ..on.clone() }, State { no_tab: true, ..on }] {
            assert_eq!(names("ask a", state), ["Ask Assistant"]);
        }
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
        assert!(answers("2024-01-15").is_empty() && answers("24/7").is_empty() && answers("-5").is_empty());
        assert!(answers("9 ^ 9 ^ 9 / 7").iter().all(|a| a.label.chars().count() <= MAX_ANSWER_LABEL));
        assert_eq!(answers("2 ^ 64")[0].label, "= 1.844674407e19");
    }

    #[test]
    fn enter_runs_the_typed_row_and_never_an_unpicked_answer() {
        assert_eq!(default_row(1, 4), Some(1));
        assert_eq!(default_row(0, 3), Some(0));
        assert_eq!(default_row(2, 2), None);
    }

    fn tab(id: Id, kind: TabKind, url: &str, last_active: i64) -> Tab {
        Tab {
            id,
            space_id: 1,
            folder_id: None,
            kind,
            url: url.into(),
            title: format!("T{id}"),
            position: id,
            last_active,
            group_id: None,
            pinned_url: None,
            custom_title: String::new(),
            web_engine: None,
        }
    }

    fn bookmark(url: Option<&str>, title: &str) -> Bookmark {
        Bookmark { id: 1, profile_id: 1, parent_id: None, url: url.map(Into::into), title: title.into(), position: 0, created_at: 0 }
    }

    fn visit(url: &str, visits: i64, visited_at: i64) -> HistoryEntry {
        HistoryEntry { url: url.into(), title: String::new(), visited_at, visits, typed: 0 }
    }

    #[test]
    fn the_empty_bar_lists_its_sections_in_order_without_repeating_a_page() {
        let tabs = [
            tab(1, TabKind::Pinned, "https://mail.test/", 0),
            tab(2, TabKind::Ephemeral, "https://a.test/", 10),
            tab(3, TabKind::Ephemeral, "https://b.test/", 30),
            tab(4, TabKind::Ephemeral, "https://c.test/", 20),
        ];
        let bookmarks = [bookmark(Some("https://mail.test/"), ""), bookmark(None, "Folder"), bookmark(Some("https://docs.test/"), "Docs")];
        let now = 100 * 86_400_000;
        let history = [
            visit("https://a.test/", 50, now),
            visit("https://old.test/", 3, now - 200 * 86_400_000),
            visit("https://new.test/", 3, now),
            visit("https://docs.test/", 9, now),
        ];
        let data = Data { tabs: &tabs, active: Some(4), bookmarks: &bookmarks, history: &history, now };
        let found = sections(&State { reopenable: true, ..State::default() }, |_| true, &data);
        let heads: Vec<Section> = found.iter().map(|(s, _)| *s).collect();
        assert_eq!(heads, [Section::Now, Section::Favorites, Section::RecentTabs, Section::Frequent, Section::QuickActions]);
        let items = |ix: usize| found[ix].1.iter().map(|e| e.item.clone()).collect::<Vec<_>>();
        assert_eq!(found[0].1.len(), NOW_ROWS);
        assert_eq!(items(1), [Item::Tab(1), Item::Bookmark("https://docs.test/".into())]);
        assert_eq!(found[1].1[1].label, "Docs");
        assert_eq!(items(2), [Item::Tab(3), Item::Tab(2)], "most recent first, without the active tab");
        assert_eq!(items(3), [Item::History("https://new.test/".into()), Item::History("https://old.test/".into())]);
        assert_eq!(found[4].1[0].item, Item::Command(Command::ReopenClosedTab));
        assert!(found[4].1.len() <= SECTION_ROWS);
    }

    #[test]
    fn a_private_space_lists_no_recent_tabs_or_frequent_pages() {
        let tabs = [tab(1, TabKind::Ephemeral, "https://a.test/", 1), tab(2, TabKind::Ephemeral, "https://b.test/", 2)];
        let history = [visit("https://c.test/", 4, 0)];
        let data = Data { tabs: &tabs, active: Some(1), bookmarks: &[], history: &history, now: 0 };
        let heads: Vec<Section> = sections(&State { private: true, ..State::default() }, |_| true, &data).into_iter().map(|(s, _)| s).collect();
        assert_eq!(heads, [Section::Now, Section::QuickActions]);
    }

    #[test]
    fn sections_list_only_what_the_tab_and_the_client_allow() {
        let data = Data { tabs: &[], active: None, bookmarks: &[], history: &[], now: 0 };
        let found = sections(&State { no_tab: true, no_page: true, ..State::default() }, |c| c != Command::Settings, &data);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, Section::QuickActions);
        assert!(found[0].1.iter().all(|e| !matches!(e.item, Item::Command(Command::Settings | Command::ReopenClosedTab | Command::Ask))));
        assert!(sectioned("  ") && !sectioned("a"));
    }

    #[test]
    fn each_row_kind_has_its_actions() {
        let kinds = |item: Item, spaces: &[&str]| actions(&item, spaces).into_iter().map(|(_, a)| a).collect::<Vec<_>>();
        assert_eq!(kinds(Item::Command(Command::Settings), &[]), [Action::Run]);
        assert_eq!(kinds(Item::Tab(1), &["Work", "Home"]), [Action::Switch, Action::Close, Action::CopyLink, Action::MoveToSpace(0), Action::MoveToSpace(1)]);
        assert_eq!(actions(&Item::Tab(1), &["Work"])[3].0, "Move to Work");
        assert_eq!(kinds(Item::Bookmark("https://a.test/".into()), &[]), [Action::Open, Action::OpenInNewTab, Action::CopyLink]);
        assert_eq!(kinds(Item::History("https://a.test/".into()), &[])[3], Action::RemoveFromHistory);
    }

    #[test]
    fn the_shake_swings_and_comes_to_rest() {
        assert_eq!(shake(0.), 0.);
        assert!(shake(1.).abs() < 1e-4);
        assert!((0..=100).map(|i| shake(i as f32 / 100.).abs()).all(|x| x <= 6.));
        assert!((0..=100).any(|i| shake(i as f32 / 100.) < -2.) && (0..=100).any(|i| shake(i as f32 / 100.) > 2.));
    }

    #[test]
    fn listing_needs_a_leading_angle_bracket() {
        assert!(listing("  > copy"));
        assert!(!listing("copy >"));
    }
}
