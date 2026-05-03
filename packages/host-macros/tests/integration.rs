//! End-to-end behavior tests for `#[tynd::handler]`.
//!
//! These exercise the macro's runtime contract — the generated
//! `dispatch(method, args)` function must:
//!
//! 1. Route every `pub fn` arm to the right `Self::method(...)` call.
//! 2. Per-arg deserialize from the JSON object value, with the
//!    expected `<api>.<method>: missing '<key>' argument` error
//!    for absent required args.
//! 3. Tolerate absent fields when the parameter type is `Option<T>`,
//!    binding them to `None`.
//! 4. Map plain `T`, `()`, and `Result<T, E>` returns to the right
//!    `Result<Value, String>` shape.
//! 5. Surface `///` doc comments via the sibling
//!    `__tynd_handler_doc(method)` fn so the gen-types pipeline can
//!    forward them to JSDoc.
//!
//! The host crate (`tynd-host`) ships a real `crate::ipc_error::IpcError`
//! that the macro's generated code reaches through the `crate::` path.
//! We replicate the minimum surface here in `crate::ipc_error` so the
//! macro's expansion compiles in isolation.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tynd_host_macros as tynd;

/// Stand-in for `tynd_host::ipc_error::IpcError` so the macro's
/// generated code resolves `crate::ipc_error::IpcError::*` against
/// something with the same shape. The host's real impl lives in
/// `host-rs/src/ipc_error.rs` and tests there cover the inner
/// formatting; we just need a compatible type signature.
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
pub struct DemoApi;

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EchoOpts {
    pub upper: bool,
}

#[tynd::handler(api = "demo")]
impl DemoApi {
    /// Add two numbers — exercises required-arg extraction + `Result`
    /// return mapping.
    pub fn add(a: i64, b: i64) -> Result<i64, String> {
        Ok(a + b)
    }

    /// Echo `text`, optionally upper-casing it. Tests `Option<T>`
    /// args (default to `None` when absent) plus a struct deser.
    pub fn echo(text: String, opts: Option<EchoOpts>) -> String {
        if opts.map(|o| o.upper).unwrap_or(false) {
            text.to_uppercase()
        } else {
            text
        }
    }

    /// Returns `()` — must surface as `Value::Null` on the wire.
    pub fn ping() {}

    /// Internal helper marked with `#[tynd::skip]` — must NOT appear
    /// in the dispatch table even though it's `pub`. Demonstrates
    /// the opt-out path for `impl`-private utilities that happen to
    /// share visibility with the wire methods.
    #[tynd::skip]
    pub fn helper() -> i32 {
        42
    }
}

#[test]
fn dispatch_routes_required_args() {
    let out = dispatch("add", &json!({ "a": 2, "b": 3 })).unwrap();
    assert_eq!(out, json!(5));
}

#[test]
fn dispatch_missing_required_arg_errors_with_uniform_shape() {
    let err = dispatch("add", &json!({ "a": 2 })).unwrap_err();
    assert!(
        err.contains("demo.add"),
        "expected `<api>.<method>:` prefix, got {err:?}"
    );
    assert!(err.contains("missing 'b'"), "got {err:?}");
}

#[test]
fn dispatch_optional_arg_defaults_to_none_when_absent() {
    let out = dispatch("echo", &json!({ "text": "hi" })).unwrap();
    assert_eq!(out, json!("hi"));
}

#[test]
fn dispatch_optional_arg_deserialized_when_present() {
    let out = dispatch("echo", &json!({ "text": "hi", "opts": { "upper": true } })).unwrap();
    assert_eq!(out, json!("HI"));
}

#[test]
fn dispatch_unit_return_is_value_null() {
    let out = dispatch("ping", &json!({})).unwrap();
    assert_eq!(out, Value::Null);
}

#[test]
fn dispatch_unknown_method_errors_with_uniform_shape() {
    let err = dispatch("nope", &json!({})).unwrap_err();
    assert!(err.contains("demo.nope"));
    assert!(err.contains("unknown method"));
}

#[test]
fn dispatch_skips_method_marked_with_skip() {
    // `helper` should NOT be reachable via dispatch — it carries
    // `#[tynd::skip]`. The unknown-method path is the canonical "this
    // method isn't in the table" signal.
    let err = dispatch("helper", &json!({})).unwrap_err();
    assert!(err.contains("unknown method"));
}

#[test]
fn helper_method_remains_callable_directly() {
    // `#[tynd::skip]` must not affect the original `impl` — the
    // method stays callable from Rust under its own name. The macro
    // strips the skip marker but otherwise leaves the body alone.
    assert_eq!(DemoApi::helper(), 42);
}

#[test]
fn doc_forwarder_exposes_jsdoc_lines() {
    let doc = __tynd_handler_doc("add").expect("add has docs");
    assert!(doc.contains("Add two numbers"));
    let doc = __tynd_handler_doc("ping").expect("ping has docs");
    assert!(doc.contains("Returns"));
    // Methods that exist but have no doc comments resolve to None.
    // (None of our test handlers omit docs, so this asserts the
    // unknown-method fallthrough on the doc fn returns None too.)
    assert!(__tynd_handler_doc("not_a_method").is_none());
}

#[test]
fn argument_with_wrong_shape_surfaces_serde_error() {
    // `b` should be an integer but we send a string — the macro
    // routes the failure through `IpcError::new(api, method, ...)`
    // with the same `<api>.<method>: invalid '<key>': <serde>` shape
    // every dispatch arm used to hand-format.
    let err = dispatch("add", &json!({ "a": 2, "b": "not a number" })).unwrap_err();
    assert!(err.contains("demo.add"), "got {err:?}");
    assert!(err.contains("invalid 'b'"), "got {err:?}");
}
