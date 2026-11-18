use std::cell::{Cell, Ref, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Instant, UNIX_EPOCH};

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

mod context;

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
pub use browser_core::tabs::{SpaceRef, TabKey, folder_tree};

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
    /// The page shown now has an article for the reader.
    pub article: bool,
    /// The address the user last opened in this tab from the address field or a bookmark, until a
    /// load of it commits: that visit counts toward inline completion (`suggest::typed_visit`).
    pub typed: Option<String>,
    /// The file this tab opened with Open File: a slept, rebuilt or moved view shows it again (S6).
    pub opened_file: Option<Url>,
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
}

pub struct DownloadNotice {
    pub private: bool,
    pub profile_id: Id,
    pub result: Result<(std::path::PathBuf, bool), String>,
}

impl DownloadNotice {
    /// A failure, a file that is not quarantined or one that can run code.
    pub fn warns(&self) -> bool {
        match &self.result {
            Ok((path, quarantined)) => !quarantined || path.file_name().is_some_and(|n| engine::downloads::is_dangerous(&n.to_string_lossy())),
            Err(_) => true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CommandMode {
    NewTab,
    Current,
    FindInTabs,
}

pub enum Target {
    Tab(TabKey),
    Url(Url),
    Input(Input),
    Action(Box<dyn gpui::Action>),
    /// A tab whose text, or title and address, contains the query.
    Find(TabKey, String),
}

pub struct Suggestion {
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
    crate::shared::each(cx, |b, _, cx| {
        if let Some(id) = id {
            b.set_transfer((private, profile_id, id), None, cx);
        }
        b.push_download_notice(DownloadNotice { private, profile_id, result: result.clone() });
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
    pub sidebar_width: f32,
    pub(crate) toast: crate::toast::Toasts,
    /// New tabs whose first load is a page's link, so S10 judges every redirect hop.
    link_loads: HashMap<TabKey, Option<Url>>,
    pub suggestions: Vec<Suggestion>,
    pub selected: usize,
    pub(crate) suggestions_scroll: gpui::ScrollHandle,
    history_cache: Vec<HistoryEntry>,
    /// The last page-text search: profile, query and the visits it found.
    history_text: Option<(Id, String, Vec<HistoryEntry>)>,
    history_text_search: Option<Task<()>>,
    /// The query and completion the suggestions were last built for, so a late text search can
    /// rebuild them without completing the field again.
    suggested: Option<(String, Option<suggest::Completion>)>,
    pub pending_external: Option<PendingExternal>,
    pub download: Vec<DownloadNotice>,
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
    pub(crate) sidebar_toggled: Option<Instant>,
    pub(crate) page_fade: Option<(Rc<WebView>, Instant)>,
    pub(crate) overlay_exit: Option<Instant>,
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
    pub(crate) splits: browser_core::split::Splits<TabKey>,
    pub(crate) split_shown: crate::split::Shown,
    /// The row a dragged tab is over, when the drop would split with it what the drop does, and
    /// otherwise whether the tab lands below that row rather than above it.
    pub(crate) split_drop: Option<(TabKey, Option<&'static str>, bool)>,
    /// The tab being dragged, which a drop outside the window tears off.
    pub(crate) tab_drag: Option<TabKey>,
    /// The torn tab this new window takes from another while it picks its first tab.
    adopting: Option<TabKey>,
    pub(crate) ghost: crate::tear_off::Ghosting,
    pub shortcuts_open: bool,
    pub(crate) shortcuts_scroll: gpui::ScrollHandle,
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

impl Browser {
    /// `torn` is the tab dragged out of another window that this one opens for.
    pub fn new(saved: SavedWindow, torn: Option<browser_core::tabs::Torn>, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        let sidebar_open = crate::prefs::sidebar_open(disk);
        let sidebar_width = crate::prefs::sidebar_width(disk);
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
            sidebar_width,
            toast: Default::default(),
            link_loads: HashMap::new(),
            suggestions: Vec::new(),
            selected: 0,
            suggestions_scroll: gpui::ScrollHandle::new(),
            history_cache: Vec::new(),
            history_text: None,
            history_text_search: None,
            suggested: None,
            pending_external: None,
            download: Vec::new(),
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
            swipe: crate::gestures::Swipe::new(engine::menu::natural_scrolling()),
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
            heavy: Vec::new(),
            sidebar_toggled: None,
            page_fade: None,
            overlay_exit: None,
            fog: cx.new(|_| Default::default()),
            search_fx: cx.new(|_| Default::default()),
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
            splits: Default::default(),
            split_shown: Default::default(),
            split_drop: None,
            tab_drag: None,
            adopting: None,
            ghost: Default::default(),
            shortcuts_open: false,
            shortcuts_scroll: gpui::ScrollHandle::new(),
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
        window.focus(&browser.focus);
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

    /// The user closes the window: its row goes, and its temporary space with the tabs in it.
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
        let mut taken = None;
        crate::shared::others(cx, |b, window, cx| {
            if taken.is_none() {
                taken = b.give_up_view(key, window, cx);
            }
        });
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
        view.set_gestures(self.pref(crate::prefs::Pref::Gestures));
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

    pub(crate) fn store(&self, private: bool) -> &Store {
        self.core.store(private)
    }

    pub fn page(&self, key: TabKey) -> PageState {
        self.pages.get(&key).cloned().unwrap_or_default()
    }

    pub fn loading(&self, key: TabKey) -> bool {
        self.pages.get(&key).is_some_and(|p| p.loading)
    }

    /// Books the redraw of the spinners' next step while a spinner of this space turns.
    pub(crate) fn keep_spinning(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reloading = self.reload_spin.is_some_and(|(_, at)| at.elapsed() < crate::toolbar::SPIN);
        if self.spinning || self.still() || !reloading && !self.model.tabs.iter().any(|t| self.loading(self.model.key(t))) {
            return;
        }
        self.spinning = true;
        spin_step(window, cx.entity().downgrade(), Instant::now());
    }

    /// Whether spinners stand still: motion is reduced or Tarn is in the background.
    pub(crate) fn still(&self) -> bool {
        !engine::app_active() || self.pref(crate::prefs::Pref::ReduceMotion)
    }

    pub(crate) fn reload_spaces(&mut self) {
        self.model.reload_spaces(&self.core);
    }

    pub(crate) fn reload_space(&mut self) {
        let old = self.model.reload_space(&self.core);
        self.space_shown(&old);
    }

    /// Catches the shell up with the current space's lists: its profile's site icons, the tabs
    /// that fade in and its bookmarks.
    pub(crate) fn space_shown(&mut self, old: &[Tab]) {
        self.note_new_tabs(old);
        let profile = self.model.profile();
        if let Some((private, profile_id)) = profile {
            self.load_favicons(private, profile_id);
        }
        if profile != self.bookmarks_of {
            self.bookmarks_of = profile;
            self.set_bookmarks(profile.map(|(private, id)| self.store(private).bookmarks(id).unwrap_or_default()).unwrap_or_default());
        }
    }

    /// A space's tabs, folders and groups as stored, with its profile's site icons loaded.
    pub(crate) fn space_lists(&mut self, ix: usize) -> browser_core::tabs::Lists {
        if let Some(space) = self.model.spaces.get(ix) {
            self.load_favicons(space.private, space.space.profile_id);
        }
        self.model.space_lists(&self.core, ix)
    }

    /// Stored icons are small 64 px PNGs, so a profile's set is read once and kept in memory.
    fn load_favicons(&mut self, private: bool, profile_id: Id) {
        let store = self.core.store(private);
        self.favicons.load(&(private, profile_id), || {
            let icons = store.favicons(profile_id).unwrap_or_default();
            let icons = icons.into_iter().map(|(host, png)| (host, Arc::new(Image::from_bytes(ImageFormat::Png, png)))).collect();
            (icons, store.stale_favicons(profile_id, now() - favicon::MAX_AGE).unwrap_or_default())
        });
        self.favicon_memo.take();
    }

    fn runtime(&mut self, private: bool, profile_id: Id) -> Option<Ref<'_, ProfileRuntime>> {
        let downloads = || -> Rc<dyn Fn(Event)> {
            let events = self.core.downloads.clone();
            Rc::new(move |event| drop(events.unbounded_send((private, profile_id, event))))
        };
        if self.core.open_profile(private, profile_id, downloads)? {
            let profiles = self.core.profiles.borrow();
            let data = &profiles[&(private, profile_id)].data;
            self.core.apply_blocking_to(data);
            data.set_sort_downloads(self.pref(crate::prefs::Pref::SortDownloads));
            drop(profiles);
            self.core.apply_extensions(private, profile_id);
            if !private {
                crate::tarn_ext::opened(&self.core, profile_id);
            }
            self.apply_site_styles(private, profile_id);
            crate::zaps::apply(&self.core, private, profile_id);
            // Chromium refuses a profile's requests until its Shield choices arrive.
            #[cfg(windows)]
            self.core.sync_shield();
        }
        self.loaded(private, profile_id)
    }

    /// The runtime of a profile already opened; the borrow ends with the returned guard.
    pub(crate) fn loaded(&self, private: bool, profile_id: Id) -> Option<Ref<'_, ProfileRuntime>> {
        Ref::filter_map(self.core.profiles.borrow(), |p| p.get(&(private, profile_id))).ok()
    }

    pub fn set_filter_updates(&mut self, on: bool, cx: &mut Context<Self>) {
        match self.core.disk.set_setting(filters::SETTING, if on { "on" } else { "off" }) {
            Ok(()) => {
                self.core.filters.borrow_mut().updates = on;
                if self.warning.as_deref().is_some_and(|w| w.starts_with(FILTER_SETTING_WARNING)) {
                    self.warning = None;
                }
            }
            Err(e) => self.warning = Some(format!("{FILTER_SETTING_WARNING} ({e}).")),
        }
        cx.notify();
    }

    fn current_runtime(&mut self) -> Option<Ref<'_, ProfileRuntime>> {
        let space = self.model.current_space()?;
        let (private, profile_id) = (space.private, space.space.profile_id);
        self.runtime(private, profile_id)
    }

    pub fn level_for(&self, url: &str) -> ShieldLevel {
        let Some((private, profile_id)) = self.model.profile() else { return ShieldLevel::default() };
        self.core.level(private, profile_id, Url::parse(url).ok().as_ref()).unwrap_or_default()
    }

    pub fn has_override(&self, host: &str) -> bool {
        self.model.profile().is_some_and(|(private, profile_id)| self.core.has_override(private, profile_id, host))
    }

    /// Events are tagged with the view that sent them, so a replaced view's late events are dropped.
    pub(crate) fn sink(&mut self, key: TabKey) -> Rc<dyn Fn(Event)> {
        self.next_generation += 1;
        let generation = self.next_generation;
        self.view_generation.insert(key, generation);
        let events = self.events.clone();
        Rc::new(move |event| {
            events.unbounded_send(Msg::Tab(key, generation, event)).ok();
        })
    }

    /// Events of the link peek's view, tagged so a closed peek's late events are dropped.
    pub(crate) fn peek_sink(&mut self) -> (u64, Rc<dyn Fn(Event)>) {
        self.next_generation += 1;
        let (generation, events) = (self.next_generation, self.events.clone());
        (generation, Rc::new(move |event| drop(events.unbounded_send(Msg::Peek(generation, event)))))
    }

    #[cfg(feature = "chromium")]
    pub(crate) fn web_engine_for(&mut self, key: TabKey, url: &str) -> WebEngine {
        let tab = self.model.tab(key).and_then(|t| t.web_engine);
        let Some((private, profile_id)) = self.model.profile() else { return WebEngine::WebKit };
        self.runtime(private, profile_id);
        self.core.web_engine(private, profile_id, tab, Url::parse(url).ok().as_ref()).unwrap_or(WebEngine::WebKit)
    }

    /// A web view of the profile's data store with the Shield rules for `url`.
    pub(crate) fn web_view(&mut self, host: &Host, private: bool, profile_id: Id, url: &str, sink: Rc<dyn Fn(Event)>) -> Option<WebView> {
        let developer = self.developer;
        let sets = self.core.sets.clone();
        let rt = self.runtime(private, profile_id)?;
        let site_for = browser_core::site::site_for(rt.profile.clone(), rt.overrides.clone(), rt.rules.clone(), sets);
        let site = Url::parse(url).map_or(nav_policy::Site::new(rt.profile.shield_level), |u| site_for(&u));
        let inspectable = (developer || rt.profile.is_dev) && site.level != ShieldLevel::Lockdown;
        Some(WebView::new(host, &rt.data, site.level, site.filtered, inspectable, site_for, sink))
    }

    #[cfg_attr(not(feature = "chromium"), allow(unused_variables))]
    pub(crate) fn create_view(&mut self, key: TabKey, url: &str, cx: &mut Context<Self>) -> Option<Rc<WebView>> {
        let host = self.host.clone()?;
        let sink = self.sink(key);
        let space = self.model.current_space()?;
        let (private, profile_id) = (space.private, space.space.profile_id);
        if self.core.awaits_extensions(private, profile_id) {
            return None;
        }
        #[cfg(feature = "chromium")]
        let wants_chromium = !private && self.web_engine_for(key, url) == WebEngine::Chromium;
        #[cfg(feature = "chromium")]
        if wants_chromium && !self.chromium_current(key, cx) {
            return None;
        }
        #[cfg(feature = "chromium")]
        let developer = self.developer;
        #[cfg(feature = "chromium")]
        let chromium = wants_chromium
            .then(|| self.runtime(private, profile_id))
            .flatten()
            .and_then(|rt| WebView::chromium(&host, &rt.data, &data_dir().join("Chromium"), &rt.own_store, developer || rt.profile.is_dev, sink.clone()));
        #[cfg(not(feature = "chromium"))]
        let chromium = None;
        #[cfg(feature = "chromium")]
        if chromium.is_some() {
            if self.core.chromium_profile.get() != Some(profile_id) {
                crate::shared::count_chromium_blocks(&self.core);
            }
            let owner_changed = self.core.chromium_profile.replace(Some(profile_id)) != Some(profile_id);
            self.core.sync_shield();
            if owner_changed {
                crate::tarn_ext::chromium_owner_changed(&self.core, profile_id);
            }
            let ageing = browser_core::staleness::Staleness::Ageing;
            if self.core.chromium_staleness.get() == ageing && self.pages.get(&key).is_none_or(|p| p.notice.is_none()) {
                self.notify_offer(key, ageing.message().to_owned(), None);
            }
        }
        let view = Rc::new(match chromium {
            Some(view) => view,
            None => self.web_view(&host, private, profile_id, url, sink)?,
        });
        view.set_corner_radius(page_corner());
        view.set_dark(!ui::theme::Theme::is_light());
        view.set_gestures(self.pref(crate::prefs::Pref::Gestures));
        self.views.insert(key, view.clone());
        self.attach_tarn(key);
        Some(view)
    }

    /// Typed and incoming URLs are user-initiated, so stripping tracking parameters here keeps
    /// the request's context; links from pages are left alone.
    pub fn clean_user_url(&self, url: Url) -> Url {
        self.core.clean_user_url(self.model.profile(), url)
    }

    /// `clean_user_url` for a load the user starts now; removed parameters count on the current
    /// profile's Shield card.
    fn clean_user_load(&mut self, url: Url, cx: &mut Context<Self>) -> Url {
        let clean = self.clean_user_url(url.clone());
        if let Some((private, profile_id)) = self.model.current_space().map(|s| (s.private, s.space.profile_id)) {
            self.count_shield(private, profile_id, Counter::TrackingParams, shield::url_clean::removed_params(&url, &clean), clean.host_str(), cx);
        }
        clean
    }

    /// Counts one event for the site the tab shows, as the page that caused it.
    fn count_on_page(&mut self, key: TabKey, counter: Counter, cx: &mut Context<Self>) {
        let host = self.tab_url(key).and_then(|u| Url::parse(&u).ok()).and_then(|u| page_host(&u));
        if let Some(profile_id) = self.profile_of(key) {
            self.count_shield(key.private, profile_id, counter, 1, host.as_deref(), cx);
        }
    }

    /// Adds to a Shield card counter and repaints every window, so an open card follows.
    pub(crate) fn count_shield(&mut self, private: bool, profile_id: Id, counter: Counter, n: i64, host: Option<&str>, cx: &mut Context<Self>) {
        if n <= 0 || self.store(private).count(profile_id, counter, n, host).is_err() {
            return;
        }
        crate::shield_card::touch();
        crate::shared::others(cx, |_, _, cx| cx.notify());
        cx.notify();
    }

    fn current_download_ix(&self) -> Option<usize> {
        let space = self.model.current_space()?;
        self.download.iter().position(|d| d.private == space.private && d.profile_id == space.space.profile_id)
    }

    pub fn current_download(&self) -> Option<&DownloadNotice> {
        self.current_download_ix().map(|ix| &self.download[ix])
    }

    pub(crate) fn push_download_notice(&mut self, notice: DownloadNotice) {
        browser_core::toast::push_notice(&mut self.download, notice, DownloadNotice::warns);
    }

    pub(crate) fn dismiss_download(&mut self) {
        if let Some(ix) = self.current_download_ix() {
            self.download.remove(ix);
        }
    }

    pub fn active_view(&self) -> Option<Rc<WebView>> {
        self.views.get(&self.model.active?).cloned()
    }

    pub fn activate(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.tab(key).is_none() {
            return;
        }
        let switched = self.model.active != Some(key);
        if switched {
            self.leave_page_fullscreen(window);
        }
        let mut fading = false;
        if switched
            && let Some((outgoing, _)) = self.page_fade.take()
            && !self.views.get(&key).is_some_and(|v| Rc::ptr_eq(v, &outgoing))
        {
            outgoing.set_visible(false);
        }
        let partners = self.split_partners(key, cx);
        if let Some(prev) = self.model.active.filter(|p| *p != key && !partners.contains(p)).and_then(|p| self.views.get(&p)).cloned() {
            fading = partners.is_empty() && !self.pref(crate::prefs::Pref::ReduceMotion) && !self.overlay_open() && self.active_json().is_none();
            if fading {
                self.page_fade = Some((prev, Instant::now()));
            } else {
                prev.set_visible(false);
            }
        }
        let left = Instant::now();
        self.seen.extend(self.model.active.into_iter().chain([key]).map(|k| (k, left)));
        let evicted = self.model.lifecycle.activate_with(&partners, key);
        for evicted in evicted {
            if evicted == key || partners.contains(&evicted) {
                // The split's panes stay awake above the live-tab limit while it lasts.
                self.model.lifecycle.keep(evicted);
            } else {
                self.sleep_unless_playing(evicted, cx);
            }
        }
        let view = self.wake(key, cx);
        self.model.active = Some(key);
        if let Some(view) = view {
            if fading {
                view.raise();
                view.set_alpha(0.);
            } else if switched {
                view.set_alpha(1.);
            }
            view.set_visible(!self.overlay_open());
            if !self.overlay_open() {
                view.focus();
            }
        }
        if !partners.is_empty() {
            // A page still fading out would show between the panes.
            if let Some((outgoing, _)) = self.page_fade.take() {
                outgoing.set_visible(false);
            }
            for pane in &partners {
                if let Some(view) = self.wake(*pane, cx) {
                    view.set_alpha(1.);
                    view.set_visible(!self.overlay_open());
                }
            }
        }
        self.model.touch(&self.core, key, now());
        if !self.overlay_open() {
            window.focus(&self.focus);
        }
        if switched {
            self.layout_changed();
            if self.active_prompt().is_some() {
                self.prepare_prompt(window, cx);
            }
        }
        cx.notify();
    }

    /// The tab's view: this window's, else one taken over from another window, else a new one
    /// that restores the tab's saved state or loads its page.
    fn wake(&mut self, key: TabKey, cx: &mut Context<Self>) -> Option<Rc<WebView>> {
        if let Some(view) = self.views.get(&key).cloned().or_else(|| self.adopt_view(key, cx)) {
            return Some(view);
        }
        let tab = self.model.tab(key).cloned()?;
        let as_link = self.link_loads.get(&key).cloned();
        self.create_view(key, &tab.url, cx).inspect(|view| {
            let restorable = self.level_for(&tab.url).policy().save_interaction_state;
            let saved = self.store(key.private).interaction_state(key.id).ok().flatten();
            // Windows saves Tarn's own format; anything else loads the tab's page instead.
            let ours = |state: &&[u8]| !cfg!(windows) || engine::view_policy::SavedHistory::from_bytes(state).is_some();
            let url = Url::parse(&tab.url).ok();
            // A restored history is never an Open File (S6), so an opened file loads afresh.
            let opened = url.is_some() && self.pages.get(&key).and_then(|p| p.opened_file.as_ref()) == url.as_ref();
            match saved.as_deref().filter(|state| !opened && restorable && !view.is_chromium() && ours(state)) {
                Some(state) => view.restore_interaction_state(state),
                None => {
                    if let Some(url) = url {
                        match &as_link {
                            _ if opened => view.load_file(&url),
                            // Says why a file tab no Open File vouches for stays empty.
                            _ if url.scheme() == "file" => view.load(&url),
                            _ if nav_policy::decide(&url, Source::Sync) != Decision::Allow => {}
                            Some(from) => view.load_link(&url, from.as_ref()),
                            None => view.load(&url),
                        }
                    }
                }
            }
        })
    }

    fn save_state(&self, key: TabKey) {
        let Some(view) = self.views.get(&key) else { return };
        let url = view.url().map(String::from).unwrap_or_default();
        let state = self.level_for_tab(key, &url).policy().save_interaction_state.then(|| view.interaction_state()).flatten();
        self.store(key.private).set_interaction_state(key.id, state.as_deref()).ok();
    }

    /// Whether `key` still shows the view a request was sent to; tab ids are reused after a close.
    pub(crate) fn same_view(&self, key: TabKey, asked: &std::rc::Weak<WebView>) -> bool {
        asked.upgrade().is_some_and(|v| self.views.get(&key).is_some_and(|live| Rc::ptr_eq(live, &v)))
    }

    /// Drops this window's view of `key`, and the fade still showing it.
    fn take_view(&mut self, key: TabKey) -> Option<Rc<WebView>> {
        let view = self.views.remove(&key)?;
        if self.page_fade.as_ref().is_some_and(|(fading, _)| Rc::ptr_eq(fading, &view)) {
            self.page_fade = None;
        }
        Some(view)
    }

    pub(crate) fn hibernate(&mut self, key: TabKey, cx: &mut Context<Self>) {
        self.save_state(key);
        self.take_view(key);
        self.json_pages.remove(&key);
        self.drop_reader(key);
        self.view_generation.remove(&key);
        self.drop_prompts(key, cx);
        self.model.lifecycle.remove(key);
        if let Some(page) = self.pages.get_mut(&key) {
            page.loading = false;
        }
        if self.model.active == Some(key) {
            self.model.active = None;
        }
    }

    pub(crate) fn save_all(&self) {
        for key in self.views.keys() {
            self.save_state(*key);
        }
    }

    pub fn new_tab(&mut self, url: &Url, window: &mut Window, cx: &mut Context<Self>) {
        self.new_tab_in(url, None, window, cx);
    }

    /// A new tab whose `tabs.web_engine` is `web_engine`, as for a copy or a pop-up of a tab.
    pub(crate) fn new_tab_in(&mut self, url: &Url, web_engine: Option<WebEngine>, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.burner_shown()
            && let Some(key) = self.model.tabs.first().map(|t| self.model.key(t))
        {
            self.activate(key, window, cx);
            return self.navigate(url, window, cx);
        }
        let Some(key) = self.model.new_tab(&self.core, url.as_str(), web_engine.or(self.saved.web_engine)) else { return };
        self.reload_space();
        self.activate(key, window, cx);
    }

    pub fn navigate(&mut self, url: &Url, window: &mut Window, cx: &mut Context<Self>) {
        match (self.active_view(), self.model.active) {
            (Some(view), Some(key)) if view.is_chromium() && self.level_for(url.as_str()) == ShieldLevel::Lockdown => self.rebuild_view(key, url, cx),
            (Some(view), _) => view.load(url),
            (None, Some(key)) => {
                self.store(key.private).update_tab_page(key.id, url.as_str(), "").ok();
                self.reload_space();
                self.activate(key, window, cx);
            }
            (None, None) => self.new_tab(url, window, cx),
        }
    }

    pub fn close_tab(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tabs(vec![key], window, cx);
    }

    /// Unloads a pinned tab, or closes an unpinned one and returns what reopening it needs. The
    /// caller reloads the space and picks the next active tab.
    fn close_one(&mut self, key: TabKey, cx: &mut Context<Self>) -> Option<ClosedTab> {
        let tab = self.model.tab(key).cloned()?;
        self.splits.leave(key);
        if tab.kind != TabKind::Ephemeral {
            self.hibernate(key, cx);
            return None;
        }
        let closed = self.closed_tab(&tab, key);
        self.tab_leaving(&tab, cx);
        if !key.private {
            self.core.tarn.borrow().send(crate::tarn_ext::Msg::TabClosed(key.id as u64));
        }
        self.take_view(key);
        self.link_loads.remove(&key);
        self.json_pages.remove(&key);
        self.drop_reader(key);
        self.view_generation.remove(&key);
        self.drop_prompts(key, cx);
        self.model.lifecycle.remove(key);
        self.pages.remove(&key);
        self.store(key.private).delete_tab(key.id).ok();
        if self.model.active == Some(key) {
            self.model.active = None;
        }
        Some(closed)
    }

    pub(crate) fn pick_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let adopting = self.adopting;
        match self.model.pick(|k| Some(k) != adopting && crate::shared::live_elsewhere(k, cx)) {
            Some(key) => self.activate(key, window, cx),
            None => {
                self.model.active = None;
                cx.notify();
            }
        }
    }

    pub fn move_tab(&mut self, key: TabKey, kind: TabKind, folder: Option<Id>, index: usize, cx: &mut Context<Self>) {
        if self.model.burner_shown() && kind != TabKind::Ephemeral {
            return;
        }
        if self.store(key.private).move_tab(key.id, kind, folder, index).is_ok() {
            self.reload_space();
            cx.notify();
        }
    }

    /// The `n`th visible tab, counted from 1; 9 is the last, as in other browsers.
    pub fn select_tab(&mut self, n: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.model.slot(n) {
            self.switch_to(key, window, cx);
        }
    }

    pub fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.model.cycled(forward) {
            self.switch_to(key, window, cx);
        }
    }

    /// A tab the user picks; an open address field or launcher closes so the tab shows.
    pub fn switch_to(&mut self, key: TabKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.switch_to(key, window, cx)) {
            return;
        }
        if self.command_mode.is_some() {
            self.close_command(window, cx);
        }
        self.activate(key, window, cx);
    }

    pub fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        if let Some(key) = self.model.active_tab().map(|t| self.model.key(t)) {
            self.toggle_pin_tab(key, cx);
        }
    }

    pub fn new_folder(&mut self, parent: Option<Id>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(folder) = self.model.new_folder(&self.core, parent) else { return };
        self.reload_space();
        self.begin_rename(crate::groups::RenameTarget::Folder(folder.id), window, cx);
    }

    pub fn move_folder(&mut self, id: Id, parent: Option<Id>, cx: &mut Context<Self>) {
        if self.model.move_folder(&self.core, id, parent) {
            self.reload_space();
            cx.notify();
        }
    }

    pub fn toggle_folder(&mut self, id: Id, cx: &mut Context<Self>) {
        self.model.toggle_folder(id);
        cx.notify();
    }

    pub fn switch_space(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.model.spaces.len() || self.in_main(window, cx, move |b, window, cx| b.switch_space(ix, window, cx)) {
            return;
        }
        self.commit_rename(cx);
        self.hide_hover_card(cx);
        if self.command_mode.is_some() {
            self.close_command(window, cx);
        }
        if let Some(view) = self.active_view() {
            view.set_visible(false);
        }
        self.motion = Default::default();
        self.floating = Default::default();
        let burning = self.model.burner_shown();
        let Some((from, old)) = self.model.switch_space(&self.core, ix) else { return };
        self.motion.forget_pointer();
        if from != ix {
            self.space_switched_at = Some(now());
        }
        self.current_runtime();
        if self.responsive.is_some() && !self.devtools_on() {
            self.set_responsive(None, cx);
        }
        self.space_shown(&[]);
        self.slide_from(from, old);
        self.pick_active(window, cx);
        if burning && !self.model.burner_shown() {
            self.burn(cx);
        }
        cx.notify();
    }

    pub fn open_private_space(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.model.private_space(&self.core) {
            self.switch_space(ix, window, cx);
        }
    }

    pub fn open_command(&mut self, mode: CommandMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.open_command(mode, window, cx)) {
            return;
        }
        self.new_tab_group = None;
        self.overlay_exit = None;
        self.shortcuts_open = false;
        let text = match mode {
            CommandMode::Current => {
                self.active_view().and_then(|v| v.url()).map(String::from).or_else(|| self.model.active_tab().map(|t| t.url.clone())).unwrap_or_default()
            }
            CommandMode::NewTab | CommandMode::FindInTabs => String::new(),
        };
        let placeholder = if mode == CommandMode::FindInTabs { "Find in this space's tabs" } else { "Search or enter address" };
        let private = self.model.private();
        self.history_cache = self.model.current_space().map(|s| s.space.profile_id).and_then(|p| self.store(private).history(p, 2000).ok()).unwrap_or_default();
        self.history_text = None;
        self.command_mode = Some(mode);
        if self.devtools_on() {
            self.refresh_local_servers(cx);
        }
        self.command.update(cx, |input, cx| {
            input.set_placeholder(placeholder);
            input.set_text(text, cx);
        });
        if self.model.active.is_some() && !self.profile_editor.as_ref().is_some_and(|e| !e.floating) {
            self.open_inline(window, cx);
        } else if let Some(view) = self.active_view() {
            view.set_visible(false);
        }
        if let Some(host) = &self.host {
            host.focus();
        }
        let focus = self.command.focus_handle(cx);
        window.focus(&focus);
        self.update_suggestions(cx);
        cx.notify();
    }

    /// F6 and Shift+F6: moves the keyboard between the page, the address field and the sidebar.
    pub(crate) fn cycle_focus(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        use browser_core::focus::{Region, next_region};
        let open = self.command_mode.is_some();
        let from = if open {
            Region::Address
        } else if self.focus.is_focused(window) {
            Region::Page
        } else {
            Region::Sidebar
        };
        match next_region(from, back, self.active_view().is_some(), self.sidebar_shown() > 0.) {
            Region::Address => self.open_command(CommandMode::Current, window, cx),
            Region::Page if open => self.close_command(window, cx),
            Region::Page => {
                window.focus(&self.focus);
                if let Some(view) = self.active_view() {
                    view.focus();
                }
            }
            Region::Sidebar => {
                if open {
                    self.close_command(window, cx);
                }
                if let Some(host) = &self.host {
                    host.focus();
                }
                window.focus(&self.focus);
                window.focus_next();
            }
        }
    }

    pub fn close_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let field = self.command_mode.is_some();
        let covered = field || self.shortcuts_open;
        self.launcher_from = None;
        self.close_inline(cx);
        self.overlay_exit = None;
        self.command_mode = None;
        self.history_text_search = None;
        self.profile_editor = None;
        self.shortcuts_open = false;
        self.reader = self.reader.take().filter(|r| field || crate::reader::keeps_reader(covered, r.speaking()));
        window.focus(&self.focus);
        for key in self.model.active.and_then(|a| Some(self.splits.of(a)?.others(a))).unwrap_or_default() {
            if let Some(view) = self.views.get(&key) {
                view.set_visible(true);
            }
        }
        if let Some(view) = self.active_view().filter(|_| !self.overlay_open()) {
            view.set_visible(true);
            view.focus();
        }
        self.layout_changed();
        self.prepare_prompt(window, cx);
        cx.notify();
    }

    /// The command card and profile editor replace the page, so the native web view is hidden under
    /// them; the address field opened in the toolbar leaves it visible.
    pub fn overlay_open(&self) -> bool {
        self.card_open() || self.reader_shown()
    }

    /// An overlay other than the reader.
    pub(crate) fn card_open(&self) -> bool {
        (self.command_mode.is_some() && !self.inline_editing()) || self.profile_editor.as_ref().is_some_and(|e| !e.floating) || self.shortcuts_open
    }

    pub(crate) fn reader_shown(&self) -> bool {
        self.reader.as_ref().is_some_and(|r| self.model.active == Some(r.key))
    }

    pub(crate) fn select_suggestion(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.select_suggestion_at(suggest::step_selection(self.selected, self.suggestions.len(), forward), cx);
    }

    pub(crate) fn select_suggestion_at(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = ix.min(self.suggestions.len().saturating_sub(1));
        self.suggestions_scroll.scroll_to_item(self.selected);
        cx.notify();
    }

    /// Tab, Page Up and Page Down move the highlight in the field's suggestion list.
    pub(crate) fn suggestion_key(&mut self, event: &gpui::KeyDownEvent, cx: &mut Context<Self>) {
        let last = self.suggestions.len().saturating_sub(1);
        match event.keystroke.key.as_str() {
            "tab" => self.select_suggestion(!event.keystroke.modifiers.shift, cx),
            "pageup" => self.select_suggestion_at(0, cx),
            "pagedown" => self.select_suggestion_at(last, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn on_command_event(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Changed => self.update_suggestions(cx),
            TextInputEvent::Submit => self.submit_command(self.selected, window, cx),
            TextInputEvent::Cancel => self.dismiss_overlay(window, cx),
            TextInputEvent::Up => self.select_suggestion(false, cx),
            TextInputEvent::Down => self.select_suggestion(true, cx),
            TextInputEvent::MenuClosed if !window.is_window_active() && self.inline_editing() => self.close_command(window, cx),
            TextInputEvent::MenuClosed => {}
        }
    }

    /// "Switch to" and "Move Tab to" rows for the other spaces, ranked like the command table.
    fn space_commands(&self, query: &str) -> Vec<Suggestion> {
        let mut rows: Vec<(String, Box<dyn gpui::Action>)> = Vec::new();
        for (ix, s) in self.model.spaces.iter().enumerate().filter(|(ix, _)| *ix != self.model.space_ix) {
            rows.push((format!("Switch to {}", ui::text::bounded_title(&s.space.name)), Box::new(crate::menubar::SwitchToSpace { ix })));
        }
        if let Some(key) = self.model.active {
            for ix in self.model.move_targets(&self.core, key) {
                let name = ui::text::bounded_title(&self.model.spaces[ix].space.name);
                rows.push((format!("Move Tab to {name}"), Box::new(crate::menubar::MoveToSpace { ix })));
            }
        }
        command::rank_commands(query, rows.iter().map(|(label, _)| (label.as_str(), "space workspace")))
            .into_iter()
            .map(|ix| Suggestion { label: rows[ix].0.clone(), detail: String::new(), target: Target::Action(rows[ix].1.boxed_clone()) })
            .collect()
    }

    fn update_suggestions(&mut self, cx: &mut Context<Self>) {
        self.suggest(None, cx);
    }

    /// Builds the suggestions for the field's text, or `again` for the query and completion they
    /// were last built for.
    fn suggest(&mut self, again: Option<(String, Option<suggest::Completion>)>, cx: &mut Context<Self>) {
        let rebuilt = again.is_some();
        let (query, kept) = again.map_or_else(|| (self.command.read(cx).text().trim().to_owned(), None), |(q, c)| (q, Some(c)));
        if !rebuilt {
            (self.history_text_search, self.suggested) = (None, None);
        }
        if self.command_mode == Some(CommandMode::FindInTabs) {
            return self.find_in_tabs(query, cx);
        }
        let mut out = Vec::new();
        let mut commands = command::rank_commands(&query, crate::COMMANDS.iter().map(|c| (c.0, c.1)));
        let has_site = self.site().is_some();
        commands.retain(|ix| has_site || !crate::COMMANDS[*ix].2.partial_eq(&crate::site_settings::SiteSettings));
        // Which engine the tab can switch to reads the profile and a lock file, so only when a
        // switch command is ranked at all.
        let switching = commands.iter().any(|ix| {
            let action = crate::COMMANDS[*ix].2;
            action.partial_eq(&crate::tabs::ReloadInChromium) || action.partial_eq(&crate::tabs::ReloadInWebKit)
        });
        let to = self.model.active.filter(|_| switching).and_then(|k| self.web_engine_switch(k));
        let private = self.model.private();
        let zappable = self.site().is_some_and(|(k, u)| self.zappable(k, &u));
        let titled = self.model.active_tab().is_some_and(|t| !t.custom_title.is_empty());
        let drifted = self.model.active.and_then(|k| self.pinned_drift(k)).is_some();
        let split = self.model.active.is_some_and(|a| self.splits.contains(a));
        let pinned = self.model.active_tab().is_some_and(|t| t.kind != store::TabKind::Ephemeral);
        let muted = self.model.active.is_some_and(|k| self.tab_muted(k));
        let reopenable = !self.model.closed_tabs.is_empty();
        let toast_command = self.toast_command();
        let duplicates = !self.duplicate_tabs().is_empty();
        commands.retain(|ix| {
            let action = crate::COMMANDS[*ix].2;
            let is = |a: &dyn gpui::Action| action.partial_eq(a);
            !(is(&crate::tabs::ReloadInChromium) && to != Some(WebEngine::Chromium)
                || is(&crate::tabs::ReloadInWebKit) && to != Some(WebEngine::WebKit)
                || private && (is(&crate::library::SaveSnapshot) || is(&crate::library::RestoreSnapshot))
                || !zappable && is(&crate::zaps::ZapElement)
                || !titled && is(&crate::tabs::ResetTabTitle)
                || !drifted && (is(&crate::tabs::ReturnToPinnedUrl) || is(&crate::tabs::SetPinnedUrl))
                || !reopenable && is(&crate::ReopenClosedTab)
                || toast_command.is_none() && is(&crate::toast::ToastAction)
                || !duplicates && is(&crate::tabs::CloseDuplicateTabs)
                || !split
                    && (is(&crate::split::FocusNextPane)
                        || is(&crate::split::FocusPreviousPane)
                        || is(&crate::split::SwapPanes)
                        || is(&crate::split::RotateSplitLayout)
                        || is(&crate::split::RemoveFromSplitView)
                        || is(&crate::split::CloseSplitView)))
        });
        let command = |ix: usize| {
            let (label, _, action) = crate::COMMANDS[ix];
            let is = |a: &dyn gpui::Action| action.partial_eq(a);
            let on = is(&crate::TogglePin) && pinned || is(&crate::menubar::ToggleMute) && muted || is(&crate::split::ToggleSplitView) && split;
            let shown = crate::tabs::state_label(action, on).map(str::to_owned);
            let label = shown.or_else(|| toast_command.clone().filter(|_| is(&crate::toast::ToastAction))).unwrap_or_else(|| label.to_owned());
            Suggestion { label, detail: crate::shortcuts::shown(action, cx), target: Target::Input(Input::Command(ix)) }
        };
        let space_rows = self.space_commands(&query);
        if query.starts_with('>') {
            self.suggestions = commands.into_iter().map(command).chain(space_rows).collect();
            self.select_suggestion_at(0, cx);
            return;
        }
        let engine = &command::ENGINES[self.search_engine()];
        let input = command::classify_input(&query, engine);
        if let Some(Input::Navigate(u)) = &input {
            out.extend(self.open_tabs_at(&self.clean_user_url(u.clone())));
        }
        let exact = commands.first().is_some_and(|ix| crate::COMMANDS[*ix].0.eq_ignore_ascii_case(&query));
        if exact {
            out.extend(commands.iter().copied().map(command));
        }
        if let Some(input) = input {
            let (label, detail) = match &input {
                Input::Navigate(u) => (u.to_string(), "Open".to_owned()),
                Input::Search(_) => (query.clone(), format!("Search {}", engine.name)),
                Input::External(u) => (u.to_string(), format!("Open in another app ({})", u.scheme())),
                Input::Rejected => (query.clone(), "This kind of address cannot be opened".to_owned()),
                Input::Command(_) => unreachable!("classify_input never names a command"),
            };
            out.push(Suggestion { label, detail, target: Target::Input(input) });
        }
        if !exact {
            out.extend(commands.into_iter().map(command));
        }
        out.extend(space_rows);
        let suggest_on = self.pref(crate::prefs::Pref::Suggestions);
        let private = self.model.private();
        let tabs = &self.model.tabs;
        let mut pages: Vec<Page> = tabs.iter().map(Page::from).collect();
        if !query.is_empty() {
            pages.extend(self.history_cache.iter().map(Page::from));
        }
        let field = self.command.read(cx);
        let completion = kept.unwrap_or_else(|| {
            (suggest_on && !exact && suggest::may_complete(field.text(), field.inserted(), field.caret_at_end()))
                .then(|| suggest::inline_completion(&query, &pages, now()))
                .flatten()
        });
        if let Some(c) = &completion {
            let shown = format!("{query}{}", c.suffix);
            // Enter opens the visited page the field completes to, or the tab already showing it.
            let (target, detail) = match tabs.iter().find(|t| t.url == c.url.as_str() && self.model.active != Some(TabKey { private, id: t.id })) {
                Some(t) => (Target::Tab(TabKey { private, id: t.id }), "Switch to Tab".to_owned()),
                None => {
                    let detail = self.clean_user_url(c.url.clone()).as_str().trim_start_matches("https://").trim_end_matches('/').to_owned();
                    (Target::Input(Input::Navigate(c.url.clone())), detail)
                }
            };
            out.insert(0, Suggestion { label: ui::text::bounded_url(&shown), detail: ui::text::bounded_url(&detail), target });
        }
        let typed = out.len();
        let listed: HashSet<TabKey> = out.iter().filter_map(|s| if let Target::Tab(k) = s.target { Some(k) } else { None }).collect();
        let completed = completion.as_ref().map(|c| c.url.as_str());
        let skip = |ix: usize| {
            if ix < tabs.len() {
                let key = TabKey { private, id: tabs[ix].id };
                listed.contains(&key) || self.model.active == Some(key)
            } else {
                Some(pages[ix].url) == completed
            }
        };
        let mut ranked: Vec<usize> = suggest::rank_rows(&query, &pages, tabs.len(), skip, now());
        let search_text = !private && suggest::searches_text(&query) && suggest_on && self.pref(crate::prefs::Pref::HistoryText);
        let text_profile = self.model.current_space().filter(|_| search_text).map(|s| s.space.profile_id);
        let cached = self.history_text.as_ref().filter(|(p, q, _)| Some(*p) == text_profile && *q == query).map(|(_, _, found)| found.clone());
        let search_later = text_profile.filter(|_| cached.is_none());
        let found = cached.unwrap_or_default();
        // Text matches get up to half the room left; a page in both lists shows once, where it ranks if it stays there.
        let room = MAX_SUGGESTIONS.saturating_sub(out.len());
        let open: HashSet<&str> = tabs.iter().map(|t| t.url.as_str()).collect();
        let found: Vec<&HistoryEntry> = found.iter().filter(|f| !open.contains(f.url.as_str()) && Some(f.url.as_str()) != completed).collect();
        let top = &ranked[..ranked.len().min(room)];
        let extra = found.iter().filter(|f| !top.iter().any(|&ix| pages[ix].url == f.url)).count();
        ranked.truncate(room - extra.min(room / 2));
        let mut found: Vec<&HistoryEntry> = found.into_iter().filter(|f| !ranked.iter().any(|&ix| pages[ix].url == f.url)).collect();
        found.truncate(room - ranked.len());
        let history_row = |url: &str, title: &str| {
            Url::parse(url).ok().map(|target| Suggestion {
                label: ui::text::bounded_title(if title.is_empty() { url } else { title }),
                detail: ui::text::bounded_url(url.trim_start_matches("https://").trim_end_matches('/')),
                target: Target::Url(target),
            })
        };
        for ix in ranked {
            let page = pages[ix];
            if ix < tabs.len() {
                out.push(Suggestion {
                    label: ui::text::bounded_title(if page.title.is_empty() { page.url } else { page.title }),
                    detail: "Switch to Tab".to_owned(),
                    target: Target::Tab(TabKey { private, id: tabs[ix].id }),
                });
            } else {
                out.extend(history_row(page.url, page.title));
            }
        }
        out.extend(found.into_iter().filter_map(|h| history_row(&h.url, &h.title)));
        out.truncate(if suggest_on { MAX_SUGGESTIONS } else { typed });
        // Late text matches move rows; the highlight stays on the row the user chose, if it is still listed.
        // An empty field highlights nothing, so Enter opens nothing until the user picks a row.
        let chosen = self.suggestions.get(self.selected).filter(|_| rebuilt).map(|s| (s.label.clone(), s.detail.clone()));
        self.suggestions = out;
        let first = if query.is_empty() { usize::MAX } else { 0 };
        self.selected = chosen.and_then(|(label, detail)| self.suggestions.iter().position(|s| s.label == label && s.detail == detail)).unwrap_or(first);
        self.suggested = Some((query.clone(), completion.clone()));
        self.history_text_search = search_later.map(|profile| self.search_history_text_later(profile, query, cx));
        if let (Some(c), false) = (completion, rebuilt) {
            self.command.update(cx, |input, cx| input.append_selected(&c.suffix, cx));
        }
        cx.notify();
    }

    pub(crate) fn forget_text_matches(&mut self) {
        (self.history_text, self.history_text_search) = (None, None);
    }

    fn search_history_text_later(&self, profile: Id, query: String, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(suggest::TEXT_SEARCH_DELAY).await;
            this.update(cx, |b, cx| {
                let found = b.core.disk.search_history_text(profile, &query, MAX_TEXT_MATCHES).unwrap_or_default();
                b.history_text = Some((profile, query.clone(), found));
                if b.command_mode.is_some()
                    && let Some(again) = b.suggested.clone().filter(|(q, _)| *q == query)
                {
                    b.suggest(Some(again), cx);
                }
            })
            .ok();
        })
    }

    pub fn submit_command(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mode) = self.command_mode else { return };
        let Some(suggestion) = self.suggestions.get(ix) else { return };
        // A search's results page is not an address the user typed, so it earns no completion credit.
        let credit = !matches!(suggestion.target, Target::Input(Input::Search(_)));
        let url = match &suggestion.target {
            Target::Tab(key) => {
                let key = *key;
                self.close_command(window, cx);
                if self.model.tab(key).is_none()
                    && let Ok(tab) = self.store(key.private).tab(key.id)
                    && let Some(ix) = self.model.spaces.iter().position(|s| s.private == key.private && s.space.id == tab.space_id)
                {
                    self.switch_space(ix, window, cx);
                }
                self.activate(key, window, cx);
                return;
            }
            Target::Find(key, query) => {
                let (key, query) = (*key, query.clone());
                self.close_command(window, cx);
                return self.open_text_match(key, query, window, cx);
            }
            Target::Input(Input::Command(ix)) => {
                let action = crate::COMMANDS[*ix].2.boxed_clone();
                self.close_command(window, cx);
                window.dispatch_action(action, cx);
                return;
            }
            Target::Action(action) => {
                let action = action.boxed_clone();
                self.close_command(window, cx);
                window.dispatch_action(action, cx);
                return;
            }
            Target::Url(u) => u.clone(),
            Target::Input(Input::Navigate(u)) => self.clean_user_load(u.clone(), cx),
            Target::Input(Input::Search(u)) => u.clone(),
            Target::Input(Input::External(u)) => {
                self.pending_external = Some(PendingExternal { tab: None, origin: None, url: u.clone() });
                self.layout_changed();
                self.close_command(window, cx);
                return;
            }
            Target::Input(Input::Rejected) => return,
        };
        let group = self.new_tab_group.take();
        self.close_command(window, cx);
        match mode {
            CommandMode::NewTab => {
                let before = self.model.active;
                let routed = self.open_routed(&url, window, cx);
                if !routed {
                    self.new_tab(&url, window, cx);
                    if credit && self.model.active != before {
                        self.mark_typed(&url);
                    }
                }
                if let Some((key, group)) = self.model.active.filter(|k| !routed && Some(*k) != before).zip(group) {
                    self.set_tab_group(key, Some(group), cx);
                }
            }
            CommandMode::Current | CommandMode::FindInTabs => {
                self.navigate(&url, window, cx);
                if credit {
                    self.mark_typed(&url);
                }
            }
        }
    }

    pub(crate) fn mark_typed(&mut self, url: &Url) {
        if let Some(key) = self.model.active {
            self.pages.entry(key).or_default().typed = Some(url.to_string());
        }
    }

    /// "Switch to Tab" rows for open tabs showing `url`: the current space's first, then other
    /// spaces of the same profile and privacy, named in the detail.
    fn open_tabs_at(&self, url: &Url) -> Vec<Suggestion> {
        self.model
            .tabs_showing(&self.core, url)
            .into_iter()
            .map(|(key, t, space)| Suggestion {
                label: ui::text::bounded_title(if t.title.is_empty() { &t.url } else { &t.title }),
                detail: browser_core::tabs::switch_detail(space.as_deref()),
                target: Target::Tab(key),
            })
            .collect()
    }

    pub fn resolve_external(&mut self, open: bool, cx: &mut Context<Self>) {
        if open && (self.pending_external.is_none() || !self.layout_stable()) {
            return;
        }
        if let Some(pending) = self.pending_external.take()
            && open
        {
            engine::open_external(&pending.url);
        }
        cx.notify();
    }

    pub fn dismiss_error(&mut self, cx: &mut Context<Self>) {
        if let Some(page) = self.model.active.and_then(|k| self.pages.get_mut(&k)) {
            page.error = None;
            page.notice = None;
            page.offer = None;
        }
        cx.notify();
    }

    fn on_event(&mut self, key: TabKey, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        // A local-network refusal is counted once, as `Event::Protected`.
        match &event {
            Event::Blocked { category: Some(_), .. } => self.count_on_page(key, Counter::ListedStopped, cx),
            Event::PopupBlocked(url) if !url.as_ref().is_some_and(shield::rules::local_target) => self.count_on_page(key, Counter::PopupsBlocked, cx),
            _ => {}
        }
        match event {
            Event::Loading(on) => self.pages.entry(key).or_default().loading = on,
            Event::Committed { url, title, can_go_back, can_go_forward } => {
                engine::startup::mark("first page commit");
                // A link stays the page's until it shows a page: a view slept or never made loads it as a link again.
                if url.as_str() != "about:blank" {
                    self.link_loads.remove(&key);
                }
                self.drop_prompts(key, cx);
                self.tarn_committed(key, &url, window, cx);
                self.json_pages.remove(&key);
                let previous = self.store(key.private).tab(key.id).ok().and_then(|t| tab_host(&t.url));
                self.page_changed(key, &url, &title, can_go_back, can_go_forward, true);
                // An Open Anyway offer belongs to the page that was refused, not to the next one; the
                // blank document a new view starts with is no page.
                if let Some(page) = self.pages.get_mut(&key).filter(|p| url.as_str() != "about:blank" && matches!(p.offer, Some(Offer::Open(_)))) {
                    (page.notice, page.offer) = (None, None);
                }
                let left_host = previous.as_deref() != url.host_str();
                self.apply_site_zoom(key, Some(&url), left_host);
                self.apply_site_mute(key, &url, left_host);
                let page = self.pages.entry(key).or_default();
                page.blocked = None;
                page.visit = None;
                (page.local_counted, page.https_counted) = (false, false);
                self.forget_article(key, window, cx);
                let unfiltered = self.views.get(&key).is_some_and(|v| !v.is_chromium() && !v.filtered());
                if unfiltered && !crate::blocking::site_host(&url).is_some_and(|h| self.site_paused_in(key, h)) {
                    let message = "Ad, tracker and cookie-banner blocking is off in this tab since it left a site where you paused it.".to_owned();
                    self.notify_offer(key, message, Some(Offer::Reload));
                }
            }
            Event::Finished { url, title, can_go_back, can_go_forward } => {
                self.page_changed(key, &url, &title, can_go_back, can_go_forward, false);
                let page = self.pages.entry(key).or_default();
                page.crashed = false;
                let typed = suggest::typed_visit(&mut page.typed, url.as_str());
                self.record_visit(key, &url, &title, typed);
                self.request_page_text(key, &url);
                self.request_favicon(key, &url);
                #[cfg(feature = "translation")]
                self.offer_translation(key, &url, cx);
                self.probe_article(key, &url);
                self.save_state(key);
                self.after_load_devtools(key, &url);
                if self.find_after_load.take_if(|k| *k == key).is_some() && self.find_open && self.model.active == Some(key) {
                    self.find(false, cx);
                }
            }
            Event::Blocked { url, message, category } => {
                suggest::typed_visit(&mut self.pages.entry(key).or_default().typed, url.as_str());
                match category {
                    Some(category) if category.warns() => self.show_blocked(key, url, category),
                    Some(_) => self.notify_offer(key, message, Some(Offer::Open(url))),
                    // Open Anyway loads it as the user's own navigation, which S10 lets through; the
                    // notice names the address, since the page chose it.
                    None if message == nav_policy::LOCAL_NETWORK_BLOCKED => match nav_policy::open_anyway_notice(&url) {
                        Some(notice) => self.notify_offer(key, notice, Some(Offer::Open(url))),
                        None => self.notify_offer(key, message, None),
                    },
                    None => self.notify_offer(key, message, None),
                }
            }
            Event::StoreInstall(id) => self.store_link(key, id, window, cx),
            Event::PopupBlocked(url) => {
                let message = nav_policy::popup_blocked_notice(url.as_ref());
                let offer = url.filter(|u| nav_policy::decide(u, Source::Typed) == Decision::Allow).map(Offer::NewTab);
                self.notify_offer(key, message, offer);
            }
            Event::PopupOffered(url) => {
                let message = nav_policy::popup_blocked_notice(Some(&url));
                let offer = (nav_policy::decide(&url, Source::Typed) == Decision::Allow).then_some(Offer::NewTab(url));
                self.notify_offer(key, message, offer);
            }
            Event::WebEngineOutdated => {
                #[cfg(any(feature = "chromium", windows))]
                crate::shared::check_chromium(cx);
                #[cfg(windows)]
                if let Some(message) = self.chromium_closed() {
                    self.notify_offer(key, message.to_owned(), None);
                }
            }
            Event::WebEngineRequired(url) => {
                let host = |u: &Url| ui::text::bounded_origin(u.host_str().unwrap_or_default());
                let target = host(&url);
                let message = match self.views.get(&key).and_then(|v| v.url()).filter(|u| u.host_str().is_some()) {
                    Some(from) => format!("{} asked to open {target}. This site is set to Lockdown and opens in WebKit.", host(&from)),
                    None => format!("{target} is set to Lockdown and opens in WebKit."),
                };
                self.notify_offer(key, message, Some(Offer::WebKit(url)));
            }
            Event::Failed { url, code, message } => {
                // The latches stay: a page can make a load fail at will while it keeps showing, and an
                // undercount after a failed upgrade beats a counter a page can run up.
                let page = self.pages.entry(key).or_default();
                page.loading = false;
                if let Some(url) = url {
                    suggest::typed_visit(&mut page.typed, url.as_str());
                }
                if !IGNORED_LOAD_ERRORS.contains(&code) {
                    page.error = Some(message);
                }
            }
            Event::TitleChanged(title) => {
                self.store(key.private).set_tab_title(key.id, &title).ok();
                // WebKit can report an empty title for the outgoing page just before the next one commits.
                if let (false, Some(profile_id), Some((visit, url))) =
                    (title.is_empty(), self.profile_of(key), self.pages.get(&key).and_then(|p| p.visit.clone()))
                {
                    self.store(key.private).retitle_visit(profile_id, visit, &url, &title).ok();
                }
                if self.private_matches(key)
                    && let Some(t) = self.model.tabs.iter_mut().find(|t| t.id == key.id)
                {
                    t.title = title;
                }
            }
            Event::UrlChanged { url, can_go_back, can_go_forward } => {
                let page = self.pages.entry(key).or_default();
                page.can_back = can_go_back;
                page.can_forward = can_go_forward;
                if let Ok(tab) = self.store(key.private).tab(key.id)
                    && tab.url != url.as_str()
                    && nav_policy::decide(&url, Source::Link) == Decision::Allow
                {
                    self.store(key.private).update_tab_page(key.id, url.as_str(), &tab.title).ok();
                    if self.private_matches(key)
                        && let Some(t) = self.model.tabs.iter_mut().find(|t| t.id == key.id)
                    {
                        t.url = url.to_string();
                    }
                    // A same-document navigation (pushState) is a visit like a page load, but pages
                    // rewrite their URL constantly (maps, anchors, search as you type): fragment
                    // changes, very long URLs and more than one visit per tab per interval are not
                    // recorded.
                    let mut previous = Url::parse(&tab.url).ok();
                    if let Some(p) = previous.as_mut() {
                        p.set_fragment(None);
                    }
                    let mut target = url.clone();
                    target.set_fragment(None);
                    // A page that rewrites its own address shows another article, or none; it is asked
                    // again once it has had a moment to render, as no load finishes to tell.
                    if previous.as_ref() != Some(&target) {
                        self.forget_article(key, window, cx);
                        let url = url.clone();
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                            this.update(cx, |b, _| {
                                if b.views.get(&key).and_then(|v| v.url()).as_ref() == Some(&url) {
                                    b.probe_article(key, &url);
                                }
                            })
                            .ok();
                        })
                        .detach();
                    }
                    let page = self.pages.entry(key).or_default();
                    let due = page.last_visit.is_none_or(|at| at.elapsed() >= SAME_DOCUMENT_VISIT_INTERVAL);
                    if previous.as_ref() != Some(&target) && url.as_str().len() <= MAX_SAME_DOCUMENT_URL && due {
                        page.last_visit = Some(Instant::now());
                        self.record_visit(key, &url, &tab.title, false);
                    } else {
                        page.visit = None;
                    }
                }
            }
            Event::Prompt(prompt) => {
                // A prompt shows with the shell's origin chip, never over a full-screen page.
                if self.model.active == Some(key) {
                    self.leave_page_fullscreen(window);
                }
                self.queue_prompt(key, prompt, window, cx);
            }
            Event::Popup(popup) => self.adopt_popup(key, popup, window, cx),
            Event::ShieldLevelChanged(url) => self.rebuild_view(key, &url, cx),
            Event::RebuildForLink(url) => self.rebuild_view_for_link(key, &url, cx),
            Event::PeekLink(url, from) => {
                if self.model.active == Some(key) {
                    self.open_link_peek(key, url, from, window, cx);
                }
            }
            // As a pop-up's tab: never in the burner, and its first load is the page's link, so S10 judges every hop.
            Event::OpenTab { url, from, background, split } => {
                let Some(opener) = self.model.tab(key).filter(|_| !self.model.burner_shown()) else { return cx.notify() };
                let engine = opener.web_engine.or(self.saved.web_engine);
                let routed = self.routed_space(&url);
                if let Some(ix) = routed {
                    self.switch_space(ix, window, cx);
                }
                if let Some(tab) = self.model.new_tab(&self.core, url.as_str(), engine) {
                    self.reload_space();
                    self.link_loads.insert(tab, from.clone());
                    if split && routed.is_none() {
                        self.split_link(key, tab, window, cx);
                    } else if !background || routed.is_some() {
                        self.activate(tab, window, cx);
                    } else if let Some(view) = self.create_view(tab, url.as_str(), cx) {
                        // Loaded now, as a pop-up is: a tab restored later would load its URL as the user's own.
                        view.set_visible(false);
                        view.load_link(&url, from.as_ref());
                    }
                }
            }
            Event::ExternalScheme { url, origin } => {
                let showing = self.pending_external.as_ref().is_some_and(|p| p.tab.is_none_or(|t| t == key));
                if self.model.active == Some(key) && !showing {
                    self.pending_external = Some(PendingExternal { tab: Some(key), origin, url });
                    self.layout_changed();
                }
            }
            event @ (Event::DownloadFinished { .. }
            | Event::DownloadFailed { .. }
            | Event::DownloadCancelled(_)
            | Event::DownloadsActive(_)
            | Event::DownloadProgress { .. }) => {
                if let Some(profile_id) = self.profile_of(key) {
                    self.core.downloads.unbounded_send((key.private, profile_id, event)).ok();
                }
                return;
            }
            Event::Protected(protection, target) => {
                let page = self.pages.entry(key).or_default();
                let (latch, counter) = match protection {
                    engine::Protection::HttpsUpgrade => (&mut page.https_counted, Counter::HttpsUpgrades),
                    engine::Protection::LocalNetwork => (&mut page.local_counted, Counter::LocalBlocked),
                };
                let counter = (!std::mem::replace(latch, true)).then_some(counter);
                // An upgrade is the upgraded site's; a refusal is the page's that tried it, unless the
                // tab shows nothing but the refused address (a link from another app).
                let page = self.tab_url(key).and_then(|u| Url::parse(&u).ok()).filter(|u| !shield::rules::local_target(u));
                let host = target.or_else(|| page.as_ref().and_then(page_host));
                if let (Some(counter), Some(profile_id)) = (counter, self.profile_of(key)) {
                    self.count_shield(key.private, profile_id, counter, 1, host.as_deref(), cx);
                }
            }
            Event::FullScreen(on) => self.set_page_fullscreen(key, on, window),
            Event::ExtensionMessage { world, frame, origin, json } => {
                if let Some(profile) = self.profile_of(key).filter(|_| !key.private) {
                    self.core.tarn.borrow().send(crate::tarn_ext::Msg::Tab { profile, tab: key.id as u64, world, frame, origin, json });
                }
            }
            Event::ContentProcessTerminated => {
                if self.model.active == Some(key) {
                    self.leave_page_fullscreen(window);
                }
                self.drop_prompts(key, cx);
                let page = self.pages.entry(key).or_default();
                if page.crashed {
                    page.loading = false;
                    page.error = Some("The page crashed again and was not reloaded.".into());
                } else if let Some(view) = self.views.get(&key) {
                    page.crashed = true;
                    view.recover();
                }
            }
        }
        cx.notify();
    }

    /// The active tab's page alone fills the window, in full screen, while it shows an element
    /// full screen; leaving restores the window as it was.
    fn set_page_fullscreen(&mut self, key: TabKey, on: bool, window: &mut Window) {
        if self.model.active != Some(key) {
            return;
        }
        if !on {
            return self.leave_page_fullscreen(window);
        }
        let Some(view) = self.views.get(&key).cloned() else { return };
        // Nothing the shell must show may be hidden: a pending prompt keeps the toolbar and its origin.
        let Some(entered) = self.page_fullscreen.enter(self.active_prompt().is_some(), window.is_fullscreen()) else { return };
        if entered {
            window.toggle_fullscreen();
        }
        view.set_corner_radius(0.);
        view.raise();
        self.layout_changed();
    }

    /// Whether the window may show the full-screen page alone: no prompt, overlay, address field,
    /// find bar or notice of the shell waits to be seen, and the page is still the active one.
    pub(crate) fn page_fullscreen_holds(&self) -> bool {
        browser_core::fullscreen::Shell {
            page: self.active_view().is_some(),
            prompt: self.active_prompt().is_some(),
            overlay: self.overlay_open(),
            address: self.command_mode.is_some(),
            find: self.find_open,
            notice: self.model.active.and_then(|k| self.pages.get(&k)).is_some_and(|p| p.notice.is_some() || p.blocked.is_some()),
        }
        .lets_page_fill_window()
    }

    /// Gives the window back its sidebar, toolbar and size: when the page leaves full screen, and
    /// when a prompt, another tab or a crash needs the shell. The page itself may stay full screen
    /// inside the tab until it is put away.
    pub(crate) fn leave_page_fullscreen(&mut self, window: &mut Window) {
        let Some(entered) = self.page_fullscreen.leave() else { return };
        if entered && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        if let Some(view) = self.active_view() {
            view.set_corner_radius(page_corner());
        }
        self.layout_changed();
    }

    fn record_visit(&mut self, key: TabKey, url: &Url, title: &str, typed: bool) {
        if let Some(profile_id) = self.profile_of(key)
            && matches!(url.scheme(), "http" | "https")
            && self.core.keeps_records(key.private, profile_id)
        {
            let visit = self.store(key.private).add_history(profile_id, url.as_str(), title, now(), typed).ok().map(|id| (id, url.to_string()));
            self.menu.visited(url.as_str());
            self.pages.entry(key).or_default().visit = visit;
        }
    }

    /// Only after a full load: a same-document visit would read the previous route's text.
    fn request_page_text(&mut self, key: TabKey, url: &Url) {
        let visit = self.pages.get(&key).and_then(|p| p.visit.clone()).filter(|(_, u)| u == url.as_str());
        let (Some(profile_id), Some((visit, page)), Some(view)) = (self.profile_of(key), visit, self.views.get(&key)) else { return };
        if key.private || !self.pref(crate::prefs::Pref::HistoryText) || self.level_for_tab(key, url.as_str()) == ShieldLevel::Lockdown {
            return;
        }
        let events = self.events.clone();
        view.page_text(url, store::MAX_HISTORY_TEXT, move |text| {
            events.unbounded_send(Msg::PageText(profile_id, visit, page.clone(), text)).ok();
        });
    }

    /// Only the disk store keeps page text, so the private space never does.
    fn store_page_text(&mut self, profile_id: Id, visit: Id, url: &str, text: String) {
        if text.is_empty() || !self.pref(crate::prefs::Pref::HistoryText) {
            return;
        }
        self.core.disk.set_history_text(profile_id, visit, url, store::clip_history_text(&text)).ok();
    }

    /// Replaces the tab's view with one built for `url`'s Shield level and rules, then loads it.
    pub(crate) fn rebuild_view(&mut self, key: TabKey, url: &Url, cx: &mut Context<Self>) {
        self.rebuild_view_with(key, url, None, cx);
    }

    /// Rebuilds the tab's view for a page's navigation, which the new view judges as the page's link.
    fn rebuild_view_for_link(&mut self, key: TabKey, url: &Url, cx: &mut Context<Self>) {
        let opener = self.views.get(&key).and_then(|v| v.url());
        self.rebuild_view_loading(key, url, None, Some(opener), cx);
    }

    /// Rebuilds the tab's view, handing `storage` to the new one before its first load.
    pub(crate) fn rebuild_view_with(&mut self, key: TabKey, url: &Url, storage: Option<engine::handover::Storage>, cx: &mut Context<Self>) {
        self.rebuild_view_loading(key, url, storage, None, cx);
    }

    /// With `link`, the URL loads as a link from that page (or from none), not as typed.
    fn rebuild_view_loading(&mut self, key: TabKey, url: &Url, storage: Option<engine::handover::Storage>, link: Option<Option<Url>>, cx: &mut Context<Self>) {
        if self.model.tab(key).is_some() && self.views.remove(&key).is_some() {
            // The old view's last events are dropped; the new one says when its load starts.
            self.pages.entry(key).or_default().loading = false;
            self.drop_prompts(key, cx);
            let active = self.model.active == Some(key);
            let visible = (active || self.in_split(key)) && !self.overlay_open();
            if let Some(view) = self.create_view(key, url.as_str(), cx) {
                if let Some(storage) = storage {
                    view.hand_over(storage);
                }
                match &link {
                    None if self.pages.get(&key).and_then(|p| p.opened_file.as_ref()) == Some(url) => view.load_file(url),
                    Some(opener) => view.load_link(url, opener.as_ref()),
                    None => view.load(url),
                }
                view.set_visible(visible);
                if visible && active {
                    view.focus();
                }
            }
        }
    }

    fn site_paused_in(&self, key: TabKey, host: &str) -> bool {
        self.profile_of(key).and_then(|p| self.loaded(key.private, p)).is_some_and(|rt| rt.rules.paused.borrow().contains(host))
    }

    /// Shield level for `url` under the profile that owns `key`, which may not be the current space's.
    pub fn level_for_tab(&self, key: TabKey, url: &str) -> ShieldLevel {
        self.profile_of(key).and_then(|p| self.core.level(key.private, p, Url::parse(url).ok().as_ref())).unwrap_or(ShieldLevel::Lockdown)
    }

    pub(crate) fn queue_prompt(&mut self, key: TabKey, prompt: Prompt, window: &mut Window, cx: &mut Context<Self>) {
        if !self.views.contains_key(&key) {
            return;
        }
        if let Some(permission) = prompt.kind.permission_key() {
            let page = prompt.top_origin.as_deref().unwrap_or(&prompt.origin);
            let level = self.level_for_tab(key, page).max(self.level_for_tab(key, &prompt.origin));
            if level == ShieldLevel::Lockdown {
                return;
            }
            let stored = (level == ShieldLevel::Standard).then(|| self.profile_of(key)).flatten().and_then(|p| {
                self.store(key.private).permission(p, &permission_origin(&prompt.origin, prompt.top_origin.as_deref()), permission).ok().flatten()
            });
            match stored.as_deref() {
                Some("allow") => return prompt.accept("", None),
                Some("deny") => return,
                _ => {}
            }
        }
        let queue = self.prompts.entry(key).or_default();
        if queue.len() >= MAX_QUEUED_PROMPTS {
            return;
        }
        queue.push_back(prompt);
        if queue.len() == 1 && self.model.active == Some(key) {
            self.prepare_prompt(window, cx);
        }
    }

    /// Discards a tab's prompts (each declines on drop) and its external-app request, and clears what
    /// the user typed into them.
    fn drop_prompts(&mut self, key: TabKey, cx: &mut Context<Self>) {
        if self.pending_external.as_ref().is_some_and(|p| p.tab == Some(key)) {
            self.pending_external = None;
        }
        if self.prompts.remove(&key).is_none_or(|q| q.is_empty()) || self.model.active != Some(key) {
            return;
        }
        for input in [&self.prompt_input, &self.auth_user, &self.auth_pass] {
            input.update(cx, |i, cx| i.set_text("", cx));
        }
        self.refocus = true;
        if self.model.active == Some(key)
            && let Some(view) = self.views.get(&key)
        {
            view.focus();
        }
    }

    /// The find, prompt or command field has focus while the bar or card holding it is not drawn.
    pub(crate) fn hidden_field_focused(&self, window: &Window, cx: &App) -> bool {
        let focused = |input: &Entity<TextInput>| input.focus_handle(cx).is_focused(window);
        let card = !self.overlay_open();
        (focused(&self.find_input) && !(card && self.find_open && self.active_json().is_none()))
            || ([&self.prompt_input, &self.auth_user, &self.auth_pass].into_iter().any(focused) && !(card && self.active_prompt().is_some()))
            || (focused(&self.command) && self.command_mode.is_none())
    }

    pub fn active_prompt(&self) -> Option<&Prompt> {
        self.prompts.get(&self.model.active?)?.front()
    }

    fn prepare_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay_open() {
            return;
        }
        let Some(kind) = self.active_prompt().map(|p| p.kind.clone()) else { return };
        self.remember_permission = false;
        self.layout_changed();
        if let Some(host) = &self.host {
            host.focus();
        }
        let focus = match kind {
            PromptKind::Text { default } => {
                self.prompt_input.update(cx, |i, cx| i.set_text(default, cx));
                self.prompt_input.focus_handle(cx)
            }
            PromptKind::Credentials { .. } => {
                self.auth_user.update(cx, |i, cx| i.set_text("", cx));
                self.auth_pass.update(cx, |i, cx| i.set_text("", cx));
                self.auth_user.focus_handle(cx)
            }
            _ => self.focus.clone(),
        };
        window.focus(&focus);
        cx.notify();
    }

    pub fn answer_prompt(&mut self, accept: bool, silence: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        if accept && self.prompt_guarded() {
            return;
        }
        let Some(prompt) = self.prompts.get_mut(&key).and_then(VecDeque::pop_front) else { return };
        let page = prompt.top_origin.clone().unwrap_or_else(|| prompt.origin.clone());
        if matches!(prompt.kind, PromptKind::ExtensionAccess { .. }) {
            match (self.remember_permission, silence) {
                (true, false) => prompt.remember(accept),
                (false, false) if accept => prompt.accept("", None),
                _ => prompt.decline(),
            }
            return self.after_prompt(window, cx);
        }
        if let Some(permission) = prompt.kind.permission_key()
            && self.remember_permission
            && self.level_for_tab(key, &page).max(self.level_for_tab(key, &prompt.origin)) == ShieldLevel::Standard
            && let Some(profile_id) = self.profile_of(key)
        {
            let decision = if accept { "allow" } else { "deny" };
            self.store(key.private).set_permission(profile_id, &permission_origin(&prompt.origin, prompt.top_origin.as_deref()), permission, decision).ok();
        }
        match (accept, silence) {
            (_, true) => {
                prompt.decline_and_silence_page();
                self.drop_prompts(key, cx);
            }
            (true, _) => {
                let text = self.prompt_input.read(cx).text().to_owned();
                let user = self.auth_user.read(cx).text().to_owned();
                let pass = self.auth_pass.read(cx).text().to_owned();
                let credentials = matches!(prompt.kind, PromptKind::Credentials { .. }).then_some((user.as_str(), pass.as_str()));
                prompt.accept(&text, credentials);
            }
            (false, _) => prompt.decline(),
        }
        self.after_prompt(window, cx);
    }

    /// Esc declines the prompt in front, as Cancel or Deny does, without remembering the answer.
    pub(crate) fn dismiss_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_permission = false;
        self.answer_prompt(false, false, window, cx);
    }

    /// Enter answers an alert or confirm whose card holds the focus; permission and download
    /// warnings stay mouse-only, behind their click guard.
    pub(crate) fn prompt_key(&mut self, event: &gpui::KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let simple = self.active_prompt().is_some_and(|p| matches!(p.kind, PromptKind::Alert | PromptKind::Confirm));
        let plain = !event.keystroke.modifiers.modified();
        if event.keystroke.key == "enter" && !event.is_held && plain && simple && !self.overlay_open() && self.focus.is_focused(window) {
            cx.stop_propagation();
            self.answer_prompt(true, false, window, cx);
        }
    }

    fn after_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.prompt_input, &self.auth_user, &self.auth_pass] {
            input.update(cx, |i, cx| i.set_text("", cx));
        }
        if self.active_prompt().is_some() {
            self.prepare_prompt(window, cx);
        } else {
            window.focus(&self.focus);
            if let Some(view) = self.active_view() {
                view.focus();
            }
            cx.notify();
        }
    }

    /// A permission prompt's Allow and Remember, and Keep Anyway on a malware warning, ignore clicks for a
    /// moment after they appear, so a page cannot time a click to land on them.
    pub(crate) fn prompt_guarded(&self) -> bool {
        self.active_prompt().is_some_and(|p| p.kind.is_permission() || matches!(p.kind, PromptKind::MaliciousDownload { .. })) && !self.layout_stable()
    }

    fn on_prompt_input_event(&mut self, input: &Entity<TextInput>, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Submit if input == &self.auth_user => {
                let focus = self.auth_pass.focus_handle(cx);
                window.focus(&focus);
            }
            TextInputEvent::Submit => self.answer_prompt(true, false, window, cx),
            TextInputEvent::Cancel => self.answer_prompt(false, false, window, cx),
            _ => {}
        }
    }

    /// In the reader, finds on the page it was made from, which the reader then gives way to.
    pub fn open_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_view().is_none() || self.card_open() || self.active_json().is_some() {
            return;
        }
        if self.reader_shown() {
            self.toggle_reader(window, cx);
        }
        self.find_open = true;
        self.find_count = None;
        let text = self.find_input.read(cx).text().to_owned();
        self.find_input.update(cx, |i, cx| i.set_text(text, cx));
        if let Some(host) = &self.host {
            host.focus();
        }
        let focus = self.find_input.focus_handle(cx);
        window.focus(&focus);
        cx.notify();
    }

    pub fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = false;
        self.find_typing = None;
        window.focus(&self.focus);
        if let Some(view) = self.active_view() {
            view.clear_find();
            view.focus();
        }
        cx.notify();
    }

    pub fn find(&mut self, backwards: bool, cx: &mut Context<Self>) {
        let text = self.find_input.read(cx).text().to_owned();
        let Some(view) = self.active_view().filter(|_| !text.is_empty()) else { return };
        let events = self.events.clone();
        view.find(&text, backwards, move |found, current| {
            events.unbounded_send(Msg::Found(found, current)).ok();
        });
    }

    fn on_find_event(&mut self, _: &Entity<TextInput>, event: &TextInputEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            TextInputEvent::Submit | TextInputEvent::Down => {
                self.find_typing = None;
                self.find(false, cx)
            }
            TextInputEvent::Up => {
                self.find_typing = None;
                self.find(true, cx)
            }
            TextInputEvent::Cancel => self.close_find(window, cx),
            TextInputEvent::Changed => {
                self.find_count = None;
                if self.find_input.read(cx).text().is_empty() {
                    self.find_typing = None;
                    if let Some(view) = self.active_view() {
                        view.clear_find();
                    }
                } else {
                    self.find_typing = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(browser_core::find::TYPING_DELAY).await;
                        this.update(cx, |b, cx| {
                            if b.find_open {
                                b.find(false, cx);
                            }
                        })
                        .ok();
                    }));
                }
                cx.notify();
            }
            TextInputEvent::MenuClosed => {}
        }
    }

    pub fn print(&mut self) {
        if let Some(view) = self.active_view() {
            view.print();
        }
    }

    fn adopt_popup(&mut self, opener: TabKey, popup: Popup, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else { return };
        if self.model.tab(opener).is_none() || self.model.burner_shown() {
            return;
        }
        // A script's window.open keeps its opener, which sign-in and payment pop-ups need, so only links are routed.
        if popup.link && popup.url.clone().is_some_and(|url| self.open_routed(&url, window, cx)) {
            return;
        }
        let Some(space) = self.model.current_space() else { return };
        let (private, space_id) = (space.private, space.space.id);
        let url = popup.url.as_ref().map_or("about:blank".to_owned(), Url::to_string);
        let Ok(tab) = self.store(private).create_tab(space_id, TabKind::Ephemeral, None, &url, "") else {
            return;
        };
        let key = TabKey { private, id: tab.id };
        // On macOS only WebKit views adopt pop-ups, so an opener with an engine of its own ran in WebKit.
        if cfg!(target_os = "macos") {
            self.store(private).set_tab_web_engine(tab.id, self.model.tab(opener).and_then(|t| t.web_engine).map(|_| WebEngine::WebKit)).ok();
        }
        let inspectable = self.developer || self.is_dev_tab(opener);
        let Some(view) = WebView::adopt(popup, &host, inspectable, self.sink(key)) else {
            self.store(private).delete_tab(tab.id).ok();
            return;
        };
        view.set_corner_radius(page_corner());
        view.set_dark(!ui::theme::Theme::is_light());
        view.set_gestures(self.pref(crate::prefs::Pref::Gestures));
        self.views.insert(key, Rc::new(view));
        self.attach_tarn(key);
        self.reload_space();
        self.activate(key, window, cx);
    }

    fn page_changed(&mut self, key: TabKey, url: &Url, title: &str, back: bool, forward: bool, loading: bool) {
        let page = self.pages.entry(key).or_default();
        page.can_back = back;
        page.can_forward = forward;
        page.loading = loading;
        page.error = None;
        self.store(key.private).update_tab_page(key.id, url.as_str(), title).ok();
        if self.private_matches(key)
            && let Some(t) = self.model.tabs.iter_mut().find(|t| t.id == key.id)
        {
            t.url = url.to_string();
            t.title = title.to_owned();
        }
    }

    fn private_matches(&self, key: TabKey) -> bool {
        key.private == self.model.private()
    }

    pub(crate) fn profile_of(&self, key: TabKey) -> Option<Id> {
        self.model.profile_of(&self.core, key)
    }

    pub fn back(&mut self) {
        if let Some(v) = self.active_view() {
            v.back();
        }
    }

    pub fn forward(&mut self) {
        if let Some(v) = self.active_view() {
            v.forward();
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let dev = self.model.active.is_some_and(|k| self.is_dev_tab(k));
        self.reload_page(dev, cx);
    }

    pub fn reload_page(&mut self, from_origin: bool, cx: &mut Context<Self>) {
        let (Some(key), Some(v)) = (self.model.active, self.active_view()) else { return };
        if from_origin {
            v.reload_from_origin()
        } else {
            v.reload()
        }
        self.reload_spin = Some((key, Instant::now()));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(crate::toolbar::SPIN).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        cx.notify();
    }

    pub fn stop_loading(&mut self, cx: &mut Context<Self>) {
        self.reload_spin = None;
        if let Some(view) = self.active_view() {
            view.stop();
        }
        cx.notify();
    }

    fn is_dev_tab(&self, key: TabKey) -> bool {
        self.profile_of(key).and_then(|p| self.loaded(key.private, p)).is_some_and(|rt| rt.profile.is_dev)
    }

    fn after_load_devtools(&mut self, key: TabKey, url: &Url) {
        let dev = self.is_dev_tab(key);
        if dev && let Some(rt) = self.profile_of(key).and_then(|p| self.loaded(key.private, p)) {
            rt.data.clear_cache();
            #[cfg(feature = "chromium")]
            engine::chromium::clear_cache(&data_dir().join("Chromium"), &rt.own_store);
        }
        if !(self.developer || dev) {
            return;
        }
        let Some(view) = self.views.get(&key) else { return };
        let (events, page) = (self.events.clone(), url.clone());
        view.json_text(url, devkit::json_view::MAX_BYTES, move |text| {
            if !text.is_empty() {
                events.unbounded_send(Msg::Json(key, page.clone(), text)).ok();
            }
        });
    }

    fn show_json(&mut self, key: TabKey, url: Url, text: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let page = cx.background_spawn(async move { JsonPage::parse(url, &text) }).await;
            this.update(cx, |b, cx| {
                let current = b.views.get(&key).and_then(|v| v.url());
                if let Ok(page) = page
                    && current.as_ref() == Some(&page.url)
                {
                    b.json_pages.insert(key, page);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn set_json_raw(&mut self, key: TabKey, raw: bool, cx: &mut Context<Self>) {
        if let Some(page) = self.json_pages.get_mut(&key) {
            page.set_raw(raw);
            cx.notify();
        }
    }

    pub fn toggle_json_node(&mut self, key: TabKey, path: Vec<usize>, cx: &mut Context<Self>) {
        if let Some(page) = self.json_pages.get_mut(&key) {
            page.toggle(path);
            cx.notify();
        }
    }

    pub fn active_json(&self) -> Option<(TabKey, &JsonPage)> {
        let key = self.model.active?;
        self.json_pages.get(&key).map(|p| (key, p))
    }

    pub fn current_is_dev(&self) -> bool {
        self.current_loaded().is_some_and(|rt| rt.profile.is_dev)
    }

    pub fn devtools_on(&self) -> bool {
        self.developer || self.current_is_dev()
    }

    /// Web Inspector availability is fixed when a view is created, so every live view is rebuilt.
    pub fn set_developer(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.core.disk.set_setting(DEVELOPER_SETTING, if on { "on" } else { "off" }).is_err() {
            return;
        }
        self.apply_developer(on, cx);
        crate::shared::others(cx, |b, _, cx| b.apply_developer(on, cx));
        let profiles: Vec<(bool, Id)> = self.core.profiles.borrow().iter().filter(|(_, rt)| !rt.profile.is_dev).map(|(k, _)| *k).collect();
        for (private, id) in profiles {
            self.restart_profile_views(private, id, None, false, window, cx);
        }
        cx.notify();
    }

    fn apply_developer(&mut self, on: bool, cx: &mut Context<Self>) {
        self.developer = on;
        if on {
            self.refresh_local_servers(cx);
        } else if !self.current_is_dev() {
            self.set_responsive(None, cx);
        }
        cx.notify();
    }

    pub fn create_dev_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self.core.disk.profiles().unwrap_or_default().into_iter().find(|p| p.is_dev);
        let space = match existing {
            Some(p) => self.core.disk.spaces(p.id).ok().and_then(|s| s.into_iter().next()),
            None => self.core.disk.create_dev_profile(&uuid::Uuid::new_v4().to_string()).ok().map(|(_, space)| space),
        };
        let Some(space) = space else { return };
        self.refresh_local_servers(cx);
        self.reload_spaces();
        let ix = self.model.spaces.iter().position(|s| !s.private && s.space.id == space.id).unwrap_or(0);
        self.switch_space(ix, window, cx);
    }

    pub fn refresh_local_servers(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let ports = cx.background_spawn(async { devkit::localhost::open_dev_ports() }).await;
            this.update(cx, |b, cx| {
                b.local_servers = Some(ports);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn open_local_server(&mut self, port: u16, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(url) = Url::parse(&format!("http://localhost:{port}/")) {
            self.new_tab(&url, window, cx);
        }
    }

    pub fn cycle_responsive(&mut self, cx: &mut Context<Self>) {
        let next = match self.responsive {
            None => Some(0),
            Some(i) if i + 1 < devkit::responsive::PRESETS.len() => Some(i + 1),
            Some(_) => None,
        };
        self.set_responsive(next, cx);
    }

    fn set_responsive(&mut self, preset: Option<usize>, cx: &mut Context<Self>) {
        match (self.responsive.is_some(), preset.is_some()) {
            (false, true) => self.zoom_before_responsive = self.views.iter().map(|(k, v)| (*k, v.zoom())).collect(),
            (true, false) => {
                for (key, view) in &self.views {
                    view.set_zoom(self.zoom_before_responsive.get(key).copied().unwrap_or(1.0));
                }
                self.zoom_before_responsive.clear();
            }
            _ => {}
        }
        self.responsive = preset;
        cx.notify();
    }

    /// The preset applies only while the developer tools that control it are shown.
    pub fn responsive_preset(&self) -> Option<devkit::responsive::Preset> {
        self.responsive.filter(|_| self.devtools_on()).and_then(|i| devkit::responsive::PRESETS.get(i)).copied()
    }

    fn responsive_active(&self) -> bool {
        self.responsive_preset().is_some()
    }

    /// The first click asks; a second click on the same tab and host clears.
    pub fn clear_site_data(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.model.active else { return };
        let Some(host) = self.active_view().and_then(|v| v.url()).and_then(|u| u.host_str().map(str::to_owned)) else { return };
        if self.clear_site_armed.take() != Some((key, host.clone())) {
            self.clear_site_armed = Some((key, host.clone()));
            self.notify_offer(key, format!("Click Clear Site Data again to remove cookies, storage and caches of {host}."), None);
            cx.notify();
            return;
        }
        self.clear_site(key, host, cx);
    }

    /// Removes every WebKit data type of `host` in the tab's profile, then reloads the tab.
    pub(crate) fn clear_site(&mut self, key: TabKey, host: String, cx: &mut Context<Self>) {
        if let Some(profile_id) = self.profile_of(key).filter(|_| !key.private)
            && self.core.disk.unzap_element(profile_id, &host, None).is_ok()
        {
            crate::zaps::apply(&self.core, false, profile_id);
        }
        let Some(rt) = self.profile_of(key).and_then(|p| self.loaded(key.private, p)) else { return };
        #[cfg(feature = "chromium")]
        self.clear_chromium_site(&rt.own_store, &host);
        let this = cx.entity().downgrade();
        let mut async_cx = cx.to_async();
        rt.data.clear_site_data(&host.clone(), move |count| {
            this.update(&mut async_cx, |b, cx| {
                let message = if count == 0 { format!("{host} had no stored data.") } else { format!("Cleared stored data for {host}.") };
                b.notify_offer(key, message, None);
                if let Some(view) = b.views.get(&key) {
                    view.reload();
                }
                cx.notify();
            })
            .ok();
        });
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.set_sidebar_open(!self.sidebar_open, cx);
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, |b, window, cx| b.open_settings(window, cx)) {
            return;
        }
        if let Some(handle) = self.settings_window
            && handle.update(cx, |_, settings_window, _| settings_window.activate_window()).is_ok()
        {
            return;
        }
        let Some(main) = window.window_handle().downcast::<Browser>() else { return };
        let browser = cx.entity().downgrade();
        // The new window renders from the browser at once, so it must open after this update ends.
        cx.defer(move |cx| {
            let handle = SettingsWindow::open(browser.clone(), main, cx);
            browser.update(cx, |b, _| b.settings_window = handle).ok();
        });
    }

    pub fn open_settings_pane(&mut self, pane: crate::settings::SettingsPane, window: &mut Window, cx: &mut Context<Self>) {
        if self.in_main(window, cx, move |b, window, cx| b.open_settings_pane(pane, window, cx)) {
            return;
        }
        self.open_settings(window, cx);
        let browser = cx.entity().downgrade();
        cx.defer(move |cx| {
            if let Some(handle) = browser.upgrade().and_then(|b| b.read(cx).settings_window) {
                handle.update(cx, |settings, _, cx| settings.set_pane(pane, cx)).ok();
            }
        });
    }

    pub fn current_profile(&self) -> Option<Profile> {
        self.current_loaded().map(|rt| rt.profile.clone())
    }

    /// The current profile without copying it, for checks that run every frame.
    pub(crate) fn current_loaded(&self) -> Option<Ref<'_, ProfileRuntime>> {
        let space = self.model.current_space()?;
        self.loaded(space.private, space.space.profile_id)
    }

    pub fn site_overrides(&self) -> Vec<(String, ShieldLevel)> {
        let Some(space) = self.model.current_space() else { return Vec::new() };
        let Some(rt) = self.loaded(space.private, space.space.profile_id) else { return Vec::new() };
        let mut list: Vec<_> = rt.overrides.borrow().iter().map(|(h, l)| (h.clone(), *l)).collect();
        list.sort();
        list
    }

    pub fn all_profiles(&self) -> Vec<Profile> {
        let burners = self.core.burners.borrow();
        [false, true]
            .into_iter()
            .flat_map(|private| self.store(private).profiles().unwrap_or_default())
            .filter(|p| !(p.is_private && burners.contains(&p.id)))
            .collect()
    }

    pub(crate) fn shield_wipes_data(&self, level: ShieldLevel) -> bool {
        self.current_loaded().is_some_and(|rt| browser_core::profiles::shield_wipes_data(level, self.model.private(), rt.profile.shield_level))
    }

    pub(crate) fn shield_target(&self) -> Option<(bool, Id)> {
        self.model.current_space().map(|space| (space.private, space.space.profile_id))
    }

    pub fn set_profile_shield(&mut self, level: ShieldLevel, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.shield_target() else { return };
        if !self.shield_wipes_data(level) {
            return self.apply_profile_shield(target, level, window, cx);
        }
        let name = self.current_profile().map(|p| ui::text::bounded_title(&p.name)).unwrap_or_default();
        let question = crate::settings::lockdown_question(&name);
        let answer = window.prompt(gpui::PromptLevel::Warning, &question, Some(crate::settings::LOCKDOWN_WIPE), &["Use Lockdown", "Cancel"], cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |b, window, cx| b.apply_profile_shield(target, level, window, cx)).ok();
            }
        })
        .detach();
    }

    pub(crate) fn apply_profile_shield(&mut self, (private, profile_id): (bool, Id), level: ShieldLevel, window: &mut Window, cx: &mut Context<Self>) {
        if self.store(private).set_profile_shield_level(profile_id, level).is_err() {
            return;
        }
        let new_store = match self.core.profiles.borrow_mut().get_mut(&(private, profile_id)) {
            Some(rt) => {
                let crossing = !private && (rt.profile.shield_level == ShieldLevel::Lockdown) != (level == ShieldLevel::Lockdown);
                rt.profile.shield_level = level;
                crossing
            }
            None => false,
        };
        if !new_store {
            self.core.apply_extensions(private, profile_id);
        }
        if !private {
            crate::tarn_ext::level_changed(&self.core, profile_id, cx);
        }
        self.restart_profile_views(private, profile_id, None, new_store, window, cx);
        #[cfg(feature = "chromium")]
        if new_store
            && level == ShieldLevel::Lockdown
            && let Some(own_store) = self.loaded(private, profile_id).map(|rt| rt.own_store.clone())
        {
            self.wipe_chromium(&own_store, &own_store);
        }
    }

    pub fn remove_site_override(&mut self, host: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.set_site_shield(host, None, window, cx);
    }

    /// Content rules are fixed when a web view is created, so a level change needs new views, in
    /// every window. `new_store` also replaces the profile's data store, for a move into or out of
    /// Lockdown.
    pub(crate) fn restart_profile_views(
        &mut self,
        private: bool,
        profile_id: Id,
        host: Option<&str>,
        new_store: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.core.sync_shield();
        self.restart_views_here(private, profile_id, host, new_store, window, cx);
        crate::shared::others(cx, |b, window, cx| b.restart_views_here(private, profile_id, host, false, window, cx));
    }

    pub(crate) fn restart_views_here(
        &mut self,
        private: bool,
        profile_id: Id,
        host: Option<&str>,
        new_store: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let on_host = |view: &WebView| host.is_none_or(|h| view.url().is_some_and(|u| u.host_str().map(|s| s.trim_end_matches('.')) == Some(h)));
        let keys: Vec<TabKey> =
            self.views.iter().filter(|(k, v)| k.private == private && self.profile_of(**k) == Some(profile_id) && on_host(v)).map(|(k, _)| *k).collect();
        self.remake_views(keys, window, cx, |b| {
            #[cfg(windows)]
            if new_store {
                crate::shared::count_chromium_blocks(&b.core);
            }
            let old = if new_store { b.core.profiles.borrow_mut().remove(&(private, profile_id)) } else { None };
            if let Some(old) = old {
                // WebKit holds a download's delegate weakly, so the old store outlives its downloads.
                // shortcut: kept until quit; crossings are rare, prune by active downloads if that changes.
                old.data.set_extensions(&[], &[], old.profile.shield_level);
                b.core.retired_stores.borrow_mut().push(old.data);
            }
            b.runtime(private, profile_id);
        });
    }

    /// Makes the views of `profiles` again so their pages load with the profile's new style
    /// script; with `chromium_only`, only Chromium views, which keep the script they were created
    /// with, while WebKit views take it on their next load. Windows never asks: its Chromium
    /// views are restyled in place.
    pub(crate) fn restart_views_of(&mut self, profiles: &[(bool, Id)], chromium_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        let keys: Vec<TabKey> = self
            .views
            .iter()
            .filter(|(k, v)| (!chromium_only || v.is_chromium()) && self.profile_of(**k).is_some_and(|id| profiles.contains(&(k.private, id))))
            .map(|(k, _)| *k)
            .collect();
        if !keys.is_empty() {
            self.remake_views(keys, window, cx, |_| {});
        }
    }

    /// Drops the views of `keys` (their pages reload when shown), runs `between`, then shows the
    /// active tab and split again.
    fn remake_views(&mut self, keys: Vec<TabKey>, window: &mut Window, cx: &mut Context<Self>, between: impl FnOnce(&mut Self)) {
        let active = self.model.active;
        let split = self.shown_split().is_some_and(|s| s.panes().iter().any(|p| keys.contains(p)));
        for key in keys {
            self.hibernate(key, cx);
        }
        between(self);
        if let Some(key) = active.filter(|_| split || self.model.active.is_none()) {
            self.activate(key, window, cx);
        }
        cx.notify();
    }

    /// Tabs of the profile stop renaming visits that were just deleted.
    fn forget_visits(&mut self, private: bool, profile_id: Id) {
        let keys: Vec<TabKey> = self.pages.keys().copied().filter(|k| k.private == private && self.profile_of(*k) == Some(profile_id)).collect();
        for key in keys {
            if let Some(page) = self.pages.get_mut(&key) {
                page.visit = None;
            }
        }
    }

    pub fn clear_history(&mut self, private: bool, profile_id: Id, range: browser_core::library::HistoryRange, cx: &mut Context<Self>) {
        browser_core::library::clear_history(self.store(private), profile_id, range, now()).ok();
        let all = range == browser_core::library::HistoryRange::All;
        if all {
            crate::zaps::apply(&self.core, private, profile_id);
        }
        let forget = move |b: &mut Self, cx: &mut Context<Self>| {
            b.forget_history(private, profile_id);
            if all {
                b.reapply_site_zooms(private, profile_id, None);
                b.set_site_muted(private, profile_id, None, false, false);
            }
            cx.notify();
        };
        forget(self, cx);
        crate::shared::others(cx, |b, _, cx| forget(b, cx));
    }

    pub fn remove_history_url(&mut self, url: &str, cx: &mut Context<Self>) {
        let Some((private, profile_id)) = self.model.profile() else { return };
        self.store(private).delete_history_url(profile_id, url).ok();
        let forget = move |b: &mut Self, cx: &mut Context<Self>| {
            b.forget_visits(private, profile_id);
            b.history_cache.clear();
            b.forget_text_matches();
            b.menu.history_cleared();
            cx.notify();
        };
        forget(self, cx);
        crate::shared::others(cx, |b, _, cx| forget(b, cx));
    }

    /// Forgets what this window keeps in memory of a profile's history: visits, suggestions,
    /// recently closed tabs and site icons.
    pub(crate) fn forget_history(&mut self, private: bool, profile_id: Id) {
        self.forget_visits(private, profile_id);
        self.history_cache.clear();
        self.forget_text_matches();
        self.menu.history_cleared();
        self.model.closed_tabs.clear_profile(private, profile_id);
        self.forget_favicons((private, profile_id));
    }

    /// The private profile is never written to disk.
    pub fn export_profile(&mut self, private: bool, profile_id: Id, cx: &mut Context<Self>) {
        let notice = match self.write_export(private, profile_id) {
            Ok(path) => format!("Exported to {}.", path.display()),
            Err(e) => format!("Export failed: {e}."),
        };
        self.privacy_notice = Some(((private, profile_id), notice));
        cx.notify();
    }

    fn write_export(&self, private: bool, profile_id: Id) -> Result<PathBuf, String> {
        use std::io::Write;
        #[cfg(unix)]
        use std::os::unix::fs::OpenOptionsExt;
        if private {
            return Err("private data is not saved".into());
        }
        let store = self.store(private);
        let name = store.profile(profile_id).map_err(|e| e.to_string())?.name;
        let json = store.export_profile(profile_id).map_err(|e| e.to_string())?;
        let dir = std::env::home_dir().ok_or("no home folder")?.join("Downloads");
        let file = browser_core::profile_data::export_name(&name);
        let path = engine::downloads::free_path(&dir, &file, |_| false);
        let mut options = std::fs::OpenOptions::new();
        #[cfg(unix)]
        options.mode(0o600);
        options.write(true).create_new(true).open(&path).and_then(|mut f| f.write_all(json.as_bytes())).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// Files already saved to Downloads stay; the profile row (name, Shield level) stays.
    pub fn delete_profile_data(&mut self, private: bool, profile_id: Id, window: &mut Window, cx: &mut Context<Self>) {
        let target = (private, profile_id);
        self.forget_visits(private, profile_id);
        let keys: Vec<TabKey> = self.views.keys().filter(|k| k.private == private && self.profile_of(**k) == Some(profile_id)).copied().collect();
        let folders: Vec<Id> = self
            .model
            .spaces
            .iter()
            .filter(|s| s.private == private && s.space.profile_id == profile_id)
            .flat_map(|s| self.store(private).folders(s.space.id).unwrap_or_default())
            .map(|f| f.id)
            .collect();
        let reset = match self.core.reset_profile_data(private, profile_id) {
            Ok(reset) => reset,
            Err(e) => {
                self.privacy_notice = Some((target, format!("Deleting failed ({e}).")));
                cx.notify();
                return;
            }
        };
        let space = reset.space;
        Self::sync_other_windows(cx);
        crate::shared::others(cx, |b, _, cx| {
            b.forget_history(private, profile_id);
            b.download.retain(|d| (d.private, d.profile_id) != target);
            cx.notify();
        });
        for key in keys {
            self.hibernate(key, cx);
        }
        self.model.forget_folders(private, &folders);
        self.download.retain(|d| (d.private, d.profile_id) != target);
        self.pending_external = None;
        self.refocus |= std::mem::take(&mut self.find_open);
        self.suggestions.clear();
        let store = self.core.store(private);
        self.pages.retain(|k, _| k.private != private || store.tab(k.id).is_ok());
        self.history_cache.clear();
        self.forget_text_matches();
        self.model.closed_tabs.clear_profile(private, profile_id);
        self.forget_favicons(target);
        self.privacy_notice = Some((target, "Deleting website data…".into()));
        let own_store = reset.own_store;
        // WebKit's data removal leaves its tracking-prevention and site-security databases behind,
        // so a profile with a store of its own moves to a new one and the old one goes as a whole.
        if reset.fresh_store.is_some() {
            #[cfg(feature = "chromium")]
            self.wipe_chromium(&own_store, reset.fresh_store.as_deref().unwrap_or_default());
            #[cfg(windows)]
            crate::shared::count_chromium_blocks(&self.core);
            if let Some(old) = self.core.profiles.borrow_mut().remove(&(false, profile_id)) {
                old.data.set_extensions(&[], &[], old.profile.shield_level);
                if self.downloads_popover.active.get(&target).is_some_and(|n| *n > 0) {
                    self.core.retired_stores.borrow_mut().push(old.data);
                }
            }
            self.core.extensions.borrow_mut().forget_running(false, profile_id);
            DataStore::wipe_persistent(&own_store);
            DataStore::wipe_extensions(&own_store);
            crate::zaps::forget(&own_store);
            self.retire_store(&own_store, cx);
            self.runtime(false, profile_id);
            self.core.sync_shield();
            self.privacy_notice = Some((target, "All data of this profile was deleted.".into()));
            self.reload_spaces();
            let ix = self.model.spaces.iter().position(|s| !s.private && s.space.id == space.id).unwrap_or(0);
            self.switch_space(ix, window, cx);
            return;
        }
        #[cfg(feature = "chromium")]
        if !private {
            self.wipe_chromium(&own_store, &own_store);
        }
        let (core, app_cx) = (self.core.clone(), cx.to_async());
        let reached = match self.runtime(private, profile_id) {
            Some(rt) => {
                rt.overrides.borrow_mut().clear();
                rt.data.set_site_styles(&crate::themes::page_sheets(&core, private, profile_id));
                if !private {
                    crate::zaps::forget(&own_store);
                    rt.data.set_rule_lists("zaps", None, false);
                    #[cfg(windows)]
                    rt.data.set_hidden_elements(&[]);
                }
                rt.rules.clear();
                if !private && rt.profile.shield_level == ShieldLevel::Lockdown {
                    DataStore::wipe_persistent(&rt.profile.data_store_uuid);
                    DataStore::wipe_extensions(&own_store);
                }
                let this = cx.entity().downgrade();
                let mut async_cx = cx.to_async();
                rt.data.remove_extension_data(move || {
                    core.extensions.borrow_mut().forget_running(private, profile_id);
                    if core.apply_extensions(private, profile_id) {
                        core.sync_shield();
                        app_cx.update(|cx| crate::shared::each(cx, |b, window, cx| b.restart_views_here(private, profile_id, None, false, window, cx))).ok();
                    }
                });
                rt.data.remove_all_data(move || {
                    this.update(&mut async_cx, |b, cx| {
                        b.privacy_notice = Some((target, "All data of this profile was deleted.".into()));
                        cx.notify();
                    })
                    .ok();
                });
                true
            }
            None => false,
        };
        if !reached {
            self.privacy_notice = Some((target, "Stored data was deleted; website data could not be reached.".into()));
        }
        self.core.sync_shield();
        self.reload_spaces();
        let ix = self.model.spaces.iter().position(|s| s.private == private && s.space.id == space.id).unwrap_or(0);
        self.switch_space(ix, window, cx);
    }

    pub(crate) fn forget_favicons(&mut self, profile: (bool, Id)) {
        self.favicons.forget(&profile);
        self.favicon_memo.take();
        #[cfg(feature = "translation")]
        self.translation_offered.retain(|(p, id, _)| (*p, *id) != profile);
    }

    /// The host's icon, else the nearest parent's within its site, so a subdomain never visited
    /// still shows its site's icon.
    /// Every tab row asks once per frame, so answers are kept until an icon arrives or goes.
    pub fn favicon(&self, url: &str) -> Option<Arc<Image>> {
        let space = self.model.current_space()?;
        let profile = (space.private, space.space.profile_id);
        let mut memo = self.favicon_memo.borrow_mut();
        // shortcut: a full memo starts over, so URLs a long session visited do not pile up.
        if memo.as_ref().is_none_or(|(p, urls)| *p != profile || urls.len() >= 4096) {
            *memo = Some((profile, HashMap::new()));
        }
        let urls = &mut memo.as_mut()?.1;
        if let Some(found) = urls.get(url) {
            return found.clone();
        }
        let found = self.find_favicon(profile, url);
        urls.insert(url.to_owned(), found.clone());
        found
    }

    fn find_favicon(&self, profile: (bool, Id), url: &str) -> Option<Arc<Image>> {
        self.favicons.find(&profile, &tab_host(url)?).cloned()
    }

    /// Lockdown pages get no extra requests.
    fn request_favicon(&mut self, key: TabKey, url: &Url) {
        let lockdown = self.level_for_tab(key, url.as_str()) == ShieldLevel::Lockdown;
        let (Some(profile_id), Some(host)) = (self.profile_of(key), favicon::icon_host(url, lockdown)) else { return };
        self.load_favicons(key.private, profile_id);
        let Some(view) = self.views.get(&key) else { return };
        let profile = (key.private, profile_id);
        let Some(ticket) = self.favicons.request(&profile, &host) else { return };
        let icon = (profile, host, ticket);
        let (events, page) = (self.events.clone(), url.clone());
        view.icon_links(url, move |links| {
            events.unbounded_send(Msg::IconLinks(key, icon.clone(), page.clone(), links)).ok();
        });
    }

    fn fetch_favicon(&mut self, key: TabKey, icon: IconKey, page: &Url, links: &str) {
        let urls = favicon::candidates(page, links);
        let still_there = |v: &&Rc<WebView>| v.url().is_some_and(|u| u.origin() == page.origin());
        let view = self.views.get(&key).filter(still_there).filter(|_| !urls.is_empty() && self.favicons.is_requested(&icon.0, &icon.1, icon.2));
        let Some(view) = view else {
            self.favicons.withdraw(&icon.0, &icon.1, icon.2);
            return;
        };
        let events = self.events.clone();
        view.fetch_icon(page, &urls, favicon::MAX_BYTES, move |data| {
            events.unbounded_send(Msg::IconData(icon.clone(), data)).ok();
        });
    }

    fn store_favicon(&mut self, icon: IconKey, data: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let png = cx.background_spawn(async move { favicon::normalize(&data) }).await;
            this.update(cx, |b, cx| {
                let (profile, host, ticket) = icon;
                // A history clear while the icon was loading withdraws the request.
                let wanted = b.favicons.is_requested(&profile, &host, ticket);
                if let Some(png) = png.as_ref().filter(|_| wanted && b.core.keeps_records(profile.0, profile.1)) {
                    b.store(profile.0).set_favicon(profile.1, &host, png).ok();
                }
                if b.favicons.accept(&profile, &host, ticket, png.map(|png| Arc::new(Image::from_bytes(ImageFormat::Png, png)))) {
                    b.favicon_memo.take();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The zoom is kept per host and profile and shared by its open tabs; Lockdown pages keep none.
    pub fn zoom(&mut self, factor: Option<f64>, cx: &mut Context<Self>) {
        if self.responsive_active() {
            return self.show_toast("Zoom is off while Responsive Size is on", None, cx);
        }
        let (Some(key), Some(v)) = (self.model.active, self.active_view()) else { return };
        v.set_zoom(factor.map_or(1.0, |f| v.zoom() * f));
        self.show_toast(format!("Zoom {}%", (v.zoom() * 100.).round()), None, cx);
        let (Some(profile_id), Some(url)) = (self.profile_of(key), v.url()) else { return };
        let Some(host) = url.host_str() else { return };
        let Some(zoom) = zoom_to_save(v.zoom(), self.level_for_tab(key, url.as_str()) == ShieldLevel::Lockdown) else { return };
        if self.store(key.private).set_site_zoom(profile_id, host, zoom).is_ok() {
            self.reapply_site_zooms(key.private, profile_id, Some(host));
        }
        cx.notify();
    }

    /// Sets the host's saved zoom; without one, `reset` returns the page to 100% and otherwise leaves it.
    fn apply_site_zoom(&mut self, key: TabKey, url: Option<&Url>, reset: bool) {
        let Some(view) = self.views.get(&key).cloned() else { return };
        let saved = self.profile_of(key).zip(url.and_then(Url::host_str)).and_then(|(p, host)| self.store(key.private).site_zoom(p, host).ok().flatten());
        let Some(zoom) = saved.or(reset.then_some(1.0)) else { return };
        if self.responsive_active() {
            self.zoom_before_responsive.insert(key, zoom);
        } else {
            view.set_zoom(zoom);
        }
    }

    /// Re-applies saved zooms to the profile's open pages on `host`, or on every host.
    fn reapply_site_zooms(&mut self, private: bool, profile_id: Id, host: Option<&str>) {
        let views: Vec<(TabKey, Option<Url>)> = self
            .views
            .iter()
            .filter(|(k, _)| k.private == private && self.profile_of(**k) == Some(profile_id))
            .map(|(k, v)| (*k, v.url()))
            .filter(|(_, url)| host.is_none() || url.as_ref().and_then(Url::host_str) == host)
            .collect();
        for (key, url) in views {
            self.apply_site_zoom(key, url.as_ref(), true);
        }
    }

    fn saved_mute(&self, key: TabKey, host: &str) -> bool {
        self.profile_of(key).is_some_and(|p| self.store(key.private).muted_sites(p).unwrap_or_default().iter().any(|h| h == host))
    }

    /// A page starts muted on a muted host; like zoom, an unsaved Lockdown mute lasts until the tab leaves the host.
    fn apply_site_mute(&self, key: TabKey, url: &Url, left_host: bool) {
        let saved = url.host_str().is_some_and(|host| self.saved_mute(key, host));
        if let Some(view) = self.views.get(&key).filter(|v| saved || left_host || v.is_muted()) {
            view.set_muted(saved || !left_host);
        }
    }

    pub fn tab_muted(&self, key: TabKey) -> bool {
        match self.views.get(&key) {
            Some(view) => view.is_muted(),
            None => self.tab_url(key).and_then(|u| tab_host(&u)).is_some_and(|host| self.saved_mute(key, &host)),
        }
    }

    /// Mutes or unmutes the tab's host in its profile, for its open tabs and later visits; Lockdown saves nothing.
    pub fn toggle_site_muted(&mut self, key: TabKey, cx: &mut Context<Self>) {
        let Some(url) = self.tab_url(key).and_then(|u| Url::parse(&u).ok()) else { return };
        let (Some(profile_id), Some(host)) = (self.profile_of(key), url.host_str()) else { return };
        let muted = !self.tab_muted(key);
        let save = browser_core::site::saves_mute(muted, self.level_for_tab(key, url.as_str()));
        self.set_site_muted(key.private, profile_id, Some(host), muted, save);
        let host = crate::toolbar::host_tail(host.trim_start_matches("www."), 40);
        self.show_toast(if muted { format!("Muted {host}") } else { format!("Unmuted {host}") }, None, cx);
    }

    /// Applies a mute to the profile's open pages on `host`, or on every host, saving it when `save` is set.
    pub fn set_site_muted(&mut self, private: bool, profile_id: Id, host: Option<&str>, muted: bool, save: bool) {
        if let Some(host) = host.filter(|_| save) {
            self.store(private).set_site_muted(profile_id, host, muted).ok();
        }
        for (key, view) in &self.views {
            let on_host = host.is_none() || view.url().as_ref().and_then(Url::host_str) == host;
            if key.private == private && on_host && self.profile_of(*key) == Some(profile_id) {
                view.set_muted(muted);
            }
        }
    }

    pub fn muted_sites(&self) -> Vec<String> {
        let Some(space) = self.model.current_space() else { return Vec::new() };
        self.store(space.private).muted_sites(space.space.profile_id).unwrap_or_default()
    }

    pub fn site_zooms(&self) -> Vec<(String, f64)> {
        let Some(space) = self.model.current_space() else { return Vec::new() };
        self.store(space.private).site_zooms(space.space.profile_id).unwrap_or_default()
    }

    pub fn remove_site_zoom(&mut self, private: bool, profile_id: Id, host: Option<&str>, cx: &mut Context<Self>) {
        let store = self.store(private);
        match host {
            Some(host) => store.set_site_zoom(profile_id, host, None),
            None => store.clear_site_zooms(profile_id),
        }
        .ok();
        self.reapply_site_zooms(private, profile_id, host);
        cx.notify();
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
