//! Backward-compat re-export — the OS event emitter lives in the
//! `tynd-host-core` leaf crate (U12 phase 1 split). Background threads
//! reach this through the `crate::os::events` path that all `os/*`
//! modules already use; the underlying types and `set_emitter` /
//! `emit` symbols come from `tynd_host_core::events`.

pub use tynd_host_core::events::*;
