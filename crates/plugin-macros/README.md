# `tynd-plugin-macros`

Proc-macros backing `#[tynd_plugin::commands]` for Tynd native plugins.
You almost never depend on this crate directly — the macros are
re-exported through `tynd-plugin-sdk` as `tynd_plugin`:

```rust
use tynd_plugin_sdk::tynd_plugin;

/// Doc forwarded to the TS module-level doc.
#[tynd_plugin::commands]
pub mod commands {
    /// Args for greet.
    pub struct GreetArgs {
        /// User name.
        pub name: String,
        /// Optional formality. Defaults to `false`.
        pub formal: Option<bool>,
    }

    /// Internal helper — not exported to TS.
    #[tynd_plugin::skip]
    pub struct InternalState { pub counter: u32 }

    /// Stream event.
    pub enum ProgressEvent {
        Started,
        Progress { percent: f32 },
        Done {
            /// Total duration in milliseconds.
            duration_ms: u64,
        },
    }
}
```

What `#[commands]` does:

- Adds `#[derive(Serialize, Deserialize, specta::Type)]` plus
  `#[specta(crate = ::tynd_plugin_sdk::specta)]` to every `pub struct`
  / `pub enum` (skipped if you already wrote them). The `crate = ...`
  helper points the specta derive at the SDK re-export so plugin
  authors don't need a direct `specta` dep.
- Annotates `u64` / `i64` (and `Option<u64>`, `Vec<u64>`, …) fields with
  `#[specta(type = f64)]` so the generated TS lands as `number` rather
  than `bigint`. User-supplied `#[specta(type = …)]` always wins.
- Emits `__tynd_export_command_types_<modname>() -> String` next to
  the module — call it from a `gen-types` bin. The fn ident carries
  the source-module name as a suffix so a single crate can host
  multiple `#[commands]`-annotated modules (`pub mod foo` + `pub mod
  bar`) without colliding on a duplicate symbol at link time.
  Distinct base name from `#[plugin_impl]`'s `__tynd_export_types` so
  the same crate can use both macros without a duplicate-symbol
  compile error.
- Forwards Rust `///` doc comments to the generated TS as JSDoc.

What `#[skip]` does:

- On a struct/enum: omit from auto-derive **and** from the TS export.
- On a field: skip the `u64 → f64` rewrite (use this if you genuinely
  want `bigint` on the TS side and ship a custom Typescript exporter).

`gen-types` bin for a `#[commands]`-only crate (no `#[plugin_impl]`).
The suffix on the helper matches the module name — `pub mod commands`
emits `__tynd_export_command_types_commands`:

```rust
// src/bin/gen-types.rs
fn main() {
    print!("{}", plugin_my_plugin::__tynd_export_command_types_commands());
}
```

`gen_types_bin!(plugin_my_plugin)` is for crates that also use
`#[plugin_impl]`: it emits a `main` calling `__tynd_export_types`,
which `#[plugin_impl]` produces (and which already covers the
`#[commands]` types). Two routes:

- `#[plugin_impl]` users → `tynd_plugin_sdk::tynd_plugin::gen_types_bin!(plugin_my_plugin);`
- `#[commands]`-only users → manual `print!("{}", crate::__tynd_export_command_types_<modname>());`

## Visibility rules

Every non-private item is exported to TS: `pub`, `pub(crate)`,
`pub(super)`, and `pub(in some::path)` all participate in the
`__tynd_export_command_types_<modname>` surface and command-pair detection.

This matches where the generated sibling export function can legally
reach from Rust. If you want a helper kept Rust-side only, make it
private or mark it with `#[tynd_plugin::skip]`.

## Limitations

- `#[plugin_impl]` generates a full `impl TyndPlugin for <Self>` block
  filling `name` and `dispatch` only; `init`, `shutdown`, and
  `dispatch_rkyv` keep their trait defaults. Plugin authors **cannot**
  override those by writing a second `impl TyndPlugin for <Self>`
  block — rustc rejects the duplicate impl with `E0119`.

  If a plugin needs `init` (background-thread spawn, eager handle
  table warm-up) or `shutdown` (flush DB, close sockets) or a
  hand-rolled `dispatch_rkyv`, drop `#[plugin_impl]` and write the
  `impl TyndPlugin` block manually. The `#[tynd_plugin::commands]`
  macro on the args/result module covers the type-export side
  independently — both macros compose, but `#[plugin_impl]` is
  all-or-nothing on the trait impl.

## Compile-error ergonomics

- Putting `#[commands]` on something that isn't a `mod`:
  `error: #[tynd_plugin::commands] expected mod — the macro must be placed on a module declaration like pub mod commands { ... }`.
- Unknown `#[plugin_command(...)]` flags fail compilation pointing at
  the offending ident:
  `error: unknown #[plugin_command(...)] flag \`foo\` (recognised: \`zero_copy\`)`.

## Validation

```bash
cargo check --manifest-path crates/plugin-macros/Cargo.toml
```