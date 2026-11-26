//! Network interface enumeration via `if-addrs` (`getifaddrs` /
//! `GetAdaptersAddresses`). Collapses the per-IP rows into one entry per
//! interface for a friendlier shape.

use serde_json::{json, Value};

use crate::api::HostApi;
use crate::ipc::error::IpcError;
use crate::tynd;

/// Stateless dispatch marker.
#[derive(Debug, Default)]
pub(crate) struct NetworkApi;

/// Row of the host API table (`api::HOST_APIS`).
pub(crate) const API: HostApi = HostApi::new("network", |method, args| dispatch(method, &args));

#[tynd::handler(api = "network")]
impl NetworkApi {
    /// Enumerate every network interface on the host. Each entry
    /// groups one interface's assigned IPs together — the underlying
    /// `if-addrs` API returns one row per (interface, IP) pair, so we
    /// collapse here for a friendlier wire shape.
    pub fn interfaces() -> Result<Value, String> {
        interfaces_impl()
    }
}

/// Internal bind-validation endpoints. `network-loopback` and `network-public`
/// are capability names of the `network` API, not catalog APIs: a backend
/// plugin asks the host to validate a bind address under the matching
/// permission before it opens a listener itself.
pub(crate) const LOOPBACK_API: HostApi = HostApi::new("network-loopback", |method, args| {
    dispatch_loopback(method, &args)
});
/// See [`LOOPBACK_API`].
pub(crate) const PUBLIC_API: HostApi = HostApi::new("network-public", |method, args| {
    dispatch_public(method, &args)
});

fn dispatch_loopback(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "bind" => {
            validate_port(args.get("port"))?;
            Ok(json!({ "ok": true }))
        },
        _ => Err(format!("Unknown network-loopback method: '{method}'")),
    }
}

fn dispatch_public(method: &str, args: &Value) -> Result<Value, String> {
    match method {
        "bind" => {
            let host = args.get("host").and_then(Value::as_str).ok_or_else(|| {
                IpcError::new("network-public", "bind", "missing 'host'").into_string()
            })?;
            if is_loopback_host(host) {
                return Err(IpcError::new(
                    "network-public",
                    "bind",
                    format!("host '{host}' is loopback; use network-loopback.bind"),
                )
                .into_string());
            }
            validate_port(args.get("port"))?;
            Ok(json!({ "ok": true }))
        },
        _ => Err(format!("Unknown network-public method: '{method}'")),
    }
}

fn validate_port(value: Option<&Value>) -> Result<(), String> {
    let Some(port) = value.and_then(Value::as_u64) else {
        return Ok(());
    };
    if u16::try_from(port).is_ok() {
        Ok(())
    } else {
        Err(IpcError::new("network", "bind", format!("port out of range: {port}")).into_string())
    }
}

fn is_loopback_host(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
    h == "localhost" || h == "::1" || h.starts_with("127.")
}

fn interfaces_impl() -> Result<Value, String> {
    let raw = if_addrs::get_if_addrs()
        .map_err(|e| IpcError::new("network", "interfaces", e).into_string())?;

    let rows = raw.into_iter().map(|iface| {
        InterfaceRow::new(
            iface.name.clone(),
            iface.ip().to_string(),
            iface.is_loopback(),
        )
    });
    Ok(group_interface_rows(rows))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterfaceRow {
    name: String,
    ip: String,
    is_loopback: bool,
}

impl InterfaceRow {
    fn new(name: impl Into<String>, ip: impl Into<String>, is_loopback: bool) -> Self {
        Self {
            name: name.into(),
            ip: ip.into(),
            is_loopback,
        }
    }
}

fn group_interface_rows(rows: impl IntoIterator<Item = InterfaceRow>) -> Value {
    // Group by interface name. `if-addrs` 0.13 returns one entry per
    // (interface, IP) pair; the friendlier shape is one entry per
    // interface with an `ips: [...]` array. Order of first appearance is
    // preserved so the result is stable across calls on the same host.
    let mut order: Vec<String> = Vec::new();
    let mut grouped: ahash::HashMap<String, (Vec<String>, bool)> = ahash::HashMap::default();

    for iface in rows {
        let entry = grouped.entry(iface.name.clone()).or_insert_with(|| {
            order.push(iface.name.clone());
            (Vec::new(), false)
        });
        entry.0.push(iface.ip);
        if iface.is_loopback {
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

    Value::Array(arr)
}
