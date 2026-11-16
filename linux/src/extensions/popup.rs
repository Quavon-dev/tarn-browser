use std::cell::RefCell;
use std::rc::Rc;

use nsh_core::JsonValue;
use nsh_platform_linux::gtk4 as gtk;
use nsh_platform_linux::{ComponentContext, NativeComponent};
use webkit6::prelude::*;

use crate::webview::ext::PageView;

pub const NAME: &str = "tarn.extpopup";

#[derive(Default)]
struct Shown {
    profile: Option<i64>,
    install: String,
    page: Option<Rc<PageView>>,
}

pub struct Popup {
    container: gtk::Box,
    shown: Rc<RefCell<Shown>>,
}

impl Popup {
    pub fn new(_context: ComponentContext) -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);
        Popup { container, shown: Rc::default() }
    }

    fn close(&self) {
        let (profile, install, page) = {
            let mut shown = self.shown.borrow_mut();
            (shown.profile, shown.install.clone(), shown.page.take())
        };
        if let Some(page) = page {
            self.container.remove(&page.web);
        }
        if let Some(profile) = profile.filter(|_| !install.is_empty()) {
            super::popup_closed(profile, &install);
        }
    }

    fn open(&self) {
        let (Some(profile), install) = ({ self.shown.borrow().profile }, self.shown.borrow().install.clone()) else { return };
        if install.is_empty() {
            return;
        }
        let container = self.container.clone();
        let shown = Rc::downgrade(&self.shown);
        super::open_popup(profile, &install.clone(), move |page| {
            let Some(shown) = shown.upgrade() else { return };
            if shown.borrow().install != install {
                return;
            }
            page.web.set_hexpand(true);
            page.web.set_vexpand(true);
            container.append(&page.web);
            shown.borrow_mut().page = Some(page);
        });
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        self.close();
    }
}

impl NativeComponent for Popup {
    fn widget(&self) -> gtk::Widget {
        self.container.clone().upcast()
    }

    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        let value = value.cloned().unwrap_or(JsonValue::Null);
        match name {
            "profile" => {
                let profile = value.as_i64();
                if self.shown.borrow().profile != profile {
                    self.close();
                    self.shown.borrow_mut().profile = profile;
                    self.open();
                }
            }
            "install" => {
                let install = value.as_str().unwrap_or_default().to_owned();
                if self.shown.borrow().install != install {
                    self.close();
                    self.shown.borrow_mut().install = install;
                    self.open();
                }
            }
            _ => {}
        }
    }

    fn command(&mut self, _name: &str, _args: &JsonValue) {}
}
