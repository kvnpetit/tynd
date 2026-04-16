// Hide the console window in Windows release builds
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod args;
mod embed;
mod quickjs;

use args::Args;

fn main() {
    // Embedded-assets mode: vorn dist appends a packed section to this binary.
    if let Some(embedded) = embed::try_load_embedded() {
        let (bridge, _reload) = quickjs::start(
            &embedded.bundle_path,
            Some(embedded.frontend_dir.clone()),
            None,
            embedded.icon_path.clone(),
        );
        vorn_host::app::run_app(bridge, false);
    }

    let args = Args::parse();

    let (bridge, reload) = quickjs::start(
        &args.bundle_path,
        args.frontend_dir,
        args.dev_url,
        None, // no icon in dev mode
    );

    // Dev mode: listen on stdin for admin commands from `vorn dev`.
    // A "reload\n" line hot-reloads the backend by spawning a fresh QuickJS
    // runtime; the host process and the WebView stay alive.
    if args.debug {
        std::thread::spawn(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                if line.trim() == "reload" {
                    reload.reload();
                }
            }
        });
    }

    vorn_host::app::run_app(bridge, args.debug);
}
