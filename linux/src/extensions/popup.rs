use nsh_core::JsonValue;
use nsh_platform_linux::gtk4 as gtk;
use nsh_platform_linux::gtk4::prelude::*;
use nsh_platform_linux::{ComponentContext, NativeComponent};

use crate::webview::ext;

pub const NAME: &str = "tarn.extpopup";

pub struct Popup {
    container: gtk::Box,
    page: Option<u64>,
}

impl Popup {
    pub fn new(_context: ComponentContext) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        Popup { container, page: None }
    }
}

impl NativeComponent for Popup {
    fn widget(&self) -> gtk::Widget {
        self.container.clone().upcast()
    }

    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        if name != "page" {
            return;
        }
        let page = value.and_then(JsonValue::as_u64);
        if page == self.page {
            return;
        }
        while let Some(child) = self.container.first_child() {
            self.container.remove(&child);
        }
        self.page = page;
        if let Some(page) = page {
            ext::show_page(page, &self.container);
        }
    }

    fn command(&mut self, _name: &str, _args: &JsonValue) {}
}
