pub mod app;
pub mod cleanup;
pub mod embed;
pub mod ipc;
pub(crate) mod menu;
pub mod os;
pub mod plugins;
pub mod runtime;
pub mod scheme;
pub mod scheme_bin;
pub(crate) mod tray;
pub mod window;

// Re-export the plugin SDK so consumer crates (preset runtimes + the
// CLI-generated workspace) can reference a single authoritative version.
pub use tynd_plugin_sdk as plugin_sdk;

// Re-export tracing so the `tynd_log!` / `tynd_info!` / `tynd_warn!` /
// `tynd_error!` / `tynd_debug!` macros below resolve through `$crate`
// without consumer crates needing their own `tracing` dep. Hidden from
// docs — consumers should prefer the macros or pull in `tracing`
// themselves if they want spans/instrumentation.
#[doc(hidden)]
pub use tracing as __tracing;

/// Initialize the framework's structured logger (`tracing` + `tracing-subscriber`).
/// Idempotent — safe to call multiple times. Reads `TYND_LOG` (or `RUST_LOG`)
/// to decide which levels surface; defaults to `info`.
///
/// Hosts (`tynd-full` / `tynd-lite`) call this once at startup. Plugins or
/// embeddings that want their own log layout can skip this and configure
/// `tracing_subscriber` themselves.
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
    });
}

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
