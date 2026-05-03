//! Proc-macros for the Tynd native host.
//!
//! The headline export is [`macro@handler`] — `#[tynd::handler]` collapses
//! the hand-rolled `dispatch(method, args)` boilerplate every `os/*`
//! module ships today (a `match method { ... }` ladder + per-arm
//! `args.get("k").and_then(...)` extraction + per-arm `serde_json::to_value`
//! return mapping) into one attribute on the API's `impl` block.
//!
//! ```ignore
//! use tynd_host_macros as tynd;
//!
//! pub struct ComputeApi;
//!
//! #[tynd::handler(api = "compute")]
//! impl ComputeApi {
//!     /// Generate `n` random bytes (base64-encoded).
//!     pub fn random_bytes(n: u64) -> Result<String, String> {
//!         // ... user code ...
//!         Ok(String::new())
//!     }
//! }
//! ```
//!
//! Effects:
//!
//! * The original `impl` block is preserved verbatim. Methods stay
//!   callable from Rust code under their existing names.
//! * A free-standing `pub fn dispatch(method: &str, args: &serde_json::Value)
//!   -> Result<serde_json::Value, String>` is emitted next to the
//!   `impl`. Each `pub fn` becomes a `match` arm:
//!   - Each parameter is deserialized via
//!     `serde_json::from_value` against the JSON object key matching
//!     the parameter ident (camelCased). `Option<T>` arguments default
//!     to `None` when absent; required arguments produce
//!     `IpcError::missing_arg(api, method, key)`.
//!   - The return value is converted: `Result<T, E>` → `to_value(t)`
//!     plus `e.into()` for the error, plain `T` → `to_value(t)`,
//!     `()` → `Value::Null`.
//! * Method names are translated to camelCase for the wire match arm
//!   (`read_text` → `"readText"`).
//! * `///` doc comments on each handler method are captured into a
//!   `pub fn __tynd_handler_doc(method: &str) -> Option<&'static str>`
//!   sibling fn — used by future `gen-types` integration to forward
//!   JSDoc into the `.gen.ts` output without re-parsing the source.
//!
//! Methods may take `&self` or no receiver. `&mut self` is rejected
//! (the dispatch entrypoint is `&Value` → `Value`, with no caller-side
//! handle to a mutable instance). Use a `LazyLock<Mutex<…>>` inside
//! the body for state.
//!
//! Skip the auto-rewrite for an internal helper with `#[tynd::skip]`:
//!
//! ```ignore
//! #[tynd::handler(api = "fs")]
//! impl FsApi {
//!     pub fn read(&self, path: String) -> Result<String, String> { Ok(path) }
//!
//!     #[tynd::skip]
//!     fn internal_helper(&self) { /* not exposed via dispatch */ }
//! }
//! ```

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemImpl};

mod analyze;
mod codegen;

/// `#[tynd::handler(api = "<name>")]` — see crate-level docs.
///
/// The `api` argument is the wire-format API name used by error
/// messages (`<api>.<method>: ...` shape, matches `IpcError::new`).
/// Required: omitting it yields a friendly compile error pointing at
/// the call site so the dispatcher can't accidentally emit unqualified
/// errors that would break the host's uniform error format.
#[proc_macro_attribute]
pub fn handler(args: TokenStream, input: TokenStream) -> TokenStream {
    let attr_args = proc_macro2::TokenStream::from(args);
    let api_name = match analyze::parse_api_arg(attr_args) {
        Ok(name) => name,
        Err(e) => return e.to_compile_error().into(),
    };

    let item = parse_macro_input!(input as ItemImpl);
    match codegen::expand(&api_name, item) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

/// `#[tynd::skip]` — opt-out marker consumed by [`macro@handler`].
///
/// Defined here so the path resolves at parse time; the `handler` macro
/// strips the attribute before re-emitting the `impl` block, so this
/// expansion never actually runs in well-formed code. The empty
/// passthrough is a defensive fallback for the case where a user puts
/// `#[tynd::skip]` outside a `#[handler]` block.
#[proc_macro_attribute]
pub fn skip(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

/// Internal smoke-test helper — not part of the public API. Re-exports
/// the `quote!`-based code path so the integration suite can call into
/// the same pipeline the proc-macro entrypoint uses without paying the
/// rustc proc-macro harness round-trip.
#[doc(hidden)]
#[proc_macro]
pub fn __tynd_host_macros_marker(_input: TokenStream) -> TokenStream {
    quote! { const _: () = (); }.into()
}
