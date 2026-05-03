// Smoke test for the Tynd swift-rs bridge.
//
// `@_cdecl` exposes the function with a stable C-ABI symbol that
// matches the `swift!` declaration on the Rust side
// (`host-rs/src/os/macos_swift.rs::tynd_swift_system_uptime`).

import Foundation

@_cdecl("tynd_swift_system_uptime")
public func tyndSwiftSystemUptime() -> Double {
    return ProcessInfo.processInfo.systemUptime
}
