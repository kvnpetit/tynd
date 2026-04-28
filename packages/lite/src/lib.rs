//! Library-style entry for the `tynd-lite` runtime. Mirrors
//! `tynd-full::run()` so the preset binary and CLI-generated custom
//! runtimes share the same startup code.

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod args;
mod embed;
mod polyfills;
mod quickjs;

use args::Args;

/// Start the Tynd host in lite-mode. Blocks until the event loop exits.
pub fn run() -> ! {
    tynd_host::init_tracing();

    #[cfg(feature = "bundled-plugins")]
    register_bundled_plugins();

    if let Some(embedded) = embed::try_load_embedded() {
        let (bridge, _reload) = quickjs::start(
            &embedded.bundle_path,
            Some(embedded.frontend_dir.clone()),
            None,
            embedded.icon_path.clone(),
        );
        tynd_host::app::run_app(bridge, false);
    }

    let args = Args::parse();

    let (bridge, reload) = quickjs::start(&args.bundle_path, args.frontend_dir, args.dev_url, None);

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

/// Register every official plugin linked through the `bundled-plugins`
/// feature. Runs before the event loop so dispatch is ready the moment
/// the frontend boots.
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
