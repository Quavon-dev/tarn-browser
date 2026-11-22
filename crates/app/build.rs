/// Link arguments of a dependency do not reach this binary, so engine's Swift shims need them
/// here too: Swift Concurrency is named through @rpath when linked for a macOS older than 12
/// (Rust's default), Translation exists only from macOS 15 and FoundationModels from macOS 26.
fn main() {
    let frameworks: Vec<&str> = [("CARGO_FEATURE_TRANSLATION", "Translation"), ("CARGO_FEATURE_FOUNDATION_MODELS", "FoundationModels")]
        .into_iter()
        .filter(|(feature, _)| std::env::var_os(feature).is_some())
        .map(|(_, framework)| framework)
        .collect();
    if !frameworks.is_empty() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    }
    for framework in frameworks {
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,{framework}");
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
