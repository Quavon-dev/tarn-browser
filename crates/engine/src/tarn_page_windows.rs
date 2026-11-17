//! An install's own page on Windows (`tarn_ext::PageKind`): a Chromium view of the profile's
//! store that loads only the install's `tarn-extension://` origin. The background page stays
//! hidden; the popup is placed under the toolbar button until it is closed.

use std::rc::Rc;

use shield::ShieldLevel;
use url::Url;

use crate::tarn_ext::{PageSpec, SCHEME};
use crate::{DataStore, Event, Host};

const POPUP_SIZE: (f64, f64) = (360.0, 480.0);

pub struct ExtensionPage {
    view: crate::chromium::WebView,
    origin: String,
    closed: Rc<dyn Fn()>,
}

impl ExtensionPage {
    /// `closed` runs when the popup is closed. A Lockdown `level` makes no page.
    pub fn new(host: &Host, data: &DataStore, level: ShieldLevel, spec: PageSpec, sink: Rc<dyn Fn(Event)>, closed: Rc<dyn Fn()>) -> Option<Self> {
        if level == ShieldLevel::Lockdown {
            return None;
        }
        let origin = spec.origin.clone();
        let inspectable = spec.inspectable;
        let view = crate::chromium::WebView::for_store(host, data, false, true, inspectable, sink);
        view.set_extension_page(spec);
        view.set_visible(false);
        Some(ExtensionPage { view, origin, closed })
    }

    pub fn load(&self, url: &Url) {
        if url.scheme() == SCHEME && url.host_str() == Some(self.origin.as_str()) {
            self.view.load(url);
        }
    }

    pub fn post(&self, json: &str) {
        self.view.post_to_page(json);
    }

    /// Shows the page below `x, y` (shell points, origin top-left), its right edge there.
    pub fn show_popup(&self, _host: &Host, x: f64, y: f64) {
        self.view.set_frame((x - POPUP_SIZE.0).max(0.0), y, POPUP_SIZE.0, POPUP_SIZE.1);
        self.view.set_visible(true);
        self.view.raise();
        self.view.focus();
    }

    pub fn close_popup(&self) {
        self.view.set_visible(false);
        (self.closed)();
    }
}
