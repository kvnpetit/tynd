//! Async WebSocket backend skeleton (feature `ws-async`).
//!
//! Scaffolding only — the production dispatcher in `os::websocket`
//! still routes through the blocking `tungstenite` poll loop. This
//! module exists so the feature flag, dependency wiring, and runtime
//! handle live together at the right place; the full migration to
//! `tokio-tungstenite` lands in a follow-up sprint.
//!
//! Architectural notes (for the implementer):
//!
//! - Tynd's runtime contract is sync-first. The async backend spawns
//!   a single multi-thread Tokio runtime on first use (lazy `OnceLock`)
//!   and parks WebSocket sessions onto it. Sync callers (the existing
//!   `dispatch` API + the lite QuickJS `WebSocket` polyfill) bridge
//!   through `tokio::sync::mpsc` into the runtime; the runtime drives
//!   the actual `WebSocketStream` futures.
//!
//! - The 25 ms `set_read_timeout` busy-poll in the blocking backend
//!   exists because `tungstenite::WebSocket::read` is fully blocking
//!   and would starve the outbound `Cmd` queue otherwise. Once the
//!   stream is split into `(SinkHalf, StreamHalf)` and driven by
//!   `tokio::select!`, that floor disappears.
//!
//! - Reuse the existing `Cmd` enum + `events::emit` sink shape so the
//!   public `dispatch()` API never changes.

use std::sync::OnceLock;

use tokio::runtime::{Builder, Runtime};

/// Process-wide Tokio runtime for `ws-async`. Built on first reach
/// inside this module — sync callers go through `runtime()` so
/// startup cost is paid only when an async session is actually
/// requested.
static RT: OnceLock<Runtime> = OnceLock::new();

/// Lazy multi-thread runtime constructor. Single worker is enough for
/// the WebSocket fan-out — the heavy I/O is done by the kernel and
/// the framing is cheap. Bumping workers later is a one-liner if a
/// chat-app workload ever pushes a single-thread runtime past its limit.
#[allow(dead_code)]
pub(super) fn runtime() -> &'static Runtime {
    RT.get_or_init(|| {
        Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .enable_time()
            .thread_name("tynd-ws-async")
            .build()
            .expect("ws-async: tokio runtime build failed")
    })
}

/// Placeholder hook for the async session driver. Wires in once the
/// `WebSocketStream` split + `tokio::select!` loop is ported from the
/// blocking implementation. Until then, the parent module keeps
/// routing every connect through `run_session_with`.
#[allow(dead_code)]
pub(super) fn spawn_async_session_placeholder(_url: String) {
    // The full impl will:
    //   1. `tokio_tungstenite::connect_async(url).await`
    //   2. `let (sink, stream) = ws.split();`
    //   3. `tokio::select! { Some(cmd) = rx.recv() => …, Some(msg) = stream.next() => … }`
    //   4. emit through the same `events::emit` sink path the sync
    //      backend uses, preserving the wire shape.
    let _ = runtime();
}
