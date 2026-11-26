//! `gen-types` — Emits TypeScript declarations for every wire-shape
//! defined in `tynd_plugin_terminal::commands` to stdout.
//!
//! Usage (from a checkout root):
//! ```bash
//! cargo run -p tynd-plugin-terminal --bin tynd-plugin-terminal-gen-types --quiet > plugins/standalone/terminal/terminal.gen.ts
//! ```
//! [`tynd_plugin::gen_types_bin!`](tynd_plugin_sdk::tynd_plugin::gen_types_bin).

tynd_plugin_sdk::tynd_plugin::gen_types_bin!(tynd_plugin_terminal);
