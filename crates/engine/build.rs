use std::env;
use std::path::PathBuf;
use std::process::Command;

/// With the `translation` feature, compiles `src/translation.swift` into a static library. The
/// Swift runtime ships with macOS; Translation is weak-linked, since it exists only from macOS 15.
fn main() {
    println!("cargo:rerun-if-changed=src/translation.swift");
    if env::var_os("CARGO_FEATURE_TRANSLATION").is_none() {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x86_64",
        _ => "arm64",
    };
    let deployment = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "14.0".into());
    let status = Command::new("xcrun")
        .args(["swiftc", "-emit-library", "-static", "-parse-as-library", "-swift-version", "5", "-O", "-module-name", "TarnTranslation"])
        .args(["-target", &format!("{arch}-apple-macos{deployment}")])
        .arg("src/translation.swift")
        .arg("-o")
        .arg(out.join("libtarn_translation.a"))
        .status()
        .expect("xcrun swiftc");
    assert!(status.success(), "swiftc failed");
    let sdk = Command::new("xcrun").args(["--show-sdk-path"]).output().expect("xcrun --show-sdk-path");
    let sdk = String::from_utf8(sdk.stdout).expect("SDK path").trim().to_owned();
    let swiftc = Command::new("xcrun").args(["--find", "swiftc"]).output().expect("xcrun --find swiftc");
    let swiftc = PathBuf::from(String::from_utf8(swiftc.stdout).expect("swiftc path").trim());
    let toolchain = swiftc.ancestors().nth(2).expect("toolchain usr").join("lib/swift/macosx");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    println!("cargo:rustc-link-search=native={}", toolchain.display());
    println!("cargo:rustc-link-lib=static=tarn_translation");
    // Rust links for an older macOS than Swift Concurrency shipped in, so it is named @rpath.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rustc-link-arg=-weak_framework");
    println!("cargo:rustc-link-arg=Translation");
}
