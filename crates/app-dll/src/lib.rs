#![cfg(windows)]

use std::ffi::c_void;

/// Called by CEF's bootstrap launcher with the Chromium sandbox it set up for this process
/// (`include/cef_sandbox_win.h`), in the browser process and in every child. The launcher's CEF
/// version (`_version`) is not compared: packaging copies it from the same CEF build.
///
/// # Safety
/// Only CEF's launcher calls it, with its own `sandbox_info`, valid for the whole process.
#[unsafe(export_name = "RunWinMain")]
pub unsafe extern "C" fn run_win_main(_instance: *mut c_void, _command_line: *mut u16, _show: i32, sandbox_info: *mut c_void, _version: *mut c_void) -> i32 {
    // SAFETY: the launcher passes its own sandbox information, which lives as long as the process.
    unsafe { engine::chromium::set_sandbox_info(sandbox_info) };
    app::run();
    0
}
