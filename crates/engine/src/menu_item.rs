/// One entry of a native context menu; `value` is handed back when the entry is chosen.
pub enum Item<T> {
    Action { title: String, key: String, checked: bool, enabled: bool, image: Option<crate::menu::Image>, value: T },
    Submenu(String, Vec<Item<T>>),
    Separator,
}

impl<T> Item<T> {
    pub fn action(title: impl Into<String>, value: T) -> Self {
        Item::Action { title: title.into(), key: String::new(), checked: false, enabled: true, image: None, value }
    }

    /// Shows a shortcut such as `cmd-shift-t` next to the title.
    pub fn key(mut self, shortcut: impl Into<String>) -> Self {
        if let Item::Action { key, .. } = &mut self {
            *key = shortcut.into();
        }
        self
    }

    pub fn checked(mut self, on: bool) -> Self {
        if let Item::Action { checked, .. } = &mut self {
            *checked = on;
        }
        self
    }

    /// Shows `picture` beside the title, where the system's menus can.
    pub fn image(mut self, picture: Option<crate::menu::Image>) -> Self {
        if let Item::Action { image, .. } = &mut self {
            *image = picture;
        }
        self
    }

    pub fn enabled(mut self, on: bool) -> Self {
        if let Item::Action { enabled, .. } = &mut self {
            *enabled = on;
        }
        self
    }
}
