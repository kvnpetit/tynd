//! Swift↔Rust bridge — minimal smoke test that proves the
//! `swift-rs` toolchain is wired correctly.
//!
//! The bridge is the cleanest way for plugin authors to reach Swift-
//! only Apple frameworks (AVFoundation media engines, ScreenCaptureKit,
//! the new Foundation Translation API, …) without going through
//! `objc2`'s very large generated binding crates.
//!
//! Wiring overview:
//!  1. `host-rs/build.rs` (compiled when the `swift-bridge` feature
//!     is on) calls `swift_rs::build::SwiftLinker::new(...).link()`.
//!     That step compiles every `.swift` file under `host-rs/swift/`
//!     into a static library and emits the `cargo:rustc-link-...`
//!     directives.
//!  2. This module declares Rust↔Swift `extern "C"` bindings via
//!     `swift_rs::swift!`, which the Swift compiler matches by name
//!     (`@_cdecl("...")` annotations on the Swift side).
//!  3. Plugin authors extend by dropping new `.swift` files into
//!     `host-rs/swift/` and adding the matching `swift!` declaration
//!     here (or in a plugin crate that re-imports `swift_rs`).
//!
//! Smoke test: `system_uptime_swift()` returns
//! `NSProcessInfo.processInfo.systemUptime`. It's a stable Apple API
//! that doesn't require entitlements, so we can call it from any
//! sandboxed test process.
//!
//! macOS only — the entire module is `#[cfg(target_os = "macos")]`.

#![cfg(all(target_os = "macos", feature = "swift-bridge"))]

use swift_rs::swift;

swift! {
    /// Returns the time elapsed since the system was booted, in
    /// seconds. Mirrors `NSProcessInfo.processInfo.systemUptime`.
    pub fn tynd_swift_system_uptime() -> f64;
}

/// Safe wrapper around the FFI call. Rounds down to a whole number
/// of seconds so the IPC envelope stays integer-typed (frontend
/// timers don't care about the sub-second part).
#[must_use]
pub fn system_uptime_seconds() -> u64 {
    // SAFETY: `tynd_swift_system_uptime` is a no-arg, no-allocation
    // Swift entry point compiled by `swift-rs::build`. It never
    // panics across the FFI boundary (Swift errors are caught on the
    // Swift side and converted to NaN, which we floor to 0).
    let raw = unsafe { tynd_swift_system_uptime() };
    if raw.is_finite() && raw >= 0.0 {
        raw as u64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_is_positive() {
        // Even on a freshly booted CI host, NSProcessInfo's uptime
        // should be > 0. This test only runs on macOS with the
        // `swift-bridge` feature on.
        assert!(system_uptime_seconds() > 0);
    }
}
