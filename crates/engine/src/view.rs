use std::rc::Rc;

use shield::ShieldLevel;
use url::Url;

use crate::handover::Storage;
use crate::nav_policy::SiteFor;
use crate::webview::{self, Host, Popup};
use crate::{DataStore, Event};

pub enum WebView {
    WebKit(webview::WebView),
    #[cfg(feature = "chromium")]
    Chromium(crate::chromium::WebView),
}

macro_rules! both {
    ($self:ident, $v:ident => $e:expr) => {
        match $self {
            WebView::WebKit($v) => $e,
            #[cfg(feature = "chromium")]
            WebView::Chromium($v) => $e,
        }
    };
}

macro_rules! webkit_only {
    ($self:ident, $v:ident => $e:expr, $otherwise:expr) => {
        match $self {
            WebView::WebKit($v) => $e,
            #[cfg(feature = "chromium")]
            WebView::Chromium(_) => $otherwise,
        }
    };
}

impl WebView {
    pub fn new(host: &Host, data: &DataStore, level: ShieldLevel, filtered: bool, inspectable: bool, site_for: SiteFor, sink: Rc<dyn Fn(Event)>) -> Self {
        WebView::WebKit(webview::WebView::new(host, data, level, filtered, inspectable, site_for, sink))
    }

    /// A Chromium view for the profile with this UUID, Chromium's data under `root`; `data` is the
    /// profile's WebKit store, whose cookies the bridge shares. `None` when Chromium cannot run in
    /// this build or belongs to another profile.
    #[cfg(feature = "chromium")]
    pub fn chromium(host: &Host, data: &DataStore, root: &std::path::Path, profile: &str, inspectable: bool, sink: Rc<dyn Fn(Event)>) -> Option<Self> {
        crate::chromium::WebView::new(host, data, root, profile, inspectable, sink).map(WebView::Chromium)
    }

    pub fn is_chromium(&self) -> bool {
        webkit_only!(self, _v => false, true)
    }

    pub fn adopt(popup: Popup, host: &Host, inspectable: bool, sink: Rc<dyn Fn(Event)>) -> Option<Self> {
        webview::WebView::adopt(popup, host, inspectable, sink).map(WebView::WebKit)
    }

    pub fn rehost(self, host: &Host, sink: Rc<dyn Fn(Event)>) -> Self {
        match self {
            WebView::WebKit(v) => WebView::WebKit(v.rehost(host, sink)),
            #[cfg(feature = "chromium")]
            WebView::Chromium(v) => WebView::Chromium(v.rehost(host, sink)),
        }
    }

    pub fn load(&self, url: &Url) {
        both!(self, v => v.load(url))
    }

    /// Shows a file the user picked with Open File; `load` refuses `file:` URLs.
    pub fn load_file(&self, url: &Url) {
        both!(self, v => v.load_file(url))
    }

    /// A load the page, not the user, is credited with, so S10 applies to it in both engines.
    pub fn load_link(&self, url: &Url, opener: Option<&Url>) {
        match self {
            WebView::WebKit(v) => v.load_link(url, opener),
            #[cfg(feature = "chromium")]
            WebView::Chromium(v) => v.load_link(url),
        }
    }

    pub fn set_frame(&self, x: f64, y: f64, width: f64, height: f64) {
        both!(self, v => v.set_frame(x, y, width, height))
    }

    pub fn set_corner_radius(&self, radius: f64) {
        webkit_only!(self, v => v.set_corner_radius(radius), ())
    }

    pub fn set_dark(&self, dark: bool) {
        both!(self, v => v.set_dark(dark))
    }

    pub fn set_gestures(&self, on: bool) {
        webkit_only!(self, v => v.set_gestures(on), ())
    }

    pub fn set_visible(&self, visible: bool) {
        both!(self, v => v.set_visible(visible))
    }

    pub fn set_alpha(&self, alpha: f64) {
        both!(self, v => v.set_alpha(alpha))
    }

    pub fn raise(&self) {
        both!(self, v => v.raise())
    }

    pub fn focus(&self) {
        both!(self, v => v.focus())
    }

    pub fn filtered(&self) -> bool {
        webkit_only!(self, v => v.filtered(), false)
    }

    pub fn url(&self) -> Option<Url> {
        both!(self, v => v.url())
    }

    pub fn find(&self, text: &str, backwards: bool, found: impl Fn(usize, Option<usize>) + 'static) {
        both!(self, v => v.find(text, backwards, found))
    }

    pub fn clear_find(&self) {
        both!(self, v => v.clear_find())
    }

    pub fn find_text(&self, query: &str, done: impl Fn(Vec<String>) + 'static) {
        both!(self, v => v.find_text(query, done))
    }

    pub fn pick_element(&self, done: impl Fn(String) + 'static) {
        webkit_only!(self, v => v.pick_element(done), done(String::new()))
    }

    pub fn icon_links(&self, origin: &Url, done: impl Fn(String) + 'static) {
        both!(self, v => v.icon_links(origin, done))
    }

    pub fn fetch_icon(&self, origin: &Url, urls: &[Url], max_bytes: usize, done: impl Fn(String) + 'static) {
        both!(self, v => v.fetch_icon(origin, urls, max_bytes, done))
    }

    pub fn page_text(&self, origin: &Url, max_chars: usize, done: impl Fn(String) + 'static) {
        both!(self, v => v.page_text(origin, max_chars, done))
    }

    pub fn article(&self, origin: &Url, probe: bool, done: impl Fn(String) + 'static) {
        both!(self, v => v.article(origin, probe, done))
    }

    pub fn json_text(&self, origin: &Url, max_bytes: usize, done: impl Fn(String) + 'static) {
        webkit_only!(self, v => v.json_text(origin, max_bytes, done), done(String::new()))
    }

    pub fn translation_step(&self, step: &crate::translation::Step, done: impl Fn(String) + 'static) {
        webkit_only!(self, v => v.translation_step(step, done), done(String::new()))
    }

    pub fn is_muted(&self) -> bool {
        both!(self, v => v.is_muted())
    }

    pub fn media_playing(&self, done: impl Fn(bool) + 'static) {
        both!(self, v => v.media_playing(done))
    }

    /// The page's own process; in Chromium the renderer of the top document, once it said which.
    pub fn process_id(&self) -> Option<i32> {
        both!(self, v => v.process_id())
    }

    pub fn set_muted(&self, muted: bool) {
        both!(self, v => v.set_muted(muted))
    }

    pub fn print(&self) {
        both!(self, v => v.print())
    }

    pub fn snapshot_png(&self, width: Option<f64>, done: impl Fn(Option<Vec<u8>>) + 'static) {
        webkit_only!(self, v => v.snapshot_png(width, done), done(None))
    }

    pub fn pdf(&self, done: impl Fn(Option<Vec<u8>>) + 'static) {
        webkit_only!(self, v => v.pdf(done), done(None))
    }

    pub fn picture_in_picture(&self, done: impl Fn(String) + 'static) {
        webkit_only!(self, v => v.picture_in_picture(done), done("refused".into()))
    }

    pub fn title(&self) -> String {
        both!(self, v => v.title())
    }

    pub fn back(&self) {
        both!(self, v => v.back())
    }

    pub fn forward(&self) {
        both!(self, v => v.forward())
    }

    pub fn reload_from_origin(&self) {
        both!(self, v => v.reload_from_origin())
    }

    pub fn reload(&self) {
        both!(self, v => v.reload())
    }

    pub fn recover(&self) {
        both!(self, v => v.recover())
    }

    pub fn stop(&self) {
        both!(self, v => v.stop())
    }

    pub fn zoom(&self) -> f64 {
        both!(self, v => v.zoom())
    }

    pub fn set_zoom(&self, zoom: f64) {
        both!(self, v => v.set_zoom(zoom))
    }

    /// The storage of the page's top-level origin, read in an isolated world, for an engine switch
    /// the user started.
    pub fn read_storage(&self, done: impl FnOnce(Option<Storage>) + 'static) {
        both!(self, v => v.read_storage(done))
    }

    /// Hands `storage` to this new view of the same tab and profile; call before its first load.
    /// The caller never hands over a Lockdown site's storage.
    pub fn hand_over(&self, storage: Storage) {
        both!(self, v => v.hand_over(storage))
    }

    pub fn interaction_state(&self) -> Option<Vec<u8>> {
        webkit_only!(self, v => v.interaction_state(), None)
    }

    pub fn restore_interaction_state(&self, bytes: &[u8]) {
        webkit_only!(self, v => v.restore_interaction_state(bytes), ())
    }
}

impl crate::tarn_ext::ExtensionView for WebView {
    fn set_extension_gate(&self, gate: crate::tarn_ext::Gate) {
        both!(self, v => v.set_extension_gate(gate))
    }

    fn add_extension_scripts(&self, scripts: Rc<crate::tarn_ext::WorldScripts>) {
        both!(self, v => v.add_extension_scripts(scripts))
    }

    fn remove_extension_scripts(&self, world: &crate::tarn_ext::World) {
        both!(self, v => v.remove_extension_scripts(world))
    }

    fn post_extension_message(&self, world: &crate::tarn_ext::World, frame: Option<crate::tarn_ext::FrameId>, json: &str) {
        both!(self, v => v.post_extension_message(world, frame, json))
    }
}
