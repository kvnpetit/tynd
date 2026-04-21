import { fireClose, fireReady, onCloseFns, onReadyFns, setEmitFn } from "../lifecycle.ts"
import { tynd } from "../logger.js"
import type { AppConfig } from "../types.js"

// Silence unused-import warnings when the module is tree-shaken away in full
// builds — the exported handler lists still need to be referenced.
void onReadyFns
void onCloseFns

/**
 * Lite-mode boot.
 *
 * No stdio: config is written to `globalThis.__tynd_config__` for the Rust
 * host to read after eval; the native host injects `__tynd_emit__` for
 * event delivery and calls `__tynd_on_ready__` / `__tynd_on_close__`
 * directly for lifecycle.
 */
export function startLite(config: AppConfig): void {
  const g = globalThis as Record<string, unknown>
  g["__tynd_config__"] = JSON.stringify({
    window: config.window ?? {},
    menu: config.menu ?? [],
    tray: config.tray ?? null,
    // devUrl / frontendDir from app.start() — CLI args take priority in quickjs::start()
    devUrl: config.devUrl ?? null,
    frontendDir: config.frontendDir ?? null,
  })

  // Look up on every emit — if tests or alt hosts inject it late, events
  // flow once the global appears instead of being silently dropped forever.
  let warned = false
  setEmitFn((name, payload, to) => {
    const nativeEmit = g["__tynd_emit__"] as
      | ((name: string, payload: string, to?: string) => void)
      | undefined
    if (nativeEmit) {
      nativeEmit(name, JSON.stringify(payload), to)
    } else if (!warned) {
      warned = true
      tynd.warn(`emit("${name}"): __tynd_emit__ not available — event dropped`)
    }
  })

  g["__tynd_on_ready__"] = fireReady
  g["__tynd_on_close__"] = fireClose
}
