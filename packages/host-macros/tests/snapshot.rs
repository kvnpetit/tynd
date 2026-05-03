//! `insta` snapshot tests for `#[tynd::handler]`.
//!
//! The integration suite next to this file (`integration.rs`) checks
//! invariants against the generated dispatch — "missing required arg
//! errors with the uniform shape", "Option<T> defaults to None", etc.
//! Those assertions are stable against codegen drift but they don't
//! pin the **byte-for-byte** wire shape every per-arm response
//! produces.
//!
//! This file commits canonical wire snapshots for representative
//! handler shapes (plain return, Result return, unit return, missing
//! arg, unknown method, doc forwarder). Any cosmetic codegen drift
//! shows up as a snapshot diff in review — `cargo insta accept`
//! re-blesses the snapshot when the change is intentional, otherwise
//! it's an immediate red flag.
//!
//! Running locally:
//! - `cargo test --test snapshot` — checks; fails if the output drifts.
//! - `cargo insta review` (after `cargo install cargo-insta`) — diff +
//!   accept/reject loop for blessed changes.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tynd_host_macros as tynd;

mod ipc_error {
    use std::fmt::Display;

    #[derive(Debug)]
    pub struct IpcError(String);
    impl IpcError {
        pub fn new(api: &str, method: &str, msg: impl Display) -> Self {
            Self(format!("{api}.{method}: {msg}"))
        }
        pub fn missing_arg(api: &str, method: &str, arg: &str) -> Self {
            Self::new(api, method, format!("missing '{arg}' argument"))
        }
        pub fn unknown_method(api: &str, method: &str) -> Self {
            Self::new(api, method, "unknown method")
        }
    }
    impl From<IpcError> for String {
        fn from(e: IpcError) -> String {
            e.0
        }
    }
}

#[derive(Debug, Default)]
pub struct SnapApi;

#[derive(Debug, Serialize, Deserialize)]
pub struct EchoOpts {
    pub upper: bool,
}

#[tynd::handler(api = "snap")]
impl SnapApi {
    /// Add two numbers.
    pub fn add(a: i64, b: i64) -> Result<i64, String> {
        Ok(a + b)
    }

    /// Echo with optional opts.
    pub fn echo(text: String, opts: Option<EchoOpts>) -> String {
        if opts.map(|o| o.upper).unwrap_or(false) {
            text.to_uppercase()
        } else {
            text
        }
    }

    /// Returns nothing.
    pub fn ping() {}
}

/// Render the relevant wire-facing outputs into a single deterministic
/// string the snapshot can pin. Pre-formatting locally instead of
/// snapshotting `Result<Value, String>` directly keeps the snapshot
/// human-readable and robust to non-load-bearing reorderings.
fn render(method: &str, args: Value) -> String {
    match dispatch(method, &args) {
        Ok(v) => format!("OK   {method}({args}) => {v}"),
        Err(e) => format!("ERR  {method}({args}) => {e}"),
    }
}

#[test]
fn snapshot_dispatch_wire_shapes() {
    let mut out = String::new();
    out.push_str(&render("add", json!({ "a": 2, "b": 3 })));
    out.push('\n');
    out.push_str(&render("add", json!({ "a": 2 })));
    out.push('\n');
    out.push_str(&render("add", json!({ "a": 2, "b": "nope" })));
    out.push('\n');
    out.push_str(&render("echo", json!({ "text": "hi" })));
    out.push('\n');
    out.push_str(&render(
        "echo",
        json!({ "text": "hi", "opts": { "upper": true } }),
    ));
    out.push('\n');
    out.push_str(&render("ping", json!({})));
    out.push('\n');
    out.push_str(&render("nope", json!({})));
    insta::assert_snapshot!("dispatch_wire_shapes", out);
}

#[test]
fn snapshot_doc_forwarder_lines() {
    let mut out = String::new();
    for method in ["add", "echo", "ping", "missing"] {
        match __tynd_handler_doc(method) {
            Some(s) => out.push_str(&format!("{method}: {s:?}\n")),
            None => out.push_str(&format!("{method}: <none>\n")),
        }
    }
    insta::assert_snapshot!("doc_forwarder_lines", out);
}
