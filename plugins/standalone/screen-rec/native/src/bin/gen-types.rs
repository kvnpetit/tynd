//! `gen-types` — Emits TypeScript declarations for every wire-shape
//! defined in `tynd_plugin_screen_rec::commands` to stdout.
//!
//! Usage (from a checkout root):
//! ```bash
//! cargo run -p tynd-plugin-screen-rec --bin tynd-plugin-screen-rec-gen-types --quiet > plugins/standalone/screen-rec/screen-rec.gen.ts
//! ```
//! [`tynd_plugin::gen_types_bin!`](tynd_plugin_sdk::tynd_plugin::gen_types_bin).

tynd_plugin_sdk::tynd_plugin::gen_types_bin!(tynd_plugin_screen_rec);
