// Hide the console window in Windows release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    // Install the human-panic hook before anything else so any panic
    // inside `tynd_full::run()` (host bootstrap, plugin init, embed
    // unpacking, ...) gets a friendly markdown report path instead of
    // the raw `thread 'main' panicked at ...` Rust default. Calling it
    // first is required — `setup_panic!` only catches panics that fire
    // *after* the hook is registered.
    human_panic::setup_panic!(
        human_panic::Metadata::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
            .homepage("https://tynd.kvnpetit.com/")
            .repository("https://github.com/tynd-apps/tynd")
            .support("- Open an issue at https://github.com/tynd-apps/tynd/issues")
    );
    tynd_full::run();
}
