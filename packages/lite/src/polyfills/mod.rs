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
    ctx.eval::<(), _>(POLYFILLS_JS.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests;

/// Concatenated polyfill JS. Order encodes dependency: base64 + text
/// first (define `btoa`/`atob` + `__tynd_b64_*`/`TextEncoder`), then
/// `_bridge.js` (publishes `__tynd_to_bytes` + the polyfill-event
/// dispatcher used by crypto/fetch/websocket), then abort (fetch
/// prereq), then crypto, then fetch, then the assorted standalones.
pub(super) const POLYFILLS_JS: &str = concat!(
    include_str!("base64.js"),
    include_str!("text.js"),
    include_str!("_bridge.js"),
    include_str!("abort.js"),
    include_str!("crypto.js"),
    include_str!("fetch.js"),
    include_str!("url.js"),
    include_str!("websocket.js"),
    include_str!("perf.js"),
    include_str!("clone.js"),
    include_str!("blob.js"),
    include_str!("event-source.js"),
);
