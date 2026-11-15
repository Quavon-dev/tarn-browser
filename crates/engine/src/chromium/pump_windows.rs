//! Runs jobs on the main thread from GPUI's message loop through a message-only window: the
//! Windows counterpart of the main dispatch queue.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetFocus, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CBT_CREATEWNDW, CBTACTIVATESTRUCT, CallNextHookEx, CreateWindowExW, DefWindowProcW, GetClassNameW, GetCursorPos, HCBT_ACTIVATE, HCBT_CREATEWND,
    HCBT_DESTROYWND, HTCAPTION, HWND_MESSAGE, IsZoomed, KillTimer, MSG, PM_REMOVE, PostMessageW, RegisterClassW, SC_MAXIMIZE, SC_RESTORE, SetTimer,
    SetWindowsHookExW, TranslateMessage, WH_CBT, WH_GETMESSAGE, WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_MOUSEMOVE, WM_NCLBUTTONDBLCLK,
    WM_NCLBUTTONDOWN, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_NULL, WM_RBUTTONDOWN, WM_SYSCOMMAND, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW,
    WindowFromPoint,
};

use crate::host_windows::wide;

const RUN: u32 = WM_APP + 1;
/// SC_MOVE with HTCAPTION: move the window with the pressed mouse.
const SC_DRAGMOVE: u32 = 0xF012;
/// Sent once the mouse left a window that tracks it (WinUser.h; in windows-sys only with Controls).
const WM_MOUSELEAVE: u32 = 0x02A3;
const WAKE: u32 = WM_APP + 2;
/// The timer id of CEF's pump; job ids start at one and stay below it.
const PUMP_TIMER: usize = usize::MAX;

type Job = Box<dyn FnOnce()>;

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(std::ptr::null_mut()) };
    static JOBS: RefCell<HashMap<usize, Job>> = RefCell::new(HashMap::new());
    static NEXT: Cell<usize> = const { Cell::new(1) };
    /// A move to clear GPUI's hover is on its way; the leave it causes is not answered.
    static CLEARING: Cell<bool> = const { Cell::new(false) };
    static INACTIVE_NEXT: Cell<bool> = const { Cell::new(false) };
    static INACTIVE: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

static MAIN_THREAD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
/// The pump window, for `wake` from CEF's threads; null until `init`.
static PUMP_WINDOW: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());
static PUMP: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

unsafe extern "system" fn proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TIMER if wparam == PUMP_TIMER => {
            // SAFETY: the pump timer was set on this window.
            unsafe { KillTimer(hwnd, PUMP_TIMER) };
            if let Some(pump) = PUMP.get() {
                pump();
            }
            0
        }
        WAKE => {
            match (wparam, PUMP.get()) {
                (0, Some(pump)) => pump(),
                // SAFETY: our window; a newer delay replaces the pending one.
                (ms, Some(_)) => unsafe {
                    SetTimer(hwnd, PUMP_TIMER, u32::try_from(ms).unwrap_or(u32::MAX), None);
                },
                _ => {}
            }
            0
        }
        WM_TIMER | RUN => {
            if message == WM_TIMER {
                // SAFETY: the timer was set on this window with this id.
                unsafe { KillTimer(hwnd, wparam) };
            }
            if let Some(job) = JOBS.with_borrow_mut(|jobs| jobs.remove(&wparam)) {
                job();
            }
            0
        }
        // SAFETY: default handling for every other message of our own window.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn window() -> HWND {
    let current = WINDOW.get();
    if !current.is_null() {
        return current;
    }
    let class = wide("TarnChromiumPump");
    // SAFETY: registers a class whose procedure is `proc` and creates a message-only window of it
    // on this thread; a second registration fails harmlessly.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        CreateWindowExW(0, class.as_ptr(), std::ptr::null(), 0, 0, 0, 0, 0, HWND_MESSAGE, std::ptr::null_mut(), instance, std::ptr::null())
    };
    WINDOW.set(hwnd);
    hwnd
}

/// Creates the pump window on the main thread; `pump` runs there whenever `wake` asks.
pub(crate) fn init(pump: fn()) {
    PUMP.get_or_init(|| pump);
    PUMP_WINDOW.store(window(), Ordering::Release);
}

/// Hooks GPUI's thread; call before GPUI opens a window.
pub(crate) fn install_hooks() {
    // SAFETY: hooks on this thread only, removed with the thread.
    unsafe {
        SetWindowsHookExW(WH_GETMESSAGE, Some(translate_keys), std::ptr::null_mut(), GetCurrentThreadId());
        SetWindowsHookExW(WH_CBT, Some(keep_popups_inactive), std::ptr::null_mut(), GetCurrentThreadId());
    }
}

/// Whether the next GPUI pop-up opens without taking focus; GPUI on Windows shows every window
/// activated.
pub fn open_next_window_inactive(inactive: bool) {
    INACTIVE_NEXT.set(inactive);
}

/// GPUI shows each pop-up window activated, which deactivates the browser window and closes what
/// it had open; as on macOS, a pop-up opened without focus becomes active only when it is clicked.
unsafe extern "system" fn keep_popups_inactive(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let key = wparam;
    match code as u32 {
        // SAFETY: for HCBT_CREATEWND, `lparam` is the CBT_CREATEWNDW whose `lpcs` describes the window.
        HCBT_CREATEWND if is_gpui(wparam as HWND) && unsafe { (*(*(lparam as *const CBT_CREATEWNDW)).lpcs).dwExStyle } & WS_EX_TOOLWINDOW != 0 => {
            if INACTIVE_NEXT.take() {
                INACTIVE.with_borrow_mut(|set| set.insert(key));
            }
        }
        HCBT_DESTROYWND => {
            INACTIVE.with_borrow_mut(|set| set.remove(&key));
        }
        // SAFETY: for HCBT_ACTIVATE, `lparam` is the window's CBTACTIVATESTRUCT.
        HCBT_ACTIVATE if unsafe { (*(lparam as *const CBTACTIVATESTRUCT)).fMouse } == 0 && INACTIVE.with_borrow(|set| set.contains(&key)) => return 1,
        _ => {}
    }
    // SAFETY: passes the hook on unchanged.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// GPUI's loop dispatches messages without translating them and translates key presses only for
/// its own windows, so Chromium's windows would get no characters; this translates theirs. A click
/// on a GPUI window takes keyboard focus back from a page, which GPUI does not do itself. GPUI marks
/// every mouse press handled, so Windows never moves a window by its drag areas; a press there
/// starts the system move instead, and a double click maximizes or restores. GPUI also gets
/// a move just outside its window when the pointer leaves it, which clears its hover.
unsafe extern "system" fn translate_keys(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: for HC_ACTION, `lparam` points to the message being retrieved.
    unsafe {
        if code == 0 && wparam == PM_REMOVE as usize {
            let msg = &mut *(lparam as *mut MSG);
            if msg.wParam == HTCAPTION as usize && matches!(msg.message, WM_NCLBUTTONDOWN | WM_NCLBUTTONDBLCLK) && is_gpui(msg.hwnd) {
                let command = match msg.message {
                    WM_NCLBUTTONDOWN => SC_DRAGMOVE,
                    _ if IsZoomed(msg.hwnd) != 0 => SC_RESTORE,
                    _ => SC_MAXIMIZE,
                };
                PostMessageW(msg.hwnd, WM_SYSCOMMAND, command as usize, msg.lParam);
                msg.message = WM_NULL;
            }
            if matches!(msg.message, WM_KEYDOWN | WM_SYSKEYDOWN) && !is_gpui(msg.hwnd) {
                TranslateMessage(msg);
            }
            if CLEARING.get() && matches!(msg.message, WM_MOUSEMOVE | WM_NCMOUSEMOVE) && msg.lParam != 0xFFFF_FFFF && is_gpui(msg.hwnd) {
                CLEARING.set(false);
            }
            if matches!(msg.message, WM_MOUSELEAVE | WM_NCMOUSELEAVE) && is_gpui(msg.hwnd) {
                // GPUI keeps the hover of where the pointer was when it went into a page's child
                // window or elsewhere; a move just outside the window (-1, -1) clears it. Only when
                // another window is under the pointer: between caption and client GPUI still has it.
                // That move makes GPUI track the mouse again, so the leave it causes is let through.
                let mut cursor = POINT { x: 0, y: 0 };
                let elsewhere = GetCursorPos(&mut cursor) != 0 && WindowFromPoint(cursor) != msg.hwnd;
                if elsewhere && !CLEARING.replace(true) {
                    PostMessageW(msg.hwnd, WM_MOUSEMOVE, 0, 0xFFFF_FFFF);
                } else if !elsewhere {
                    CLEARING.set(false);
                }
            }
            if matches!(msg.message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_NCLBUTTONDOWN) && is_gpui(msg.hwnd) && GetFocus() != msg.hwnd {
                SetFocus(msg.hwnd);
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }
}

fn is_gpui(hwnd: HWND) -> bool {
    let mut name = [0u16; 32];
    // SAFETY: `name` holds as many characters as passed.
    let len = unsafe { GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32) };
    String::from_utf16_lossy(&name[..usize::try_from(len).unwrap_or(0)]) == "Zed::Window"
}

/// Asks for `pump` on the main thread in `delay_ms`; callable from any thread, as CEF schedules
/// its work from its own threads.
pub(crate) fn wake(delay_ms: i64) {
    let hwnd = PUMP_WINDOW.load(Ordering::Acquire);
    if !hwnd.is_null() {
        // SAFETY: posting to a window of another thread is thread-safe; the window lives as long
        // as the main thread's message loop.
        unsafe { PostMessageW(hwnd, WAKE, usize::try_from(delay_ms.max(0)).unwrap_or(0), 0) };
    }
}

/// Runs `job` on the main thread after `delay`; `job` need not be `Send`, so the caller must be on
/// the main thread too.
pub(crate) fn after(delay: Duration, job: impl FnOnce() + 'static) {
    // SAFETY: reads the calling thread's id.
    let thread = unsafe { GetCurrentThreadId() };
    assert_eq!(*MAIN_THREAD.get_or_init(|| thread), thread, "pump::after called off the main thread");
    let hwnd = window();
    let id = NEXT.get();
    NEXT.set(if id + 1 == PUMP_TIMER { 1 } else { id + 1 });
    JOBS.with_borrow_mut(|jobs| jobs.insert(id, Box::new(job)));
    let ms = u32::try_from(delay.as_millis()).unwrap_or(u32::MAX);
    // SAFETY: our window on this thread; `id` keys the job just stored.
    let scheduled = unsafe { if ms == 0 { PostMessageW(hwnd, RUN, id, 0) != 0 } else { SetTimer(hwnd, id, ms, None) != 0 } };
    if !scheduled {
        JOBS.with_borrow_mut(|jobs| jobs.remove(&id));
    }
}
