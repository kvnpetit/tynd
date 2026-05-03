//! Network interface enumeration.
//!
//! Backed by `if-addrs` — wraps `getifaddrs(3)` on Unix and
//! `GetAdaptersAddresses` on Windows. Returns one entry per interface
//! with all of its assigned IPs grouped together (the underlying call
//! lists one row per IP, so we collapse here for a friendlier shape).

use serde_json::{json, Value};

use crate::ipc_error::IpcError;

pub fn dispatch(method: &str, args: &Value) -> Result<Value, String> {
    let _ = args;
    match method {
        "interfaces" => interfaces(),
        _ => Err(IpcError::unknown_method("network", method).into_string()),
    }
}

fn interfaces() -> Result<Value, String> {
    let raw = if_addrs::get_if_addrs()
        .map_err(|e| IpcError::new("network", "interfaces", e).into_string())?;

    // Group by interface name. `if-addrs` 0.13 returns one entry per
    // (interface, IP) pair; the friendlier shape is one entry per
    // interface with an `ips: [...]` array. Order of first appearance is
    // preserved so the result is stable across calls on the same host.
    let mut order: Vec<String> = Vec::new();
    let mut grouped: ahash::HashMap<String, (Vec<String>, bool)> = ahash::HashMap::default();

    for iface in raw {
        let entry = grouped
            .entry(iface.name.clone())
            .or_insert_with(|| {
                order.push(iface.name.clone());
                (Vec::new(), false)
            });
        entry.0.push(iface.ip().to_string());
        if iface.is_loopback() {
            entry.1 = true;
        }
    }

    let arr: Vec<Value> = order
        .into_iter()
        .filter_map(|name| {
            let (ips, is_loopback) = grouped.remove(&name)?;
            // `if-addrs` 0.13 doesn't expose MAC or up/down state cross-platform.
            // Surface them as null so the JS shape stays stable; a future
            // crate bump (or a Win32 / netlink fast-path) can fill them in
            // without breaking consumers.
            Some(json!({
                "name": name,
                "ips": ips,
                "mac": Value::Null,
                "is_up": Value::Null,
                "is_loopback": is_loopback,
            }))
        })
        .collect();

    Ok(Value::Array(arr))
}
