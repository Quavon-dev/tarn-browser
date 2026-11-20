use std::cell::Cell;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::NSObjectProtocol;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBitmapImageFileType, NSBitmapImageRep, NSImage,
    NSPanel, NSPrintInfo, NSView, NSWindowOrderingMode, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSBundle, NSData, NSDictionary, NSError, NSKeyValueObservingOptions, NSNumber, NSObjectNSKeyValueObserverRegistration, NSPoint, NSRect, NSSize, NSString,
    NSURL, NSURLRequest,
};
use objc2_web_kit::{
    WKBackForwardListItem, WKContentWorld, WKFindConfiguration, WKFindResult, WKNavigation, WKSnapshotConfiguration, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use shield::ShieldLevel;
use url::Url;

use crate::data_store::Builtin;
use crate::delegates::{Delegate, DelegateState, serialized_origin, to_url};
use crate::extensions::Tab;
use crate::handover::{self, Storage};
use crate::page_scripts::{
    PICKER, PICTURE_IN_PICTURE, fetch_icon_script, find_count, find_count_script, find_snippets, find_text_script, icon_links_script, json_text_script,
    page_text_script,
};
use crate::{DataStore, Event};

use crate::nav_policy::{self, SiteFor};

/// A popup WebKit asked for. Dropping it discards the popup.
pub struct Popup {
    pub url: Option<Url>,
    /// Opened by a link the user activated, not by a script's `window.open`.
    pub link: bool,
    pub(crate) parts: Option<(Retained<WKWebView>, Retained<Delegate>)>,
}

impl Drop for Popup {
    fn drop(&mut self) {
        if let Some((view, _)) = &self.parts {
            crate::tarn_webkit::forget_view(view);
            detach(view);
        }
    }
}

impl std::fmt::Debug for Popup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Popup")
    }
}

/// The native view of the shell window that web views are placed into.
#[derive(Clone)]
pub struct Host {
    pub(crate) view: Retained<NSView>,
}

impl Host {
    pub fn from_window(window: &impl HasWindowHandle) -> Option<Self> {
        MainThreadMarker::new()?;
        let RawWindowHandle::AppKit(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        // SAFETY: an AppKit window handle points to a live NSView owned by the window, and we
        // are on the main thread. Retaining it keeps it alive for as long as the Host exists.
        let view = unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }?;
        crate::menu::keep_caret_keys_standard();
        Some(Host { view })
    }

    /// Gives keyboard focus back to the shell view, e.g. when the command bar opens.
    pub fn focus(&self) {
        if let Some(window) = self.view.window() {
            window.makeFirstResponder(Some(&self.view));
        }
    }

    /// Hides this GPUI pop-up window while another app is in front: its pop-up menu level would
    /// otherwise draw it over every app's windows.
    pub fn hide_on_deactivate(&self) {
        if let Some(window) = self.view.window() {
            window.setHidesOnDeactivate(true);
        }
    }

    /// Takes the frame and shadow off a GPUI pop-up window, so only what it draws is visible.
    pub fn strip_frame(&self) {
        self.hide_on_deactivate();
        if let Some(window) = self.view.window() {
            window.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
            window.setHasShadow(false);
            // A new style mask rebuilds the frame view, which drops the shell view's key focus.
            window.makeFirstResponder(Some(&self.view));
        }
    }

    /// Lets the pointer pass through this pop-up window to the window below, as a tooltip must.
    pub fn ignore_mouse(&self) {
        if let Some(window) = self.view.window() {
            window.setIgnoresMouseEvents(true);
        }
    }

    /// macOS draws a pop-up's transparent margin as it is; Windows clips it (see there).
    pub fn round_corners(&self, _top: f64, _left: f64, _right: f64, _bottom: f64, _radius: f64) {}

    /// Frames the window of `popup` at `x, y, width, height` in this view's coordinates (points,
    /// origin top-left), so a popup can follow a shell control; GPUI cannot move a window. The
    /// popup panel becomes key only when a text field in it needs to. Call it outside a GPUI
    /// update, or GPUI misses the resize.
    pub fn place(&self, popup: &Host, x: f64, y: f64, width: f64, height: f64) {
        let (Some(window), Some(panel)) = (self.view.window(), popup.view.window()) else { return };
        panel.setHidesOnDeactivate(true);
        let rect = self.view.convertRect_toView(self.rect(x, y, width, height), None);
        panel.setFrame_display(window.convertRectToScreen(rect), true);
        // A key panel that resized away from the pointer would hand key status to the window under it.
        if let Ok(panel) = panel.downcast::<NSPanel>()
            && !panel.isKeyWindow()
        {
            panel.setBecomesKeyOnlyIfNeeded(true);
        }
    }

    /// Makes `popup`'s window a child of this view's window, so it moves with it when dragged.
    pub fn follow(&self, popup: &Host) {
        if let (Some(window), Some(panel)) = (self.view.window(), popup.view.window())
            && panel.parentWindow().is_none()
        {
            // SAFETY: main-thread call on two live windows; AppKit drops the link when either closes.
            unsafe { window.addChildWindow_ordered(&panel, NSWindowOrderingMode::Above) };
        }
    }

    pub(crate) fn rect(&self, x: f64, y: f64, width: f64, height: f64) -> NSRect {
        let flipped_y = if self.view.isFlipped() { y } else { self.view.bounds().size.height - y - height };
        NSRect::new(NSPoint::new(x, flipped_y), NSSize::new(width.max(0.0), height.max(0.0)))
    }
}

pub const MIN_ZOOM: f64 = 0.3;

pub struct WebView {
    view: Retained<WKWebView>,
    host: Host,
    muted: Cell<bool>,
    _delegate: Retained<Delegate>,
    tab: Option<Retained<Tab>>,
    builtin: Builtin,
    queued: Rc<Queued>,
}

/// Loads held until the built-in rule list is attached: only the latest runs, and a session
/// restore not yet applied still answers `interaction_state`, so saving the empty view loses nothing.
#[derive(Default)]
struct Queued {
    latest: Cell<u64>,
    restore: std::cell::RefCell<Option<Vec<u8>>>,
}

impl WebView {
    /// `filtered` picks the profile's rule set with ad, tracker and cookie-banner blocking, off for paused sites.
    pub fn new(host: &Host, data: &DataStore, level: ShieldLevel, filtered: bool, inspectable: bool, site_for: SiteFor, sink: Rc<dyn Fn(Event)>) -> Self {
        let mtm = host.view.mtm();
        // SAFETY: all calls below configure freshly created objects on the main thread. The
        // configuration gets no script message handler, so page scripts cannot reach the shell.
        unsafe {
            let config = WKWebViewConfiguration::new(mtm);
            config.setWebsiteDataStore(&data.store);
            config.setUserContentController(data.controller(level, filtered));
            if level != ShieldLevel::Lockdown {
                config.setWebExtensionController(data.extension_controller().as_deref());
            }
            if let Some(name) = safari_application_name() {
                config.setApplicationNameForUserAgent(Some(&name));
            }
            let prefs = config.defaultWebpagePreferences();
            prefs.setAllowsContentJavaScript(level.policy().javascript);
            config.preferences().setElementFullscreenEnabled(true);
            // Pop-ups need a user gesture; the delegate then applies the site's rule and the sets.
            config.preferences().setJavaScriptCanOpenWindowsAutomatically(false);
            config.preferences().setFraudulentWebsiteWarningEnabled(data.safe_browsing());
            allow_picture_in_picture(&config.preferences());

            let view = WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), NSRect::ZERO, &config);
            let delegate = Delegate::new(mtm, DelegateState::new(sink, site_for, level, filtered, data.downloads.clone(), false));
            delegate.attach(&view);
            view.setInspectable(inspectable);
            host.view.addSubview(&view);
            observe(&view, &delegate);
            let tab = Tab::track(&view);
            WebView { view, host: host.clone(), muted: Cell::new(false), _delegate: delegate, tab, builtin: data.builtin(level), queued: Rc::default() }
        }
    }

    /// Takes over a popup WebKit created, routing its events to `sink`.
    pub fn adopt(mut popup: Popup, host: &Host, inspectable: bool, sink: Rc<dyn Fn(Event)>) -> Option<Self> {
        let (view, delegate) = popup.parts.take()?;
        // SAFETY: main-thread call on a live web view.
        unsafe { view.setInspectable(inspectable) };
        delegate.set_sink(sink.clone());
        let title = crate::delegates::page_title(&view);
        objc2::DefinedClass::ivars(&*delegate).last_title.replace(title.clone());
        if !title.is_empty() {
            sink(Event::TitleChanged(title));
        }
        host.view.addSubview(&view);
        observe(&view, &delegate);
        let tab = Tab::track(&view);
        Some(WebView { view, host: host.clone(), muted: Cell::new(false), _delegate: delegate, tab, builtin: Builtin::ready(), queued: Rc::default() })
    }

    /// Moves the view, with its page and history, into another shell window; its events go to `sink`.
    pub fn rehost(mut self, host: &Host, sink: Rc<dyn Fn(Event)>) -> Self {
        host.view.addSubview(&self.view);
        self.host = host.clone();
        self._delegate.set_sink(sink);
        self
    }

    /// Loads a URL that the caller has already passed through `nav_policy`; never a `file:` URL.
    pub fn load(&self, url: &Url) {
        if url.scheme() == "file" {
            return self._delegate.emit(Event::Blocked { url: url.clone(), message: nav_policy::FILE_NOT_OPENED.to_owned(), category: None });
        }
        let url = url.clone();
        self.when_protected(Some(url.clone()), move |view, delegate| {
            delegate.mark_shell_load(&url);
            delegate.set_user_load(load_url(view, &url));
        });
    }

    /// Shows a file the user picked with Open File (`nav_policy::Source::FileOpen`); the page may
    /// read its own folder.
    pub fn load_file(&self, url: &Url) {
        let Some(folder) = nav_policy::file_read_access(url) else { return };
        let url = url.clone();
        self.when_protected(Some(url.clone()), move |view, delegate| {
            let (Some(file), Some(folder)) =
                (NSURL::URLWithString(&NSString::from_str(url.as_str())), folder.to_str().map(|f| NSURL::fileURLWithPath(&NSString::from_str(f))))
            else {
                return;
            };
            delegate.mark_shell_load(&url);
            objc2::DefinedClass::ivars(delegate).opened_files.borrow_mut().insert(url.clone());
            // SAFETY: main-thread call with valid file URLs.
            delegate.set_user_load(unsafe { view.loadFileURL_allowingReadAccessToURL(&file, &folder) });
        });
    }

    /// Loads a link the page chose, for a peek. Unlike `load` it is not the user's navigation, so it
    /// and its redirects get the public-to-private check (S10), judged from `opener` while the view
    /// has no page of its own.
    pub fn load_link(&self, url: &Url, opener: Option<&Url>) {
        let (url, opener) = (url.clone(), opener.cloned());
        self.when_protected(Some(url.clone()), move |view, delegate| {
            delegate.mark_link_load(&url, opener.as_ref());
            load_url(view, &url);
            delegate.set_user_load(None);
        });
    }

    /// Runs `load` once the profile's built-in rule list for this view's level is attached; that
    /// list carries the private-network and HTTPS rules, so if it failed nothing is loaded.
    fn when_protected(&self, url: Option<Url>, load: impl FnOnce(&WKWebView, &Delegate) + 'static) {
        let ticket = self.queued.latest.get() + 1;
        self.queued.latest.set(ticket);
        let (view, delegate, queued) = (Weak::from_retained(&self.view), Weak::from_retained(&self._delegate), Rc::downgrade(&self.queued));
        self.builtin.then(move |result| {
            let (Some(view), Some(delegate), Some(queued)) = (view.load(), delegate.load(), queued.upgrade()) else { return };
            if queued.latest.get() != ticket {
                return;
            }
            match result {
                Ok(()) => {
                    queued.restore.take();
                    load(&view, &delegate);
                }
                Err(error) => {
                    delegate.refuse(url, format!("Tarn could not prepare its protections ({error}), so the page was not opened. Restart Tarn to try again."))
                }
            }
        });
    }

    /// Positions the view in shell coordinates: logical points, origin top-left.
    pub fn set_frame(&self, x: f64, y: f64, width: f64, height: f64) {
        self.view.setFrame(self.host.rect(x, y, width, height));
    }

    pub fn set_corner_radius(&self, radius: f64) {
        self.view.setWantsLayer(true);
        if let Some(layer) = self.view.layer() {
            layer.setCornerRadius(radius);
            layer.setMasksToBounds(true);
            // SAFETY: a constant string static exported by Core Animation.
            layer.setCornerCurve(unsafe { objc2_quartz_core::kCACornerCurveContinuous });
        }
    }

    /// The colour scheme pages see (`prefers-color-scheme`) and form controls take: the shell's
    /// theme, not the system's.
    pub fn set_dark(&self, dark: bool) {
        // SAFETY: constant appearance names exported by AppKit.
        let name = unsafe { if dark { NSAppearanceNameDarkAqua } else { NSAppearanceNameAqua } };
        self.view.setAppearance(NSAppearance::appearanceNamed(name).as_deref());
    }

    /// Two-finger swipe back and forward and pinch to zoom.
    pub fn set_gestures(&self, on: bool) {
        // SAFETY: main-thread calls on a live web view.
        unsafe {
            self.view.setAllowsBackForwardNavigationGestures(on);
            self.view.setAllowsMagnification(on);
        }
    }

    pub fn set_visible(&self, visible: bool) {
        self.view.setHidden(!visible);
        if let Some(tab) = self.tab.as_ref().filter(|_| visible) {
            tab.activate();
        }
    }

    pub fn set_alpha(&self, alpha: f64) {
        self.view.setAlphaValue(alpha);
    }

    /// Orders the view above the other web views in the window.
    pub fn raise(&self) {
        self.host.view.addSubview_positioned_relativeTo(&self.view, NSWindowOrderingMode::Above, None);
    }

    pub fn focus(&self) {
        if let Some(window) = self.view.window() {
            window.makeFirstResponder(Some(&self.view));
        }
    }

    /// Whether ad, tracker and cookie-banner rules apply in this view; a page-initiated navigation away from a
    /// paused site keeps them off until the shell loads a page again.
    pub fn filtered(&self) -> bool {
        self._delegate.filtered()
    }

    pub fn url(&self) -> Option<Url> {
        // SAFETY: main-thread read of a live web view.
        self._delegate.shown_url(to_url(unsafe { self.view.URL() }))
    }

    /// Highlights the next (or previous) match; `found` receives how many matches the page has,
    /// and which one is highlighted when the engine says (WebKit does not).
    pub fn find(&self, text: &str, backwards: bool, found: impl Fn(usize, Option<usize>) + 'static) {
        let mtm = self.view.mtm();
        let found = Rc::new(found);
        let view = self.view.clone();
        let count = find_count_script(text);
        // No script of Tarn's runs on a Lockdown page, so the native result stands as one match.
        let lockdown = self._delegate.level() == ShieldLevel::Lockdown;
        // SAFETY: main-thread calls on a fresh configuration and a live web view; the block is
        // copied by WebKit and receives a valid result.
        unsafe {
            let config = WKFindConfiguration::new(mtm);
            config.setBackwards(backwards);
            config.setWraps(true);
            let done = RcBlock::new(move |result: std::ptr::NonNull<WKFindResult>| {
                if !result.as_ref().matchFound() {
                    return found(0, None);
                }
                if lockdown {
                    return found(1, None);
                }
                let found = found.clone();
                run_isolated(&view, &count, move |answer| found(find_count(true, &answer), None));
            });
            self.view.findString_withConfiguration_completionHandler(&NSString::from_str(text), Some(&config), &done);
        }
    }

    /// Hides the marker a find left; the match stays selected, as in Safari.
    pub fn clear_find(&self) {
        // WebKit has no public way to hide the match's marker; its private `_hideFindUI` does where it exists.
        if self.view.respondsToSelector(sel!(_hideFindUI)) {
            // SAFETY: the selector exists on this WKWebView (checked above), takes no arguments and returns nothing.
            let _: () = unsafe { msg_send![&*self.view, _hideFindUI] };
        }
    }

    /// Up to `FIND_MATCHES` snippets of the page's text around `query`, matched without case.
    pub fn find_text(&self, query: &str, done: impl Fn(Vec<String>) + 'static) {
        self.run_isolated(&find_text_script(query), move |found| done(find_snippets(&found)));
    }

    /// Lets the user point at an element of the top document: hovering outlines it, a click
    /// answers its selector, Escape or another pick answers an empty string. The selector comes
    /// from the page's own ids and classes, so callers check it before use. Nothing runs under
    /// Lockdown, where no script of Tarn's runs on the page.
    pub fn pick_element(&self, done: impl Fn(String) + 'static) {
        if self._delegate.level() == ShieldLevel::Lockdown {
            return;
        }
        self.run_isolated(PICKER, done);
    }

    pub fn icon_links(&self, origin: &Url, done: impl Fn(String) + 'static) {
        match icon_links_script(origin) {
            Some(script) => self.run_isolated(&script, done),
            None => done(String::new()),
        }
    }

    /// The page's article for `reader::Article::parse`, or "1" with `probe` when it has one; empty
    /// when it has none. Read in the engine's isolated world.
    pub fn article(&self, origin: &Url, probe: bool, done: impl Fn(String) + 'static) {
        self.run_isolated(&crate::reader::script(&origin.origin().ascii_serialization(), probe), done);
    }

    pub fn fetch_icon(&self, origin: &Url, urls: &[Url], max_bytes: usize, done: impl Fn(String) + 'static) {
        match fetch_icon_script(origin, urls, max_bytes) {
            Some(script) => self.run_isolated(&script, done),
            None => done(String::new()),
        }
    }

    pub fn page_text(&self, origin: &Url, max_chars: usize, done: impl Fn(String) + 'static) {
        match page_text_script(origin, max_chars) {
            Some(script) => self.run_isolated(&script, done),
            None => done(String::new()),
        }
    }

    /// The text of a JSON document WebKit shows at `origin`, if it is at most `max_bytes` in UTF-8;
    /// empty for any other document or once the tab shows another origin.
    pub fn json_text(&self, origin: &Url, max_bytes: usize, done: impl Fn(String) + 'static) {
        let Some(script) = json_text_script(origin, max_bytes) else { return };
        self.run_isolated(&script, done);
    }

    /// Runs a translation step in WebKit's client world; `done` gets an empty string under
    /// Lockdown, where no script of Tarn's runs on the page.
    pub fn translation_step(&self, step: &crate::translation::Step, done: impl Fn(String) + 'static) {
        if self._delegate.level() == ShieldLevel::Lockdown {
            return done(String::new());
        }
        self.run_isolated(&step.script(), done);
    }

    /// The storage of the top-level origin, read in WebKit's client world; `None` under Lockdown,
    /// for a non-web page, empty storage or more than `handover::MAX_BYTES`.
    pub fn read_storage(&self, done: impl FnOnce(Option<Storage>) + 'static) {
        let origin = self.url().as_ref().and_then(serialized_origin).filter(|_| self._delegate.level() != ShieldLevel::Lockdown);
        let Some(origin) = origin else { return done(None) };
        let done = std::cell::RefCell::new(Some(done));
        let script = format!("return {};", handover::read_script(&origin));
        self.run_isolated(&script, move |json| {
            if let Some(done) = done.take() {
                done(Storage::new(&origin, json));
            }
        });
    }

    /// Writes `storage` into the first document this view commits, if it has that origin; call
    /// before the first load. Only for a switch the user started, in the same profile; never
    /// under Lockdown. The script sits in the controller the view shares with the profile's
    /// other views at its level until that commit.
    pub fn hand_over(&self, storage: Storage) {
        if self._delegate.level() == ShieldLevel::Lockdown {
            return;
        }
        self._delegate.run_after_commit();
        let mtm = self.view.mtm();
        // SAFETY: main-thread calls on a live web view and a freshly created script; the client
        // world is out of page scripts' reach.
        unsafe {
            let controller = self.view.configuration().userContentController();
            let script = WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
                WKUserScript::alloc(mtm),
                &NSString::from_str(&handover::write_script(&storage)),
                WKUserScriptInjectionTime::AtDocumentStart,
                true,
                &WKContentWorld::defaultClientWorld(mtm),
            );
            controller.addUserScript(&script);
            self._delegate.after_commit(move || remove_user_script(&controller, &script));
        }
    }

    /// Runs `body` as an async function in WebKit's client content world, which page scripts can
    /// neither see nor alter; `done` receives its string result, or an empty string.
    fn run_isolated(&self, body: &str, done: impl Fn(String) + 'static) {
        run_isolated(&self.view, body, done);
    }

    pub fn is_muted(&self) -> bool {
        self.muted.get()
    }

    /// What sleeping would lose; nothing to read under Lockdown, where no script of Tarn's runs on the page.
    pub fn page_keep(&self, done: impl Fn(crate::page_scripts::PageKeep) + 'static) {
        if self._delegate.level() == ShieldLevel::Lockdown {
            return done(Default::default());
        }
        self.run_isolated(crate::page_scripts::PAGE_KEEP, move |answer| done(crate::page_scripts::page_keep(&answer)));
    }

    pub fn scroll_to(&self, offset: (f64, f64)) {
        if self._delegate.level() != ShieldLevel::Lockdown {
            self.run_isolated(&crate::page_scripts::scroll_script(offset), |_| {});
        }
    }

    /// Whether the page is playing video or audio now, in any frame, or using the camera or microphone.
    pub fn media_playing(&self, done: impl Fn(bool) + 'static) {
        use objc2_web_kit::WKMediaCaptureState;
        // SAFETY: main-thread calls on a live web view; WebKit copies the block.
        unsafe {
            if self.view.cameraCaptureState() != WKMediaCaptureState::None || self.view.microphoneCaptureState() != WKMediaCaptureState::None {
                return done(true);
            }
            let handler = RcBlock::new(move |state: objc2_web_kit::WKMediaPlaybackState| done(state == objc2_web_kit::WKMediaPlaybackState::Playing));
            self.view.requestMediaPlaybackStateWithCompletionHandler(&handler);
        }
    }

    /// The id of the WebContent process the page runs in. WebKit has no public way to get it; its
    /// private `_webProcessIdentifier` is used where it exists.
    pub fn process_id(&self) -> Option<i32> {
        if !self.view.respondsToSelector(sel!(_webProcessIdentifier)) {
            return None;
        }
        // SAFETY: the selector exists on this WKWebView (checked above) and returns a `pid_t`, 0
        // while the page has no process.
        let pid: i32 = unsafe { msg_send![&*self.view, _webProcessIdentifier] };
        (pid > 0).then_some(pid)
    }

    /// WebKit has no public way to mute a page. Its private `_setPageMuted:` is used where it
    /// exists; otherwise the page's media elements are muted from the client content world.
    pub fn set_muted(&self, muted: bool) {
        self.muted.set(muted);
        let (get, set) = (sel!(_mediaMutedState), sel!(_setPageMuted:));
        if self.view.respondsToSelector(get) && self.view.respondsToSelector(set) {
            // SAFETY: both selectors exist on this WKWebView (checked above); they read and write
            // `_WKMediaMutedState`, an NSUInteger bit set whose bit 0 is audio. Other bits (capture
            // devices) are kept.
            unsafe {
                let state: usize = msg_send![&*self.view, _mediaMutedState];
                let _: () = msg_send![&*self.view, _setPageMuted: (state & !1) | usize::from(muted)];
            }
            return;
        }
        let script = format!(
            r"const muted = {muted};
            if (!window.tarnMuted) {{
                window.tarnMuted = new Set();
                document.addEventListener('play', e => {{
                    if (window.tarnMute && !e.target.muted) {{ e.target.muted = true; window.tarnMuted.add(e.target); }}
                }}, true);
            }}
            window.tarnMute = muted;
            for (const m of muted ? document.querySelectorAll('audio, video') : []) if (!m.muted) {{ m.muted = true; window.tarnMuted.add(m); }}
            if (!muted) {{ for (const m of window.tarnMuted) m.muted = false; window.tarnMuted.clear(); }}
            return '';"
        );
        self.run_isolated(&script, |_| {});
    }

    pub fn print(&self) {
        let Some(window) = self.view.window() else { return };
        // SAFETY: main thread; the operation prints this live view into a sheet on its window.
        unsafe {
            let operation = self.view.printOperationWithPrintInfo(&NSPrintInfo::sharedPrintInfo());
            if let Some(view) = operation.view() {
                view.setFrame(self.view.bounds());
            }
            operation.runOperationModalForWindow_delegate_didRunSelector_contextInfo(&window, None, None, std::ptr::null_mut());
        }
    }

    /// The visible part of the page as PNG, scaled to `width` points when given, or None when
    /// WebKit could not draw it. A scaled snapshot does not wait for a screen update, which a
    /// hidden background view may never get.
    pub fn snapshot_png(&self, width: Option<f64>, done: impl Fn(Option<Vec<u8>>) + 'static) {
        // SAFETY: main-thread calls on a live web view and a fresh configuration; WebKit copies
        // the block and passes an image or null, read only for the duration of the call.
        unsafe {
            let config = width.map(|w| {
                let config = WKSnapshotConfiguration::new(self.view.mtm());
                config.setSnapshotWidth(Some(&NSNumber::new_f64(w)));
                config.setAfterScreenUpdates(false);
                config
            });
            let handler = RcBlock::new(move |image: *mut NSImage, _: *mut NSError| done(image.as_ref().and_then(png)));
            self.view.takeSnapshotWithConfiguration_completionHandler(config.as_deref(), &handler);
        }
    }

    /// The whole page as PDF, or None when WebKit could not create it.
    pub fn pdf(&self, done: impl Fn(Option<Vec<u8>>) + 'static) {
        // SAFETY: as in `snapshot_png`; the data or null is read only for the duration of the call.
        unsafe {
            let handler = RcBlock::new(move |data: *mut NSData, _: *mut NSError| done(data.as_ref().map(|d| d.to_vec())));
            self.view.createPDFWithConfiguration_completionHandler(None, &handler);
        }
    }

    /// Downloads the document on screen when it is a PDF WebKit shows inline, so the user keeps
    /// the original file rather than a re-rendered copy; false for any other page.
    pub fn download_pdf(&self) -> bool {
        self._delegate.download_pdf(&self.view)
    }

    /// Moves the largest playing video (else the largest video) into Picture in Picture, or back
    /// out of it. WebKit treats the shell's script as a user gesture. `done` receives "pip",
    /// "exit", "none" or "refused".
    pub fn picture_in_picture(&self, done: impl Fn(String) + 'static) {
        self.run_isolated(PICTURE_IN_PICTURE, done);
    }

    pub fn title(&self) -> String {
        crate::delegates::page_title(&self.view)
    }

    pub fn back(&self) {
        // SAFETY: main-thread calls on a live web view and its back-forward list.
        unsafe { self.history_step(item_url(self.view.backForwardList().backItem()), |view| view.goBack()) };
    }

    pub fn forward(&self) {
        // SAFETY: main-thread calls on a live web view and its back-forward list.
        unsafe { self.history_step(item_url(self.view.backForwardList().forwardItem()), |view| view.goForward()) };
    }

    pub fn reload_from_origin(&self) {
        // SAFETY: main-thread calls on a live web view.
        unsafe { self.history_step(to_url(self.view.URL()).into_iter().collect(), |view| view.reloadFromOrigin()) };
    }

    pub fn reload(&self) {
        // SAFETY: main-thread calls on a live web view.
        unsafe { self.history_step(to_url(self.view.URL()).into_iter().collect(), |view| view.reload()) };
    }

    /// Counts the step as the user's only if WebKit starts a navigation for it, and only for a
    /// navigation to one of `urls`.
    fn history_step(&self, urls: Vec<Url>, step: impl FnOnce(&WKWebView) -> Option<Retained<WKNavigation>>) {
        self._delegate.set_history_step(urls);
        if step(&self.view).is_none() {
            self._delegate.set_history_step(Vec::new());
        }
    }

    /// Reloads a page whose content process ended; not the user's navigation, as a page can end
    /// its own process. An opened file loads again with its read grant.
    pub fn recover(&self) {
        let opened = self.url().filter(|u| objc2::DefinedClass::ivars(&*self._delegate).opened_files.borrow().contains(u));
        match opened {
            Some(url) => self.load_file(&url),
            // SAFETY: main-thread call on a live web view.
            None => drop(unsafe { self.view.reload() }),
        }
    }

    pub fn stop(&self) {
        // SAFETY: main-thread call on a live web view.
        unsafe { self.view.stopLoading() };
    }

    pub fn zoom(&self) -> f64 {
        // SAFETY: main-thread read of a live web view.
        unsafe { self.view.pageZoom() }
    }

    pub fn set_zoom(&self, zoom: f64) {
        // SAFETY: main-thread call on a live web view.
        unsafe { self.view.setPageZoom(zoom.clamp(MIN_ZOOM, 3.0)) };
    }

    /// WebKit's opaque back-forward state, for hibernation and session restore.
    pub fn interaction_state(&self) -> Option<Vec<u8>> {
        if let Some(pending) = self.queued.restore.borrow().clone() {
            return Some(pending);
        }
        // SAFETY: main-thread read; the value is an NSData on macOS, checked by the downcast.
        let state = unsafe { self.view.interactionState() }?;
        Some(state.downcast::<NSData>().ok()?.to_vec())
    }

    pub fn restore_interaction_state(&self, bytes: &[u8]) {
        let data = NSData::with_bytes(bytes);
        *self.queued.restore.borrow_mut() = Some(bytes.to_vec());
        self.when_protected(None, move |view, delegate| {
            // SAFETY: main-thread calls; WebKit validates the archive and ignores invalid data.
            let url = unsafe {
                view.setInteractionState(Some(&data));
                view.URL()
            };
            delegate.set_history_step(to_url(url).into_iter().collect());
        });
    }
}

impl Drop for WebView {
    fn drop(&mut self) {
        for key in OBSERVED {
            // SAFETY: removes the observations added in `new`/`adopt` while both objects are alive.
            unsafe { self.view.removeObserver_forKeyPath(&self._delegate, &NSString::from_str(key)) };
        }
        if let Some(tab) = &self.tab {
            tab.close();
        }
        crate::tarn_webkit::forget_view(&self.view);
        self._delegate.run_after_commit();
        detach(&self.view);
    }
}

/// `canGoBack`/`canGoForward` too, since `pushState` to the current URL changes only those.
const OBSERVED: [&str; 4] = ["title", "URL", "canGoBack", "canGoForward"];

fn observe(view: &WKWebView, delegate: &Delegate) {
    for key in OBSERVED {
        // SAFETY: the WebView owns both objects and removes the observation in `Drop` before
        // either is released.
        unsafe {
            view.addObserver_forKeyPath_options_context(delegate, &NSString::from_str(key), NSKeyValueObservingOptions::empty(), std::ptr::null_mut());
        }
    }
}

fn detach(view: &WKWebView) {
    // SAFETY: main-thread teardown; clearing the delegates before the owner drops them.
    unsafe {
        view.stopLoading();
        view.setNavigationDelegate(None);
        view.setUIDelegate(None);
    }
    view.removeFromSuperview();
}

fn item_url(item: Option<Retained<WKBackForwardListItem>>) -> Vec<Url> {
    // SAFETY: main-thread read of a live back-forward list item.
    item.and_then(|item| to_url(Some(unsafe { item.URL() }))).into_iter().collect()
}

fn load_url(view: &WKWebView, url: &Url) -> Option<Retained<objc2_web_kit::WKNavigation>> {
    let ns_url = NSURL::URLWithString(&NSString::from_str(url.as_str()))?;
    let request = NSURLRequest::requestWithURL(&ns_url);
    // SAFETY: main-thread call with a valid request.
    unsafe { view.loadRequest(&request) }
}

/// macOS WebKit has no public switch for Picture in Picture, and video elements report it as
/// unsupported until WebKit's private preference is set; skipped where it does not exist.
fn allow_picture_in_picture(prefs: &objc2_web_kit::WKPreferences) {
    let set = sel!(_setAllowsPictureInPictureMediaPlayback:);
    if prefs.respondsToSelector(set) {
        // SAFETY: the selector exists on this WKPreferences (checked above) and takes a BOOL.
        unsafe {
            let _: () = msg_send![prefs, _setAllowsPictureInPictureMediaPlayback: objc2::runtime::Bool::YES];
        }
    }
}

fn png(image: &NSImage) -> Option<Vec<u8>> {
    let rep = NSBitmapImageRep::imageRepWithData(&*image.TIFFRepresentation()?)?;
    // SAFETY: an empty dictionary is a valid property list for PNG.
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}

/// "Version/x Safari/y" so the user agent matches Safari's instead of standing out (S15).
pub(crate) fn safari_application_name() -> Option<Retained<NSString>> {
    let bundle = NSBundle::bundleWithPath(&NSString::from_str("/Applications/Safari.app"))?;
    let info = bundle.infoDictionary()?;
    let version = info.objectForKey(&NSString::from_str("CFBundleShortVersionString"))?;
    let version = version.downcast::<NSString>().ok()?;
    Some(NSString::from_str(&format!("Version/{version} Safari/605.1.15")))
}

/// WebKit can only remove all user scripts at once, so the others are added back.
fn remove_user_script(controller: &WKUserContentController, script: &WKUserScript) {
    // SAFETY: main thread (WebKit's navigation delegate calls run there); both objects are live.
    unsafe {
        let others: Vec<_> = controller.userScripts().iter().filter(|s| !std::ptr::eq(&**s, script)).collect();
        controller.removeAllUserScripts();
        for other in others {
            controller.addUserScript(&other);
        }
    }
}

impl crate::tarn_ext::ExtensionView for WebView {
    fn set_extension_gate(&self, gate: crate::tarn_ext::Gate) {
        crate::tarn_webkit::set_gate(&self.view, &self._delegate, gate);
    }

    fn add_extension_scripts(&self, scripts: Rc<crate::tarn_ext::WorldScripts>) {
        crate::tarn_webkit::add_scripts(&self.view, scripts);
    }

    fn remove_extension_scripts(&self, world: &crate::tarn_ext::World) {
        crate::tarn_webkit::remove_scripts(&self.view, world);
    }

    fn post_extension_message(&self, world: &crate::tarn_ext::World, frame: Option<crate::tarn_ext::FrameId>, json: &str) {
        crate::tarn_webkit::post(&self.view, world, frame, json);
    }
}

/// Whether ⌥ is down now, wherever the key events go.
pub fn option_held() -> bool {
    objc2_app_kit::NSEvent::modifierFlags_class().contains(objc2_app_kit::NSEventModifierFlags::Option)
}

/// Whether Shift is down now, also before any window has had a key event.
pub fn shift_held() -> bool {
    objc2_app_kit::NSEvent::modifierFlags_class().contains(objc2_app_kit::NSEventModifierFlags::Shift)
}

/// Whether Tarn is the frontmost app.
pub fn app_active() -> bool {
    objc2::MainThreadMarker::new().is_some_and(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).isActive())
}

/// Shows a file in Finder without opening it.
pub fn reveal_in_finder(path: &std::path::Path) {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&objc2_foundation::NSArray::from_retained_slice(&[url]));
}

/// The system "Reduce motion" accessibility setting.
pub fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// The running app bundle's identifier; `None` for a binary run outside a bundle.
pub fn bundle_identifier() -> Option<String> {
    NSBundle::mainBundle().bundleIdentifier().map(|id| id.to_string())
}

/// Whether macOS opens web links from other apps in this app bundle; false outside a bundle.
pub fn is_default_browser() -> bool {
    let bundle = NSBundle::mainBundle();
    if bundle.bundleIdentifier().is_none() {
        return false;
    }
    let Some(probe) = NSURL::URLWithString(&NSString::from_str("https://example.com/")) else { return false };
    let handler = NSWorkspace::sharedWorkspace().URLForApplicationToOpenURL(&probe);
    let path = |url: &NSURL| url.URLByStandardizingPath().and_then(|u| u.path()).map(|p| p.to_string());
    handler.as_deref().and_then(path).is_some_and(|h| Some(h) == path(&bundle.bundleURL()))
}

/// Asks macOS to make this app bundle the handler for `http` and `https`; macOS confirms with
/// the user first, so the result shows later in `is_default_browser`.
pub fn make_default_browser() {
    let bundle = NSBundle::mainBundle();
    if bundle.bundleIdentifier().is_none() {
        return;
    }
    let workspace = NSWorkspace::sharedWorkspace();
    for scheme in ["http", "https"] {
        workspace.setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(&bundle.bundleURL(), &NSString::from_str(scheme), None);
    }
}

/// A modal alert for when no window can show the message; returns once the user dismisses it.
pub fn alert(message: &str, detail: &str) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    objc2_app_kit::NSApplication::sharedApplication(mtm).activate();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(message));
    alert.setInformativeText(&NSString::from_str(detail));
    alert.runModal();
}

/// Hands a URL to the app registered for its scheme. Callers must have asked the user first.
pub fn open_external(url: &Url) -> bool {
    NSURL::URLWithString(&NSString::from_str(url.as_str())).is_some_and(|u| NSWorkspace::sharedWorkspace().openURL(&u))
}

/// Runs `body` as an async function in the shell's isolated world and hands `done` its string
/// answer, empty for anything else.
fn run_isolated(view: &WKWebView, body: &str, done: impl Fn(String) + 'static) {
    let mtm = view.mtm();
    // SAFETY: main-thread call on a live web view; WebKit copies the block and passes a
    // retained result or null, which is only read as an NSString after a class check.
    unsafe {
        let world = WKContentWorld::defaultClientWorld(mtm);
        let handler = RcBlock::new(move |result: *mut objc2::runtime::AnyObject, _error: *mut objc2_foundation::NSError| {
            let text = result.as_ref().and_then(|r| r.downcast_ref::<NSString>()).map(|s| s.to_string()).unwrap_or_default();
            done(text);
        });
        view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(&NSString::from_str(body), None, None, &world, Some(&handler));
    }
}
