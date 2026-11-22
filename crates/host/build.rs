//! Build script for the Tynd host crate.
//!
//! Re-runs when `TYND_UPDATER_PUBKEY` changes so the compiled-in updater verifying key stays in
//! sync with the deployer-supplied value (see `api::updater`).

fn main() {
    println!("cargo:rerun-if-env-changed=TYND_UPDATER_PUBKEY");
}
