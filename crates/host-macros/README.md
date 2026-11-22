# tynd-host-macros

Procedural macros for the Tynd native host — `#[tynd::handler]` collapses OS API dispatch boilerplate into a single attribute.

## Overview

The host's OS API modules (`os/fs`, `os/http`, etc.) today hand-roll dispatch boilerplate: a `match method { ... }` ladder, per-arm `args.get("k").and_then(...)` extraction, per-arm `serde_json::to_value` return mapping. `#[tynd::handler]` eliminates this by expanding the original `impl` block into:

1. The original `impl` block, preserved verbatim (methods callable from Rust code under their existing names).
2. A free-standing `pub(crate) fn dispatch(method: &str, args: &serde_json::Value) -> Result<serde_json::Value, String>` that routes incoming IPC calls.

## Architecture

### `#[tynd::handler(api = "<name>")]` attribute

Applied to an `impl` block:

```rust
#[tynd::handler(api = "fs")]
impl FsApi {
    pub fn read_text(&self, path: String) -> Result<String, String> {
        // user code unchanged
        Ok(content)
    }
}
```

Expands into:

- **Original `impl` block** — unchanged, so `FsApi::read_text(...)` works from Rust code.
- **Generated `dispatch` function** — matches on wire method names (camelCase), deserializes JSON args by parameter name, calls the handler, serializes the result back to JSON.

Per-arm deserialization:
- `Option<T>` arguments default to `None` when absent.
- Required arguments produce `IpcError::missing_arg(api, method, key)` when missing.
- Type mismatches surface through `IpcError::new(api, method, "invalid '<key>': …")`.

Return value handling:
- `Result<T, E>` → `Ok(to_value(t))` or `Err(e.into())`.
- Plain `T` → `Ok(to_value(t))`.
- `()` → `Ok(Value::Null)`.

### Method name translation

Snake_case methods map to camelCase wire names: `read_text` ↔ `"readText"`. Matches the JavaScript API surface.

### Doc comment capture

Doc comments (`///`) on each handler method are captured into a sibling function:

```rust
pub fn __tynd_handler_doc(method: &str) -> Option<&'static str>
```

Forwarded to `gen-types` for JSDoc generation without re-parsing the source.

### `#[tynd::skip]` opt-out

Decorate individual methods to exclude them from the dispatch rewrite:

```rust
#[tynd::handler(api = "fs")]
impl FsApi {
    pub fn read(&self, path: String) -> Result<String, String> { Ok(path) }

    #[tynd::skip]
    fn internal_helper(&self) { /* not exposed via dispatch */ }
}
```

The helper stays callable from Rust but doesn't appear in the dispatch table.

### Receiver constraints

Methods may take `&self` or no receiver. Owned/mutable receivers, async, unsafe, extern, variadic, const, and generic methods are rejected with a diagnostic at the handler. Use `LazyLock<Mutex<…>>` inside the body for mutable state.

## Use

Every host API module (`src/api/`) applies this macro to its API struct:

```rust
use tynd_host_macros as tynd;

pub struct FileSystemApi;

#[tynd::handler(api = "fs")]
impl FileSystemApi {
    pub fn read_text(&self, path: String) -> Result<String, String> { /* … */ }
    pub fn write_text(&self, path: String, content: String) -> Result<(), String> { /* … */ }
}
```

The expanded `dispatch` function is wired into the host's main RPC router so all incoming `fs.*` calls are dispatched here.

## Validation

Run `cargo check -p tynd-host-macros` after changing macro parsing or code generation.
## Dependencies

- `syn` (2.x, `full` + `extra-traits`) — AST parsing.
- `quote` (1.x) — code generation.
- `proc-macro2` (1.x) — token stream manipulation.


## Compiler metadata

Declared as `proc-macro = true` in `Cargo.toml` so rustc's plugin harness loads it as a macro library, not a regular crate.
