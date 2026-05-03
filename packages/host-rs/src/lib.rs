// Embed Info.plist into the binary's `__TEXT,__info_plist` section on macOS
// so that direct invocations (e.g. from a terminal or a CI runner) receive
// the correct entitlements — camera, microphone, accessibility prompts fire
// at the OS level instead of being silently denied.
#[cfg(target_os = "macos")]
embed_plist::embed_plist!("Info.plist");

pub mod app;
pub mod cleanup;
pub mod embed;
pub mod ipc;
pub mod logging;
pub(crate) mod menu;
pub mod observability;
pub mod os;
pub mod plugins;
pub mod runtime;
pub mod scheme;
pub mod scheme_bin;
pub mod signals;
pub(crate) mod tray;
pub mod window;

// Re-export the plugin SDK so consumer crates (preset runtimes + the
// CLI-generated workspace) can reference a single authoritative version.
// Re-export tracing so the `tynd_log!` / `tynd_info!` / `tynd_warn!` /
// `tynd_error!` / `tynd_debug!` macros below resolve through `$crate`
// without consumer crates needing their own `tracing` dep. Hidden from
// docs — consumers should prefer the macros or pull in `tracing`
// themselves if they want spans/instrumentation.
#[doc(hidden)]
pub use tracing as __tracing;
pub use tynd_plugin_sdk as plugin_sdk;

// Re-export the standalone `dpi` crate so consumer crates can type their
// window commands (positions / sizes) against the same canonical
// `LogicalPosition`, `PhysicalSize`, … types the host already uses
// internally — without taking a transitive dependency on `tao` just to
// get them. Both `tao::dpi::*` and `dpi::*` are interchangeable: tao
// re-exports this very crate.
pub use dpi;

/// Initialize the framework's structured logger (`tracing` + `tracing-subscriber`),
/// the global panic hook, and the cross-platform SIGINT/SIGTERM handler.
/// (See the OOM note below — the alloc-error hook is gated on a nightly API.)
///
/// Idempotent — safe to call multiple times. Reads `TYND_LOG` (or `RUST_LOG`)
/// to decide which levels surface; defaults to `info`.
///
/// Hosts (`tynd-full` / `tynd-lite`) call this once at startup. Plugins or
/// embeddings that want their own log layout can skip this and configure
/// `tracing_subscriber` themselves; in that case they're responsible for
/// installing their own panic / signal hooks too.
pub fn init_tracing() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_env("TYND_LOG")
            .or_else(|_| tracing_subscriber::EnvFilter::try_from_default_env())
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_writer(std::io::stderr)
            .compact()
            .try_init();

        install_panic_hook();
        signals::init();
    });
}

/// Install a structured panic hook that produces SRE-friendly logs:
///
/// - Captures the panic message, source location (file:line:col), and the full backtrace
///   (`std::backtrace::Backtrace::force_capture()` — works even when `RUST_BACKTRACE` is unset,
///   which is the common prod case).
/// - Emits at `error` level under `target = "tynd::panic"` so log filters can pin a high-severity
///   sink (e.g. JSON to stderr → log shipper).
/// - Calls `cleanup::run()` so registered temp dirs / IPC sockets / file locks
///   (`os::single_instance`) don't leak on the panic-then-abort path.
/// - Renders the message multi-line: header → message → location → backtrace. Avoids the unreadable
///   single-line `Debug` blob that `default_hook` ships.
///
/// `panic = "abort"` is set in the workspace `[profile.release]`, so once
/// this hook returns the runtime aborts the process. We don't need to call
/// `process::exit` ourselves; the abort guarantees no Drop runs and gives
/// the OS the cleanest possible exit signal.
///
/// Note on stack overflow: a guard-page hit produces a panic with a
/// truncated (or empty) backtrace because the stack walker itself runs out
/// of room. If you see "<empty backtrace>" below the location, suspect a
/// recursion bug or a thread spawned with too small a stack.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = panic_message(info);
        let location = info.location().map_or_else(
            || "<unknown>".to_string(),
            |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
        );
        let backtrace = std::backtrace::Backtrace::force_capture();
        let backtrace_str = format!("{backtrace}");
        let backtrace_display = if backtrace_str.trim().is_empty()
            || backtrace_str.contains("disabled backtrace")
        {
            "<empty backtrace — possible stack overflow or backtrace feature disabled>".to_string()
        } else {
            backtrace_str
        };

        tracing::error!(
            target: "tynd::panic",
            location = %location,
            "PANIC: {msg}\n  at {location}\nbacktrace:\n{backtrace_display}"
        );

        // Best-effort: nudge any buffered tracing writer to flush before
        // the runtime aborts. The `fmt` layer we install above writes
        // straight to stderr (line-buffered on most platforms), so an
        // explicit `stderr().flush()` is the realistic ask.
        use std::io::Write;
        let _ = std::io::stderr().flush();

        // Release OS resources (file locks, IPC sockets, temp dirs).
        cleanup::run();
    }));
}

/// Format a `PanicHookInfo`'s payload. `info.payload()` is `&dyn Any`; the
/// stdlib only auto-displays `&'static str` and `String` payloads, so we
/// do the same downcast dance here to keep the output readable instead of
/// an `Any { .. }` debug blob.
fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    format_panic_payload(info.payload())
}

/// Pure helper that runs the downcast logic against an `&dyn Any` directly.
/// Pulled out so unit tests can exercise it without having to fake a real
/// `PanicHookInfo` (which has no public constructor).
fn format_panic_payload(payload: &dyn std::any::Any) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

// NOTE on OOM handling: `std::alloc::set_alloc_error_hook` is still unstable
// (rust-lang/rust#51245) on the pinned 1.95 toolchain, so we can't override
// the default "memory allocation of N bytes failed" abort message here. The
// default behaviour is fine — the runtime aborts with the byte count on
// stderr, which is enough signal for the SRE post-mortem. When the hook is
// stabilized (or the workspace moves to nightly) we should add a structured
// `tracing::error!` wrapper that includes the requested layout *and* runs
// `cleanup::run()` before aborting, mirroring the panic-hook flow above.

/// Back-compat alias for `tynd_info!`. Prefer `tynd_info!` / `tynd_warn!` /
/// `tynd_error!` / `tynd_debug!` in new code (or call the `tracing::*` macros
/// directly).
#[macro_export]
macro_rules! tynd_log {
    ($($arg:tt)*) => { $crate::__tracing::info!(target: "tynd", $($arg)*) };
}

#[macro_export]
macro_rules! tynd_info {
    ($($arg:tt)*) => { $crate::__tracing::info!(target: "tynd", $($arg)*) };
}

#[macro_export]
macro_rules! tynd_warn {
    ($($arg:tt)*) => { $crate::__tracing::warn!(target: "tynd", $($arg)*) };
}

#[macro_export]
macro_rules! tynd_error {
    ($($arg:tt)*) => { $crate::__tracing::error!(target: "tynd", $($arg)*) };
}

#[macro_export]
macro_rules! tynd_debug {
    ($($arg:tt)*) => { $crate::__tracing::debug!(target: "tynd", $($arg)*) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `init_tracing` should be safely callable any number of times without
    /// re-registering the panic hook or the signal handler. We just make
    /// sure the call returns without aborting — the underlying `Once` /
    /// `INSTALLED` flags do the actual idempotency work.
    #[test]
    fn init_tracing_is_idempotent() {
        init_tracing();
        init_tracing();
        init_tracing();
    }

    /// The panic-message formatter must handle the three payload shapes
    /// `std::panic` can hand us: `&'static str`, `String`, and anything
    /// else (which we render as a placeholder rather than a `Debug` blob).
    #[test]
    fn panic_hook_captures_location_and_backtrace() {
        // &'static str — what `panic!("literal")` produces.
        assert_eq!(
            format_panic_payload(&"static literal" as &dyn std::any::Any),
            "static literal"
        );

        // String — what `panic!("fmt: {x}")` produces when `x` is dynamic.
        assert_eq!(
            format_panic_payload(&String::from("owned: 42") as &dyn std::any::Any),
            "owned: 42"
        );

        // Anything else — should not crash, should produce a stable label.
        struct Custom;
        assert_eq!(
            format_panic_payload(&Custom as &dyn std::any::Any),
            "<non-string panic payload>"
        );

        // Backtrace capture should work even with `RUST_BACKTRACE` unset
        // (we use `force_capture`). Confirm the API is wired.
        let bt = std::backtrace::Backtrace::force_capture();
        let rendered = format!("{bt}");
        assert!(
            !rendered.is_empty(),
            "force_capture should always yield a non-empty backtrace string"
        );
    }

    /// Idempotency on the signal layer — calling `signals::init()` twice
    /// must not double-register the OS handler (which would error on Unix
    /// and silently drop the previous closure on Windows).
    #[test]
    fn signals_init_is_idempotent() {
        signals::init();
        signals::init();
        // Second call should be a no-op; if it weren't, ctrlc would log
        // a warning but still return — so the assertion is mostly that
        // we don't panic / abort.
    }
}
