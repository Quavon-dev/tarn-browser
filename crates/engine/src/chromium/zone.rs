//! Chromium's `EarlyMallocZoneRegistration` (`base/allocator/early_zone_registration_apple.cc`).
//! When the CEF framework loads, PartitionAlloc makes its malloc zone the default. Without a
//! delegating zone registered first it unregisters and re-registers the system zone, and a free
//! on another thread in between finds no owning zone and traps. With the delegating zone in
//! front, PartitionAlloc replaces it and the system zone stays registered. Unlike Chromium's,
//! this zone answers `size` for the system zone and passes typed allocations on, because it stays
//! the default for the whole session when no Chromium tab opens.

use std::ffi::{c_char, c_uint, c_void};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};

#[repr(C)]
struct Zone {
    reserved: [*mut c_void; 2],
    size: Option<unsafe extern "C" fn(*mut Zone, *const c_void) -> usize>,
    malloc: Option<unsafe extern "C" fn(*mut Zone, usize) -> *mut c_void>,
    calloc: Option<unsafe extern "C" fn(*mut Zone, usize, usize) -> *mut c_void>,
    valloc: Option<unsafe extern "C" fn(*mut Zone, usize) -> *mut c_void>,
    free: Option<unsafe extern "C" fn(*mut Zone, *mut c_void)>,
    realloc: Option<unsafe extern "C" fn(*mut Zone, *mut c_void, usize) -> *mut c_void>,
    destroy: Option<unsafe extern "C" fn(*mut Zone)>,
    zone_name: *const c_char,
    batch_malloc: Option<unsafe extern "C" fn(*mut Zone, usize, *mut *mut c_void, c_uint) -> c_uint>,
    batch_free: Option<unsafe extern "C" fn(*mut Zone, *mut *mut c_void, c_uint)>,
    introspect: *mut Introspection,
    version: c_uint,
    memalign: Option<unsafe extern "C" fn(*mut Zone, usize, usize) -> *mut c_void>,
    free_definite_size: Option<unsafe extern "C" fn(*mut Zone, *mut c_void, usize)>,
    pressure_relief: Option<unsafe extern "C" fn(*mut Zone, usize) -> usize>,
    claimed_address: Option<unsafe extern "C" fn(*mut Zone, *mut c_void) -> u32>,
    try_free_default: Option<unsafe extern "C" fn(*mut Zone, *mut c_void)>,
    malloc_with_options: Option<unsafe extern "C" fn(*mut Zone, usize, usize, u64) -> *mut c_void>,
    malloc_type_malloc: Option<unsafe extern "C" fn(*mut Zone, usize, u64) -> *mut c_void>,
    malloc_type_calloc: Option<unsafe extern "C" fn(*mut Zone, usize, usize, u64) -> *mut c_void>,
    malloc_type_realloc: Option<unsafe extern "C" fn(*mut Zone, *mut c_void, usize, u64) -> *mut c_void>,
    malloc_type_memalign: Option<unsafe extern "C" fn(*mut Zone, usize, usize, u64) -> *mut c_void>,
    malloc_type_malloc_with_options: Option<unsafe extern "C" fn(*mut Zone, usize, usize, u64, u64) -> *mut c_void>,
}

#[repr(C)]
struct Statistics {
    blocks_in_use: c_uint,
    size_in_use: usize,
    max_size_in_use: usize,
    size_allocated: usize,
}

#[repr(C)]
struct Introspection {
    enumerator: Option<unsafe extern "C" fn(u32, *mut c_void, c_uint, usize, *mut c_void, *mut c_void) -> i32>,
    good_size: Option<unsafe extern "C" fn(*mut Zone, usize) -> usize>,
    check: Option<unsafe extern "C" fn(*mut Zone) -> u32>,
    print: Option<unsafe extern "C" fn(*mut Zone, u32)>,
    log: Option<unsafe extern "C" fn(*mut Zone, *mut c_void)>,
    force_lock: Option<unsafe extern "C" fn(*mut Zone)>,
    force_unlock: Option<unsafe extern "C" fn(*mut Zone)>,
    statistics: Option<unsafe extern "C" fn(*mut Zone, *mut Statistics)>,
    zone_locked: Option<unsafe extern "C" fn(*mut Zone) -> u32>,
    enable_discharge_checking: Option<unsafe extern "C" fn(*mut Zone) -> u32>,
    disable_discharge_checking: Option<unsafe extern "C" fn(*mut Zone)>,
    discharge: Option<unsafe extern "C" fn(*mut Zone, *mut c_void)>,
    enumerate_discharged_pointers: *mut c_void,
    reinit_lock: Option<unsafe extern "C" fn(*mut Zone)>,
    newer: [*mut c_void; 3],
}

unsafe extern "C" {
    fn malloc_default_purgeable_zone() -> *mut Zone;
    fn malloc_get_all_zones(task: u32, reader: *mut c_void, addresses: *mut *mut *mut Zone, count: *mut c_uint) -> i32;
    fn malloc_zone_register(zone: *mut Zone);
    fn malloc_zone_unregister(zone: *mut Zone);
    static mach_task_self_: u32;
}

const NAME: &std::ffi::CStr = c"DelegatingDefaultZoneForPartitionAlloc";
static SYSTEM: AtomicPtr<Zone> = AtomicPtr::new(null_mut());

fn zones() -> Vec<*mut Zone> {
    let (mut list, mut count) = (null_mut(), 0);
    // SAFETY: a null reader reads this task's own zone list, which libmalloc keeps alive.
    if unsafe { malloc_get_all_zones(mach_task_self_, null_mut(), &raw mut list, &raw mut count) } != 0 || list.is_null() {
        return Vec::new();
    }
    // SAFETY: libmalloc returned `count` zone pointers at `list`; they are copied before any
    // register or unregister rewrites that array.
    unsafe { std::slice::from_raw_parts(list, count as usize) }.to_vec()
}

fn system() -> *mut Zone {
    SYSTEM.load(Ordering::Relaxed)
}

macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;)*) => {$(
        unsafe extern "C" fn $name(_: *mut Zone, $($arg: $ty),*) $(-> $ret)? {
            let z = system();
            // SAFETY: `z` is the registered system zone, set before this zone is registered; each
            // forwarder is installed only where the system zone has that function.
            unsafe { (*z).$name.unwrap_unchecked()(z, $($arg),*) }
        }
    )*};
}

forward! {
    malloc(size: usize) -> *mut c_void;
    calloc(n: usize, size: usize) -> *mut c_void;
    valloc(size: usize) -> *mut c_void;
    realloc(ptr: *mut c_void, size: usize) -> *mut c_void;
    batch_malloc(size: usize, out: *mut *mut c_void, n: c_uint) -> c_uint;
    memalign(align: usize, size: usize) -> *mut c_void;
    size(ptr: *const c_void) -> usize;
    free(ptr: *mut c_void);
    free_definite_size(ptr: *mut c_void, size: usize);
    batch_free(ptrs: *mut *mut c_void, n: c_uint);
    try_free_default(ptr: *mut c_void);
    malloc_with_options(align: usize, size: usize, options: u64) -> *mut c_void;
    malloc_type_malloc(size: usize, kind: u64) -> *mut c_void;
    malloc_type_calloc(n: usize, size: usize, kind: u64) -> *mut c_void;
    malloc_type_realloc(ptr: *mut c_void, size: usize, kind: u64) -> *mut c_void;
    malloc_type_memalign(align: usize, size: usize, kind: u64) -> *mut c_void;
    malloc_type_malloc_with_options(align: usize, size: usize, options: u64, kind: u64) -> *mut c_void;
}

unsafe extern "C" fn good_size(_: *mut Zone, size: usize) -> usize {
    let z = system();
    // SAFETY: as in `forward!`; every zone has an introspection table with `good_size`.
    unsafe { (*(*z).introspect).good_size.unwrap_unchecked()(z, size) }
}
unsafe extern "C" fn pressure_relief(_: *mut Zone, _: usize) -> usize {
    0
}
unsafe extern "C" fn enumerator(_: u32, _: *mut c_void, _: c_uint, _: usize, _: *mut c_void, _: *mut c_void) -> i32 {
    0
}
unsafe extern "C" fn yes(_: *mut Zone) -> u32 {
    1
}
unsafe extern "C" fn no(_: *mut Zone) -> u32 {
    0
}
unsafe extern "C" fn nothing(_: *mut Zone) {}
unsafe extern "C" fn print(_: *mut Zone, _: u32) {}
unsafe extern "C" fn statistics(_: *mut Zone, stats: *mut Statistics) {
    // SAFETY: libmalloc passes a writable statistics record and adds every zone's into a total.
    unsafe { stats.write(Statistics { blocks_in_use: 0, size_in_use: 0, max_size_in_use: 0, size_allocated: 0 }) }
}
unsafe extern "C" fn ignore(_: *mut Zone, _: *mut c_void) {}

/// Puts the delegating zone in front of the system zone; later calls do nothing. Call first in
/// `main`, while the process has one thread: registering briefly removes the system zone.
pub fn register_early_malloc_zone() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(register);
}

fn register() {
    static mut INTROSPECT: Introspection = Introspection {
        enumerator: Some(enumerator),
        good_size: Some(good_size),
        check: Some(yes),
        print: Some(print),
        log: Some(ignore),
        force_lock: Some(nothing),
        force_unlock: Some(nothing),
        statistics: Some(statistics),
        zone_locked: Some(no),
        enable_discharge_checking: Some(no),
        disable_discharge_checking: Some(nothing),
        discharge: Some(ignore),
        enumerate_discharged_pointers: null_mut(),
        reinit_lock: Some(nothing),
        newer: [null_mut(); 3],
    };
    static mut DELEGATING: Zone = Zone {
        reserved: [null_mut(); 2],
        size: Some(size),
        malloc: Some(malloc),
        calloc: Some(calloc),
        valloc: Some(valloc),
        free: Some(free),
        realloc: Some(realloc),
        destroy: None,
        zone_name: NAME.as_ptr(),
        batch_malloc: None,
        batch_free: None,
        introspect: null_mut(),
        version: 0,
        memalign: None,
        free_definite_size: None,
        pressure_relief: Some(pressure_relief),
        claimed_address: None,
        try_free_default: None,
        malloc_with_options: None,
        malloc_type_malloc: None,
        malloc_type_calloc: None,
        malloc_type_realloc: None,
        malloc_type_memalign: None,
        malloc_type_malloc_with_options: None,
    };
    let named = |z: &*mut Zone, name: &[u8]| {
        // SAFETY: registered zones live for the process; a zone name is NUL-terminated.
        unsafe { !(**z).zone_name.is_null() && std::ffi::CStr::from_ptr((**z).zone_name).to_bytes() == name }
    };
    if zones().iter().any(|z| named(z, NAME.to_bytes()) || named(z, b"PartitionAlloc")) {
        return;
    }
    // SAFETY: `ONCE` makes this the only access to the statics; the zone and its table are
    // static because libmalloc keeps raw pointers to them; the system zone's fields are read
    // only up to the version it reports.
    unsafe {
        let purgeable = malloc_default_purgeable_zone();
        let Some(&default) = zones().first() else { return };
        SYSTEM.store(default, Ordering::Relaxed);
        let (d, s, v) = (&raw mut DELEGATING, default, (*default).version);
        (*d).introspect = &raw mut INTROSPECT;
        (*d).version = v.min(16);
        (*d).batch_malloc = (*s).batch_malloc.and(Some(batch_malloc));
        (*d).batch_free = (*s).batch_free.and(Some(batch_free));
        if v >= 5 {
            (*d).memalign = (*s).memalign.and(Some(memalign));
        }
        if v >= 6 {
            (*d).free_definite_size = (*s).free_definite_size.and(Some(free_definite_size));
        }
        if v >= 13 {
            (*d).try_free_default = (*s).try_free_default.and(Some(try_free_default));
        }
        if v >= 15 {
            (*d).malloc_with_options = (*s).malloc_with_options.and(Some(malloc_with_options));
        }
        if v >= 16 {
            (*d).malloc_type_malloc = (*s).malloc_type_malloc.and(Some(malloc_type_malloc));
            (*d).malloc_type_calloc = (*s).malloc_type_calloc.and(Some(malloc_type_calloc));
            (*d).malloc_type_realloc = (*s).malloc_type_realloc.and(Some(malloc_type_realloc));
            (*d).malloc_type_memalign = (*s).malloc_type_memalign.and(Some(malloc_type_memalign));
            (*d).malloc_type_malloc_with_options = (*s).malloc_type_malloc_with_options.and(Some(malloc_type_malloc_with_options));
        }
        malloc_zone_register(d);
        malloc_zone_unregister(default);
        malloc_zone_register(default);
        malloc_zone_unregister(purgeable);
        malloc_zone_register(purgeable);
    }
}
