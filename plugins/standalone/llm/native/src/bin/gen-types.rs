//! `gen-types` — Emits TypeScript declarations for every wire-shape
//! defined in `tynd_plugin_llm::commands` to stdout.
//!
//! Usage (from a checkout root):
//! ```bash
//! cargo run -p tynd-plugin-llm --bin tynd-plugin-llm-gen-types --quiet > plugins/standalone/llm/llm.gen.ts
//! ```
//! [`tynd_plugin::gen_types_bin!`](tynd_plugin_sdk::tynd_plugin::gen_types_bin).

tynd_plugin_sdk::tynd_plugin::gen_types_bin!(tynd_plugin_llm);
