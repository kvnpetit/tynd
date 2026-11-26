//! Proc-macros for Tynd native plugins.
//!
//! Reach the attributes through the `tynd_plugin_sdk::tynd_plugin`
//! re-export so the user only depends on a single SDK crate:
//!
//! ```ignore
//! use tynd_plugin_sdk::tynd_plugin;
//!
//! /// Doc forwarded to the TypeScript module-level doc.
//! #[tynd_plugin::commands]
//! pub mod commands {
//!     /// Args for greet.
//!     pub struct GreetArgs { pub name: String }
//!
//!     /// Internal — not exported.
//!     #[tynd_plugin::skip]
//!     pub struct Internal { pub counter: u32 }
//! }
//! ```
//!
//! Effects of `#[commands]`:
//!
//! * Every `pub struct` / `pub enum` (sans `#[skip]`) gains `#[derive(Serialize, Deserialize,
//!   specta::Type)]` when missing.
//! * Each `u64` / `i64` / `Option<u64>` / `Vec<u64>` field is annotated `#[specta(type = f64)]` so
//!   the generated TS lands as `number` instead of `bigint`. User-supplied `#[specta(type = …)]`
//!   wins.
//! * A sibling `__tynd_export_command_types_<modname>() -> String` fn is emitted next to the module
//!   — call from a `gen-types` bin. The module-name suffix lets a single crate host multiple
//!   `#[commands]` modules without colliding on the symbol. Distinct base name from
//!   `#[plugin_impl]`'s `__tynd_export_types` so the same crate can use both macros without a
//!   duplicate-symbol compile error.
//!
//! Effects of `#[skip]`:
//!
//! * On a struct/enum: skip auto-derive + registration.
//! * On a field: skip the `u64` → `f64` rewrite (use when you genuinely want `bigint` on the TS
//!   side).
//!
//! Effects of `#[plugin_command(...)]`:
//!
//! * `zero_copy`: also derive `rkyv::Archive` / `rkyv::Serialize` / `rkyv::Deserialize`, reaching
//!   rkyv through the `tynd_plugin_sdk::rkyv::*` re-export. Pair with a hand-rolled
//!   `TyndPlugin::dispatch_rkyv` arm to opt the method into the zero-copy wire path.

#![forbid(unsafe_code)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Item};

mod analyze;
mod codegen;
mod plugin_attr;
mod plugin_impl;
mod util;

/// `#[tynd_plugin::commands]` — see crate-level docs.
///
/// Must be placed on a module (`pub mod commands { ... }`); applying
/// elsewhere yields a friendly compile error.
#[proc_macro_attribute]
pub fn commands(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        let err = syn::Error::new(
            proc_macro2::TokenStream::from(args)
                .into_iter()
                .next()
                .map_or_else(proc_macro2::Span::call_site, |t| t.span()),
            "#[tynd_plugin::commands] does not accept arguments",
        );
        return err.to_compile_error().into();
    }

    let item: Item = parse_macro_input!(input as Item);
    match item {
        Item::Mod(module) => codegen::expand(module)
            .unwrap_or_else(|e| e.to_compile_error())
            .into(),
        other => codegen::err_expected_mod(&other).to_compile_error().into(),
    }
}

/// `#[tynd_plugin::skip]` — opt-out marker consumed by
/// [`macro@commands`]. The `commands` macro strips the attribute before
/// re-emitting, so this passthrough only fires on the misuse path
/// (skip applied outside a `#[commands]` module).
#[proc_macro_attribute]
pub fn skip(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

/// `#[tynd_plugin::plugin_command(...)]` — per-item knobs consumed by
/// [`macro@commands`]. Today the only flag is `zero_copy`, which adds
/// rkyv `Archive` / `Serialize` / `Deserialize` derives alongside the
/// serde + specta trio. The marker is stripped before re-emitting; this
/// passthrough only fires on the misuse path.
///
/// ```ignore
/// #[tynd_plugin::commands]
/// pub mod commands {
///     #[tynd_plugin::plugin_command(zero_copy)]
///     pub struct ProcessArgs {
///         pub payload: Vec<u8>,
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn plugin_command(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

/// `#[tynd_plugin::plugin(name = "sql")]` — single-attr plugin
/// declaration.
///
/// Place on the plugin's main struct. Generates `pub fn __tynd_plugin()
/// -> Arc<dyn TyndPlugin>` returning a `Self::default()` instance —
/// equivalent to writing `register_plugin!(MyPlugin::default())` by hand.
///
/// **Mutually exclusive with `register_plugin!` — choose one.** Combining
/// both produces a duplicate-symbol compile error because the two routes
/// emit the same `__tynd_plugin` constructor.
///
/// The `name` arg is the wire-protocol identifier the host dispatches
/// against (`os_call("plugin", "sql:open", …)` → `name = "sql"`). Author
/// must still impl `Default` (or derive it) — this macro doesn't try to
/// guess field defaults.
///
/// ```ignore
/// #[tynd_plugin::plugin(name = "sql")]
/// pub struct SqlPlugin {
///     connections: Registry<Connection>,
/// }
///
/// // `Registry<T>::new(max)` takes the hard-cap explicitly, so the
/// // struct can't `#[derive(Default)]` — write the impl by hand.
/// impl Default for SqlPlugin {
///     fn default() -> Self {
///         Self { connections: Registry::new(64) }
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn plugin(args: TokenStream, input: TokenStream) -> TokenStream {
    plugin_attr::expand_plugin_attr(args.into(), input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// `#[tynd_plugin::plugin_impl]` — auto-dispatch from typed handlers.
///
/// Place on a `impl SqlPlugin { … }` block where each method tagged
/// `#[command]` becomes a wire-callable RPC handler. The macro generates:
///
/// 1. A `<MethodName>Args` struct from the method's params (excluding `&self` and an optional `cx:
///    &PluginCx` first arg). Single-param methods get an `{ <param>: T }` shape; multi-param wraps
///    all fields. `0`-param methods get a unit struct (`pub struct <MethodName>Args;`) — empty wire
///    blob still validates against it.
/// 2. A `<MethodName>Result` type alias = the method's return type (with `PluginResult<T>` /
///    `Result<T, _>` unwrapped to `T`).
/// 3. `impl TyndPlugin for SqlPlugin` with a `dispatch` arm per command, deserialising args +
///    serialising the return Value.
/// 4. `__tynd_export_types()` returning the TS bindings + catalog.
///
/// Reserved untagged methods
/// `plugin_init(&self, &PluginCx) -> PluginResult<()>` and
/// `plugin_shutdown(&self, &PluginCx) -> PluginResult<()>` are forwarded to
/// the matching native lifecycle hooks. They are not renderer commands.
///
/// ```ignore
/// #[tynd_plugin::plugin_impl]
/// impl SqlPlugin {
///     /// Open a SQLite database.
///     #[command]
///     pub fn open(&self, path: String) -> PluginResult<u64> { ... }
///
///     #[command]
///     pub async fn exec(&self, id: u64, sql: String) -> PluginResult<u64> { ... }
///
///     pub fn plugin_shutdown(&self, _cx: &PluginCx) -> PluginResult<()> {
///         self.close_all()
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn plugin_impl(args: TokenStream, input: TokenStream) -> TokenStream {
    if !args.is_empty() {
        let err = syn::Error::new(
            proc_macro2::TokenStream::from(args)
                .into_iter()
                .next()
                .map_or_else(proc_macro2::Span::call_site, |t| t.span()),
            "#[tynd_plugin::plugin_impl] does not accept arguments",
        );
        return err.to_compile_error().into();
    }
    plugin_impl::expand_plugin_impl(proc_macro2::TokenStream::new(), input.into())
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

/// `#[tynd_plugin::command]` — marker consumed by [`plugin_impl`].
///
/// Tags a method inside a `#[plugin_impl] impl` block as a wire-callable
/// handler. The marker itself emits no code; the parent `plugin_impl`
/// macro reads + strips it during expansion. Applying `#[command]`
/// outside a `#[plugin_impl]` block is a no-op.
#[proc_macro_attribute]
pub fn command(_args: TokenStream, input: TokenStream) -> TokenStream {
    input
}

/// `tynd_plugin::gen_types_bin!(my_plugin_crate)` — emit the entire
/// `gen-types` bin in one line. Plugin authors invoke this from
/// `src/bin/gen-types.rs`; the bin prints the generated TS to stdout.
///
/// Plugin authors only carry a single `tynd_plugin_sdk` dep, so reach
/// for the macro through the SDK re-export path:
///
/// ```ignore
/// // src/bin/gen-types.rs
/// tynd_plugin_sdk::tynd_plugin::gen_types_bin!(plugin_my_plugin);
/// ```
///
/// `$crate_path` is the library crate hosting the
/// `#[tynd_plugin::plugin_impl]` block. Redirect stdout into the
/// plugin's `*.gen.ts` file:
///
/// ```bash
/// cargo run -p plugin_my_plugin --bin gen-types --quiet > my-plugin.gen.ts
/// ```
///
/// Resolves to `<crate>::__tynd_export_types()` — the symbol that
/// `#[plugin_impl]` emits. For `#[tynd_plugin::commands]`-only crates
/// (no `#[plugin_impl]` block), call
/// `__tynd_export_command_types_<modname>()` directly instead of
/// using this macro.
#[proc_macro]
pub fn gen_types_bin(input: TokenStream) -> TokenStream {
    let crate_path = proc_macro2::TokenStream::from(input);
    let expanded = quote! {
        fn main() {
            let s = #crate_path :: __tynd_export_types();
            // print! — Typescript::export already emits the trailing newline.
            ::std::print!("{s}");
        }
    };
    expanded.into()
}
