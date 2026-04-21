import * as v from "valibot"
import { dispatchFrontendEmit, FRONTEND_EMIT_FN } from "../frontend-events.js"
import { fireClose, fireReady, onCloseFns, setEmitFn } from "../lifecycle.ts"
import { tynd } from "../logger.js"
import type { AppConfig } from "../types.js"
import { redirectConsoleToStderr } from "./console.ts"

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

/** Initial per-stream credit. Enough buffer for sub-round-trip bursts; bounded
 * so a runaway generator can't fill frontend memory. Must match the ACK_CHUNK
 * on the shim so credit is replenished before it hits zero under normal load. */
const STREAM_CREDIT = 64

interface StreamState {
  iter: AsyncIterator<unknown>
  /** How many more chunks the backend may yield before it has to wait. */
  credit: number
  /** Resolved by the next `ack` when credit was 0 at yield time. */
  waiter: (() => void) | null
}

const _activeStreams = new Map<string, StreamState>()

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

/**
 * Full-mode boot.
 *
 * 1. Redirect console.* -> stderr so stdout stays clean for IPC JSON.
 * 2. Send window/frontend config as the first stdout line.
 * 3. Wire emit() to write JSON events to stdout.
 * 4. Read IPC calls from stdin.
 */
export function startFull(config: AppConfig): void {
  redirectConsoleToStderr()

  const configMsg = JSON.stringify({
    type: "tynd:config",
    window: config.window ?? {},
    devUrl: config.devUrl ?? process.env["TYND_DEV_URL"],
    frontendDir: process.env["TYND_FRONTEND_DIR"] ?? config.frontendDir,
    menu: config.menu ?? [],
    tray: config.tray ?? null,
  })
  process.stdout.write(`${configMsg}\n`)

  setEmitFn((name, payload, to) => {
    process.stdout.write(`${JSON.stringify({ type: "event", name, payload, to })}\n`)
  })

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

async function handleLine(line: string, entryPath: string): Promise<void> {
  let raw: unknown
  try {
    raw = JSON.parse(line)
  } catch {
    tynd.error(`Invalid JSON from Rust: ${line}`)
    return
  }

  const parsed = v.safeParse(IpcMsgSchema, raw)
  if (!parsed.success) {
    tynd.error(`Invalid IPC message shape: ${line}`)
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
        stream.credit += msg.n
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
      dispatchFrontendEmit(args[0] as string, args[1])
      process.stdout.write(`${JSON.stringify({ type: "return", id, ok: true, value: null })}\n`)
      return
    }
    const mod = await getModule(entryPath)
    const handler = mod[fn]
    if (typeof handler !== "function") {
      throw new Error(`"${fn}" is not an exported function`)
    }
    const resolved = await (handler as (...a: unknown[]) => unknown)(...args)

    if (_isAsyncIterable(resolved)) {
      await streamIterable(id, resolved)
    } else {
      process.stdout.write(`${JSON.stringify({ type: "return", id, ok: true, value: resolved })}\n`)
    }
  } catch (e) {
    _activeStreams.delete(id)
    process.stdout.write(`${JSON.stringify({ type: "return", id, ok: false, error: String(e) })}\n`)
  }
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
      process.stdout.write(`${JSON.stringify({ type: "yield", id, value: step.value })}\n`)
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
  process.stdout.write(`${JSON.stringify({ type: "return", id, ok: true, value: final })}\n`)
}
