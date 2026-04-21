/**
 * @tynd/core — backend API
 * Import from "@tynd/core" in your Bun/Node backend only.
 * For frontend use: import from "@tynd/core/client"
 */

export type {
  AppConfig,
  Emitter,
  EmitterMap,
  MenuItem,
  MenuSubmenu,
  TrayConfig,
  WindowConfig,
} from "./types.js"

import * as v from "valibot"
import { getEmitFn, lifecycleFlags, onCloseFns, onReadyFns } from "./lifecycle.ts"
import { tynd } from "./logger.js"
import { startFull } from "./runtime/full.ts"
import { startLite } from "./runtime/lite.ts"
import { type AppConfig, AppConfigSchema, type Emitter, type EmitterMap } from "./types.js"

export { onFrontendEmit } from "./frontend-events.js"

/**
 * Create a typed event emitter. Export the result from your backend module.
 * The frontend subscribes via `api.on("eventName", handler)`.
 *
 * @example
 * export const events = createEmitter<{ userCreated: User }>()
 * events.emit("userCreated", newUser)
 */
export function createEmitter<T extends EmitterMap>(): Emitter<T> {
  return {
    __tynd_emitter__: true,
    __tynd_event_types__: undefined as unknown as T,
    emit<K extends keyof T>(event: K & string, payload: T[K]) {
      const fn = getEmitFn()
      if (!fn) {
        tynd.warn(`emit("${event}") before app.start() — event dropped`)
        return
      }
      fn(event, payload)
    },
    emitTo<K extends keyof T>(label: string, event: K & string, payload: T[K]) {
      const fn = getEmitFn()
      if (!fn) {
        tynd.warn(`emitTo("${event}") before app.start() — event dropped`)
        return
      }
      fn(event, payload, label)
    },
  }
}

export const app = {
  /**
   * Start the Tynd app. Call this once at the end of your backend entry file.
   *
   * In **full** mode: spawns the Bun/Node IPC listener (stdin/stdout).
   * In **lite** mode: writes window config to `globalThis.__tynd_config__`
   * for the Rust host to read, then exports lifecycle hooks.
   *
   * @example
   * app.start({ window: { title: "My App" } })
   */
  start(config: AppConfig = {}): void {
    const result = v.safeParse(AppConfigSchema, config)
    if (!result.success) {
      const issues = result.issues
        .map((i) => {
          const path = i.path?.map((p) => p.key).join(".") ?? ""
          return path ? `  • ${path}: ${i.message}` : `  • ${i.message}`
        })
        .join("\n")
      tynd.error(`app.start() received invalid config:\n${issues}`)
      throw new Error("Invalid app.start() config")
    }
    // The CLI's bundler replaces `globalThis.__TYND_RUNTIME__` at build
    // time with a string literal ("lite" | "full") so the unused branch
    // below is dead-code eliminated per runtime bundle. The `__tynd_lite__`
    // fallback is the unbundled-dev escape hatch set by the QuickJS host.
    const g = globalThis as { __TYND_RUNTIME__?: string; __tynd_lite__?: unknown }
    if (
      g.__TYND_RUNTIME__ === "lite" ||
      (g.__TYND_RUNTIME__ === undefined && g.__tynd_lite__ !== undefined)
    ) {
      startLite(result.output)
    } else {
      startFull(result.output)
    }
  },

  /** Fires when the WebView window is ready (page fully loaded) */
  onReady(fn: () => void): void {
    if (lifecycleFlags.readyFired) {
      queueMicrotask(fn)
      return
    }
    onReadyFns.push(fn)
  },

  /** Fires when the window is about to close */
  onClose(fn: () => void): void {
    if (lifecycleFlags.closeFired) {
      queueMicrotask(fn)
      return
    }
    onCloseFns.push(fn)
  },
}
