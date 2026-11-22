//! `gen-types` — Emits TypeScript declarations for every wire-shape
//! defined in `tynd_plugin_audio::commands` to stdout.
//!
//! Usage (from a checkout root):
//! ```bash
//! cargo run -p tynd-plugin-audio --bin tynd-plugin-audio-gen-types --quiet > plugins/standalone/audio/audio.gen.ts
//! ```
//! [`tynd_plugin::gen_types_bin!`](tynd_plugin_sdk::tynd_plugin::gen_types_bin).

tynd_plugin_sdk::tynd_plugin::gen_types_bin!(tynd_plugin_audio);
