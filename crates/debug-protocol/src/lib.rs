//! Reusable debug protocol pieces shared by Tynd host runtimes.
//!
//! Host-specific RPC methods live outside this crate. This package owns the
//! wire format, framing, transport loop, and debug taps.

pub mod ipc_tap;
pub mod log_tap;
pub mod protocol;
pub mod server;

pub use ipc_tap::IpcFrame;
pub use log_tap::DebugLogLayer;
pub use server::{CallOutcome, Endpoint, MethodRouter, SubscriptionHandle};

#[cfg(test)]
mod tests;
