//! `gen-types` — Emits TypeScript declarations for every wire-shape
//! defined in `tynd_plugin_wgpu::commands` to stdout.
//!
//! Usage (from a checkout root):
//! ```bash
//! cargo run -p tynd-plugin-wgpu --bin tynd-plugin-wgpu-gen-types --quiet > plugins/standalone/wgpu/wgpu.gen.ts
//! ```
//!
//! `SubmitArgs` is intentionally omitted — it carries `#[tynd_plugin::skip]`
//! because its `commands` field is typed as `serde_json::Value` (free-form
//! JSON) until the `bridge.rs::RenderCommand` enum stabilises behind the
//! `native` feature, and `serde_json::Value` does not implement
//! `specta::Type`.

tynd_plugin_sdk::tynd_plugin::gen_types_bin!(tynd_plugin_wgpu);
