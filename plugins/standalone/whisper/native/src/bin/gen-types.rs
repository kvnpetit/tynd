//! `gen-types` — Emits TypeScript declarations for every wire-shape
//! defined in `tynd_plugin_whisper::commands` to stdout.
//!
//! Usage (from a checkout root):
//! ```bash
//! cargo run -p tynd-plugin-whisper --bin tynd-plugin-whisper-gen-types --quiet > plugins/standalone/whisper/whisper.gen.ts
//! ```
//! [`tynd_plugin::gen_types_bin!`](tynd_plugin_sdk::tynd_plugin::gen_types_bin).

tynd_plugin_sdk::tynd_plugin::gen_types_bin!(tynd_plugin_whisper);
