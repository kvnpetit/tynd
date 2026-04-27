//! Library-style entry for the `tynd-full` runtime.
//!
//! Preset build (`default = ["preset"]`): links every official plugin via
//! the `bundled-plugins` feature and auto-registers them before the
//! event loop starts — user apps get the full framework surface with
//! zero config.
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
mod embed;

use args::Args;

/// Start the Tynd host in full-mode. Blocks until the event loop exits.
/// Reads `TYNDPKG` from the binary tail if present, otherwise falls back
/// to the dev args (`--backend-entry <path>`).
pub fn run() -> ! {
    #[cfg(feature = "bundled-plugins")]
    register_bundled_plugins();

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

/// Register every official plugin that was linked in through the
/// `bundled-plugins` feature. Runs before the plugin event bus is wired
/// (safe — each plugin's `init()` gets called later when
/// `install_default_emit()` fires in `run_app`).
#[cfg(feature = "bundled-plugins")]
fn register_bundled_plugins() {
    use tynd_host::plugins::register;
    register(tynd_plugin_sql::__tynd_plugin());
    register(tynd_plugin_terminal::__tynd_plugin());
    register(tynd_plugin_updater::__tynd_plugin());
    register(tynd_plugin_keyring::__tynd_plugin());
    register(tynd_plugin_autolaunch::__tynd_plugin());
    register(tynd_plugin_shortcuts::__tynd_plugin());
    register(tynd_plugin_power::__tynd_plugin());
    register(tynd_plugin_log::__tynd_plugin());
    register(tynd_plugin_clipboard_history::__tynd_plugin());
}
