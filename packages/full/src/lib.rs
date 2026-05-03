//! Library-style entry for the `tynd-full` runtime.
//!
//! Preset build (`default = ["preset"]`): every OS API is linked but NO
//! plugins are bundled — keeps the base binary lightweight. Apps that
//! need plugins (sql, terminal, audio, llm, etc.) declare them in
//! `tynd.config.ts` and the CLI generates a custom wrapper crate that
//! pulls only the plugins the app actually uses.
//!
//! Custom build (`--no-default-features --features "tynd-host/api-*"`):
//! the CLI generates a tiny wrapper crate that links `tynd_full` plus
//! whatever plugins the app declared, then calls `run()`:
//!
//! ```ignore
//! fn main() {
//!     tynd_host::plugins::register(my_plugin::__tynd_plugin());
//!     tynd_full::run();
//! }
//! ```

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod args;
mod bun;
mod bun_ipc;
mod embed;

use args::Args;

/// Start the Tynd host in full-mode. Blocks until the event loop exits.
/// Reads `TYNDPKG` from the binary tail if present, otherwise falls back
/// to the dev args (`--backend-entry <path>`).
pub fn run() -> ! {
    tynd_host::init_tracing();

    // Deep-link init MUST happen before any window / IPC setup. A second
    // launch carrying a `myapp://...` URL forwards to the running master
    // via a local socket and we then exit cleanly *without* spawning a
    // webview. Skipping the early exit means the user briefly sees a
    // second window flash on screen before we tear it down.
    //
    // Gated on the local `deep-link` feature, which the `preset` build
    // turns on alongside `tynd-host/api-deep-link`. Custom builds that
    // explicitly opt out drop the call entirely.
    #[cfg(feature = "deep-link")]
    {
        if let Err(e) = tynd_host::os::deep_link::init() {
            eprintln!("deep-link init: {e}");
        }
        if tynd_host::os::deep_link::was_forwarded_to_primary() {
            std::process::exit(0);
        }
    }

    if let Some(embedded) = embed::try_load_embedded() {
        std::env::set_var("TYND_BUN_PATH", &embedded.bun_path);
        std::env::set_var("TYND_FRONTEND_DIR", &embedded.frontend_dir);
        if let Some(ref icon) = embedded.icon_path {
            std::env::set_var("TYND_ICON_PATH", icon);
        }
        let (bridge, _reload) = bun::start(&embedded.bundle_path);
        tynd_host::app::run_app(bridge, false);
    }

    let args = Args::parse();
    let (bridge, reload) = bun::start(&args.backend_entry);

    if args.debug {
        // Serve static assets from disk so editor saves land on the next reload.
        tynd_host::scheme::set_dev_mode(true);
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

    tynd_host::app::run_app(bridge, args.debug);
}

