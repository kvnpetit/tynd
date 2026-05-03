// swift-tools-version:5.5
//
// Swift package for the Tynd host's macOS-native bridge layer. Built
// by `host-rs/build.rs` via `swift-rs::build::SwiftLinker` when the
// `swift-bridge` cargo feature is on.
//
// Plugin authors can drop additional `.swift` files under
// `Sources/TyndSwift/` and add a matching `swift_rs::swift!`
// declaration in `host-rs/src/os/macos_swift.rs` (or in their plugin
// crate). The `@_cdecl` annotation in the Swift function is the
// linker-visible name swift-rs matches against.

import PackageDescription

let package = Package(
    name: "TyndSwift",
    platforms: [.macOS(.v10_15)],
    products: [
        .library(
            name: "TyndSwift",
            type: .static,
            targets: ["TyndSwift"]
        )
    ],
    targets: [
        .target(name: "TyndSwift", path: "Sources/TyndSwift")
    ]
)
