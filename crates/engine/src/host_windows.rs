use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use url::Url;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, CreateRoundRectRgn, SetWindowRgn};
use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegGetValueW, RegSetKeyValueW};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, GetKeyState, SetFocus, VK_MENU, VK_SHIFT};
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, ILCreateFromPathW, ILFree, RemoveWindowSubclass, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify, SHOpenFolderAndSelectItems,
    SetWindowSubclass, ShellExecuteW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWLP_HWNDPARENT, GetClientRect, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, HTTRANSPARENT, HWND_TOP, MB_ICONWARNING, MB_OK, MessageBoxW,
    SPI_GETCLIENTAREAANIMATION, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, SystemParametersInfoW, WM_NCDESTROY,
    WM_NCHITTEST, WM_SIZE,
};

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Chromium never hands a new window to the shell on Windows; pop-ups are offered as notices,
/// so no `Popup` is ever made.
pub struct Popup {
    pub url: Option<Url>,
    pub link: bool,
    pub(crate) never: std::convert::Infallible,
}

impl std::fmt::Debug for Popup {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.never {}
    }
}

/// The shell window that web views are placed into.
#[derive(Clone)]
pub struct Host {
    pub(crate) hwnd: HWND,
}

impl Host {
    pub fn from_window(window: &impl HasWindowHandle) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        Some(Host { hwnd: handle.hwnd.get() as HWND })
    }

    /// Gives keyboard focus back to the shell window, e.g. when the command bar opens.
    pub fn focus(&self) {
        // SAFETY: a window of this thread; a stale handle only makes the call fail.
        unsafe { SetFocus(self.hwnd) };
    }

    /// GPUI's pop-up windows have no frame on Windows.
    pub fn strip_frame(&self) {}

    /// Clips this pop-up window to its card: `top`, `left`, `right` and `bottom` inset from the
    /// window's edges and `radius` round, in points. GPUI cannot draw a transparent margin on
    /// Windows without DirectComposition, which would cover Chromium's child windows, so the
    /// margin would show black. The clip follows every resize.
    pub fn round_corners(&self, top: f64, left: f64, right: f64, bottom: f64, radius: f64) {
        CARDS.with_borrow_mut(|c| c.insert(self.hwnd as usize, [top, left, right, bottom, radius]));
        // SAFETY: a window of this thread; the subclass is removed when the window is destroyed.
        unsafe { SetWindowSubclass(self.hwnd, Some(card_window), 1, 0) };
        clip(self.hwnd);
    }

    /// Lets the pointer pass through this pop-up window to the shell window below, as a tooltip must.
    pub fn ignore_mouse(&self) {
        // SAFETY: a window of this thread; the subclass is removed when the window is destroyed.
        unsafe { SetWindowSubclass(self.hwnd, Some(see_through_window), 2, 0) };
    }

    /// A pop-up that `follow`s its browser window goes behind other apps with it, so nothing
    /// more is needed on Windows.
    pub fn hide_on_deactivate(&self) {}

    /// Keeps `popup` above this window and behind other apps with it: its owner becomes this window.
    pub fn follow(&self, popup: &Host) {
        // SAFETY: windows of this thread; GWLP_HWNDPARENT sets a top-level window's owner.
        unsafe { SetWindowLongPtrW(popup.hwnd, GWLP_HWNDPARENT, self.hwnd as isize) };
    }

    /// Device pixels per GPUI point.
    pub(crate) fn scale(&self) -> f64 {
        // SAFETY: as in `focus`.
        let dpi = unsafe { GetDpiForWindow(self.hwnd) };
        if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 }
    }

    /// A rectangle in points of this window's client area, in device pixels.
    pub(crate) fn pixels(&self, x: f64, y: f64, width: f64, height: f64) -> (i32, i32, i32, i32) {
        let s = self.scale();
        let px = |v: f64| (v * s).round() as i32;
        (px(x), px(y), px(width.max(0.0)), px(height.max(0.0)))
    }

    /// Frames the window of `popup` at `x, y, width, height` in this window's coordinates
    /// (points, origin top-left), so a popup can follow a shell control.
    pub fn place(&self, popup: &Host, x: f64, y: f64, width: f64, height: f64) {
        let (x, y, width, height) = self.pixels(x, y, width, height);
        let mut origin = POINT { x, y };
        // SAFETY: windows of this thread; `origin` is a valid out-parameter.
        unsafe {
            if ClientToScreen(self.hwnd, &mut origin) != 0 {
                SetWindowPos(popup.hwnd, HWND_TOP, origin.x, origin.y, width, height, SWP_NOACTIVATE | SWP_NOZORDER);
            }
        }
    }
}

thread_local! {
    /// The card insets and radius of each clipped pop-up window, in points.
    static CARDS: std::cell::RefCell<std::collections::HashMap<usize, [f64; 5]>> = Default::default();
}

/// Clips `hwnd` to its card's rounded rectangle at its current size.
fn clip(hwnd: HWND) {
    let Some([top, left, right, bottom, radius]) = CARDS.with_borrow(|c| c.get(&(hwnd as usize)).copied()) else { return };
    let scale = Host { hwnd }.scale();
    let px = |v: f64| (v * scale).round() as i32;
    let (mut client, mut window) = (RECT { left: 0, top: 0, right: 0, bottom: 0 }, RECT { left: 0, top: 0, right: 0, bottom: 0 });
    let mut origin = POINT { x: 0, y: 0 };
    // SAFETY: valid out-parameters; the window owns the region once it is set.
    unsafe {
        if GetClientRect(hwnd, &mut client) == 0 || GetWindowRect(hwnd, &mut window) == 0 || ClientToScreen(hwnd, &mut origin) == 0 {
            return;
        }
        // A region is in window coordinates, which include the frame GPUI keeps around the client area.
        let (x, y) = (origin.x - window.left, origin.y - window.top);
        let (l, t, r, b) = (x + px(left), y + px(top), x + client.right - px(right) + 1, y + client.bottom - px(bottom) + 1);
        // A window not yet sized, or minimized, keeps its region until its next size.
        if r > l && b > t {
            SetWindowRgn(hwnd, CreateRoundRectRgn(l, t, r, b, px(radius * 2.), px(radius * 2.)), 1);
        }
    }
}

unsafe extern "system" fn card_window(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    match message {
        WM_SIZE => clip(hwnd),
        WM_NCDESTROY => {
            CARDS.with_borrow_mut(|c| c.remove(&(hwnd as usize)));
            // SAFETY: removes the subclass this module installed.
            unsafe { RemoveWindowSubclass(hwnd, Some(card_window), 1) };
        }
        _ => {}
    }
    // SAFETY: hands the message to the window's own procedure.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// Hit-tests as transparent, so the pointer goes to the window of this thread below.
unsafe extern "system" fn see_through_window(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if message == WM_NCHITTEST {
        return HTTRANSPARENT as LRESULT;
    }
    if message == WM_NCDESTROY {
        // SAFETY: removes the subclass this module installed.
        unsafe { RemoveWindowSubclass(hwnd, Some(see_through_window), 2) };
    }
    // SAFETY: hands the message to the window's own procedure.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

pub const MIN_ZOOM: f64 = 0.3;

pub fn reveal_in_finder(path: &std::path::Path) {
    let path = wide(&path.to_string_lossy());
    // SAFETY: COM is initialised for this thread first; the item list is freed after use.
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let item = ILCreateFromPathW(path.as_ptr());
        if !item.is_null() {
            SHOpenFolderAndSelectItems(item, 0, std::ptr::null(), 0);
            ILFree(item);
        }
    }
}

/// The system "Show animations in Windows" setting, off meaning reduce motion.
pub fn reduce_motion() -> bool {
    let mut animate = 1i32;
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes a BOOL to `animate`.
    let ok = unsafe { SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animate).cast(), 0) };
    ok != 0 && animate == 0
}

/// Whether a window of this process is the foreground window.
pub fn app_active() -> bool {
    let mut pid = 0;
    // SAFETY: `pid` is a valid out-parameter; a null window yields no process.
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
    pid == std::process::id()
}

/// Whether Alt, which Windows calls the Option key's counterpart, is down now.
pub fn option_held() -> bool {
    // SAFETY: reads the key state; the high bit is set while the key is down.
    unsafe { GetKeyState(i32::from(VK_MENU)) < 0 }
}

/// Whether Shift is down now, also before any window has had a key event.
pub fn shift_held() -> bool {
    // SAFETY: reads the key state; the high bit is set while the key is down.
    unsafe { GetAsyncKeyState(i32::from(VK_SHIFT)) < 0 }
}

/// Only a macOS bundle has an identifier.
pub fn bundle_identifier() -> Option<String> {
    None
}

/// Tarn's URL handler, which the browser registration names for http and https.
const PROG_ID: &str = "TarnURL";
const CLIENT: &str = r"Software\Clients\StartMenuInternet\Tarn";
/// Marks the rest of a launch command line as one URL, as Chrome registers it.
const SINGLE_ARGUMENT: &str = "--single-argument";

fn set_value(key: &str, name: Option<&str>, value: &str) -> bool {
    let (key, name, value) = (wide(key), name.map(wide), wide(value));
    let bytes = u32::try_from(value.len() * 2).unwrap_or(u32::MAX);
    // SAFETY: NUL-terminated strings that outlive the call; the data is `bytes` long.
    let status = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), REG_SZ, value.as_ptr().cast(), bytes)
    };
    status == 0
}

fn value(key: &str, name: &str) -> Option<String> {
    let (key, name) = (wide(key), wide(name));
    let mut buffer = [0u16; 256];
    let mut size = u32::try_from(size_of_val(&buffer)).unwrap_or(0);
    // SAFETY: NUL-terminated strings; `buffer` holds `size` bytes, and RRF_RT_REG_SZ ends it with NUL.
    let status =
        unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size) };
    (status == 0).then(|| String::from_utf16_lossy(&buffer[..(size as usize / 2).saturating_sub(1)]))
}

/// Whether the user chose Tarn for https links.
pub fn is_default_browser() -> bool {
    value(r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice", "ProgId").as_deref() == Some(PROG_ID)
}

/// Registers this executable as a browser for the current user and opens Windows' default-apps
/// settings at it; Windows lets only the user make the choice, so the result shows later in
/// `is_default_browser`.
pub fn make_default_browser() {
    let Some(exe) = std::env::current_exe().ok().and_then(|p| p.to_str().map(str::to_owned)) else { return };
    // Chromium, which parses this command line first, takes everything after the marker as one
    // URL, so a link cannot add switches.
    let command = format!("\"{exe}\" {SINGLE_ARGUMENT} %1");
    let classes = format!(r"Software\Classes\{PROG_ID}");
    let registered = set_value(&classes, None, "Tarn URL")
        && set_value(&format!(r"{classes}\DefaultIcon"), None, &format!("{exe},0"))
        && set_value(&format!(r"{classes}\shell\open\command"), None, &command)
        && set_value(CLIENT, None, "Tarn")
        && set_value(&format!(r"{CLIENT}\shell\open\command"), None, &format!("\"{exe}\""))
        && set_value(&format!(r"{CLIENT}\Capabilities"), Some("ApplicationName"), "Tarn")
        && set_value(&format!(r"{CLIENT}\Capabilities"), Some("ApplicationDescription"), "A private web browser.")
        && ["http", "https"].iter().all(|scheme| set_value(&format!(r"{CLIENT}\Capabilities\URLAssociations"), Some(scheme), PROG_ID))
        && set_value(r"Software\RegisteredApplications", Some("Tarn"), &format!(r"{CLIENT}\Capabilities"));
    if !registered {
        return;
    }
    // SAFETY: tells the shell file associations changed; no item arguments.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
    let (verb, target) = (wide("open"), wide("ms-settings:defaultapps?registeredAppUser=Tarn"));
    // SAFETY: NUL-terminated strings that outlive the call.
    unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
}

/// A modal alert for when no window can show the message; returns once the user dismisses it.
pub fn alert(message: &str, detail: &str) {
    let (text, caption) = (wide(detail), wide(message));
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONWARNING) };
}

/// Hands a URL to the app registered for its scheme. Callers must have asked the user first.
/// Refused: what the Windows shell would treat as a local program or path rather than a URL.
pub fn open_external(url: &Url) -> bool {
    if !external_allowed(url) {
        return false;
    }
    let (verb, target) = (wide("open"), wide(url.as_str()));
    // SAFETY: NUL-terminated strings that outlive the call.
    let result = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), target.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL) };
    result as isize > 32
}

/// A one-letter scheme is a drive (`C:/Windows/notepad.exe`); the others run programs, open
/// folders, search the disk or hand the URL to handlers with known exploit chains.
fn external_allowed(url: &Url) -> bool {
    const REFUSED: &[&str] = &[
        "file",
        "javascript",
        "data",
        "vbscript",
        "shell",
        "ms-msdt",
        "search",
        "search-ms",
        "ms-settings",
        "ms-officecmd",
        "ms-cxh",
        "ms-cxh-full",
        "ms-appinstaller",
        "hcp",
        "its",
        "mk",
        "ms-its",
        "ldap",
        "microsoft-edge",
        "ms-word",
        "ms-excel",
        "ms-powerpoint",
        "ms-access",
        "ms-visio",
        "ms-project",
        "ms-publisher",
        "ms-infopath",
        "ms-spd",
        "onenote",
    ];
    let scheme = url.scheme();
    scheme.len() > 1 && !REFUSED.contains(&scheme)
}

#[cfg(test)]
mod tests {
    use url::Url;

    #[test]
    fn local_programs_and_shell_paths_are_refused() {
        for refused in [
            "C:/Windows/System32/cmd.exe",
            "file:///C:/x.exe",
            "shell:startup",
            "search-ms:query=x",
            "ms-msdt:/id",
            "javascript:alert(1)",
            "ms-officecmd:{}",
            "ms-appinstaller:?source=x",
        ] {
            assert!(!super::external_allowed(&Url::parse(refused).unwrap()), "{refused}");
        }
        for allowed in ["mailto:a@example.com", "zoommtg://join", "tel:+491234"] {
            assert!(super::external_allowed(&Url::parse(allowed).unwrap()), "{allowed}");
        }
    }
}
