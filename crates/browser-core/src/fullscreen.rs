#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageFullScreen {
    entered_window: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shell {
    pub page: bool,
    pub prompt: bool,
    pub overlay: bool,
    pub address: bool,
    pub find: bool,
    pub notice: bool,
}

impl Shell {
    pub fn lets_page_fill_window(self) -> bool {
        self.page && !self.prompt && !self.overlay && !self.address && !self.find && !self.notice
    }
}

impl PageFullScreen {
    pub fn is_on(self) -> bool {
        self.entered_window.is_some()
    }

    pub fn enter(&mut self, prompt: bool, window_full_screen: bool) -> Option<bool> {
        if prompt || self.is_on() {
            return None;
        }
        let enter_window = !window_full_screen;
        self.entered_window = Some(enter_window);
        Some(enter_window)
    }

    pub fn leave(&mut self) -> Option<bool> {
        self.entered_window.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_fills_the_window_until_it_leaves_and_restores_only_what_it_changed() {
        let mut page = PageFullScreen::default();
        assert_eq!(page.enter(true, false), None, "a pending prompt keeps the shell");
        assert!(!page.is_on());
        assert_eq!(page.enter(false, false), Some(true));
        assert!(page.is_on());
        assert_eq!(page.enter(false, true), None, "already on");
        assert_eq!(page.leave(), Some(true), "the window leaves the full screen it entered");
        assert_eq!(page.leave(), None);
        assert_eq!(page.enter(false, true), Some(false));
        assert_eq!(page.leave(), Some(false), "a window already in full screen stays there");
    }

    #[test]
    fn anything_the_shell_must_show_ends_the_page_filling_the_window() {
        let clear = Shell { page: true, ..Shell::default() };
        assert!(clear.lets_page_fill_window());
        assert!(!Shell::default().lets_page_fill_window());
        for shown in [
            Shell { prompt: true, ..clear },
            Shell { overlay: true, ..clear },
            Shell { address: true, ..clear },
            Shell { find: true, ..clear },
            Shell { notice: true, ..clear },
        ] {
            assert!(!shown.lets_page_fill_window(), "{shown:?}");
        }
    }
}
