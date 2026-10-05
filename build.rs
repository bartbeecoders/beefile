//! Stamp a build number into the binary.
//!
//! Cargo runs this when a file in the package changes. The counter lives in
//! the target directory, which Cargo does not treat as a package file, so
//! writing it does not schedule another build.

use std::fs;
use std::path::PathBuf;

fn main() {
    let number = next_build_number();
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    println!("cargo:rustc-env=BEEFILE_BUILD={number}");
    println!("cargo:rustc-env=BEEFILE_VERSION={version}+{number}");
}

fn counter_path() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("target")
        });
    target.join("beefile-build-number")
}

fn next_build_number() -> u64 {
    let path = counter_path();
    let current = fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let next = current.saturating_add(1);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, format!("{next}\n"));
    next
}
