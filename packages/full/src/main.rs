// Hide the console window in Windows release builds
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod args;
mod bun;
mod embed;

use args::Args;

fn main() {
    // Check for embedded assets (tynd build output — TYNDPKG appended to binary)
    if let Some(embedded) = embed::try_load_embedded() {
        // Expose paths via env vars so the backend bundle and bun::start can read them
        std::env::set_var("TYND_BUN_PATH", &embedded.bun_path);
        std::env::set_var("TYND_FRONTEND_DIR", &embedded.frontend_dir);
        if let Some(ref icon) = embedded.icon_path {
            std::env::set_var("TYND_ICON_PATH", icon);
        }
        let (bridge, _reload) = bun::start(&embedded.bundle_path);
        tynd_host::app::run_app(bridge, false);
    }

    // Dev mode — use CLI args and system Bun
    let args = Args::parse();
    let (bridge, reload) = bun::start(&args.backend_entry);

    // In dev mode, listen on stdin for admin commands from the CLI (`tynd dev`).
    // A "reload\n" line restarts the Bun subprocess without tearing down the
    // WebView, so the window keeps its position/size across HMR cycles.
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

    tynd_host::app::run_app(bridge, args.debug);
}
