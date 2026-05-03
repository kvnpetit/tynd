//! Backward-compat re-export — the actual `IpcError` builder lives in
//! the `tynd-host-core` leaf crate (U12 phase 1 split).
//!
//! Existing `use crate::ipc_error::IpcError` / `use tynd_host::ipc_error::IpcError`
//! call sites keep working through this re-export; new code in any host
//! crate should reach for `tynd_host_core::ipc_error::IpcError` directly.

pub use tynd_host_core::ipc_error::*;
