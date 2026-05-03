//! Compile-time zstd-pre-compression of the lite polyfill JS bundle (B50).
//!
//! The lite runtime ships ~50 KB of pure-JS Web-platform polyfills
//! (fetch, crypto, blob, url, …) that QuickJS evaluates at startup.
//! Source files live alongside `mod.rs` and used to be `concat!`'d via
//! `include_str!` macros — the embedded blob landed in `.rodata`
//! verbatim. JS compresses well (~3-4× with zstd 19), so pre-compressing
//! at build time saves ~30-40 KB on the final lite binary at the cost
//! of one synchronous decompression on first polyfill install.
//!
//! Output: `OUT_DIR/polyfills.zst` (a single concatenated, zstd-compressed
//! blob). Consumed by `polyfills::mod.rs` via `include_bytes!`.

use std::path::PathBuf;

/// Order of inclusion matches the previous `concat!(include_str!(...))`
/// chain in `polyfills/mod.rs` exactly. Order encodes runtime
/// dependencies — base64 + text first, then the bridge, then crypto/
/// fetch/etc. Re-arranging breaks the polyfill init.
const POLYFILL_FILES: &[&str] = &[
    "src/polyfills/base64.js",
    "src/polyfills/text.js",
    "src/polyfills/_bridge.js",
    "src/polyfills/abort.js",
    "src/polyfills/crypto.js",
    "src/polyfills/fetch.js",
    "src/polyfills/url.js",
    "src/polyfills/websocket.js",
    "src/polyfills/perf.js",
    "src/polyfills/clone.js",
    "src/polyfills/blob.js",
    "src/polyfills/event-source.js",
];

fn main() {
    // Re-run when any polyfill source changes. `cargo:rerun-if-changed`
    // entries are additive so we can list each file individually rather
    // than rerun-on-every-build.
    println!("cargo:rerun-if-changed=build.rs");
    for f in POLYFILL_FILES {
        println!("cargo:rerun-if-changed={f}");
    }

    let mut combined = Vec::with_capacity(64 * 1024);
    for path in POLYFILL_FILES {
        let bytes = std::fs::read(path)
            .unwrap_or_else(|e| panic!("polyfill source missing — '{path}': {e}"));
        combined.extend_from_slice(&bytes);
    }

    // zstd level 19 — `--ultra`-tier ratio without paying for level 22
    // (which only helps on enormous inputs). The work happens once per
    // build; runtime decompression cost is identical regardless of
    // compression level.
    let compressed = zstd::stream::encode_all(combined.as_slice(), 19)
        .expect("zstd compression of polyfills failed");

    let out_dir: PathBuf = std::env::var_os("OUT_DIR")
        .expect("OUT_DIR not set by cargo")
        .into();
    let out_file = out_dir.join("polyfills.zst");
    std::fs::write(&out_file, &compressed).unwrap_or_else(|e| {
        panic!(
            "failed to write compressed polyfills to {}: {e}",
            out_file.display()
        )
    });

    // Surface the savings in build logs so a future contributor reading
    // `cargo build -vv` output can confirm the pre-compression worked.
    let raw = combined.len();
    let zst = compressed.len();
    let saved_pct = if raw == 0 {
        0
    } else {
        100 - (zst * 100 / raw)
    };
    println!(
        "cargo:warning=lite polyfills: {raw} → {zst} bytes (zstd-19, -{saved_pct}%)"
    );
}
