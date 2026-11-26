// Embed Info.plist into the binary's `__TEXT,__info_plist` section on macOS
// so that direct invocations (e.g. from a terminal or a CI runner) receive
// the correct entitlements — camera, microphone, accessibility prompts fire
// at the OS level instead of being silently denied.
#[cfg(target_os = "macos")]
embed_plist::embed_info_plist!("../Info.plist");

// Wire formats (audio f32 lanes, rkyv payloads) reinterpret host bytes as
// little-endian; refuse to build on big-endian targets to keep that
// invariant explicit at the workspace edge.
#[cfg(target_endian = "big")]
compile_error!("tynd host only supports little-endian targets");

pub(crate) mod api;
pub(crate) mod app;
pub mod app_dirs;
pub(crate) mod cleanup;
#[cfg(feature = "debug-protocol")]
pub(crate) mod debug;
pub mod embed;
pub(crate) mod ipc;
pub mod observability;
pub(crate) mod platform;
#[cfg(feature = "plugins")]
pub mod plugins;
pub(crate) mod security;
pub(crate) mod signals;
pub(crate) mod util;
pub(crate) mod webview;
pub(crate) mod window;

// Re-exported so the `tynd_info!` / `tynd_warn!` / `tynd_error!` /
// `tynd_debug!` macros below resolve through `$crate` without consumer crates
// needing their own `tracing` dep.
#[doc(hidden)]
pub use tracing as __tracing;

/// Host proc-macros — `#[tynd::handler]` typed dispatch.
///
/// ```ignore
/// use crate::tynd;
///
/// #[tynd::handler(api = "compute")]
/// impl ComputeApi { /* ... */ }
/// ```
pub(crate) use tynd_host_macros as tynd;

// The public surface: what `tynd-runtime`, the generated custom-host
// `main.rs` and native plugins call, beside the `pub` items of `app_dirs`,
// `embed`, `observability` and `plugins` and the `tynd_*!` log macros. Every other
// module is crate-private, so the dead-code lint sees the whole crate.

/// Run one host API call (`api` and `method` are wire names) on behalf of the
/// backend, outside the renderer capability checks.
pub use api::dispatch as dispatch_api;
pub use api::window_cmd::dispatch_backend as dispatch_backend_window;
/// Build the window and run the event loop until the app exits.
pub use app::run_app;
/// Serve frontend assets from disk on every request (`tynd dev`).
pub use webview::scheme::set_dev_mode;

/// Answer a webview request forwarded to a backend `app.protocol(...)`
/// handler (`id` from the `protocol` frame; unknown ids are ignored).
pub fn resolve_backend_protocol(
    id: &str,
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) {
    webview::protocols::resolve_backend_protocol(id, status, headers, body);
}

/// Fail a forwarded backend protocol request with a 502 carrying `message`.
pub fn fail_backend_protocol(id: &str, message: &str) {
    webview::protocols::fail_backend_protocol(id, message);
}

/// Early-exit checks to run first in `main`, before any window or IPC setup.
///
/// With `api-deep-link`: a second launch that carries a `myapp://...` URL
/// forwards it to the running instance and exits here, without flashing a
/// window. A no-op otherwise.
pub fn pre_run() {
    #[cfg(feature = "api-deep-link")]
    {
        if let Err(error) = api::deep_link::init() {
            eprintln!("deep-link init: {error}");
        }
        if api::deep_link::was_forwarded_to_primary() {
            std::process::exit(0);
        }
    }
}

/// Run plugin shutdown hooks, then delete every registered temporary
/// directory. For exits that do not go through the event loop (a fatal boot
/// error, an uncaught main-thread panic); `run_app` does it itself.
pub fn cleanup() {
    cleanup::run();
}

/// Initialize the framework's structured logger (`tracing` + `tracing-subscriber`),
/// the global panic hook, and the cross-platform SIGINT/SIGTERM handler.
/// (See the OOM note below — the alloc-error hook is gated on a nightly API.)
///
/// Idempotent — safe to call multiple times. Reads `TYND_LOG` (or `RUST_LOG`)
/// to decide which levels surface; defaults to `info`.
///
/// The host calls this once at startup. Plugins or
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
        observability::install_tracing_subscriber(filter);

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
/// - Renders the message multi-line: header → message → location → backtrace. Avoids the unreadable
///   single-line `Debug` blob that `default_hook` ships.
///
/// The hook deliberately does not run global cleanup. Hooks execute before
/// `catch_unwind`, including for contained native-plugin and OS-dispatch
/// panics. Cleaning up here would tear down a healthy application after a
/// recovered plugin failure. `tynd_runtime::run` owns cleanup for an
/// uncaught main-thread panic; normal exits and signals use the event-loop
/// cleanup path.
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

        // Best-effort: nudge any buffered tracing writer to flush. The
        // `fmt` layer we install above writes
        // straight to stderr (line-buffered on most platforms), so an
        // explicit `stderr().flush()` is the realistic ask.
        use std::io::Write;
        let _ = std::io::stderr().flush();
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

// NOTE on OOM handling: `std::alloc::set_alloc_error_hook` is unstable
// (rust-lang/rust#51245) so we can't override the default "memory allocation
// of N bytes failed" abort here. Once stabilized, add a structured
// `tracing::error!` wrapper. Allocation failure aborts immediately, so the
// normal runtime cleanup guard cannot run.

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
