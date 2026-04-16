// Hide the console window in Windows release builds
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod args;
mod bun;
mod embed;

use args::Args;

fn main() {
    // Check for embedded assets (vorn build output — VORNPKG appended to binary)
    if let Some(embedded) = embed::try_load_embedded() {
        // Expose paths via env vars so the backend bundle and bun::start can read them
        std::env::set_var("VORN_BUN_PATH",      &embedded.bun_path);
        std::env::set_var("VORN_FRONTEND_DIR",  &embedded.frontend_dir);
        if let Some(ref icon) = embedded.icon_path {
            std::env::set_var("VORN_ICON_PATH", icon);
        }
        let bridge = bun::start(&embedded.bundle_path);
        vorn_host::app::run_app(bridge, false);
    }

    // Dev mode — use CLI args and system Bun
    let args = Args::parse();
    let bridge = bun::start(&args.backend_entry);
    vorn_host::app::run_app(bridge, args.debug);
}
