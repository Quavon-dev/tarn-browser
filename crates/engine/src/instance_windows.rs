//! One Tarn per data folder on Windows. A second launch, such as a link opened from another app
//! while Tarn runs, hands its links to the running Tarn and exits, as macOS does for an app.

use std::cell::RefCell;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{CreateMutexW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, FindWindowExW, GetWindowThreadProcessId, HWND_MESSAGE, RegisterClassW, SendMessageW,
    WM_COPYDATA, WNDCLASSW,
};

use crate::host_windows::wide;

/// Tags the links a launch hands over, so other data sent to the window is ignored.
const LINKS: usize = 0x5441_524E;

type OnLinks = Box<dyn Fn(Vec<String>)>;

thread_local! {
    static ON_LINKS: RefCell<Option<OnLinks>> = RefCell::new(None);
}

/// The window class and mutex that name the running Tarn of `data_dir`: FNV-1a of its
/// lowercased path, which every build computes the same.
fn class(data_dir: &str) -> Vec<u16> {
    wide(&format!("Tarn.Instance.{:016x}", crate::agent_socket::data_dir_hash(data_dir)))
}

/// The process behind `window` when it runs this very executable, so another program that made a
/// window of the same class never gets the links.
fn tarn_process(window: HWND) -> Option<u32> {
    let mut pid = 0;
    // SAFETY: `pid` is a valid out-parameter.
    unsafe { GetWindowThreadProcessId(window, &mut pid) };
    runs_this_exe(pid).then_some(pid)
}

/// Whether process `pid` runs this very executable.
pub(crate) fn runs_this_exe(pid: u32) -> bool {
    // SAFETY: query-only access; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return false;
    }
    let mut path = [0u16; 1024];
    let mut len = path.len() as u32;
    // SAFETY: `path` holds `len` characters; the handle is open with query access.
    let read = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut len) } != 0;
    // SAFETY: closes the handle opened above.
    unsafe { CloseHandle(process) };
    let theirs = std::path::PathBuf::from(String::from_utf16_lossy(&path[..len as usize]));
    let same = |ours: std::path::PathBuf| Some(theirs.canonicalize().ok()? == ours.canonicalize().ok()?);
    read && std::env::current_exe().ok().and_then(same).unwrap_or(false)
}

/// True when this launch is the Tarn of `data_dir`. Otherwise `links` went to the running one,
/// which may then come to the front, and this launch should exit.
pub fn claim(data_dir: &str, links: &[String]) -> bool {
    let name = class(data_dir);
    // SAFETY: a NUL-terminated name; the handle is kept open for the life of the process.
    let (mutex, error) = unsafe { (CreateMutexW(std::ptr::null(), 0, name.as_ptr()), GetLastError()) };
    // Without the mutex (an object of another kind holds the name) this launch runs on its own.
    if mutex.is_null() || error != ERROR_ALREADY_EXISTS {
        return true;
    }
    let text: Vec<u16> = links.join("\n").encode_utf16().collect();
    let data = COPYDATASTRUCT { dwData: LINKS, cbData: u32::try_from(text.len() * 2).unwrap_or(0), lpData: text.as_ptr().cast_mut().cast() };
    // The running Tarn makes its window once it has started, so a launch right after it waits.
    for _ in 0..300 {
        // SAFETY: finds a message-only window by its NUL-terminated class.
        let window = unsafe { FindWindowExW(HWND_MESSAGE, std::ptr::null_mut(), name.as_ptr(), std::ptr::null()) };
        if let Some(pid) = (!window.is_null()).then(|| tarn_process(window)).flatten() {
            // SAFETY: `data` and `text` outlive the synchronous send; the receiver copies them.
            let taken = unsafe {
                AllowSetForegroundWindow(pid);
                SendMessageW(window, WM_COPYDATA, 0, (&raw const data) as LPARAM) != 0
            };
            // Windows refuses the message to an elevated Tarn (UIPI); the alert below says so.
            if taken {
                return false;
            }
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    crate::host_windows::alert("Tarn is already running", "Another Tarn uses this data folder and did not take the link, so this one closes.");
    false
}

/// Takes the links later launches hand over; call once on the main thread after `claim`.
pub fn listen(data_dir: &str, on_links: impl Fn(Vec<String>) + 'static) {
    let name = class(data_dir);
    ON_LINKS.with_borrow_mut(|f| *f = Some(Box::new(on_links)));
    // SAFETY: registers and creates a message-only window of this thread with a static procedure.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = WNDCLASSW { lpfnWndProc: Some(receive), hInstance: instance, lpszClassName: name.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&class);
        CreateWindowExW(0, name.as_ptr(), std::ptr::null(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), instance, std::ptr::null());
    }
}

unsafe extern "system" fn receive(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_COPYDATA {
        // SAFETY: for WM_COPYDATA, `lparam` points to a COPYDATASTRUCT valid during this call.
        let data = unsafe { &*(lparam as *const COPYDATASTRUCT) };
        if data.dwData == LINKS {
            let text = if data.lpData.is_null() || data.cbData == 0 {
                &[][..]
            } else {
                // SAFETY: the sender passes `cbData` bytes of UTF-16; a launch without links passes none.
                unsafe { std::slice::from_raw_parts(data.lpData.cast::<u16>(), data.cbData as usize / 2) }
            };
            let links: Vec<String> = String::from_utf16_lossy(text).lines().filter(|l| !l.is_empty()).map(str::to_owned).collect();
            ON_LINKS.with_borrow(|f| f.as_ref().map(|f| f(links)));
            return 1;
        }
        return 0;
    }
    // SAFETY: forwards every other message unchanged.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_data_folder_names_one_instance_whatever_its_case() {
        assert_eq!(super::class(r"C:\Users\A\AppData\Local\Tarn"), super::class(r"c:\users\a\appdata\local\tarn"));
        assert_ne!(super::class(r"C:\Tarn"), super::class(r"C:\Tarn2"));
    }
}
