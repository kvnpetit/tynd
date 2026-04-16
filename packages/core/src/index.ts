/**
 * @vorn/core — backend API
 * Import from "@vorn/core" in your Bun/Node backend only.
 * For frontend use: import from "@vorn/core/client"
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
import { vorn } from "./logger.js"
import { type AppConfig, AppConfigSchema, type Emitter, type EmitterMap } from "./types.js"

// __VORN_RUNTIME__ is replaced at bundle time by the CLI:
//   buildLiteBundle → define: { "globalThis.__VORN_RUNTIME__": '"lite"' }
//   buildFullBundle → define: { "globalThis.__VORN_RUNTIME__": '"full"' }
// This lets Bun's DCE eliminate the dead branch entirely from each bundle.
// Fallback to __vorn_lite__ (injected by QuickJS host at runtime) for
// cases where the bundle is run without the CLI bundler's define.
const _g = globalThis as Record<string, unknown>
const IS_LITE: boolean =
  _g["__VORN_RUNTIME__"] === "lite" ||
  (_g["__VORN_RUNTIME__"] === undefined && _g["__vorn_lite__"] !== undefined)

let _emitFn: ((name: string, payload: unknown) => void) | null = null

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
    __vorn_emitter__: true,
    __vorn_event_types__: undefined as unknown as T,
    emit<K extends keyof T>(event: K & string, payload: T[K]) {
      if (!_emitFn) {
        vorn.warn(`emit("${event}") before app.start() — event dropped`)
        return
      }
      _emitFn(event, payload)
    },
  }
}

const _onReadyFns: Array<() => void> = []
const _onCloseFns: Array<() => void> = []
let _readyFired = false
let _closeFired = false

export const app = {
  /**
   * Start the Vorn app. Call this once at the end of your backend entry file.
   *
   * In **full** mode: spawns the Bun/Node IPC listener (stdin/stdout).
   * In **lite** mode: writes window config to `globalThis.__vorn_config__`
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
      vorn.error(`app.start() received invalid config:\n${issues}`)
      throw new Error("Invalid app.start() config")
    }
    const validated = result.output
    if (IS_LITE) {
      _startLite(validated)
    } else {
      _startFull(validated)
    }
  },

  /** Fires when the WebView window is ready (page fully loaded) */
  onReady(fn: () => void): void {
    if (_readyFired) {
      queueMicrotask(fn)
      return
    }
    _onReadyFns.push(fn)
  },

  /** Fires when the window is about to close */
  onClose(fn: () => void): void {
    if (_closeFired) {
      queueMicrotask(fn)
      return
    }
    _onCloseFns.push(fn)
  },
}

function _startFull(config: AppConfig): void {
  // 0. Redirect all console output to stderr so stdout stays clean for IPC JSON
  _redirectConsoleToStderr()

  // 1. Send window/frontend config to Rust (reads this as its first stdout line)
  const configMsg = JSON.stringify({
    type: "vorn:config",
    window: config.window ?? {},
    devUrl: config.devUrl ?? process.env["VORN_DEV_URL"],
    frontendDir: process.env["VORN_FRONTEND_DIR"] ?? config.frontendDir,
    menu: config.menu ?? [],
    tray: config.tray ?? null,
  })
  process.stdout.write(`${configMsg}\n`)

  // 2. Wire up the emit function (writes events to stdout for Rust to relay)
  _emitFn = (name, payload) => {
    process.stdout.write(`${JSON.stringify({ type: "event", name, payload })}\n`)
  }

  // 3. Start reading IPC calls from Rust (stdin)
  // Entry path: injected by CLI via VORN_ENTRY env var (preferred),
  // or falling back to the current module's URL (works when run directly).
  const entry = process.env["VORN_ENTRY"] ?? import.meta.path
  _startListener(entry)
}

function _startLite(config: AppConfig): void {
  // Write window config — Rust reads globalThis.__vorn_config__ after eval
  const g = globalThis as Record<string, unknown>
  g["__vorn_config__"] = JSON.stringify({
    window: config.window ?? {},
    menu: config.menu ?? [],
    tray: config.tray ?? null,
    // devUrl / frontendDir from app.start() — CLI args take priority in quickjs::start()
    devUrl: config.devUrl ?? null,
    frontendDir: config.frontendDir ?? null,
  })

  // Wire up emit — calls __vorn_emit__(name, payloadJson) injected by Rust
  const nativeEmit = g["__vorn_emit__"] as ((name: string, payload: string) => void) | undefined

  _emitFn = (name, payload) => {
    if (nativeEmit) {
      nativeEmit(name, JSON.stringify(payload))
    }
  }

  // Export lifecycle hooks — Rust calls __vorn_on_ready__ / __vorn_on_close__
  // These will be picked up from __vorn_mod__ if exported, but we also store
  // them on globalThis so main.rs can call them even without module awareness.
  g["__vorn_on_ready__"] = () => {
    for (const fn of _onReadyFns) {
      try {
        fn()
      } catch {
        /* intentional: best-effort cleanup */
      }
    }
  }
  g["__vorn_on_close__"] = () => {
    for (const fn of _onCloseFns) {
      try {
        fn()
      } catch {
        /* intentional: best-effort cleanup */
      }
    }
  }
}

let _moduleCache: Record<string, unknown> | null = null

async function _getModule(entryPath: string): Promise<Record<string, unknown>> {
  if (!_moduleCache) {
    _moduleCache = (await import(entryPath)) as Record<string, unknown>
  }
  return _moduleCache
}

function _startListener(entryPath: string): void {
  let buffer = ""

  process.stdin.setEncoding("utf8")

  process.stdin.on("data", (chunk: string) => {
    buffer += chunk
    const lines = buffer.split("\n")
    buffer = lines.pop() ?? ""

    for (const line of lines) {
      if (!line.trim()) continue
      _handleLine(line, entryPath).catch((e) => vorn.error(`IPC error: ${e}`))
    }
  })

  process.stdin.on("end", () => {
    _onCloseFns.forEach((fn) => {
      try {
        fn()
      } catch {
        /* intentional: best-effort cleanup */
      }
    })
    process.exit(0)
  })
  process.stdin.on("error", () => process.exit(0))
}

function _redirectConsoleToStderr() {
  const methods = [
    "log",
    "info",
    "debug",
    "warn",
    "error",
    "trace",
    "group",
    "groupEnd",
    "groupCollapsed",
    "table",
    "time",
    "timeEnd",
    "timeLog",
    "dir",
    "count",
    "countReset",
    "assert",
  ] as const
  const con = console as unknown as Record<(typeof methods)[number], (...args: unknown[]) => void>

  function formatArg(a: unknown): string {
    if (a === null) return "null"
    if (a === undefined) return "undefined"
    if (typeof a === "object" || typeof a === "function") {
      try {
        return JSON.stringify(a, null, 0)
      } catch {
        return String(a)
      }
    }
    return String(a)
  }

  for (const method of methods) {
    con[method] = (...args: unknown[]) => {
      process.stderr.write(`[${method.toUpperCase()}] ${args.map(formatArg).join(" ")}\n`)
    }
  }
}

const CallMsgSchema = v.object({
  type: v.literal("call"),
  id: v.string(),
  fn: v.string(),
  args: v.array(v.unknown()),
})
const LifecycleMsgSchema = v.object({
  type: v.union([v.literal("vorn:ready"), v.literal("vorn:close")]),
})
const IpcMsgSchema = v.union([CallMsgSchema, LifecycleMsgSchema])

async function _handleLine(line: string, entryPath: string): Promise<void> {
  let raw: unknown
  try {
    raw = JSON.parse(line)
  } catch {
    vorn.error(`Invalid JSON from Rust: ${line}`)
    return
  }

  const parsed = v.safeParse(IpcMsgSchema, raw)
  if (!parsed.success) {
    vorn.error(`Invalid IPC message shape: ${line}`)
    return
  }
  const msg = parsed.output

  switch (msg.type) {
    case "call": {
      const { id, fn, args } = msg
      try {
        const mod = await _getModule(entryPath)
        const handler = mod[fn]
        if (typeof handler !== "function") {
          throw new Error(`"${fn}" is not an exported function`)
        }
        const value = await (handler as (...a: unknown[]) => unknown)(...args)
        process.stdout.write(`${JSON.stringify({ type: "return", id, ok: true, value })}\n`)
      } catch (e) {
        process.stdout.write(
          `${JSON.stringify({ type: "return", id, ok: false, error: String(e) })}\n`,
        )
      }
      break
    }

    case "vorn:ready":
      _readyFired = true
      for (const fn of _onReadyFns) {
        try {
          fn()
        } catch {
          /* intentional: best-effort cleanup */
        }
      }
      break

    case "vorn:close":
      _closeFired = true
      for (const fn of _onCloseFns) {
        try {
          fn()
        } catch {
          /* intentional: best-effort cleanup */
        }
      }
      process.exit(0)
      break
  }
}
