use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};

use browser_core::palette::{Command, Item};
use browser_core::suggest::{self, Page};
use engine::nav_policy::{self, Decision, Source};
use engine::{DataStore, Event, Host, Popup, Prompt, PromptKind, WebView};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable, Image, ImageFormat, Pixels, Subscription, Task, Window, WindowHandle};
use shield::ShieldLevel;
use store::{Bookmark, Counter, HistoryEntry, Id, Profile, SavedWindow, Store, Tab, TabKind, WebEngine};
use ui::command::{self, Input};
use ui::text_input::{TextInput, TextInputEvent};
use url::Url;

use crate::blocking::{BlockedPage, Offer};
use crate::devtools::JsonPage;
use crate::filters;
use crate::settings::SettingsWindow;
use engine::favicon;

mod command_bar;
mod context;
mod events;
mod profile;
mod views;

const MAX_SUGGESTIONS: usize = 8;
type FaviconMemo = HashMap<String, Option<Arc<Image>>>;
/// Visits found by their page text, listed after the title and address matches.
const MAX_TEXT_MATCHES: usize = 3;
const IGNORED_LOAD_ERRORS: [isize; 2] = [-999, 102];
const MAX_QUEUED_PROMPTS: usize = 8;
const SAME_DOCUMENT_VISIT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const MAX_SAME_DOCUMENT_URL: usize = 2048;
const DEVELOPER_SETTING: &str = devkit::SETTING;
const FILTER_SETTING_WARNING: &str = "The filter-list setting could not be saved";
/// The page sits 1pt inside its frame, so its corners are 1pt tighter than the frame's.
pub(crate) fn page_corner() -> f64 {
    ui::theme::Theme::radius_page() as f64 - 1.
}

/// Redraws the windows that draw spinners on the first display frame `SPIN_STEP` after `drawn`, so
/// the spinners move in even steps on the display's beat. Only these windows redraw, not every
/// observer of the browser.
fn spin_step(window: &Window, browser: gpui::WeakEntity<Browser>, drawn: Instant) {
    window.on_next_frame(move |window, cx| {
        if drawn.elapsed() < ui::controls::SPIN_STEP {
            return spin_step(window, browser, drawn);
        }
        window.refresh();
        if let Ok(Some(peek)) = browser.update(cx, |b, _| {
            b.spinning = false;
            b.peek.window
        }) {
            peek.update(cx, |_, window, _| window.refresh()).ok();
        }
    });
}

impl Browser {
    /// Gives open web views the page radius and colour scheme of a theme chosen after they were made.
    pub(crate) fn follow_theme(&mut self) {
        let generation = ui::theme::Theme::generation();
        if std::mem::replace(&mut self.theme_generation, generation) != generation {
            let dark = !ui::theme::Theme::is_light();
            self.views.values().for_each(|v| {
                v.set_corner_radius(page_corner());
                v.set_dark(dark);
            });
            self.link_peek.set_dark(dark);
        }
    }
}

use browser_core::closed::ClosedTab;
pub use browser_core::tabs::{Opened, SpaceRef, TabKey, folder_tree};

#[derive(Default, Clone)]
pub struct PageState {
    pub can_back: bool,
    pub can_forward: bool,
    pub loading: bool,
    pub crashed: bool,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// What the notice's second button does.
    pub offer: Option<Offer>,
    /// When a same-document navigation was last recorded in history.
    pub last_visit: Option<Instant>,
    /// The history row (id and URL) this tab recorded for the page it shows now, which a later
    /// title change renames; cleared whenever the tab moves on without recording.
    pub visit: Option<(Id, String)>,
    /// The warning page shown instead of the web view.
    pub blocked: Option<BlockedPage>,
    /// A local-network refusal or an HTTPS upgrade was counted for the page shown now, so a page
    /// retrying cannot run the counters up.
    pub local_counted: bool,
    pub https_counted: bool,
    /// The address the user last loaded in this tab, until a page commits; Reload loads it again
    /// while the view has no page.
    pub retry: Option<Url>,
    /// The page shown now has an article for the reader.
    pub article: bool,
    /// The address the user last opened in this tab from the address field or a bookmark, until a
    /// load of it commits: that visit counts toward inline completion (`suggest::typed_visit`).
    pub typed: Option<String>,
    /// The file this tab opened with Open File: a slept, rebuilt or moved view shows it again (S6).
    pub opened_file: Option<Url>,
    /// The page and scroll offset the tab had when it went to sleep, restored when that page loads again.
    pub scroll_back: Option<(String, (f64, f64))>,
}

/// A request to hand a URL to another app; `tab` is `None` when the user typed it, `origin` is the
/// requesting frame's when known.
pub struct PendingExternal {
    pub tab: Option<TabKey>,
    pub origin: Option<String>,
    pub url: Url,
}

pub(crate) enum Msg {
    Tab(TabKey, u64, Event),
    Found(usize, Option<usize>),
    IconLinks(TabKey, IconKey, Url, String),
    IconData(IconKey, String),
    PageText(Id, Id, String, String),
    Json(TabKey, Url, String),
    Zap(TabKey, Url, String),
    Peek(u64, Event),
    Handover(TabKey, std::rc::Weak<WebView>, Option<engine::handover::Storage>),
    /// A page's article (`probe`: whether it has one), for the reader.
    Article(TabKey, Url, bool, String),
    /// A page or another tab read for the assistant: the tab, who asked and the pill.
    AssistantPill(TabKey, browser_core::assistant::Origin, browser_core::assistant::Pill),
    /// The selection a tab's menu asked the assistant about: the tab, who asked, the page it was
    /// read from, the raw text and the command.
    AssistantSelection(TabKey, browser_core::assistant::Origin, String, String, Option<String>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CommandMode {
    NewTab,
    Current,
    FindInTabs,
    Snippets,
}

pub enum Target {
    Tab(TabKey),
    Url(Url),
    Input(Input),
    Action(Box<dyn gpui::Action>),
    /// A tab whose text, or title and address, contains the query.
    Find(TabKey, String),
    /// An answer of `palette::answers`, which Enter copies.
    Copy(String),
    /// A snippet's body, which Enter types into the page; empty for the row saying there are none.
    Snippet(String),
    /// A row of the actions page: what it acts on and what it does.
    Act(Item, browser_core::palette::Action),
}

pub struct Suggestion {
    /// The empty bar's section the row is listed in; `None` derives it from the target.
    pub section: Option<browser_core::command::Section>,
    pub label: String,
    pub detail: String,
    pub target: Target,
}

pub(crate) use browser_core::now;

pub(crate) type Core = browser_core::Core<DataStore>;
pub(crate) type ProfileRuntime = browser_core::ProfileRuntime<DataStore>;

pub(crate) fn data_dir() -> PathBuf {
    let dir = browser_core::data_dir(None);
    #[cfg(windows)]
    let dir = dir.or_else(|| Some(engine::local_app_data()?.join("Tarn")));
    dir.unwrap_or_default()
}

/// Purges Chromium's data when its owner is gone or a purge is pending; before CEF starts or once it is gone.
#[cfg(feature = "chromium")]
pub(crate) fn prepare_chromium(core: &Core) {
    // An in-memory stand-in knows none of the real profiles, which would make Chromium's owner an orphan.
    if !core.disk.on_disk() {
        return;
    }
    if let Ok(profiles) = core.disk.profiles() {
        let live: Vec<String> = profiles.into_iter().filter(|p| p.shield_level != ShieldLevel::Lockdown).map(|p| p.data_store_uuid).collect();
        engine::chromium_owner::prepare(&data_dir().join("Chromium"), &live);
    }
}
/// Records a finished download once and shows the outcome in every window.
pub(crate) fn on_download(private: bool, profile_id: Id, event: Event, cx: &mut App) {
    let core = crate::shared::core(cx);
    let (id, result) = match event {
        Event::DownloadFinished { id, url, path, started, quarantined } => {
            let url = url
                .map(|mut u| {
                    let _ = u.set_username("");
                    let _ = u.set_password(None);
                    String::from(u)
                })
                .unwrap_or_default();
            let started = started.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
            let state = if quarantined { "finished" } else { "finished-unquarantined" };
            if core.keeps_record_since(private, profile_id, started) {
                core.store(private).add_download(profile_id, &url, &path.to_string_lossy(), state, started).ok();
            }
            (Some(id), Ok((path, quarantined)))
        }
        Event::DownloadFailed { id, message } => {
            if let Some(id) = id {
                record_ended(&core, private, profile_id, id, "failed", cx);
            }
            (id, Err(message))
        }
        Event::DownloadCancelled(id) => {
            record_ended(&core, private, profile_id, id, "cancelled", cx);
            return crate::shared::each(cx, |b, _, cx| {
                b.set_transfer((private, profile_id, id), None, cx);
                b.downloads_popover.changed += 1;
            });
        }
        Event::DownloadsActive(count) => return crate::shared::each(cx, |b, _, cx| b.set_active_downloads(private, profile_id, count, cx)),
        Event::DownloadProgress { id, path, done, total, paused } => {
            let transfer = crate::downloads::Transfer { path, done, total, paused, started: now() };
            return crate::shared::each(cx, |b, _, cx| b.set_transfer((private, profile_id, id), Some(transfer.clone()), cx));
        }
        _ => return,
    };
    crate::shared::each(cx, |b, window, cx| {
        if let Some(id) = id {
            b.set_transfer((private, profile_id, id), None, cx);
        }
        b.notify_download(private, profile_id, &result, window.is_window_active(), cx);
        b.downloads_popover.changed += 1;
        cx.notify();
    });
}

/// Lists a download that stopped before it finished under the file it was saving to.
fn record_ended(core: &crate::shared::AppCore, private: bool, profile_id: Id, id: u64, state: &str, cx: &App) {
    let path = cx
        .windows()
        .into_iter()
        .filter_map(|w| w.downcast::<Browser>())
        .find_map(|h| h.read(cx).ok().and_then(|b| b.downloads_popover.transfers.get(&(private, profile_id, id)).map(|t| (t.path.clone(), t.started))));
    if let Some((path, started)) = path
        && core.keeps_record_since(private, profile_id, started)
    {
        core.store(private).add_download(profile_id, "", &path.to_string_lossy(), state, started).ok();
    }
}

/// (private, profile id, host): site icons belong to a profile like the rest of its data.
type IconKey = ((bool, Id), String, favicon::Ticket);
type SiteStoppedKey = (u64, bool, Id, String);

pub struct Browser {
    pub(crate) core: Rc<crate::shared::AppCore>,
    pub(crate) warning: Option<String>,
    pub(crate) host: Option<Host>,
    pub model: browser_core::tabs::Model,
    pub(crate) views: HashMap<TabKey, Rc<WebView>>,
    /// `Theme::generation` the views' corner radius was set for.
    theme_generation: u32,
    pub pages: HashMap<TabKey, PageState>,
    pub command: Entity<TextInput>,
    pub command_mode: Option<CommandMode>,
    pub(crate) settings_window: Option<WindowHandle<SettingsWindow>>,
    pub(crate) qr: Option<WindowHandle<crate::qr::QrWindow>>,
    favicons: favicon::Tracker<(bool, Id), Arc<Image>>,
    /// `favicon` results by page URL for one profile, emptied whenever `favicons` changes.
    favicon_memo: RefCell<Option<((bool, Id), FaviconMemo)>>,
    pub(crate) site_stopped_memo: RefCell<Option<(SiteStoppedKey, i64)>>,
    #[cfg(feature = "translation")]
    pub(crate) translation_offered: HashSet<(bool, Id, String)>,
    pub sidebar_open: bool,
    pub(crate) tab_placement: browser_core::layout::TabPlacement,
    pub sidebar_width: f32,
    pub(crate) toast: crate::toast::Toasts,
    pub(crate) link_status: crate::link_status::LinkStatus,
    pub(crate) capture_overlay: Option<crate::capture_overlay::Overlay>,
    pub(crate) mini_player: crate::mini_player::MiniPlayer,
    /// The tab and committed address Insert Snippet opened on.
    pub(crate) snippet_page: Option<(TabKey, String)>,
    /// New tabs whose first load is a page's link, so S10 judges every redirect hop.
    link_loads: HashMap<TabKey, Option<Url>>,
    pub suggestions: Vec<Suggestion>,
    pub selected: usize,
    pub(crate) suggestions_scroll: gpui::ScrollHandle,
    history_cache: Vec<HistoryEntry>,
    /// The last page-text search: profile, query and the visits it found.
    history_text: Option<(Id, String, Vec<HistoryEntry>)>,
    history_text_search: Option<Task<()>>,
    /// The actions page in the command bar: the row it acts on, and the row to return to.
    pub(crate) actions_page: Option<(String, usize)>,
    /// How often Enter met an address Tarn cannot open since the field last changed; the field
    /// shakes each time, or says so under Reduce Motion.
    pub(crate) rejected: u64,
    /// The query and completion the suggestions were last built for, so a late text search can
    /// rebuild them without completing the field again.
    suggested: Option<(String, Option<suggest::Completion>)>,
    pub pending_external: Option<PendingExternal>,
    pub downloads_popover: crate::downloads::Downloads,
    pub prompts: HashMap<TabKey, VecDeque<Prompt>>,
    pub prompt_input: Entity<TextInput>,
    pub auth_user: Entity<TextInput>,
    pub auth_pass: Entity<TextInput>,
    pub remember_permission: bool,
    /// When a guarded control last appeared or moved, or the tab, window or overlay changed
    /// (`browser_core::site::BYPASS_DELAY`).
    pub layout_stable_since: Instant,
    pub find_open: bool,
    pub find_input: Entity<TextInput>,
    /// Matches the last find counted, and the highlighted one when the engine reports it.
    pub find_count: Option<(usize, Option<usize>)>,
    find_typing: Option<Task<()>>,
    /// A tab woken by Find in Tabs, searched once its page has finished loading.
    pub(crate) find_after_load: Option<TabKey>,
    pub(crate) find_tabs: Option<Task<()>>,
    /// Set when prompt inputs vanish, so the next render returns keyboard focus to the shell.
    pub refocus: bool,
    /// When (`now()`, so time asleep counts) this window last changed space by hand, which keeps
    /// it off the schedules for an hour.
    pub space_switched_at: Option<i64>,
    /// When Reload was last asked for, so its arrow spins at least `SPIN` even for a cached page.
    pub(crate) reload_spin: Option<(TabKey, Instant)>,
    pub(crate) hang: engine::lifecycle::Hang<TabKey>,
    /// A display frame is booked to redraw the spinners.
    spinning: bool,
    /// The profile whose bookmarks `bookmarks` holds; `reload_bookmarks` refreshes them.
    pub(crate) bookmarks_of: Option<(bool, Id)>,
    /// The space the burner tab was opened from, shown again when it burns.
    pub(crate) burner_return: Option<(bool, Id)>,
    view_generation: HashMap<TabKey, u64>,
    next_generation: u64,
    pub privacy_notice: Option<((bool, Id), String)>,
    pub developer: bool,
    /// A page shows an element full screen, and whether that put the window into full screen.
    pub(crate) page_fullscreen: browser_core::fullscreen::PageFullScreen,
    pub responsive: Option<usize>,
    zoom_before_responsive: HashMap<TabKey, f64>,
    clear_site_armed: Option<(TabKey, String)>,
    pub local_servers: Option<Vec<u16>>,
    pub json_pages: HashMap<TabKey, JsonPage>,
    pub(crate) events: mpsc::UnboundedSender<Msg>,
    focus: FocusHandle,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
    pub swipe: crate::gestures::Swipe,
    pub(crate) pan: crate::gestures::Pan,
    /// The region of a Windows touchpad gesture under way and the timer that ends it (`gestures`).
    #[cfg(windows)]
    pub(crate) wheel: Option<(Option<bool>, gpui::Task<()>)>,
    pub(crate) slide: Option<crate::sidebar::Slide>,
    nav_buttons: Option<engine::menu::EventMonitor>,
    page_clicks: Option<engine::menu::EventMonitor>,
    pub profile_editor: Option<crate::profiles::ProfileEditor>,
    pub renaming: Option<crate::groups::Rename>,
    pub peek: crate::peek::Peek,
    pub library: Option<WindowHandle<crate::library::LibraryWindow>>,
    pub(crate) library_closed: Option<std::time::Instant>,
    pub(crate) space_popover: Option<(WindowHandle<crate::spaces::SpacePopover>, WindowHandle<Browser>)>,
    pub(crate) space_closed: Option<std::time::Instant>,
    pub(crate) samples: crate::memory::Samples,
    pub(crate) new_tab_group: Option<Id>,
    pub(crate) launcher_from: Option<TabKey>,
    pub(crate) menu_open: bool,
    pub(crate) seen: HashMap<TabKey, Instant>,
    /// Awake tabs whose page last said it was playing, and the background tabs marked heavy.
    pub(crate) playing: HashSet<TabKey>,
    pub(crate) heavy: Vec<TabKey>,
    /// Background tabs left awake for the form input the user typed, until they are shown again.
    pub(crate) typed_in: HashSet<TabKey>,
    pub(crate) sidebar_toggled: Option<Instant>,
    /// How far the sidebar was shown when it was last toggled, so a toggle mid-slide turns back from there.
    pub(crate) sidebar_from: f32,
    /// The width each web view keeps while the sidebar slides, by the view's address.
    pub(crate) sidebar_held: Rc<RefCell<browser_core::layout::HeldWidths>>,
    /// When the window last rendered, for `TARN_FRAME_TRACE`'s frame intervals.
    pub(crate) drawn_at: Option<Instant>,
    pub(crate) page_fade: Option<(Rc<WebView>, Instant)>,
    pub(crate) overlay_exit: Option<Instant>,
    pub(crate) watch: crate::content::Watch,
    pub(crate) fog: gpui::Entity<crate::content::Fog>,
    pub(crate) search_fx: gpui::Entity<crate::content::SearchFx>,
    pub(crate) floating: crate::content::Floating,
    pub bookmarks: Vec<Bookmark>,
    pub(crate) motion: crate::sidebar::Motion,
    pub(crate) extension_clicked: Option<usize>,
    pub(crate) menu: crate::menubar::State,
    pub(crate) address: crate::address::Address,
    pub(crate) hover_card: crate::hover_card::HoverCard,
    pub(crate) site_popover: crate::site_settings::SitePopover,
    pub(crate) share_menu: crate::share::ShareMenu,
    /// The Windows menu button, as last painted, so its menu drops from it.
    pub(crate) app_menu_anchor: Rc<Cell<Bounds<Pixels>>>,
    /// The button the next menu drops from; without one it opens at the pointer.
    pub(crate) menu_at: Option<Bounds<Pixels>>,
    pub(crate) link_peek: crate::link_peek::LinkPeek,
    pub(crate) assistant: crate::assistant::Assistant,
    pub(crate) splits: browser_core::split::Splits<TabKey>,
    pub(crate) split_shown: crate::split::Shown,
    /// The row a dragged tab is over, when the drop would split with it what the drop does, and
    /// otherwise whether the tab lands below that row rather than above it.
    pub(crate) split_drop: Option<(TabKey, Option<&'static str>, bool)>,
    pub(crate) tab_drag: Option<TabKey>,
    adopting: Option<TabKey>,
    /// A private or burner window whose space could not be made: it shows no space while it closes.
    pub(crate) abandoned: bool,
    pub(crate) ghost: crate::tear_off::Ghosting,
    pub shortcuts_open: bool,
    pub(crate) shortcuts_scroll: gpui::ScrollHandle,
    pub(crate) space_strip_scroll: gpui::ScrollHandle,
    pub(crate) space_strip_shown: std::cell::Cell<Option<(usize, usize, i32)>>,
    pub reader: Option<crate::reader::Reader>,
    pub reader_layout: crate::reader::Layout,
    /// This window's row; its engine is the window engine.
    pub(crate) saved: SavedWindow,
}

impl Focusable for Browser {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The URLs among launch arguments. After `--single-argument`, which Windows passes before a link
/// it hands over, the rest of the line is one URL.
pub(crate) fn launch_urls(args: impl Iterator<Item = String>) -> Vec<String> {
    let args: Vec<String> = args.collect();
    match args.split_first() {
        Some((first, rest)) if first == "--single-argument" => vec![rest.join(" ")],
        _ => args,
    }
}

/// The space a new window opens on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WindowStart {
    Saved,
    Private,
    Burner,
}

impl Browser {
    pub(crate) fn new(saved: SavedWindow, torn: Option<browser_core::tabs::Torn>, start: WindowStart, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let core = crate::shared::core(cx);
        let disk = &core.disk;

        let command = cx.new(|cx| TextInput::new("Search or enter address", cx).with_paste_and_go());
        let subscription = cx.subscribe_in(&command, window, Self::on_command_event);
        let prompt_input = cx.new(|cx| TextInput::new("", cx));
        let auth_user = cx.new(|cx| TextInput::new("User name", cx));
        let auth_pass = cx.new(|cx| TextInput::new("Password", cx).masked());
        let find_input = cx.new(|cx| TextInput::new("Find in page", cx));
        let prompt_subs = [&prompt_input, &auth_user, &auth_pass].map(|input| cx.subscribe_in(input, window, Self::on_prompt_input_event));
        let find_sub = cx.subscribe_in(&find_input, window, Self::on_find_event);
        let assistant = crate::assistant::Assistant::new(window, cx);
        let (tx, mut rx) = mpsc::unbounded::<Msg>();
        let events_task = cx.spawn_in(window, async move |this, cx| {
            while let Some(msg) = rx.next().await {
                let handled = this.update_in(cx, |b, window, cx| match msg {
                    Msg::Tab(key, generation, event) => {
                        if b.view_generation.get(&key) == Some(&generation) {
                            b.on_event(key, event, window, cx);
                        }
                    }
                    Msg::Found(count, current) => {
                        b.find_count = Some((count, current));
                        cx.notify();
                    }
                    Msg::IconLinks(key, icon, page, links) => b.fetch_favicon(key, icon, &page, &links),
                    Msg::IconData(icon, data) => b.store_favicon(icon, data, cx),
                    Msg::PageText(profile_id, visit, url, text) => b.store_page_text(profile_id, visit, &url, text),
                    Msg::Json(key, url, text) => b.show_json(key, url, text, cx),
                    Msg::Zap(key, page, selector) => b.zap_picked(key, page, selector, cx),
                    Msg::Article(key, url, probe, text) => b.on_article(key, url, probe, text, window, cx),
                    Msg::Peek(generation, event) => b.on_peek_event(generation, event, cx),
                    Msg::AssistantPill(key, origin, pill) => b.add_read_pill(key, origin, pill, cx),
                    Msg::AssistantSelection(key, origin, url, raw, command) => {
                        if browser_core::assistant::read_here(&url, b.tab_url(key).as_deref()) {
                            b.selection_for_assistant(origin, &url, &raw, command, window, cx);
                        }
                    }
                    Msg::Handover(key, old, storage) => {
                        let current = b.views.get(&key).is_some_and(|v| old.upgrade().is_some_and(|o| Rc::ptr_eq(v, &o)));
                        if let (true, Some(url)) = (current, b.tab_url(key).and_then(|u| Url::parse(&u).ok())) {
                            b.rebuild_view_with(key, &url, storage, cx);
                            cx.notify();
                        }
                    }
                });
                if handled.is_err() {
                    break;
                }
            }
        });
        let live_tabs = crate::prefs::live_tabs(disk);
        let tab_placement = browser_core::layout::TabPlacement::load(disk);
        let sidebar_open = crate::prefs::sidebar_open(disk) && tab_placement == browser_core::layout::TabPlacement::Side;
        let sidebar_width = crate::prefs::sidebar_width(disk);
        let watch = crate::content::Watch::default();
        let mut browser = Browser {
            core,
            warning: None,
            host: Host::from_window(window),
            model: browser_core::tabs::Model::new(live_tabs),
            views: HashMap::new(),
            theme_generation: ui::theme::Theme::generation(),
            pages: HashMap::new(),
            command,
            command_mode: None,
            settings_window: None,
            qr: None,
            favicons: favicon::Tracker::default(),
            favicon_memo: RefCell::new(None),
            site_stopped_memo: RefCell::new(None),
            #[cfg(feature = "translation")]
            translation_offered: HashSet::new(),
            sidebar_open,
            tab_placement,
            sidebar_width,
            toast: Default::default(),
            link_status: Default::default(),
            capture_overlay: None,
            mini_player: Default::default(),
            snippet_page: None,
            link_loads: HashMap::new(),
            suggestions: Vec::new(),
            selected: 0,
            suggestions_scroll: gpui::ScrollHandle::new(),
            history_cache: Vec::new(),
            history_text: None,
            history_text_search: None,
            actions_page: None,
            rejected: 0,
            suggested: None,
            pending_external: None,
            downloads_popover: Default::default(),
            prompts: HashMap::new(),
            prompt_input,
            auth_user,
            auth_pass,
            remember_permission: false,
            layout_stable_since: Instant::now(),
            find_open: false,
            find_input,
            find_count: None,
            find_typing: None,
            find_after_load: None,
            find_tabs: None,
            refocus: false,
            space_switched_at: None,
            reload_spin: None,
            hang: Default::default(),
            spinning: false,
            bookmarks_of: None,
            burner_return: None,
            view_generation: HashMap::new(),
            next_generation: 0,
            privacy_notice: None,
            developer: false,
            page_fullscreen: Default::default(),
            responsive: None,
            zoom_before_responsive: HashMap::new(),
            clear_site_armed: None,
            local_servers: None,
            json_pages: HashMap::new(),
            events: tx,
            focus: cx.focus_handle(),
            _tasks: vec![events_task],
            _subscriptions: [subscription, find_sub].into_iter().chain(prompt_subs).collect(),
            swipe: crate::gestures::Swipe::natural(engine::menu::natural_scrolling()),
            pan: Default::default(),
            #[cfg(windows)]
            wheel: None,
            slide: None,
            nav_buttons: None,
            page_clicks: None,
            profile_editor: None,
            renaming: None,
            peek: Default::default(),
            library: None,
            library_closed: None,
            space_popover: None,
            space_closed: None,
            samples: Default::default(),
            new_tab_group: None,
            launcher_from: None,
            menu_open: false,
            seen: HashMap::new(),
            playing: HashSet::new(),
            typed_in: HashSet::new(),
            heavy: Vec::new(),
            sidebar_toggled: None,
            sidebar_from: 0.,
            sidebar_held: Rc::default(),
            drawn_at: None,
            page_fade: None,
            overlay_exit: None,
            fog: cx.new(|_| watch.fog()),
            search_fx: cx.new(|_| watch.search_fx()),
            watch,
            floating: Default::default(),
            bookmarks: Vec::new(),
            motion: Default::default(),
            extension_clicked: None,
            menu: Default::default(),
            address: Default::default(),
            hover_card: Default::default(),
            site_popover: Default::default(),
            share_menu: Default::default(),
            app_menu_anchor: Default::default(),
            menu_at: None,
            link_peek: Default::default(),
            assistant,
            splits: Default::default(),
            split_shown: Default::default(),
            split_drop: None,
            tab_drag: None,
            adopting: None,
            abandoned: false,
            ghost: Default::default(),
            shortcuts_open: false,
            shortcuts_scroll: gpui::ScrollHandle::new(),
            space_strip_scroll: gpui::ScrollHandle::new(),
            space_strip_shown: Default::default(),
            reader: None,
            reader_layout: Default::default(),
            saved,
        };
        browser.peek.main = window.window_handle().downcast::<Browser>();
        browser._subscriptions.push(cx.on_release(|b, cx| b.close_popups(cx)));
        let blur = cx.on_blur(&browser.command.focus_handle(cx), window, |b, window, cx| {
            if b.inline_editing() {
                b.close_command(window, cx);
            }
        });
        let appearance = cx.observe_window_appearance(window, |b, _, cx| crate::themes::appearance_changed(&b.core.clone(), cx));
        let deactivate = cx.observe_window_activation(window, |b, window, cx| {
            cx.notify();
            if window.is_window_active() {
                // Animations in every window stood still while Tarn was in the background.
                crate::shared::others(cx, |_, _, cx| cx.notify());
                b.layout_changed();
                b.menu.reset();
                b.sync_shared(window, cx);
            } else if b.inline_editing() && !b.command.read(cx).menu_open() {
                b.close_command(window, cx);
            }
        });
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            this.update(cx, |b, cx| b.closing(cx)).ok();
            true
        });
        let release = cx.on_release(|b, cx| {
            if let Some((ghost, _)) = b.ghost.popup.take() {
                ghost.update(cx, |_, window, _| window.remove_window()).ok();
            }
            b.save_all();
            b.model.burn(&b.core);
            b.address.remove_windows(cx);
        });
        browser._subscriptions.extend([blur, appearance, deactivate, release]);
        browser.watch_navigation_buttons(window, cx);
        browser.watch_split_clicks(window, cx);
        browser.watch_peek_pointer(window, cx);
        browser.watch_hangs(cx);
        browser.developer = browser.core.disk.setting(DEVELOPER_SETTING).ok().flatten().as_deref() == Some("on");
        if browser.developer {
            browser.refresh_local_servers(cx);
        }
        browser.reload_spaces();
        let restore = match torn.and_then(|t| browser.model.launch_torn(&browser.core, t)) {
            Some(ix) => ix,
            None => {
                let restore = browser.model.launch_space(&browser.core, &browser.saved);
                browser.scheduled_now().unwrap_or(restore)
            }
        };
        // Chosen before the first switch, so a private or burner window never wakes a regular tab;
        // one whose space cannot be made closes instead.
        let back = browser.model.spaces.get(restore).map(|s| (s.private, s.space.id));
        let restore = match start {
            WindowStart::Saved => Some(restore),
            WindowStart::Private => browser.model.private_space(&browser.core),
            WindowStart::Burner => browser.model.open_burner(&browser.core).inspect(|_| browser.burner_return = back),
        };
        window.focus(&browser.focus);
        let Some(restore) = restore else {
            eprintln!("tarn: could not make the space for a new private or burner window");
            browser.abandoned = true;
            cx.defer_in(window, |b, window, cx| {
                b.closing(cx);
                window.remove_window();
            });
            return browser;
        };
        browser.adopting = torn.map(|t| t.key);
        browser.switch_space(restore, window, cx);
        browser.adopting = None;
        browser.space_switched_at = None;
        browser
    }

    /// Opens the URLs the app was launched with, each in a new tab.
    pub(crate) fn open_args(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let args: Vec<String> = std::env::args().skip(1).collect();
        // Windows hands a link from another app to a starting Tarn as `--single-argument <url>`.
        let from_app = args.first().is_some_and(|a| a == "--single-argument");
        self.open_urls(launch_urls(args.into_iter()), from_app, window, cx);
    }

    /// URLs from the command line or another app, each in a new tab; other schemes are refused.
    /// A link from another app (`from_app`) loads as a page's link, so S10 refuses it, or any
    /// redirect hop of it, into the local network; typing the address or Open Anyway opens it.
    /// A loopback link loads as typed while Settings allows it (`nav_policy::app_link_typed`).
    pub(crate) fn open_urls(&mut self, urls: impl IntoIterator<Item = String>, from_app: bool, window: &mut Window, cx: &mut Context<Self>) {
        for arg in urls {
            let Some((url, typed)) = browser_core::incoming::app_link(&arg, self.pref(crate::prefs::Pref::LocalhostFromApps)) else { continue };
            if self.command_mode.is_some() {
                self.close_command(window, cx);
            }
            let url = self.clean_user_load(url, cx);
            if let Some(ix) = self.routed_space(&url) {
                self.switch_space(ix, window, cx);
            }
            if !from_app || typed {
                self.new_tab(&url, window, cx);
            } else if let Some(key) = self.model.new_tab(&self.core, url.as_str(), self.saved.web_engine) {
                self.link_loads.insert(key, None);
                self.reload_space();
                self.activate(key, window, cx);
            }
        }
    }

    /// Asks for a local file and shows it in a new tab (Open File).
    pub(crate) fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some("Open".into()) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update_in(cx, |b, window, cx| b.open_file(&path, window, cx)).ok();
        })
        .detach();
    }

    /// Shows a file the user picked in a new tab; only Open File reaches `file:` URLs (S6).
    pub(crate) fn open_file(&mut self, path: &std::path::Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) = browser_core::open_file::url_for(path) else { return };
        if self.command_mode.is_some() {
            self.close_command(window, cx);
        }
        if !self.model.burner_shown() {
            // WebKit, whose read grant keeps an opened page to its folder (verified on macOS).
            return self.new_file_tab(&url, cfg!(target_os = "macos").then_some(WebEngine::WebKit), window, cx);
        }
        // A burner keeps its one tab, which shows the file in place of its page.
        let Some(key) = self.model.active else { return };
        let earlier = self.pages.entry(key).or_default().opened_file.replace(url.clone());
        if earlier.is_some() {
            // Its view would keep the read grant of the file it showed before.
            self.rebuild_view_loading(key, &url, None, None, cx);
        } else if let Some(view) = self.active_view() {
            view.load_file(&url);
        }
    }

    /// A new tab showing a file the user opened, which `wake` loads with `load_file`.
    pub(crate) fn new_file_tab(&mut self, url: &Url, web_engine: Option<WebEngine>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.new_tab(&self.core, url.as_str(), web_engine.or(self.saved.web_engine)) else { return };
        self.pages.entry(key).or_default().opened_file = Some(url.clone());
        self.reload_space();
        self.activate(key, window, cx);
    }

    /// Another non-private space whose rule claims new tabs of `url`; none from a private space.
    pub(crate) fn routed_space(&self, url: &Url) -> Option<usize> {
        let from_private = self.model.current_space().is_none_or(|s| s.private);
        let space_id = browser_core::routing::route(&self.core.disk.space_rules().ok()?, url, from_private)?;
        self.model.spaces.iter().position(|s| !s.private && s.space.id == space_id).filter(|ix| *ix != self.model.space_ix)
    }

    /// A new tab of `url` in another space a rule names; false, opening nothing, when none does.
    pub(crate) fn open_routed(&mut self, url: &Url, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(ix) = self.routed_space(url) else { return false };
        self.switch_space(ix, window, cx);
        self.new_tab(url, window, cx);
        true
    }

    pub(crate) fn closing(&mut self, cx: &mut App) {
        self.model.release_confined(&self.core);
        crate::shared::window_closed(&self.core, self.saved.id, cx);
    }

    /// Saves the window's frame and what it shows, for the next launch.
    pub(crate) fn save_window(&self, window: &Window) {
        let [x, y, width, height] = crate::shared::window_frame(window);
        let space = self.model.current_space().map(|s| s.space.id);
        let (space_id, active_tab_id) = browser_core::windows::saved_place(self.model.private(), space, self.model.active.map(|k| k.id));
        self.core.disk.update_window(&SavedWindow { x, y, width, height, space_id, active_tab_id, ..self.saved.clone() }).ok();
    }

    /// Catches up with what other windows changed: spaces, tabs and bookmarks, and drops views of
    /// tabs they closed or archived.
    pub(crate) fn sync_shared(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.abandoned {
            return;
        }
        let lost_active = self.drop_gone_views(cx);
        let profile = self.model.current_space().map(|s| (s.private, s.space.profile_id));
        self.reload_spaces();
        self.model.closed_tabs.retain(&self.model.spaces);
        if self.model.current_space().is_none() {
            let ix = self.model.spaces.iter().position(|s| Some((s.private, s.space.profile_id)) == profile).unwrap_or(0);
            let held = self.space_switched_at;
            self.switch_space(ix, window, cx);
            self.space_switched_at = held;
            return;
        }
        self.reload_space();
        self.reload_bookmarks(cx);
        if lost_active {
            self.pick_active(window, cx);
        }
    }

    /// Drops views of tabs that are no longer stored; true when the active tab was one of them.
    pub(crate) fn drop_gone_views(&mut self, cx: &mut Context<Self>) -> bool {
        let ids = [false, true].map(|private| self.store(private).tab_ids().unwrap_or_default());
        let live = move |k: &TabKey| ids[usize::from(k.private)].contains(&k.id);
        let gone: Vec<TabKey> = self.views.keys().copied().filter(|k| !live(k)).collect();
        let lost_active = self.model.active.is_some_and(|a| gone.contains(&a));
        for key in gone {
            self.discard_tab(key, cx);
        }
        self.pages.retain(|k, _| live(k));
        self.splits.retain(|k| live(&k));
        lost_active
    }

    /// After tabs were deleted from inside a window update: views go now, before any wipe, and
    /// the rest of `sync_shared` once no window is leased, so every window sees which tabs are live.
    pub(crate) fn sync_other_windows(cx: &mut Context<Self>) {
        crate::shared::others(cx, |b, window, cx| {
            if b.drop_gone_views(cx) {
                cx.defer_in(window, |b, window, cx| {
                    if b.model.active.is_none() {
                        b.pick_active(window, cx);
                    }
                });
            }
        });
        cx.defer(|cx| crate::shared::each(cx, |b, window, cx| b.sync_shared(window, cx)));
    }

    /// Moves a live view of `key` from another window here, with its page and history.
    fn adopt_view(&mut self, key: TabKey, cx: &mut Context<Self>) -> Option<Rc<WebView>> {
        let host = self.host.clone()?;
        let (mut taken, mut link) = (None, None);
        crate::shared::others(cx, |b, window, cx| {
            link = link.take().or_else(|| b.link_loads.remove(&key));
            if taken.is_none() {
                taken = b.give_up_view(key, window, cx);
            }
        });
        if let Some(from) = link {
            self.link_loads.insert(key, from);
        }
        let Some((view, page)) = taken else {
            // A view still held there was hibernated; the tab's opened file comes along all the same.
            let mut opened = None;
            crate::shared::others(cx, |b, _, _| opened = opened.take().or_else(|| b.pages.get_mut(&key).and_then(|p| p.opened_file.take())));
            if opened.is_some() {
                self.pages.entry(key).or_default().opened_file = opened;
            }
            return None;
        };
        let view = Rc::new(view.rehost(&host, self.sink(key)));
        self.view_prefs(&view);
        self.views.insert(key, view.clone());
        self.pages.insert(key, page);
        self.attach_tarn(key);
        Some(view)
    }

    /// Hands this window's view of `key` to another window; when something here still holds the
    /// view it is hibernated instead, and the other window restores it from the saved state.
    pub(crate) fn give_up_view(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) -> Option<(WebView, PageState)> {
        let view = self.take_view(key)?;
        self.splits.leave(key);
        if self.model.active == Some(key) {
            self.model.active = None;
            // Picked once the adopting window's update is over, so this tab counts as live there.
            cx.defer_in(window, |b, window, cx| {
                if b.model.active.is_none() {
                    b.pick_active(window, cx);
                }
            });
        }
        cx.notify();
        let view = match Rc::try_unwrap(view) {
            Ok(view) => view,
            Err(view) => {
                self.views.insert(key, view);
                self.hibernate(key, cx);
                return None;
            }
        };
        view.set_visible(false);
        self.json_pages.remove(&key);
        self.drop_reader(key);
        self.view_generation.remove(&key);
        self.drop_prompts(key, cx);
        self.model.lifecycle.remove(key);
        Some((view, self.pages.remove(&key).unwrap_or_default()))
    }
}

pub fn tab_host(url: &str) -> Option<String> {
    Url::parse(url).ok()?.host_str().map(|h| h.to_ascii_lowercase())
}

pub use browser_core::command::page_host;

/// The row to write for a site's zoom: `Some(None)` forgets it, `None` writes nothing.
fn zoom_to_save(zoom: f64, lockdown: bool) -> Option<Option<f64>> {
    if (zoom - 1.0).abs() <= 0.001 {
        Some(None)
    } else if lockdown {
        None
    } else {
        Some(Some(zoom))
    }
}

/// Permissions granted to an embedded frame are remembered for that page only.
fn permission_origin(origin: &str, top_origin: Option<&str>) -> String {
    match top_origin {
        Some(top) => format!("{origin} in {top}"),
        None => origin.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_argument_link_stays_one_url() {
        let args = |a: &[&str]| launch_urls(a.iter().map(|s| (*s).to_owned()));
        assert_eq!(args(&["--single-argument", "https://a.test/x", "--type=renderer"]), ["https://a.test/x --type=renderer"]);
        assert_eq!(args(&["https://a.test", "https://b.test"]), ["https://a.test", "https://b.test"]);
    }

    #[test]
    fn lockdown_forgets_but_never_saves_a_zoom() {
        assert_eq!(zoom_to_save(1.21, false), Some(Some(1.21)));
        assert_eq!(zoom_to_save(1.21, true), None);
        assert_eq!(zoom_to_save(1.0000001, false), Some(None));
        assert_eq!(zoom_to_save(1.0, true), Some(None));
    }

    #[test]
    fn embedded_permissions_are_keyed_by_page() {
        assert_eq!(permission_origin("https://meet.test", None), "https://meet.test");
        assert_eq!(permission_origin("https://meet.test", Some("https://a.test")), "https://meet.test in https://a.test");
        assert_ne!(permission_origin("https://meet.test", Some("https://a.test")), permission_origin("https://meet.test", Some("https://b.test")));
    }
}
