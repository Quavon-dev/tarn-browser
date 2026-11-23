use std::env;
use std::path::PathBuf;
use std::process::Command;

/// Compiles the Swift shims the enabled features need into static libraries: `translation` for
/// macOS, `foundation-models` for macOS and iOS. The Swift runtime ships with both systems; each
/// framework is weak-linked, since it exists only from macOS 15 (Translation) or 26 / iOS 26.
fn main() {
    println!("cargo:rerun-if-changed=src/translation.swift");
    println!("cargo:rerun-if-changed=src/foundation_models.swift");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    println!("cargo:rerun-if-env-changed=IPHONEOS_DEPLOYMENT_TARGET");
    if env::var_os("CARGO_FEATURE_TRANSLATION").is_some() {
        swift("translation", "TarnTranslation", "Translation");
    }
    if env::var_os("CARGO_FEATURE_FOUNDATION_MODELS").is_some() {
        swift("foundation_models", "TarnFoundationModels", "FoundationModels");
    }
}

fn xcrun(args: &[&str]) -> String {
    let out = Command::new("xcrun").args(args).output().expect("xcrun");
    assert!(out.status.success(), "xcrun {args:?} failed");
    String::from_utf8(out.stdout).expect("xcrun output").trim().to_owned()
}

fn swift(name: &str, module: &str, framework: &str) {
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x86_64",
        _ => "arm64",
    };
    let simulator = [env::var("CARGO_CFG_TARGET_ABI"), env::var("CARGO_CFG_TARGET_ENV")].iter().any(|v| v.as_deref() == Ok("sim"));
    let (sdk, target) = if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        let deployment = env::var("IPHONEOS_DEPLOYMENT_TARGET").unwrap_or_else(|_| "17.0".into());
        if simulator { ("iphonesimulator", format!("{arch}-apple-ios{deployment}-simulator")) } else { ("iphoneos", format!("{arch}-apple-ios{deployment}")) }
    } else {
        ("macosx", format!("{arch}-apple-macos{}", env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "14.0".into())))
    };
    let lib = format!("tarn_{name}");
    let status = Command::new("xcrun")
        .args(["--sdk", sdk, "swiftc", "-emit-library", "-static", "-parse-as-library", "-swift-version", "5", "-O", "-module-name", module])
        .args(["-target", &target])
        .arg(format!("src/{name}.swift"))
        .arg("-o")
        .arg(out.join(format!("lib{lib}.a")))
        .status()
        .expect("xcrun swiftc");
    assert!(status.success(), "swiftc failed");
    let sdk_path = xcrun(&["--sdk", sdk, "--show-sdk-path"]);
    let swiftc = PathBuf::from(xcrun(&["--sdk", sdk, "--find", "swiftc"]));
    let toolchain = swiftc.ancestors().nth(2).expect("toolchain usr").join("lib/swift").join(sdk);
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-search=native={sdk_path}/usr/lib/swift");
    println!("cargo:rustc-link-search=native={}", toolchain.display());
    println!("cargo:rustc-link-lib=static={lib}");
    // Rust links for an older system than Swift Concurrency shipped in, so it is named @rpath.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    println!("cargo:rustc-link-arg=-weak_framework");
    println!("cargo:rustc-link-arg={framework}");
}
