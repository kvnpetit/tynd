/**
 * Build entrypoint for the bundled devalue snapshot embedded in the
 * webview shim. Run via `bun run build:devalue` (writes to crates/host/src/webview/shim)
 * to regenerate `src/devalue.bundle.js`. The Rust shim `include_str!`s
 * that file, so this build step only runs when devalue is upgraded.
 * Exposes `parse` and `stringify` on `window.__tynd_dv__` — the names
 * are short to keep the shim's eval payload compact when value/yield
 * frames stream through the webview.
 */
import { parse, stringify } from "devalue";

(globalThis as { __tynd_dv__?: unknown }).__tynd_dv__ = { parse, stringify };
