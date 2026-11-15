#[cfg(target_os = "macos")]
use std::ffi::{CStr, c_char, c_uint, c_void};

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn malloc_get_all_zones(task: u32, reader: *mut c_void, addresses: *mut *mut *const *const c_void, count: *mut c_uint) -> i32;
    static mach_task_self_: u32;
}

#[cfg(target_os = "macos")]
fn zone_names() -> Vec<String> {
    let (mut list, mut count) = (std::ptr::null_mut(), 0);
    // SAFETY: reads this task's zone list; a zone's name pointer sits after two reserved words
    // and seven function pointers in `malloc_zone_t`.
    unsafe {
        assert_eq!(malloc_get_all_zones(mach_task_self_, std::ptr::null_mut(), &raw mut list, &raw mut count), 0);
        std::slice::from_raw_parts(list, count as usize)
            .iter()
            .map(|z| {
                let name = *z.cast::<*const c_char>().add(9);
                if name.is_null() { String::new() } else { CStr::from_ptr(name).to_string_lossy().into_owned() }
            })
            .collect()
    }
}

// No test harness: registration needs a process with one thread.
#[cfg(target_os = "macos")]
fn main() {
    let before = zone_names();
    engine::chromium::register_early_malloc_zone();
    engine::chromium::register_early_malloc_zone();
    let after = zone_names();
    assert_eq!(after[0], "DelegatingDefaultZoneForPartitionAlloc");
    assert!(before.iter().all(|n| after.contains(n)), "{before:?} {after:?}");
    assert_eq!(after.iter().filter(|n| *n == "DelegatingDefaultZoneForPartitionAlloc").count(), 1);
    let bytes: Vec<u8> = (0..1 << 20).map(|i: u32| i as u8).collect();
    assert_eq!(bytes.clone().iter().map(|b| u64::from(*b)).sum::<u64>(), 4096 * 32_640);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
