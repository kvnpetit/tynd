//! `tynd-host-core` — pure leaf utilities for the Tynd native host.
//!
//! This crate is the bottom of the host's compile graph: zero
//! platform-specific code, no feature gates, no transitive OS deps.
//! Re-builds here cascade outward but stay rare because the public
//! surface is small and stable.
//!
//! Scope (U12 phase 1):
//! - [`ipc_error`] — `IpcError` builder that produces the
//!   `<api>.<method>: <msg>` error string every dispatch arm returns.
//! - [`args_ext`] — `ArgsExt` trait with typed `serde_json::Value`
//!   field extraction.
//! - [`events`] — process-global OS event emitter (the
//!   `EventLoopProxy::emit` wrapper that background threads use to
//!   push events into the webview without holding the proxy).
//!
//! Future extractions (deferred to follow-up sprints — see
//! `OPTIMIZATION_BREAKING_OK.md` U12 anchor sites): `cleanup`, `signals`,
//! `os::registry`, `os::icon`. Those carry more transitive deps
//! (parking_lot, dashmap, ctrlc, image, tao, tray-icon) and a few
//! cross-cycles (cleanup ↔ plugins, signals → cleanup) that need
//! callback-indirection refactors before the move is clean.

pub mod args_ext;
pub mod events;
pub mod ipc_error;
