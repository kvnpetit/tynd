//! Web-standard polyfills for QuickJS.
//!
//! Lite is deliberately a lightweight JS runtime — it exposes the Web
//! platform surface every backend expects (fetch, crypto, streams, URL,
//! Blob, TextEncoder, etc.) and nothing else. Node / Bun / Deno APIs
//! are intentionally not polyfilled here: users who need actual OS
//! access go through the Tynd OS APIs in `@tynd/core/client` (fs, http,
//! websocket, sql, process, store, dialog, tray, …) which work
//! identically on both runtimes.
//!
//! For gaps that have a good pure-JS library available (AES-GCM,
//! argon2, gzip, …), let userland code pull a Web-compatible lib
//! rather than growing this file.

use std::sync::mpsc;

use crossbeam_channel as bridge_chan;
use rquickjs::Ctx;
use tynd_host::runtime::BackendEvent;

use crate::quickjs::JsMsg;

mod _session;
pub(crate) mod crypto;
pub(crate) mod fetch;
pub(crate) mod node_bridges;
pub(crate) mod perf;
pub(crate) mod websocket;

/// Register every native bridge and evaluate the pure-JS polyfills.
/// Must be called inside `ctx.with(|ctx| ...)` before evaluating user code.
#[allow(clippy::elidable_lifetime_names)]
pub(crate) fn install<'js>(
    ctx: &Ctx<'js>,
    _event_tx: &bridge_chan::Sender<BackendEvent>,
    js_tx: &mpsc::Sender<JsMsg>,
) -> rquickjs::Result<()> {
    let g = ctx.globals();
    crypto::register(ctx, &g)?;
    fetch::register(ctx, &g, js_tx)?;
    websocket::register(ctx, &g, js_tx)?;
    perf::register(ctx, &g)?;
    node_bridges::register(ctx, &g)?;
    ctx.eval::<(), _>(polyfills_js().as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests;

/// Compile-time zstd-compressed polyfill bundle (B50). Source order is
/// identical to the previous `concat!(include_str!(...))` chain — see
/// `lite/build.rs` for the file list. Trades ~30-40 KB binary size for
/// one synchronous zstd decompress on first reach (a few ms during
/// QuickJS startup, hidden behind `OnceLock`).
const POLYFILLS_ZST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/polyfills.zst"));

/// Lazy-decompressed polyfill JS. The first reach pays the zstd
/// decode cost; every subsequent reach hands back the same `&'static str`.
fn polyfills_js() -> &'static str {
    static DECOMPRESSED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DECOMPRESSED.get_or_init(|| {
        let bytes = zstd::stream::decode_all(POLYFILLS_ZST)
            .expect("lite polyfills: zstd decode failed (corrupt embedded blob)");
        String::from_utf8(bytes)
            .expect("lite polyfills: post-decode bytes are not valid UTF-8")
    })
}
