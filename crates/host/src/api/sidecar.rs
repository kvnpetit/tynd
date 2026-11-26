//! Registry of sidecar binaries extracted from `TYNDPKG/sidecar/*` at
//! startup. `DashMap` for sharded lock-free reads on the call hot path.

use std::sync::LazyLock;

use dashmap::DashMap;
use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;

static SIDECARS: LazyLock<DashMap<String, String>> = LazyLock::new(DashMap::new);

pub(crate) fn register(name: &str, absolute_path: &str) {
    SIDECARS.insert(name.to_string(), absolute_path.to_string());
}

/// Dispatch marker — sidecar state lives in `SIDECARS`.
#[derive(Debug, Default)]
pub(crate) struct SidecarApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("sidecar", |method, args| dispatch(method, &args));

#[tynd::handler(api = "sidecar")]
impl SidecarApi {
    /// Resolve the absolute path of a previously-registered sidecar
    /// binary. Errors with `sidecar '<name>' is not registered` when no
    /// matching entry was extracted from the TYNDPKG bundle.
    pub fn path(name: String) -> Result<String, String> {
        SIDECARS.get(&name).map_or_else(
            || {
                Err(IpcError::new(
                    "sidecar",
                    "path",
                    format!("sidecar '{name}' is not registered"),
                )
                .into_string())
            },
            |r| Ok(r.value().clone()),
        )
    }

    /// List every registered sidecar as `{ name, path }`. Order is
    /// arbitrary (DashMap iteration order); callers that need a stable
    /// order should sort client-side.
    pub fn list() -> Value {
        let items: Vec<Value> = SIDECARS
            .iter()
            .map(|r| json!({ "name": r.key(), "path": r.value() }))
            .collect();
        Value::Array(items)
    }
}
