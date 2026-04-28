import { fireClose, fireReady, setEmitFn } from "../lifecycle";
import { tynd } from "../logger";
import type { AppConfig } from "../types";
import { safeStringify } from "./stringify";

/**
 * Lite boot: config via `globalThis.__tynd_config__`, events via injected
 * `__tynd_emit__`, plugins pre-registered by the CLI-prepended bundle header.
 */
export function startLite(config: AppConfig): void {
  const g = globalThis as Record<string, unknown>;
  // Config is app-controlled; primitives + null only, but route through
  // `safeStringify` so any future addition stays safe (BigInt, etc.).
  const configJson = safeStringify({
    window: config.window ?? {},
    menu: config.menu ?? [],
    tray: config.tray ?? null,
    // devUrl / frontendDir from app.start() — CLI args take priority in quickjs::start()
    devUrl: config.devUrl ?? null,
    frontendDir: config.frontendDir ?? null,
    quitOnLastWindowClosed: config.quitOnLastWindowClosed ?? false,
  });
  if (configJson === null) {
    tynd.error("lite: tynd.config contains a non-JSON-serializable value — aborting");
    throw new Error("unserializable tynd.config");
  }
  g.__tynd_config__ = configJson;

  // Look up on every emit — if tests or alt hosts inject it late, events
  // flow once the global appears instead of being silently dropped forever.
  let warned = false;
  setEmitFn((name, payload, to) => {
    const nativeEmit = g.__tynd_emit__ as
      | ((name: string, payload: string, to?: string) => void)
      | undefined;
    if (!nativeEmit) {
      if (!warned) {
        warned = true;
        tynd.warn(`emit("${name}"): __tynd_emit__ not available — event dropped`);
      }
      return;
    }
    // Match full-mode: surface BigInt / circular refs as explicit log
    // rather than silently JSON-crashing inside `nativeEmit`.
    const encoded = safeStringify(payload);
    if (encoded === null) {
      tynd.error(`emit("${name}"): payload is not JSON-serializable — dropped`);
      return;
    }
    nativeEmit(name, encoded, to);
  });

  g.__tynd_on_ready__ = fireReady;
  g.__tynd_on_close__ = fireClose;
}
