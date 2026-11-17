use browser_core::fullscreen::Shell;
use browser_core::tabs::TabKey;
use nsh_core::desktop::WindowMode;
use serde_json::Value;

use super::Browser;

impl Browser {
    pub(super) fn on_fullscreen(&mut self, key: TabKey, data: &Value) {
        if data["on"].as_bool() != Some(true) {
            if self.model.active == Some(key) {
                self.leave_page_fullscreen();
            }
            return;
        }
        let window_full_screen = data["windowFullScreen"].as_bool() == Some(true);
        let prompt = self.pages.get(&key).is_some_and(|p| !p.prompts.is_empty());
        let entered = if self.model.active == Some(key) { self.page_fullscreen.enter(prompt, window_full_screen) } else { None };
        self.window_request = Some(if entered.is_some() || window_full_screen { WindowMode::FullScreen } else { self.restore_mode });
    }

    pub fn leave_page_fullscreen(&mut self) {
        if let Some(entered) = self.page_fullscreen.leave() {
            self.window_request = Some(if entered { self.restore_mode } else { WindowMode::FullScreen });
        }
    }

    pub fn toggle_full_screen(&mut self) {
        if self.page_fullscreen.is_on() {
            return self.leave_page_fullscreen();
        }
        self.window_request = Some(if self.window_mode == WindowMode::FullScreen { self.restore_mode } else { WindowMode::FullScreen });
    }

    pub fn window_changed(&mut self, mode: WindowMode) {
        self.window_mode = mode;
        if mode != WindowMode::FullScreen {
            self.restore_mode = mode;
        }
    }

    pub fn keep_page_fullscreen(&mut self, overlay: bool, address: bool, find: bool) {
        let page = self.active_page();
        let shell = Shell {
            page: page.is_some(),
            prompt: page.is_some_and(|p| !p.prompts.is_empty()),
            overlay: overlay || self.reader.as_ref().is_some_and(|r| self.model.active == Some(r.key)),
            address,
            find,
            notice: self.notice.is_some(),
        };
        if self.page_fullscreen.is_on() && !shell.lets_page_fill_window() {
            self.leave_page_fullscreen();
        }
    }
}
