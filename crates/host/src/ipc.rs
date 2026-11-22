//! IPC plumbing between renderers, the backend and the host APIs: renderer
//! message dispatch, the worker pool that runs API calls, host-to-renderer
//! events, resource ownership per webview document and the typed IPC error.

pub(crate) mod call_pool;
pub(crate) mod dispatch;
pub(crate) mod error;
pub(crate) mod events;
pub(crate) mod owner;
