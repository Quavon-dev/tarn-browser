pub fn release_free() {
    #[cfg(target_env = "gnu")]
    {
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> std::ffi::c_int;
        }
        // SAFETY: glibc's malloc_trim only returns free pages to the system and may be called from any thread.
        unsafe { malloc_trim(0) };
    }
}
