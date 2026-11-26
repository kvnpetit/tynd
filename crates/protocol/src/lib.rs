//! Pure protocol types shared by Tynd host, runtimes, and embedded engines.
//!
//! This crate deliberately avoids `wry`, `tao`, and OS APIs. It owns the
//! serializable contracts that cross the host/runtime boundary.

#![forbid(unsafe_code)]

pub mod runtime;
pub mod window;

/// Hard cap, in bytes, on one JSON IPC frame: a renderer call body received by
/// the host and a newline-delimited line on the Bun stdin/stdout wire. Both
/// ends share this constant: a renderer call the host accepted but the runtime
/// then rejected would leave the frontend Promise unresolved. Large binary
/// payloads use `tynd-bin://` instead.
pub const MAX_IPC_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[cfg(test)]
mod tests;
