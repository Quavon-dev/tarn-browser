use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, SelectObject,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconFromResourceEx, CreatePopupMenu, DI_NORMAL, DestroyIcon, DestroyMenu, DrawIconEx, GetCursorPos, HMENU, LR_DEFAULTCOLOR,
    MENUITEMINFOW, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MIIM_BITMAP, SM_CXSMICON, SetMenuItemInfoW, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu,
};

use crate::Host;
use crate::host_windows::wide;
pub use crate::menu_item::Item;

/// Pops up `items` as a native menu at `at` (points from the top left of `host`'s client area,
/// such as a button's lower left corner) or else at the mouse location, and calls `chosen` with
/// the value of the entry the user picks. Windows runs a modal loop inside this call, so the
/// caller must not hold UI state borrowed across it.
pub fn show<T: 'static>(host: &Host, items: Vec<Item<T>>, at: Option<(f64, f64)>, chosen: impl FnOnce(T) + 'static) {
    let (mut values, mut bitmaps) = (Vec::new(), Vec::new());
    // SAFETY: a plain query of the host window.
    let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(host.hwnd)) };
    // SAFETY: the menu is built, tracked and destroyed on this thread; strings outlive each append,
    // and the bitmaps outlive the menu, which does not delete them. `ClientToScreen` and
    // `GetCursorPos` write only to a local POINT, and `host.hwnd` is this thread's live window.
    unsafe {
        let menu = build(items, &mut values, &mut bitmaps, size);
        if menu.is_null() {
            return;
        }
        let anchored = at.and_then(|(x, y)| {
            let (x, y, _, _) = host.pixels(x, y, 0., 0.);
            let mut point = POINT { x, y };
            (ClientToScreen(host.hwnd, &mut point) != 0).then_some(point)
        });
        let at = anchored.unwrap_or_else(|| {
            let mut point = POINT { x: 0, y: 0 };
            GetCursorPos(&mut point);
            point
        });
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, at.x, at.y, 0, host.hwnd, std::ptr::null());
        DestroyMenu(menu);
        for bitmap in bitmaps {
            DeleteObject(bitmap);
        }
        if let Some(value) = usize::try_from(id).ok().and_then(|id| id.checked_sub(1)).and_then(|ix| values.into_iter().nth(ix)) {
            chosen(value);
        }
    }
}

/// Command ids are the value's index plus one: TrackPopupMenu returns zero for no choice.
unsafe fn build<T>(items: Vec<Item<T>>, values: &mut Vec<T>, bitmaps: &mut Vec<HBITMAP>, size: i32) -> HMENU {
    // SAFETY: the caller destroys the returned menu, which owns its submenus.
    unsafe {
        let menu = CreatePopupMenu();
        for item in items {
            match item {
                Item::Separator => {
                    AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
                }
                Item::Submenu(title, children) => {
                    let submenu = build(children, values, bitmaps, size);
                    AppendMenuW(menu, MF_POPUP, submenu as usize, wide(&title.replace('&', "&&")).as_ptr());
                }
                Item::Action { title, key, checked, enabled, image, value } => {
                    values.push(value);
                    let title = title.replace('&', "&&");
                    let label = if key.is_empty() { title } else { format!("{title}\t{}", shortcut(&key)) };
                    let flags = MF_STRING | if checked { MF_CHECKED } else { 0 } | if enabled { 0 } else { MF_GRAYED };
                    AppendMenuW(menu, flags, values.len(), wide(&label).as_ptr());
                    if let Some(bitmap) = image.and_then(|i| bitmap(&i, size)) {
                        bitmaps.push(bitmap);
                        let info = MENUITEMINFOW { cbSize: size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_BITMAP, hbmpItem: bitmap, ..std::mem::zeroed() };
                        SetMenuItemInfoW(menu, values.len() as u32, 0, &info);
                    }
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

/// A `size`-pixel square 32-bit bitmap of `image`, premultiplied as menus draw it; `None` for a
/// system symbol, which Windows does not have.
fn bitmap(image: &Image, size: i32) -> Option<HBITMAP> {
    if size <= 0 || matches!(image, Image::Symbol(_)) {
        return None;
    }
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: size,
        biHeight: -size,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..Default::default()
    };
    let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
    let mut bits = std::ptr::null_mut();
    // SAFETY: a DIB section of `size` × `size` 32-bit pixels, which `bits` then points to.
    let bitmap = unsafe { CreateDIBSection(std::ptr::null_mut(), &info, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() {
        return None;
    }
    let drawn = match image {
        Image::Dot(rgb) => {
            // SAFETY: the section holds exactly `size * size` pixels and nothing else uses it yet.
            let pixels = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u32>(), (size * size) as usize) };
            dot(pixels, size, *rgb);
            true
        }
        Image::Png(png) | Image::Template(png) => icon_into(bitmap, png, size),
        Image::Symbol(_) => false,
    };
    if !drawn {
        // SAFETY: our own bitmap, not selected into any DC.
        unsafe { DeleteObject(bitmap) };
        return None;
    }
    Some(bitmap)
}

/// A filled circle of colour `rgb` (`0xRRGGBB`), as large as AppKit's menu dot, antialiased.
fn dot(pixels: &mut [u32], size: i32, rgb: u32) {
    let (centre, radius) = (f64::from(size) / 2., f64::from(size) * 5. / 16.);
    for y in 0..size {
        for x in 0..size {
            let distance = (f64::from(x) + 0.5 - centre).hypot(f64::from(y) + 0.5 - centre);
            let alpha = (radius + 0.5 - distance).clamp(0., 1.);
            let channel = |shift: u32| ((f64::from((rgb >> shift) & 0xff) * alpha).round() as u32) << shift;
            pixels[(y * size + x) as usize] = ((alpha * 255.).round() as u32) << 24 | channel(16) | channel(8) | channel(0);
        }
    }
}

/// Draws a PNG site icon into `bitmap`; Windows reads PNG data as an icon image.
fn icon_into(bitmap: HBITMAP, png: &[u8], size: i32) -> bool {
    let Ok(length) = u32::try_from(png.len()) else { return false };
    // SAFETY: `png` outlives the call, which copies what it reads; the icon and the DC are ours
    // and released below, and the bitmap is deselected before the DC goes.
    unsafe {
        let icon = CreateIconFromResourceEx(png.as_ptr(), length, 1, 0x0003_0000, size, size, LR_DEFAULTCOLOR);
        if icon.is_null() {
            return false;
        }
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        let old = SelectObject(dc, bitmap);
        let drawn = DrawIconEx(dc, 0, 0, icon, size, size, 0, std::ptr::null_mut(), DI_NORMAL) != 0;
        SelectObject(dc, old);
        DeleteDC(dc);
        DestroyIcon(icon);
        drawn
    }
}

pub enum Image {
    Png(Vec<u8>),
    Template(Vec<u8>),
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
pub struct EventMonitor(usize);

impl Drop for EventMonitor {
    fn drop(&mut self) {
        crate::chromium::unwatch(self.0);
    }
}

impl Host {
    fn watch(&self, watch: crate::chromium::Watch) -> Option<EventMonitor> {
        Some(EventMonitor(crate::chromium::watch(self.hwnd, watch)))
    }

    /// Calls `navigate(true)` for the mouse's back button and `navigate(false)` for forward over
    /// the host's pages, and keeps them from the page; GPUI sees the buttons outside the pages.
    pub fn on_navigation_buttons(&self, navigate: impl Fn(bool) + 'static) -> Option<EventMonitor> {
        self.watch(crate::chromium::Watch::Buttons(Box::new(navigate)))
    }

    /// Calls `moved(x, y, pressed)` for each pointer move over the app's windows, pages
    /// included, in the host's top-left points; `pressed` while any mouse button is down.
    pub fn on_mouse_moved(&self, moved: impl Fn(f64, f64, bool) + 'static) -> Option<EventMonitor> {
        self.watch(crate::chromium::Watch::Moved(Box::new(moved)))
    }

    /// Calls `clicked` when a mouse button goes down over a page in the host's window.
    pub fn on_page_mouse_down(&self, clicked: impl Fn() + 'static) -> Option<EventMonitor> {
        self.watch(crate::chromium::Watch::PageDown(Box::new(clicked)))
    }

    /// Calls `pressed` for Esc in the host's window, also while a page has keyboard focus, and
    /// keeps the key from the page.
    pub fn on_escape(&self, pressed: impl Fn() + 'static) -> Option<EventMonitor> {
        self.watch(crate::chromium::Watch::Escape(Box::new(pressed)))
    }

    /// Calls `clicked(x, y)` for each left mouse-down in the host's window, in its top-left
    /// points; the click still reaches its target.
    pub fn on_mouse_down(&self, clicked: impl Fn(f64, f64) + 'static) -> Option<EventMonitor> {
        self.watch(crate::chromium::Watch::Down(Box::new(clicked)))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_dot_is_a_premultiplied_circle_of_its_colour() {
        let mut pixels = vec![0u32; 16 * 16];
        super::dot(&mut pixels, 16, 0x00ff_8000);
        assert_eq!(pixels[8 * 16 + 8], 0xffff_8000, "the centre is opaque");
        assert_eq!(pixels[0], 0, "a corner is clear");
        assert!(pixels.iter().all(|p| (p >> 16) & 0xff <= p >> 24), "colour never exceeds alpha");
    }

    #[test]
    fn menu_entries_carry_dot_and_site_icon_bitmaps() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMenuItemInfoW, MENUITEMINFOW, MIIM_BITMAP};
        let icon = include_bytes!("../../../docs/media/icon.png").to_vec();
        let items = vec![
            super::Item::action("Dot", 0).image(Some(super::Image::Dot(0x3366ff))),
            super::Item::action("Site", 1).image(Some(super::Image::Png(icon))),
            super::Item::action("Symbol", 2).image(Some(super::Image::Symbol("globe"))),
        ];
        let (mut values, mut bitmaps) = (Vec::new(), Vec::new());
        // SAFETY: the menu is built, read and destroyed here, on this thread.
        unsafe {
            let menu = super::build(items, &mut values, &mut bitmaps, 16);
            let bitmap_of = |id: u32| {
                let mut info = MENUITEMINFOW { cbSize: size_of::<MENUITEMINFOW>() as u32, fMask: MIIM_BITMAP, ..std::mem::zeroed() };
                GetMenuItemInfoW(menu, id, 0, &mut info);
                info.hbmpItem
            };
            assert_eq!(bitmaps.len(), 2, "a dot and a site icon; no symbol on Windows");
            assert_eq!(bitmap_of(1), bitmaps[0]);
            assert_eq!(bitmap_of(2), bitmaps[1]);
            assert!(bitmap_of(3).is_null());
            super::DestroyMenu(menu);
            for bitmap in bitmaps {
                super::DeleteObject(bitmap);
            }
        }
    }

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
