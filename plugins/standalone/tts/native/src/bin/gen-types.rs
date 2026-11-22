//! `gen-types` — Emits TypeScript declarations for every wire-shape
//! defined in `tynd_plugin_tts::commands` to stdout.
//!
//! Usage (from a checkout root):
//! ```bash
//! cargo run -p tynd-plugin-tts --bin tynd-plugin-tts-gen-types --quiet > plugins/standalone/tts/tts.gen.ts
//! ```
//! [`tynd_plugin::gen_types_bin!`](tynd_plugin_sdk::tynd_plugin::gen_types_bin).
//! The macro expands to a `fn main()` that prints
//! `tynd_plugin_tts::__tynd_export_types()` (itself emitted by
//! `#[tynd_plugin::commands]` next to the `commands` module) to stdout.

tynd_plugin_sdk::tynd_plugin::gen_types_bin!(tynd_plugin_tts);
