/// Link arguments of a dependency do not reach this binary, so engine's translation shim needs
/// them here too: Swift Concurrency is named through @rpath when linked for a macOS older than 12
/// (Rust's default), and Translation exists only from macOS 15.
fn main() {
    if std::env::var_os("CARGO_FEATURE_TRANSLATION").is_some() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,Translation");
    }
    // Chromium needs the Windows 10 compatibility entry, without which Windows reports version 8
    // and its GPU process stops; GPUI needs per-monitor DPI awareness. Chromium's child processes
    // run in tarn.exe, whose main thread needs CEF's 8 MiB stack: the renderer overflows 1 MiB.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo::rustc-link-arg-bins=/STACK:0x800000");
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/windows/tarn.manifest");
        println!("cargo::rerun-if-changed={}", manifest.display());
        println!("cargo::rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo::rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
    }
}
