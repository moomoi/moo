//! Compiles `swift/ai.swift` (Apple's on-device model, which has no Objective-C API) into a static
//! library on macOS. The Swift runtime libraries it autolinks ship with macOS in /usr/lib/swift.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=swift/ai.swift");
    println!("cargo:rerun-if-changed=swift/http.swift");
    println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => return println!("cargo:warning=moo AI bridge: unsupported arch {other}"),
    };
    let deployment = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "14.0".into());
    let sdk = xcrun(&["--show-sdk-path"]);
    let swiftc = xcrun(&["-f", "swiftc"]);

    let status = Command::new(&swiftc)
        .args(["-parse-as-library", "-emit-library", "-static", "-O", "-swift-version", "6"])
        .args(["-module-name", "MooAI", "-target", &format!("{arch}-apple-macos{deployment}"), "-sdk", &sdk])
        .args(["swift/ai.swift", "swift/http.swift"])
        .arg("-o")
        .arg(out.join("libmoo_ai.a"))
        .status()
        .expect("run swiftc");
    assert!(status.success(), "swiftc failed to build the Swift sources");

    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=moo_ai");
    println!("cargo:rustc-link-search=native={sdk}/usr/lib/swift");
    let toolchain_lib = PathBuf::from(&swiftc).parent().unwrap().join("../lib/swift/macosx");
    println!("cargo:rustc-link-search=native={}", toolchain_lib.display());
}

fn xcrun(args: &[&str]) -> String {
    let out = Command::new("xcrun").args(args).output().expect("run xcrun");
    assert!(out.status.success(), "xcrun {args:?} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}
