//! Library-style entry for the `tynd-runtime` runtime.
//!
//! Preset build (`default = ["preset"]`): the minimal host core
//! (always includes `acl` + `log`) plus the debug protocol are linked, no
//! other OS API, and NO plugins are bundled — keeps the base binary lightweight. Apps that
//! need plugins (sql, terminal, audio, llm, etc.) declare them in
//! `tynd.config.ts` and the CLI generates a custom wrapper crate that
//! pulls only the plugins the app actually uses.
//!
//! Custom build (`--no-default-features --features "tynd-host/api-*"`):
//! the CLI generates a tiny wrapper crate that links `tynd_runtime` plus
//! whatever plugins the app declared, then calls `run()`:
//!
//! ```ignore
//! fn main() {
//!     tynd_host::plugins::register(my_plugin::__tynd_plugin());
//!     tynd_runtime::run();
//! }
//! ```

mod app_identity;
mod args;
mod bun;
mod crash;
mod embed;
mod observability;

use args::Args;

// `tauri-bundler`, which Tynd reuses only for native installer generation,
// patches this marker in each copied executable before producing an installer.
// Keeping the marker in tynd-runtime prevents a misleading warning about
// Tauri's updater; Tynd's updater remains independent.
//
// The mutable reference matches tauri-utils' marker representation. `#[used]`
// alone is insufficient under release LTO on every supported linker.
#[used]
static mut __TYND_BUNDLE_TYPE: &str = "__TAURI_BUNDLE_TYPE_VAR_UNK";

/// Start the Tynd Bun host. Blocks until the event loop exits.
/// Reads `TYNDPKG` from the binary tail if present, otherwise falls back
/// to the dev args (`--backend-entry <path>`).
pub fn run() -> ! {
    tynd_host::init_tracing();
    crash::install_panic_hook();
    let outcome: std::thread::Result<()> =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> () { run_inner() }));
    match outcome {
        Err(payload) => {
            // Local crash report (`app.crashReporter`), when the app started it.
            crash::write_escaped_panic(payload.as_ref());
            // The panic hook only logs: it also runs for plugin panics that
            // are intentionally caught. Cleanup belongs at this outermost
            // application boundary, where the panic is genuinely escaping.
            // Sentry's panic hook already captured it; make sure it is sent.
            tynd_host::observability::flush(std::time::Duration::from_secs(2));
            tynd_host::cleanup();
            std::panic::resume_unwind(payload);
        },
        // `run_inner` ends in Tao's non-returning event loop. Treat an
        // impossible normal return as fatal without manufacturing a second
        // panic (and a misleading second panic report).
        Ok(()) => std::process::abort(),
    }
}

fn run_inner() -> ! {
    // A packaged app owns its command line (`-h` / `-V` included); only the
    // unpacked runtime answers informational flags itself.
    if !embed::is_packaged() {
        Args::exit_on_informational_flags();
    }

    // MUST run before window/IPC setup: with the deep-link API compiled in, a
    // second launch carrying a `myapp://...` URL exits here without flashing
    // a window.
    tynd_host::pre_run();

    let embedded = embed::try_load_embedded();

    // Crash reporting / traces start before the Bun child and the event loop,
    // so boot-time failures and every later host panic are covered. No-op
    // unless the app configured `observability`.
    observability::init(
        embedded.as_ref().and_then(|e| e.observability.clone()),
        embedded.is_some(),
    );
    // Names the app's own directories; fixed before the Bun child (and any
    // app code) starts.
    app_identity::init(
        embedded.as_ref().and_then(|e| e.app_identifier.clone()),
        embedded.is_some(),
    );

    if let Some(embedded) = embedded {
        run_packaged(embedded);
    }
    run_dev()
}

/// Boot a packaged app from its embedded `TYNDPKG` payload.
fn run_packaged(embedded: embed::EmbeddedAssets) -> ! {
    // A packaged app serves the debug protocol only if built with
    // `tynd build --debug-protocol`, whatever `TYND_DEBUG_*` says.
    tynd_host::embed::set_pack_debug(embedded.debug);
    // SAFETY: runs single-threaded during boot — no other thread is
    // reading the env yet (deep-link init in `run_inner` is sync, plugins
    // register on the main thread, the Bun child hasn't been spawned).
    // Rust 2024 made `env::set_var` `unsafe`; the wrapping just
    // documents the precondition we always relied on.
    unsafe {
        std::env::set_var("TYND_BUN_PATH", &embedded.bun_path);
    }
    unsafe {
        std::env::set_var("TYND_FRONTEND_DIR", &embedded.frontend_dir);
    }
    if let Some(ref icon) = embedded.icon_path {
        unsafe {
            std::env::set_var("TYND_ICON_PATH", icon);
        }
    }
    #[cfg(windows)]
    if let Some(ref relative) = embedded.webview2_runtime_path {
        let runtime = std::env::current_exe()
            .ok()
            .and_then(|exe| embed::resolve_webview2_runtime_path(&exe, relative))
            .unwrap_or_else(|| {
                tynd_host::tynd_error!(
                    "Configured WebView2 fixed runtime '{relative}' is missing beside the \
                     packaged executable"
                );
                tynd_host::cleanup();
                std::process::exit(1);
            });
        // SAFETY: same single-threaded boot phase as TYND_BUN_PATH above.
        unsafe {
            std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", runtime);
        }
    }
    let (mut bridge, _reload) = bun::start(&embedded.bundle_path, args::packaged_app_args());
    if std::env::var_os("TYND_PACKAGED_SMOKE").is_some() {
        bridge.config.smoke_mode = true;
    }
    tynd_host::run_app(bridge, false);
}

/// Boot the unpacked runtime from the dev args (`--backend-entry <path>`).
fn run_dev() -> ! {
    let args = Args::parse();
    let (bridge, reload) = bun::start(&args.backend_entry, args.passthrough);
    // The reload handle owns the Bun child (`ChildSlot` kills it on drop). The
    // stdin reader thread below gets a clone; this one lives for the whole
    // process so a closed stdin (IDE launch, dead parent CLI) never kills Bun.
    let _bun_lifetime = reload.clone();

    if args.debug {
        // Serve static assets from disk so editor saves land on the next reload.
        tynd_host::set_dev_mode(true);
        std::thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                match line.trim() {
                    "reload" => reload.reload(),
                    "reload-frontend" => reload.reload_frontend_only(),
                    _ => {},
                }
            }
        });
    }

    tynd_host::run_app(bridge, args.debug);
}
