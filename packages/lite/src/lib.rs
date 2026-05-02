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

