//! How short of memory the kernel says the system is, and what one process costs.

#[cfg(target_os = "macos")]
use std::ffi::{c_char, c_int, c_void};

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn sysctlbyname(name: *const c_char, old: *mut c_void, old_len: *mut usize, new: *const c_void, new_len: usize) -> c_int;
    fn proc_pid_rusage(pid: c_int, flavor: c_int, buffer: *mut c_void) -> c_int;
    fn mach_timebase_info(info: *mut [u32; 2]) -> c_int;
    fn malloc_zone_pressure_relief(zone: *mut c_void, goal: usize) -> usize;
}

#[cfg(target_os = "macos")]
const RUSAGE_INFO_V2: c_int = 2;

/// `struct rusage_info_v2` from `<sys/resource.h>`.
#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Default)]
struct RusageInfoV2 {
    _uuid: [u8; 16],
    user_time: u64,
    system_time: u64,
    _wakeups_pageins_wired_resident: [u64; 5],
    phys_footprint: u64,
    _rest: [u64; 10],
}

/// What a process costs: the memory Activity Monitor shows and the CPU time it has used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    pub footprint: u64,
    pub cpu_ns: u64,
}

/// The usage of a process of this user, `None` when it is gone or not readable.
#[cfg(target_os = "macos")]
pub fn usage(pid: i32) -> Option<Usage> {
    let mut info = RusageInfoV2::default();
    // SAFETY: `info` has the layout of `rusage_info_v2`, the flavor written for `RUSAGE_INFO_V2`.
    if unsafe { proc_pid_rusage(pid, RUSAGE_INFO_V2, (&raw mut info).cast()) } != 0 {
        return None;
    }
    let mut base = [0u32; 2];
    // SAFETY: `base` has the layout of `mach_timebase_info_data_t` (numer, denom).
    unsafe { mach_timebase_info(&raw mut base) };
    let [numer, denom] = base.map(u128::from);
    // The CPU times are in Mach absolute time units, which are not nanoseconds on Apple silicon.
    let cpu_ns = (u128::from(info.user_time + info.system_time) * numer / denom.max(1)) as u64;
    Some(Usage { footprint: info.phys_footprint, cpu_ns })
}

/// The usage of a process of this user, `None` when it is gone or not readable: its resident
/// memory (working set) and its CPU time.
#[cfg(windows)]
pub fn usage(pid: i32) -> Option<Usage> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let pid = u32::try_from(pid).ok()?;
    // SAFETY: a handle opened here is closed below; null means no access.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    // SAFETY: all-zero counters and times are valid out-parameters of the sizes passed.
    let (mut counters, mut times): (PROCESS_MEMORY_COUNTERS, [FILETIME; 4]) = unsafe { std::mem::zeroed() };
    counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: `process` is open with query access; the pointers are valid for the calls.
    let read = unsafe {
        GetProcessMemoryInfo(process, &raw mut counters, counters.cb) != 0
            && GetProcessTimes(process, &raw mut times[0], &raw mut times[1], &raw mut times[2], &raw mut times[3]) != 0
    };
    // SAFETY: closes the handle opened above.
    unsafe { CloseHandle(process) };
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    read.then(|| Usage { footprint: counters.WorkingSetSize as u64, cpu_ns: (ticks(times[2]) + ticks(times[3])) * 100 })
}

/// Whether the kernel reports its warning or critical memory pressure level.
#[cfg(target_os = "macos")]
pub fn pressure() -> bool {
    let (mut level, mut len): (c_int, usize) = (0, size_of::<c_int>());
    // SAFETY: the name is NUL-terminated and `level` is a `c_int` of the length passed.
    let read = unsafe { sysctlbyname(c"kern.memorystatus_vm_pressure_level".as_ptr(), (&raw mut level).cast(), &raw mut len, std::ptr::null(), 0) };
    read == 0 && level > 1
}

/// Hands memory freed inside the process back to the system, after a burst such as compiling the
/// filter lists or closing a window.
#[cfg(target_os = "macos")]
pub fn release_free() {
    // SAFETY: a null zone means every malloc zone, and a goal of 0 asks for all free pages.
    unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
}

/// The Windows heap decommits free pages itself.
#[cfg(windows)]
pub fn release_free() {}

/// Windows names no pressure level; nine tenths of physical memory in use counts as one.
#[cfg(windows)]
pub fn pressure() -> bool {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: an all-zero MEMORYSTATUSEX is valid; the call needs only its length set.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    // SAFETY: `status` is a valid, sized out-parameter.
    let read = unsafe { GlobalMemoryStatusEx(&mut status) };
    read != 0 && status.dwMemoryLoad >= 90
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn the_level_is_readable() {
        let (mut level, mut len): (std::ffi::c_int, usize) = (0, size_of::<std::ffi::c_int>());
        // SAFETY: as in `pressure`.
        let read = unsafe { super::sysctlbyname(c"kern.memorystatus_vm_pressure_level".as_ptr(), (&raw mut level).cast(), &raw mut len, std::ptr::null(), 0) };
        assert_eq!(read, 0);
        assert!([1, 2, 4].contains(&level));
    }

    #[cfg(windows)]
    #[test]
    fn the_load_is_readable() {
        super::pressure();
    }

    #[test]
    fn this_process_has_a_footprint_and_uses_cpu() {
        let before = super::usage(std::process::id() as i32).unwrap();
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(50) {
            std::hint::black_box(start.elapsed());
        }
        let after = super::usage(std::process::id() as i32).unwrap();
        assert!(before.footprint > 1 << 20);
        assert!(after.cpu_ns > before.cpu_ns, "{before:?} {after:?}");
        assert!(super::usage(-1).is_none());
    }
}
