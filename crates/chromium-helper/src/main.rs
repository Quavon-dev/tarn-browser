#![forbid(unsafe_code)]

#[cfg(all(target_os = "macos", feature = "chromium"))]
fn main() {
    let args = cef::args::Args::new();
    let sandbox = std::env::args().all(|a| a != "--no-sandbox").then(|| {
        let mut sandbox = cef::sandbox::Sandbox::new();
        sandbox.initialize(args.as_main_args());
        sandbox
    });
    let exe = std::env::current_exe().expect("helper path");
    let loader = cef::library_loader::LibraryLoader::new(&exe, true);
    if !loader.load() {
        std::process::exit(1);
    }
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let code = cef::execute_process(Some(args.as_main_args()), None::<&mut cef::App>, std::ptr::null_mut());
    drop(loader);
    drop(sandbox);
    std::process::exit(code);
}

#[cfg(not(all(target_os = "macos", feature = "chromium")))]
fn main() {
    std::process::exit(1);
}
