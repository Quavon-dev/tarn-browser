use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::rc::Rc;

use browser_core::windows::{Frame, onto_screen};
use engine::Event;
use futures::StreamExt;
use futures::channel::{mpsc, oneshot};
use gpui::{
    App, AppContext, Bounds, DisplayId, Global, Pixels, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowOptions, point, px, size,
};
use store::{Id, SavedWindow};

use crate::browser::{Browser, data_dir};
use crate::prefs::Prefs;

const ARCHIVE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);
const MINUTE: std::time::Duration = std::time::Duration::from_secs(60);

/// `crate::browser::Core` with what only the GPUI shell knows: the switches, the filter-list status
/// and the extensions found at launch. Every window holds the same one.
pub(crate) struct AppCore {
    core: crate::browser::Core,
    pub prefs: Cell<Prefs>,
    pub filters: RefCell<crate::filters::Status>,
    pub extensions: RefCell<crate::extensions::Extensions>,
    /// Download events of every profile's data store, whichever window opened it.
    pub downloads: mpsc::UnboundedSender<(bool, Id, Event)>,
    /// `None` when this build names no update feed.
    pub updater: Option<platform::updater::Updater>,
    /// A browser-data import is reading or writing; only one runs at a time.
    pub importing: Cell<bool>,
    pub themes: RefCell<crate::themes::State>,
}

impl Deref for AppCore {
    type Target = crate::browser::Core;

    fn deref(&self) -> &crate::browser::Core {
        &self.core
    }
}

struct Shared(Rc<AppCore>);

impl Global for Shared {}

pub(crate) fn core(cx: &App) -> Rc<AppCore> {
    cx.global::<Shared>().0.clone()
}

pub(crate) fn browser_windows(cx: &App) -> usize {
    cx.windows().into_iter().filter(|w| w.downcast::<Browser>().is_some()).count()
}

/// Runs `f` in every browser window except one being updated already, which GPUI cannot lease twice.
pub(crate) fn each(cx: &mut App, f: impl FnMut(&mut Browser, &mut Window, &mut gpui::Context<Browser>)) {
    each_except(cx, None, f);
}

/// `each` from inside a browser's update, skipping that browser: its floating sidebar and popups
/// update it without leasing its window, so GPUI would not refuse it.
pub(crate) fn others(cx: &mut gpui::Context<Browser>, f: impl FnMut(&mut Browser, &mut Window, &mut gpui::Context<Browser>)) {
    let me = cx.entity_id();
    each_except(cx, Some(me), f);
}

fn each_except(cx: &mut App, skip: Option<gpui::EntityId>, mut f: impl FnMut(&mut Browser, &mut Window, &mut gpui::Context<Browser>)) {
    for handle in cx.windows().into_iter().filter_map(|w| w.downcast::<Browser>()) {
        // A window on the stack cannot be read; its update below fails on its own.
        if skip.is_some_and(|me| cx.update_window(handle.into(), |root, _, _| root.entity_id() == me).unwrap_or(true)) {
            continue;
        }
        handle.update(cx, |b, window, cx| f(b, window, cx)).ok();
    }
}

/// Moves the sandboxed app's data out of its container on the first unsandboxed start of a
/// release build; development bundles carry the same bundle ID and must not take an installed
/// app's data. `Ok` carries what kept it there; an error means nothing may start.
#[cfg(target_os = "macos")]
fn move_container() -> Result<Option<String>, std::io::Error> {
    use browser_core::container::{BUNDLE_ID, Outcome, move_out};
    if option_env!("TARN_RELEASE_BUILD").is_none_or(str::is_empty)
        || std::env::var_os("TARN_DATA_DIR").is_some()
        || platform::xpc::sandboxed()
        || engine::bundle_identifier().as_deref() != Some(BUNDLE_ID)
    {
        return Ok(None);
    }
    let Some(home) = std::env::home_dir() else { return Ok(None) };
    Ok(match move_out(&home)? {
        Outcome::TargetExists(target) => {
            Some(format!("Data of the sandboxed Tarn is still in ~/Library/Containers/{BUNDLE_ID}: nothing was moved because {} exists.", target.display()))
        }
        Outcome::Moved(skipped) if !skipped.is_empty() => {
            let names: Vec<String> = skipped.iter().map(|p| p.display().to_string()).collect();
            Some(format!("Some data of the sandboxed Tarn stayed in its container because it is a symbolic link: {}.", names.join(", ")))
        }
        Outcome::Nothing | Outcome::Moved(_) => None,
    })
}

/// Only the macOS App Sandbox ever kept Tarn's data elsewhere.
#[cfg(not(target_os = "macos"))]
fn move_container() -> Result<Option<String>, std::io::Error> {
    Ok(None)
}

/// Opens the app-level state and starts what runs once per app: extension discovery, the
/// filter-list task and the archive timer. False when the app quits instead.
pub(crate) fn start(cx: &mut App) -> bool {
    let kept = match move_container() {
        Ok(kept) => kept,
        // Starting would create the targets, so a later launch could never move the data.
        #[cfg(target_os = "macos")]
        Err(e) => {
            let detail = format!(
                "{e}\n\nYour data is unchanged in ~/Library/Containers/{}. Fix the cause (a symbolic link or a folder Tarn cannot write) and open Tarn again; it quits without opening anything until then.",
                browser_core::container::BUNDLE_ID
            );
            engine::alert("Tarn could not move its data out of the App Sandbox container.", &detail);
            cx.quit();
            return false;
        }
        #[cfg(not(target_os = "macos"))]
        Err(e) => {
            engine::alert("Tarn could not start.", &e.to_string());
            cx.quit();
            return false;
        }
    };
    let mut core = crate::browser::Core::open(&data_dir(), cfg!(feature = "chromium"));
    engine::startup::mark("store open");
    core.storage_warning = core.storage_warning.or(kept);
    #[cfg(windows)]
    engine::chromium::set_root(&data_dir().join("Chromium"));
    #[cfg(feature = "chromium")]
    crate::browser::prepare_chromium(&core);
    let mut filters = crate::filters::Status::default();
    filters.updates = core.disk.setting(crate::filters::SETTING).ok().flatten().as_deref() != Some("off");
    let prefs = Cell::new(Prefs::load(&core.disk));
    let (reports, mut update_reports) = mpsc::unbounded();
    let updater = platform::updater::Updater::start(prefs.get().get(crate::prefs::Pref::UpdateChecks), &data_dir().join("updates"), move |report| {
        drop(reports.unbounded_send(report))
    });
    let (downloads, mut download_events) = mpsc::unbounded();
    let core = Rc::new(AppCore {
        core,
        prefs,
        filters: RefCell::new(filters),
        extensions: RefCell::default(),
        downloads,
        updater,
        importing: Cell::new(false),
        themes: RefCell::default(),
    });
    browser_core::theme_fetch::merge_cached_catalog(&core.disk, &crate::themes::catalog_dir());
    crate::themes::remember_withdrawn_repos(&core);
    crate::themes::apply(&core, crate::themes::system_light(cx));
    if !core.chromium_allowed() {
        core.default_web_engine.set(store::WebEngine::WebKit);
    }
    core.set_update_checks(core.update_checks());
    cx.set_global(Shared(core.clone()));
    crate::profiles::delete_retired_stores(&core);
    crate::extensions::store_host_decisions(core.clone());
    let (tabs, mut extension_tabs) = mpsc::unbounded();
    engine::extensions::on_open_tab(move |seed, url, front| drop(tabs.unbounded_send((seed, url, front))));
    cx.spawn(async move |cx| {
        while let Some((seed, url, front)) = extension_tabs.next().await {
            if cx.update(|cx| crate::extensions::open_tab(&seed, &url, front, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
    #[cfg(any(feature = "chromium", windows))]
    check_chromium(cx);

    let (tx, rx) = oneshot::channel();
    engine::extensions::discover(move |found| drop(tx.send(found)));
    cx.spawn(async move |cx| {
        if let Ok(found) = rx.await {
            cx.update(|cx| crate::extensions::discovered(found, cx)).ok();
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Some((private, profile_id, event)) = download_events.next().await {
            if cx.update(|cx| crate::browser::on_download(private, profile_id, event, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Some(report) = update_reports.next().await {
            if cx.update(|cx| updater_reported(report, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
    crate::filters::spawn(data_dir().join("filters"), core, cx).detach();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(ARCHIVE_INTERVAL).await;
            let archived = cx.update(|cx| {
                each(cx, |b, _, cx| b.archive_unused_tabs(cx));
                each(cx, |b, window, cx| b.sync_shared(window, cx));
                #[cfg(any(feature = "chromium", windows))]
                check_chromium(cx);
            });
            if archived.is_err() {
                break;
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(MINUTE).await;
            let ticked = cx.update(|cx| {
                each(cx, |b, window, cx| {
                    b.sleep_idle_tabs(cx);
                    b.follow_schedule(window, cx);
                });
                #[cfg(any(feature = "chromium", windows))]
                count_chromium_blocks(&self::core(cx));
            });
            if ticked.is_err() {
                break;
            }
        }
    })
    .detach();
    cx.on_app_quit(|cx| {
        each(cx, |b, window, _| {
            b.save_all();
            b.save_window(window);
        });
        let core = self::core(cx);
        core.profiles.borrow().values().for_each(|rt| rt.data.quit_downloads());
        core.retired_stores.borrow().iter().for_each(engine::DataStore::quit_downloads);
        // Before Chromium shuts down, which can take seconds; the installer waits for Tarn to exit.
        if let Some(updater) = &self::core(cx).updater {
            updater.quitting();
        }
        #[cfg(windows)]
        {
            count_chromium_blocks(&self::core(cx));
            engine::chromium::shutdown();
        }
        #[cfg(feature = "chromium")]
        {
            engine::chromium::shutdown();
            count_chromium_blocks(&self::core(cx));
            crate::browser::prepare_chromium(&self::core(cx));
        }
        async {}
    })
    .detach();
    true
}

/// Adds the requests Chromium's filter cancelled to the Shield counters of the profile using Chromium.
#[cfg(feature = "chromium")]
#[cfg(not(windows))]
pub(crate) fn count_chromium_blocks(core: &AppCore) {
    let blocked = engine::chromium::take_blocked();
    let Some(profile_id) = core.chromium_profile.get() else { return };
    for (host, n) in blocked {
        core.disk.count(profile_id, store::Counter::ChromiumRequests, n, Some(&host)).ok();
    }
    crate::shield_card::touch();
}

/// Each profile has its own Chromium profile on Windows, so each counts its own blocks.
#[cfg(windows)]
pub(crate) fn count_chromium_blocks(core: &AppCore) {
    for (&(private, profile_id), rt) in core.profiles.borrow().iter() {
        for (host, n) in rt.data.take_blocked() {
            core.store(private).count(profile_id, store::Counter::ChromiumRequests, n, Some(&host)).ok();
        }
    }
    crate::shield_card::touch();
}

fn toast_front(text: impl Into<gpui::SharedString>, action: Option<crate::toast::Action>, cx: &mut App) {
    let front = cx.window_stack().unwrap_or_else(|| cx.windows()).into_iter().find_map(|w| w.downcast::<Browser>());
    if let Some(handle) = front {
        handle.update(cx, |b, _, cx| b.show_toast(text, action, cx)).ok();
    }
}

/// Quits through the normal path; the updater then starts the installer, which starts Tarn again.
pub(crate) fn restart_to_update(cx: &mut App) {
    if core(cx).updater.as_ref().is_some_and(platform::updater::Updater::install_on_quit) {
        cx.quit();
    }
}

fn updater_reported(report: platform::updater::Report, cx: &mut App) {
    use platform::updater::Report;
    let core = core(cx);
    match report {
        Report::FeedVerified => core.feed_verified(browser_core::now()),
        Report::CriticalFound => core.critical_found(browser_core::now()),
        Report::UpdateReady(version) => toast_front(format!("Tarn {version} is ready to install"), Some(crate::toast::Action::RestartToUpdate), cx),
        Report::UpToDate => toast_front("Tarn is up to date", None, cx),
        Report::CheckFailed => toast_front("Tarn could not check for updates", None, cx),
    }
    #[cfg(any(feature = "chromium", windows))]
    check_chromium(cx);
    cx.refresh_windows();
}

/// Judges Chromium's age, tells the engine when to stop navigating and, once Chromium is
/// unavailable, closes the Chromium views of every window.
#[cfg(any(feature = "chromium", windows))]
pub(crate) fn check_chromium(cx: &mut App) {
    use browser_core::staleness::{Staleness, closes_at, released_at};
    let core = core(cx);
    let staleness = core.check_chromium();
    let unavailable = matches!(staleness, Staleness::Unavailable(_));
    let deadline = if unavailable { i64::MIN } else { closes_at(released_at(), core.updates.get()) };
    let judged = core.judged_now();
    engine::chromium::close_in(if unavailable { 0 } else { deadline.saturating_sub(judged) }, deadline);
    if unavailable {
        #[cfg(feature = "chromium")]
        cx.defer(|cx| each(cx, |b, _, cx| b.close_stale_chromium(cx)));
        // On Windows the engine itself refuses every request once Chromium is closed.
        #[cfg(not(feature = "chromium"))]
        cx.refresh_windows();
    } else if CHROMIUM_DEADLINE.replace(deadline) != deadline {
        let wait = std::time::Duration::from_millis(deadline.saturating_sub(judged).max(0) as u64);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(wait).await;
            cx.update(|cx| {
                if CHROMIUM_DEADLINE.get() == deadline {
                    CHROMIUM_DEADLINE.set(i64::MIN);
                    check_chromium(cx);
                }
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(any(feature = "chromium", windows))]
thread_local! {
    /// The deadline a timer waits for, so open pages close when it passes and not at the next tick.
    static CHROMIUM_DEADLINE: Cell<i64> = const { Cell::new(i64::MIN) };
}

thread_local! {
    /// When links from other apps arrived lately, so a flood of them is dropped.
    static INCOMING: RefCell<Vec<std::time::Instant>> = const { RefCell::new(Vec::new()) };
}

const MAX_INCOMING: usize = 10;
const INCOMING_WINDOW: std::time::Duration = std::time::Duration::from_secs(10);

fn frame(bounds: Bounds<Pixels>) -> Frame {
    [bounds.origin.x, bounds.origin.y, bounds.size.width, bounds.size.height].map(f64::from)
}

/// shortcut: GPUI 0.2.2 reports frames relative to the window's display and every display at
/// origin zero, so a window from a second display reopens on the primary one; keeping the display
/// needs its UUID saved with the row.
pub(crate) fn window_frame(window: &Window) -> Frame {
    frame(window.window_bounds().get_bounds())
}

/// At launch: reopens the saved windows and opens the launch arguments in the first; quits when
/// no window can open.
pub(crate) fn open_windows(cx: &mut App) {
    let Some(first) = restore_windows(cx) else { return cx.quit() };
    if engine::shift_held() {
        crate::themes::enter_safe_mode(&core(cx), cx);
    }
    first.update(cx, |b, window, cx| b.open_args(window, cx)).ok();
}

/// A saved row for a new, centred window.
fn new_window_row(cx: &mut App) -> SavedWindow {
    let [x, y, width, height] = frame(Bounds::centered(None, size(px(1280.), px(820.)), cx));
    let row = SavedWindow { x, y, width, height, ..Default::default() };
    core(cx).disk.create_window(&row).unwrap_or(row)
}

/// A new window offset from the frontmost one on its screen, or centred when none is open.
pub(crate) fn new_window(cx: &mut App) {
    // The window the action came from is leased until dispatch ends; its frame is read after.
    cx.defer(open_new_window);
}

fn open_new_window(cx: &mut App) {
    let front = cx.window_stack().unwrap_or_else(|| cx.windows()).into_iter().find_map(|w| w.downcast::<Browser>());
    let placed = front.and_then(|h| h.update(cx, |_, window, cx| (window_frame(window), window.display(cx).map(|d| d.id()))).ok());
    let (row, display) = match placed {
        Some(([x, y, width, height], display)) => {
            let row = SavedWindow { x: x + 30., y: y + 30., width, height, ..Default::default() };
            (core(cx).disk.create_window(&row).unwrap_or(row), display)
        }
        None => (new_window_row(cx), None),
    };
    if let Some(handle) = open_window(row, display, cx) {
        handle.update(cx, |_, window, _| window.activate_window()).ok();
        cx.activate(true);
    }
}

/// The Dock icon clicked while no window is open brings the saved windows back, or a new one;
/// nothing while a failed start's alert is up, before the store exists.
pub(crate) fn reopen(cx: &mut App) {
    if cx.has_global::<Shared>() && !cx.windows().iter().any(|w| w.downcast::<Browser>().is_some()) {
        restore_windows(cx);
    }
}

/// Reopens every saved window, or one new window on a first launch or after the last was closed.
fn restore_windows(cx: &mut App) -> Option<gpui::WindowHandle<Browser>> {
    let core = core(cx);
    let mut saved = core.disk.windows().unwrap_or_default();
    if saved.is_empty() {
        saved.push(new_window_row(cx));
    }
    let opened: Vec<_> = saved.into_iter().filter_map(|row| open_window(row, None, cx)).collect();
    each(cx, |b, _, cx| b.archive_unused_tabs(cx));
    opened.first().copied()
}

/// `display` is the screen `row`'s frame is relative to; `None` for the primary one.
pub(crate) fn open_window(row: SavedWindow, display: Option<DisplayId>, cx: &mut App) -> Option<gpui::WindowHandle<Browser>> {
    let screen = display.and_then(|id| cx.find_display(id)).or_else(|| cx.primary_display());
    let screens: Vec<Frame> = screen.map(|d| frame(d.bounds())).into_iter().collect();
    let [x, y, width, height] = onto_screen([row.x, row.y, row.width, row.height], &screens).map(|v| v as f32);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(x), px(y)), size(px(width), px(height))))),
        display_id: display,
        titlebar: Some(TitlebarOptions { title: Some("Tarn".into()), appears_transparent: true, traffic_light_position: Some(point(px(20.), px(17.))) }),
        window_background: WindowBackgroundAppearance::Opaque,
        window_min_size: Some(size(px(720.), px(480.))),
        ..Default::default()
    };
    cx.open_window(options, |window, cx| cx.new(|cx| Browser::new(row, window, cx))).inspect_err(|e| eprintln!("tarn: could not open window: {e}")).ok()
}

/// Whether another window shows `key` in a live view.
pub(crate) fn live_elsewhere(key: crate::browser::TabKey, cx: &mut gpui::Context<Browser>) -> bool {
    let mut found = false;
    others(cx, |b, _, _| found |= b.views.contains_key(&key));
    found
}

/// How many of `n` links arriving `now` may open: at most `MAX_INCOMING` per `INCOMING_WINDOW`.
fn admit(seen: &mut Vec<std::time::Instant>, now: std::time::Instant, n: usize) -> usize {
    seen.retain(|at| now.duration_since(*at) < INCOMING_WINDOW);
    let admitted = n.min(MAX_INCOMING.saturating_sub(seen.len()));
    seen.extend(std::iter::repeat_n(now, admitted));
    admitted
}

/// Links from another app open as new tabs in the frontmost browser window that does not show a
/// burner tab, a new window when every one does, or the windows restored when none is open; a
/// flood beyond `admit` is dropped with a toast saying so.
pub(crate) fn open_urls(mut urls: Vec<String>, cx: &mut App) {
    let admitted = INCOMING.with_borrow_mut(|seen| admit(seen, std::time::Instant::now(), urls.len()));
    let dropped = urls.len() - admitted;
    urls.truncate(admitted);
    // A flood must not keep restoring windows or stealing focus.
    if admitted == 0 {
        return;
    }
    let browsers: Vec<_> = cx.window_stack().unwrap_or_else(|| cx.windows()).into_iter().filter_map(|w| w.downcast::<Browser>()).collect();
    let open = browsers.iter().copied().find(|h| h.read(cx).is_ok_and(|b| !b.model.burner_shown()));
    let handle = match open {
        Some(handle) => Some(handle),
        None if browsers.is_empty() => restore_windows(cx),
        None => {
            let row = new_window_row(cx);
            open_window(row, None, cx)
        }
    };
    let Some(handle) = handle else { return };
    handle
        .update(cx, |b, window, cx| {
            b.open_urls(urls, true, window, cx);
            if dropped > 0 {
                b.show_toast(format!("{dropped} more {} from another app not opened", if dropped == 1 { "link" } else { "links" }), None, cx);
            }
            window.activate_window();
        })
        .ok();
    cx.activate(true);
}

/// Brings the front browser window forward, restoring the saved windows when none is open.
#[cfg(windows)]
pub(crate) fn bring_forward(cx: &mut App) {
    // Counted with links, so a flood of empty launches cannot keep stealing focus.
    if INCOMING.with_borrow_mut(|seen| admit(seen, std::time::Instant::now(), 1)) == 0 {
        return;
    }
    let browsers: Vec<_> = cx.window_stack().unwrap_or_else(|| cx.windows()).into_iter().filter_map(|w| w.downcast::<Browser>()).collect();
    let open = browsers.iter().copied().find(|h| h.read(cx).is_ok_and(|b| !b.model.burner_shown())).or_else(|| browsers.first().copied());
    if let Some(handle) = open.or_else(|| restore_windows(cx)) {
        handle.update(cx, |_, window, _| window.activate_window()).ok();
        cx.activate(true);
    }
}

/// Called as a window closes: its row goes, and with the last window the private space's data,
/// which would otherwise stay in memory with no window to show it.
pub(crate) fn window_closed(core: &AppCore, id: Id, cx: &mut App) {
    core.disk.delete_window(id).ok();
    if browser_windows(cx) <= 1 {
        core.profiles.borrow_mut().retain(|(private, _), _| !*private);
        core.burners.borrow_mut().clear();
        for profile in core.store(true).profiles().unwrap_or_default() {
            core.store(true).delete_profile(profile.id).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    #[test]
    fn a_flood_of_links_is_capped_per_window_of_time() {
        let (mut seen, now) = (Vec::new(), Instant::now());
        assert_eq!(super::admit(&mut seen, now, 4), 4);
        assert_eq!(super::admit(&mut seen, now, 20), 6);
        assert_eq!(super::admit(&mut seen, now + Duration::from_secs(5), 1), 0);
        assert_eq!(super::admit(&mut seen, now + Duration::from_secs(11), 3), 3);
    }
}
