//! Build script for the Tynd host crate.
//!
//! Currently has one job: when the `swift-bridge` cargo feature is
//! enabled (macOS only), compile the bundled Swift package under
//! `swift/` into a static library and emit the `cargo:rustc-link-...`
//! directives so the resulting symbols are visible to the
//! `os::macos_swift` module's `swift!` declarations.
//!
//! On every other configuration this script is a no-op — Linux /
//! Windows builds skip it, and macOS builds without `swift-bridge`
//! emit nothing.
//!
//! NOTE: this script intentionally avoids unwrapping or panicking
//! during builds where Swift isn't expected. The `cfg` gate at the
//! top makes the swift_rs import + linker call literally not exist
//! on non-macOS targets.

#[cfg(all(target_os = "macos", feature = "swift-bridge"))]
fn main() {
    use swift_rs::SwiftLinker;

    // The package name `TyndSwift` matches `swift/Package.swift`;
    // `with_package` joins the workspace path in lockstep.
    SwiftLinker::new("10.15")
        .with_package("TyndSwift", "swift/")
        .link();

    println!("cargo:rerun-if-changed=swift/Sources");
    println!("cargo:rerun-if-changed=swift/Package.swift");
}

#[cfg(not(all(target_os = "macos", feature = "swift-bridge")))]
fn main() {
    // No-op on every other config.
}
