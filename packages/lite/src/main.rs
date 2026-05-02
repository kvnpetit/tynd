// Hide the console window in Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    // Install the human-panic hook before anything else so any panic
    // inside `tynd_lite::run()` (QuickJS bootstrap, polyfill init,
    // embed unpacking, ...) lands on a friendly markdown crash report
    // instead of the raw Rust panic banner. Must be the first line —
    // `setup_panic!` only catches panics fired *after* it runs.
    human_panic::setup_panic!(
        human_panic::Metadata::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
            .homepage("https://tynd.kvnpetit.com/")
            .repository("https://github.com/tynd-apps/tynd")
            .support("- Open an issue at https://github.com/tynd-apps/tynd/issues")
    );
    tynd_lite::run();
}
