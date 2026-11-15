use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu,
};

use crate::Host;
use crate::host_windows::wide;
pub use crate::menu_item::Item;

/// Pops up `items` as a native menu at the mouse location and calls `chosen` with the value of
/// the entry the user picks. Windows runs a modal loop inside this call, so the caller must not
/// hold UI state borrowed across it.
pub fn show<T: 'static>(host: &Host, items: Vec<Item<T>>, chosen: impl FnOnce(T) + 'static) {
    let mut values = Vec::new();
    // SAFETY: the menu is built, tracked and destroyed on this thread; strings outlive each append.
    unsafe {
        let menu = build(items, &mut values);
        if menu.is_null() {
            return;
        }
        let mut at = POINT { x: 0, y: 0 };
        GetCursorPos(&mut at);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, at.x, at.y, 0, host.hwnd, std::ptr::null());
        DestroyMenu(menu);
        if let Some(value) = usize::try_from(id).ok().and_then(|id| id.checked_sub(1)).and_then(|ix| values.into_iter().nth(ix)) {
            chosen(value);
        }
    }
}

/// Command ids are the value's index plus one: TrackPopupMenu returns zero for no choice.
unsafe fn build<T>(items: Vec<Item<T>>, values: &mut Vec<T>) -> HMENU {
    // SAFETY: the caller destroys the returned menu, which owns its submenus.
    unsafe {
        let menu = CreatePopupMenu();
        for item in items {
            match item {
                Item::Separator => {
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                }
                Item::Submenu(title, children) => {
                    let submenu = build(children, values);
                    AppendMenuW(menu, MF_POPUP, submenu as usize, wide(&title.replace('&', "&&")).as_ptr());
                }
                Item::Action { title, key, checked, enabled, value } => {
                    values.push(value);
                    let title = title.replace('&', "&&");
                    let label = if key.is_empty() { title } else { format!("{title}\t{}", shortcut(&key)) };
                    let flags = MF_STRING | if checked { MF_CHECKED } else { 0 } | if enabled { 0 } else { MF_GRAYED };
                    AppendMenuW(menu, flags, values.len(), wide(&label).as_ptr());
                }
            }
        }
        menu
    }
}

/// `cmd-shift-t` as Windows writes it, Ctrl+Shift+T, with the shell's Windows keys: ⌘ is Ctrl,
/// ⌃ with a digit is Alt, ⌘⌃ is Alt+Shift and ⌘⌃F is F11.
fn shortcut(keys: &str) -> String {
    let (mods, key) = match keys.strip_suffix("--") {
        Some(mods) => (mods, "-"),
        None => keys.rsplit_once('-').unwrap_or(("", keys)),
    };
    let has = |m: &str| mods.split('-').any(|p| p == m);
    let (cmd, ctrl) = (has("cmd"), has("ctrl"));
    if cmd && ctrl && key == "f" {
        return "F11".to_owned();
    }
    let digit = !key.is_empty() && key.chars().all(|c| c.is_ascii_digit());
    let (ctrl, alt, shift) = match (cmd, ctrl) {
        (true, true) => (false, true, true),
        (false, true) if digit => (false, true, has("shift")),
        _ => (cmd || ctrl, has("alt"), has("shift")),
    };
    let key = if key.len() == 1 { key.to_uppercase() } else { key[..1].to_uppercase() + &key[1..] };
    [(ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift")].iter().filter(|(on, _)| *on).map(|(_, m)| *m).chain([key.as_str()]).collect::<Vec<_>>().join("+")
}

/// An image beside a menu entry; Windows menus show none of them.
pub enum Image {
    Png(Vec<u8>),
    Symbol(&'static str),
    Dot(u32),
}

pub struct Mark {
    pub title: String,
    pub image: Option<Image>,
    pub checked: bool,
}

/// The app menu bar is macOS only.
pub fn decorate(_menu: &str, _marks: &[Mark]) {}

pub fn list_windows() {}

pub fn disable_saved_application_state() {}

/// Windows has no system-wide natural scrolling switch; touchpads scroll content with the fingers.
pub fn natural_scrolling() -> bool {
    true
}

/// Removes the event monitor when dropped.
pub struct EventMonitor(());

impl Host {
    /// Pages get mouse events themselves on Windows; GPUI sees those outside the page.
    pub fn on_navigation_buttons(&self, _navigate: impl Fn(bool) + 'static) -> Option<EventMonitor> {
        None
    }

    pub fn on_mouse_moved(&self, _moved: impl Fn(f64, f64, bool) + 'static) -> Option<EventMonitor> {
        None
    }

    pub fn on_page_mouse_down(&self, _clicked: impl Fn() + 'static) -> Option<EventMonitor> {
        None
    }

    pub fn on_escape(&self, _pressed: impl Fn() + 'static) -> Option<EventMonitor> {
        None
    }

    pub fn on_mouse_down(&self, _clicked: impl Fn(f64, f64) + 'static) -> Option<EventMonitor> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shortcuts_read_as_on_windows() {
        assert_eq!(super::shortcut("cmd-shift-t"), "Ctrl+Shift+T");
        assert_eq!(super::shortcut("alt-left"), "Alt+Left");
        assert_eq!(super::shortcut("cmd-ctrl-n"), "Alt+Shift+N");
        assert_eq!(super::shortcut("ctrl-cmd-n"), "Alt+Shift+N");
        assert_eq!(super::shortcut("ctrl-cmd-f"), "F11");
        assert_eq!(super::shortcut("ctrl-2"), "Alt+2");
        assert_eq!(super::shortcut("shift-ctrl-cmd-t"), "Alt+Shift+T");
        assert_eq!(super::shortcut("cmd--"), "Ctrl+-");
    }
}
