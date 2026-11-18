use std::rc::Rc;

use shield::ShieldLevel;
use url::Url;

use crate::handover::Storage;
use crate::nav_policy::SiteFor;
use crate::{DataStore, Event, Host, Popup};

/// A tab's page. Chromium is the only engine on Windows; the shell treats it as its default
/// engine, so `is_chromium` is false and no engine switch is offered.
pub struct WebView {
    view: crate::chromium::WebView,
    filtered: bool,
}

impl WebView {
    /// A Lockdown `level` makes a view without JavaScript that loads only Lockdown sites; `site_for`
    /// is enforced through the data store's Shield, which the shell keeps current with
    /// `DataStore::set_shield`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(host: &Host, data: &DataStore, level: ShieldLevel, filtered: bool, inspectable: bool, _site_for: SiteFor, sink: Rc<dyn Fn(Event)>) -> Self {
        WebView { view: crate::chromium::WebView::for_store(host, data, level == ShieldLevel::Lockdown, filtered, inspectable, sink), filtered }
    }

    pub fn is_chromium(&self) -> bool {
        false
    }

    pub fn adopt(popup: Popup, host: &Host, inspectable: bool, sink: Rc<dyn Fn(Event)>) -> Option<Self> {
        let view = popup.view.rehost(host, sink.clone());
        view.set_inspectable(inspectable);
        for event in popup.pending.take() {
            sink(event);
        }
        Some(WebView { filtered: view.filtered(), view })
    }

    pub fn rehost(self, host: &Host, sink: Rc<dyn Fn(Event)>) -> Self {
        WebView { view: self.view.rehost(host, sink), filtered: self.filtered }
    }

    pub fn load(&self, url: &Url) {
        self.view.load(url);
    }

    pub fn load_file(&self, url: &Url) {
        self.view.load_file(url);
    }

    pub fn load_link(&self, url: &Url, opener: Option<&Url>) {
        self.view.load_link_from(url, opener);
    }

    pub fn set_frame(&self, x: f64, y: f64, width: f64, height: f64) {
        self.view.set_frame(x, y, width, height);
    }

    pub fn set_corner_radius(&self, radius: f64) {
        self.view.set_corner_radius(radius);
    }

    /// Two-finger swipe back and forward on a precision touchpad, for every view at once.
    pub fn set_gestures(&self, on: bool) {
        self.view.set_gestures(on);
    }

    pub fn set_visible(&self, visible: bool) {
        self.view.set_visible(visible);
    }

    pub fn set_alpha(&self, alpha: f64) {
        self.view.set_alpha(alpha);
    }

    pub fn raise(&self) {
        self.view.raise();
    }

    pub fn focus(&self) {
        self.view.focus();
    }

    pub fn filtered(&self) -> bool {
        self.filtered
    }

    pub fn url(&self) -> Option<Url> {
        self.view.url()
    }

    pub fn find(&self, text: &str, backwards: bool, found: impl Fn(usize, Option<usize>) + 'static) {
        self.view.find(text, backwards, found);
    }

    pub fn clear_find(&self) {
        self.view.clear_find();
    }

    pub fn find_text(&self, query: &str, done: impl Fn(Vec<String>) + 'static) {
        self.view.find_text(query, done);
    }

    pub fn pick_element(&self, done: impl Fn(String) + 'static) {
        self.view.pick_element(done);
    }

    pub fn icon_links(&self, origin: &Url, done: impl Fn(String) + 'static) {
        self.view.icon_links(origin, done);
    }

    pub fn fetch_icon(&self, origin: &Url, urls: &[Url], max_bytes: usize, done: impl Fn(String) + 'static) {
        self.view.fetch_icon(origin, urls, max_bytes, done);
    }

    pub fn page_text(&self, origin: &Url, max_chars: usize, done: impl Fn(String) + 'static) {
        self.view.page_text(origin, max_chars, done);
    }

    pub fn article(&self, origin: &Url, probe: bool, done: impl Fn(String) + 'static) {
        self.view.article(origin, probe, done);
    }

    pub fn json_text(&self, origin: &Url, max_bytes: usize, done: impl Fn(String) + 'static) {
        self.view.json_text(origin, max_bytes, done);
    }

    /// On-device translation is macOS's Translation framework; there is none on Windows.
    pub fn translation_step(&self, _step: &crate::translation::Step, done: impl Fn(String) + 'static) {
        done(String::new());
    }

    pub fn process_id(&self) -> Option<i32> {
        self.view.process_id()
    }

    pub fn is_muted(&self) -> bool {
        self.view.is_muted()
    }

    pub fn media_playing(&self, done: impl Fn(bool) + 'static) {
        self.view.media_playing(done);
    }

    pub fn set_muted(&self, muted: bool) {
        self.view.set_muted(muted);
    }

    pub fn print(&self) {
        self.view.print();
    }

    pub fn snapshot_png(&self, width: Option<f64>, done: impl Fn(Option<Vec<u8>>) + 'static) {
        self.view.snapshot_png(width, done);
    }

    pub fn pdf(&self, done: impl Fn(Option<Vec<u8>>) + 'static) {
        self.view.pdf(done);
    }

    pub fn picture_in_picture(&self, done: impl Fn(String) + 'static) {
        self.view.picture_in_picture(done);
    }

    pub fn title(&self) -> String {
        self.view.title()
    }

    pub fn back(&self) {
        self.view.back();
    }

    pub fn forward(&self) {
        self.view.forward();
    }

    pub fn reload_from_origin(&self) {
        self.view.reload_from_origin();
    }

    pub fn reload(&self) {
        self.view.reload();
    }

    pub fn recover(&self) {
        self.view.recover();
    }

    pub fn stop(&self) {
        self.view.stop();
    }

    pub fn edit(&self, command: crate::chromium::EditCommand) {
        self.view.edit(command);
    }

    pub fn zoom(&self) -> f64 {
        self.view.zoom()
    }

    pub fn set_zoom(&self, zoom: f64) {
        self.view.set_zoom(zoom);
    }

    pub fn read_storage(&self, done: impl FnOnce(Option<Storage>) + 'static) {
        self.view.read_storage(done);
    }

    pub fn hand_over(&self, storage: Storage) {
        self.view.hand_over(storage);
    }

    /// Chromium cannot take a back list back, so Tarn saves the URLs of its own and steps through
    /// them itself after a restore (`chromium::WebView::restore_history`).
    pub fn interaction_state(&self) -> Option<Vec<u8>> {
        self.view.saved_history()
    }

    /// Windows only ever saved Tarn's own format; anything else is ignored.
    pub fn restore_interaction_state(&self, bytes: &[u8]) {
        self.view.restore_history(bytes);
    }
}

impl crate::tarn_ext::ExtensionView for WebView {
    fn set_extension_gate(&self, gate: crate::tarn_ext::Gate) {
        self.view.set_extension_gate(gate);
    }

    fn add_extension_scripts(&self, scripts: Rc<crate::tarn_ext::WorldScripts>) {
        self.view.add_extension_scripts(scripts);
    }

    fn remove_extension_scripts(&self, world: &crate::tarn_ext::World) {
        self.view.remove_extension_scripts(world);
    }

    fn post_extension_message(&self, world: &crate::tarn_ext::World, frame: Option<crate::tarn_ext::FrameId>, json: &str) {
        self.view.post_extension_message(world, frame, json);
    }
}
