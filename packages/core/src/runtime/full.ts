import * as v from "valibot"
import { dispatchFrontendEmit, FRONTEND_EMIT_FN } from "../frontend-events"
import { fireClose, fireReady, onCloseFns, setEmitFn } from "../lifecycle"
import { tynd } from "../logger"
import { getPluginMethod, parsePluginCall } from "../plugin"
import type { AppConfig } from "../types"
import { redirectConsoleToStderr } from "./console"
import { safeStringify } from "./stringify"

const CallMsgSchema = v.object({
  type: v.literal("call"),
  id: v.string(),
  fn: v.string(),
  args: v.array(v.unknown()),
})
const CancelMsgSchema = v.object({
  type: v.literal("cancel"),
  id: v.string(),
})
const AckMsgSchema = v.object({
  type: v.literal("ack"),
  id: v.string(),
  n: v.pipe(v.number(), v.integer(), v.minValue(1)),
})
const LifecycleMsgSchema = v.object({
  type: v.union([v.literal("tynd:ready"), v.literal("tynd:close")]),
})
const IpcMsgSchema = v.union([CallMsgSchema, CancelMsgSchema, AckMsgSchema, LifecycleMsgSchema])

/** Per-stream credit cap. Must match ACK_CHUNK on the shim so replenishment beats starvation. */
const STREAM_CREDIT = 64

interface StreamState {
  iter: AsyncIterator<unknown>
  /** How many more chunks the backend may yield before it has to wait. */
  credit: number
  /** Resolved by the next `ack` when credit was 0 at yield time. */
  waiter: (() => void) | null
}

const _activeStreams = new Map<string, StreamState>()

/** Cap single stdin chunk buffer; larger implies malformed framing since binary goes via bin scheme. */
const MAX_STDIN_BUFFER = 8 * 1024 * 1024

function _isAsyncIterable(x: unknown): x is AsyncIterable<unknown> {
  return (
    x != null &&
    (typeof x === "object" || typeof x === "function") &&
    typeof (x as { [Symbol.asyncIterator]?: unknown })[Symbol.asyncIterator] === "function"
  )
}

let _moduleCache: Record<string, unknown> | null = null

async function getModule(entryPath: string): Promise<Record<string, unknown>> {
  if (!_moduleCache) {
    _moduleCache = (await import(entryPath)) as Record<string, unknown>
  }
  return _moduleCache
}

/** Full boot: console→stderr, first stdout line = config, then emit+IPC loop. */
export function startFull(config: AppConfig): void {
  redirectConsoleToStderr()

  const configMsg = JSON.stringify({
    type: "tynd:config",
    window: config.window ?? {},
    devUrl: config.devUrl ?? process.env["TYND_DEV_URL"],
    frontendDir: process.env["TYND_FRONTEND_DIR"] ?? config.frontendDir,
    menu: config.menu ?? [],
    tray: config.tray ?? null,
    quitOnLastWindowClosed: config.quitOnLastWindowClosed ?? false,
  })
  process.stdout.write(`${configMsg}\n`)

  setEmitFn((name, payload, to) => {
    const encoded = safeStringify({ type: "event", name, payload, to })
    if (encoded === null) {
      tynd.error(`emit: payload for '${name}' is not JSON-serializable (circular ref?) — dropped`)
      return
    }
    process.stdout.write(`${encoded}\n`)
  })

  // Plugins are registered by the CLI-injected header at the top of the
  // built backend bundle, not here — by the time this function runs the
  // registry is already populated.

  // The CLI always injects `TYND_ENTRY`; missing it is a bug in the
  // launcher, not a user error worth silently handling.
  const entry = process.env["TYND_ENTRY"]
  if (!entry) {
    tynd.error("TYND_ENTRY env var not set — run this app via `tynd dev` or `tynd start`.")
    process.exit(1)
  }
  startListener(entry)
}

function startListener(entryPath: string): void {
  let buffer = ""

  process.stdin.setEncoding("utf8")

  process.stdin.on("data", (chunk: string) => {
    buffer += chunk
    // Guard against a host that sends bytes without newlines — we'd
    // otherwise grow the buffer until OOM. 8 MiB is already orders of
    // magnitude beyond a normal IPC frame.
    if (buffer.length > MAX_STDIN_BUFFER) {
      tynd.error(`IPC stdin buffer exceeded ${MAX_STDIN_BUFFER} bytes without a newline — aborting`)
      process.exit(1)
    }
    const lines = buffer.split("\n")
    buffer = lines.pop() ?? ""

    for (const line of lines) {
      if (!line.trim()) continue
      handleLine(line, entryPath).catch((e) => tynd.error(`IPC error: ${e}`))
    }
  })

  process.stdin.on("end", () => {
    for (const fn of onCloseFns) {
      try {
        fn()
      } catch {
        /* best-effort cleanup */
      }
    }
    process.exit(0)
  })
  process.stdin.on("error", () => process.exit(0))
}

/** Keep log lines terminal-friendly; a multi-MiB malformed payload would
 * otherwise blow up the log itself. */
const LOG_PREVIEW = 200
function preview(s: string): string {
  return s.length > LOG_PREVIEW ? `${s.slice(0, LOG_PREVIEW)}… (${s.length} chars)` : s
}

async function handleLine(line: string, entryPath: string): Promise<void> {
  let raw: unknown
  try {
    raw = JSON.parse(line)
  } catch {
    tynd.error(`Invalid JSON from Rust: ${preview(line)}`)
    return
  }

  const parsed = v.safeParse(IpcMsgSchema, raw)
  if (!parsed.success) {
    tynd.error(`Invalid IPC message shape: ${preview(line)}`)
    return
  }
  const msg = parsed.output

  switch (msg.type) {
    case "call":
      await handleCall(msg, entryPath)
      break

    case "cancel": {
      const stream = _activeStreams.get(msg.id)
      if (stream) {
        _activeStreams.delete(msg.id)
        // Release any pending yield so it can observe the stream is gone.
        stream.waiter?.()
        if (typeof stream.iter.return === "function") {
          stream.iter.return(undefined).catch(() => {
            /* swallow: cancel is best-effort */
          })
        }
      }
      break
    }

    case "ack": {
      const stream = _activeStreams.get(msg.id)
      if (stream) {
        // Cap credit at STREAM_CREDIT — a buggy / hostile host sending a
        // huge ack value could otherwise grow credit without bound and
        // defeat the back-pressure cap.
        stream.credit = Math.min(stream.credit + msg.n, STREAM_CREDIT)
        // Wake a yield blocked on empty credit, if any.
        if (stream.waiter) {
          const wake = stream.waiter
          stream.waiter = null
          wake()
        }
      }
      break
    }

    case "tynd:ready":
      fireReady()
      break

    case "tynd:close":
      fireClose()
      process.exit(0)
      break
  }
}

async function handleCall(
  msg: { id: string; fn: string; args: readonly unknown[] },
  entryPath: string,
): Promise<void> {
  const { id, fn, args } = msg
  try {
    // Reserved name: frontend emitter bus. Runs without touching the user
    // module so module loading failures don't block frontend -> backend
    // events, and event dispatch doesn't accidentally shadow a same-named
    // user export.
    if (fn === FRONTEND_EMIT_FN) {
      const eventName = args[0]
      if (typeof eventName !== "string") {
        throw new Error(
          `${FRONTEND_EMIT_FN}: expected args[0] to be an event name string, got ${typeof eventName}`,
        )
      }
      dispatchFrontendEmit(eventName, args[1])
      process.stdout.write(`${safeStringify({ type: "return", id, ok: true, value: null })}\n`)
      return
    }

    // Plugin calls arrive with a reserved `$plugin$<name>$<method>` fn;
    // routed to the plugin registry rather than the user module.
    const pluginCall = parsePluginCall(fn)
    let resolved: unknown
    if (pluginCall) {
      const handler = getPluginMethod(pluginCall.plugin, pluginCall.method)
      if (!handler) {
        throw new Error(
          `plugin "${pluginCall.plugin}" has no method "${pluginCall.method}" (missing from tynd.config.ts::plugins?)`,
        )
      }
      resolved = await handler(...args)
    } else {
      const mod = await getModule(entryPath)
      const handler = mod[fn]
      if (typeof handler !== "function") {
        throw new Error(`"${fn}" is not an exported function`)
      }
      resolved = await (handler as (...a: unknown[]) => unknown)(...args)
    }

    if (_isAsyncIterable(resolved)) {
      await streamIterable(id, resolved)
    } else {
      const encoded = safeStringify({ type: "return", id, ok: true, value: resolved })
      if (encoded === null) {
        throw new Error(
          `return value for '${fn}' is not JSON-serializable (circular ref / non-JSON type)`,
        )
      }
      process.stdout.write(`${encoded}\n`)
    }
  } catch (e) {
    _activeStreams.delete(id)
    process.stdout.write(
      // errorMessage() returns a plain string, so stringify cannot fail here.
      `${JSON.stringify({ type: "return", id, ok: false, error: errorMessage(e) })}\n`,
    )
  }
}

/** Keep error surfaces short — stack traces can leak file paths, env
 * values, or embedded secrets if the caller crafted them into an Error. */
function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

async function streamIterable(id: string, iterable: AsyncIterable<unknown>): Promise<void> {
  const iter = iterable[Symbol.asyncIterator]()
  const state: StreamState = { iter, credit: STREAM_CREDIT, waiter: null }
  _activeStreams.set(id, state)
  let final: unknown
  let completed = false
  try {
    while (true) {
      const step = await iter.next()
      if (step.done) {
        final = step.value
        completed = true
        break
      }
      // Block before writing if credit is exhausted — the frontend will ack
      // past-consumed chunks and wake us. Caps memory at STREAM_CREDIT.
      if (state.credit <= 0) {
        await new Promise<void>((resolve) => {
          state.waiter = resolve
        })
        // Cancelled while we waited — break out cleanly.
        if (!_activeStreams.has(id)) break
      }
      state.credit -= 1
      const encoded = safeStringify({ type: "yield", id, value: step.value })
      if (encoded === null) {
        throw new Error(
          `yielded value is not JSON-serializable (circular ref / non-JSON type) on stream ${id}`,
        )
      }
      process.stdout.write(`${encoded}\n`)
      if (!_activeStreams.has(id)) break
    }
  } finally {
    _activeStreams.delete(id)
    // Run the generator's `finally` blocks on any abnormal exit — cancel or
    // uncaught throw. Skip on normal completion where `iter` already ran its
    // finalizer. Swallow errors from `return()` itself; the stream is dead.
    if (!completed && typeof iter.return === "function") {
      try {
        await iter.return(undefined)
      } catch {
        /* intentional: generator cleanup is best-effort */
      }
    }
  }
  const encoded = safeStringify({ type: "return", id, ok: true, value: final })
  if (encoded === null) {
    process.stdout.write(
      `${JSON.stringify({ type: "return", id, ok: false, error: "final value is not JSON-serializable" })}\n`,
    )
    return
  }
  process.stdout.write(`${encoded}\n`)
}
