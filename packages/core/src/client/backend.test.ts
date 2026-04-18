import { beforeEach, describe, expect, test } from "bun:test"
import { createBackend } from "./backend.ts"

type Calls = Array<{ fn: string; args: unknown[] }>
type EventHandlers = Map<string, Array<(payload: unknown) => void>>

// Build a CallHandle-shaped object backed by an in-memory chunk list.
// Simulates async delivery: chunks + final are delivered on a microtask so
// the handle resolves even if the caller only `await`s without iterating.
function makeStreamHandle(chunks: readonly unknown[], finalValue: unknown = undefined) {
  const buffered: unknown[] = []
  const waiters: Array<(r: IteratorResult<unknown>) => void> = []
  let done = false
  let resolveFinal!: (v: unknown) => void
  const finalPromise = new Promise<unknown>((res) => {
    resolveFinal = res
  })

  function deliver(value: unknown) {
    if (waiters.length) waiters.shift()!({ value, done: false })
    else buffered.push(value)
  }
  function finish(v: unknown) {
    if (done) return
    done = true
    resolveFinal(v)
    while (waiters.length) waiters.shift()!({ value: undefined, done: true })
  }

  // Deliver one chunk per microtask so a cancel between pulls can interrupt.
  let i = 0
  function step() {
    if (done) return
    if (i >= chunks.length) {
      finish(finalValue)
      return
    }
    deliver(chunks[i++])
    queueMicrotask(step)
  }
  queueMicrotask(step)

  const iterator = {
    next(): Promise<IteratorResult<unknown>> {
      if (buffered.length) {
        return Promise.resolve({ value: buffered.shift(), done: false })
      }
      if (done) return Promise.resolve({ value: undefined, done: true })
      return new Promise((resolve) => {
        waiters.push(resolve)
      })
    },
    return() {
      finish(undefined)
      return Promise.resolve({ value: undefined, done: true as const })
    },
  }
  return {
    // biome-ignore lint/suspicious/noThenProperty: mirrors CallHandle's intentional PromiseLike shape
    then: finalPromise.then.bind(finalPromise),
    catch: finalPromise.catch.bind(finalPromise),
    finally: finalPromise.finally.bind(finalPromise),
    cancel: () => iterator.return(),
    [Symbol.asyncIterator]: () => iterator,
  }
}

// Fake the `window.__tynd__` shim so the proxy has something to call into.
function mountShim(streamChunks?: Record<string, unknown[]>): {
  calls: Calls
  events: EventHandlers
} {
  const calls: Calls = []
  const events: EventHandlers = new Map()
  ;(globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      call(fn: string, args: unknown[]) {
        calls.push({ fn, args })
        if (streamChunks && fn in streamChunks) {
          return makeStreamHandle(streamChunks[fn] ?? [], { fn, args })
        }
        return Promise.resolve({ fn, args })
      },
      os_call() {
        return Promise.resolve(null)
      },
      os_on() {
        return () => undefined
      },
      on(name: string, handler: (p: unknown) => void) {
        if (!events.has(name)) events.set(name, [])
        events.get(name)!.push(handler)
        return () => undefined
      },
      off(name: string, handler: (p: unknown) => void) {
        const list = events.get(name)
        if (!list) return
        const idx = list.indexOf(handler)
        if (idx >= 0) list.splice(idx, 1)
      },
    },
  }
  return { calls, events }
}

beforeEach(() => {
  mountShim()
})

describe("createBackend proxy", () => {
  test("method access routes to __tynd__.call", async () => {
    const api = createBackend<{ greet: (s: string) => Promise<string> }>()
    const result = (await api.greet("Alice")) as unknown as { fn: string; args: unknown[] }
    expect(result.fn).toBe("greet")
    expect(result.args).toEqual(["Alice"])
  })

  test("then / catch / finally are NOT routed (no thenable Proxy trap)", () => {
    const api = createBackend<Record<string, never>>()
    expect((api as unknown as { then: unknown }).then).toBeUndefined()
    expect((api as unknown as { catch: unknown }).catch).toBeUndefined()
    expect((api as unknown as { finally: unknown }).finally).toBeUndefined()
  })

  test("symbol access returns undefined (proxy stays opaque)", () => {
    const api = createBackend<Record<string, never>>()
    expect((api as unknown as Record<symbol, unknown>)[Symbol.iterator]).toBeUndefined()
  })

  test("on subscribes and returns unsubscribe", () => {
    const api = createBackend<Record<string, never>>()
    const handler = () => undefined
    const unsub = (api as unknown as { on: (n: string, h: () => void) => () => void }).on(
      "ready",
      handler,
    )
    expect(typeof unsub).toBe("function")
  })

  test("async-iterable handle yields chunks via `for await`", async () => {
    mountShim({ ask: ["hel", "lo ", "world"] })
    const api = createBackend<{
      ask: (q: string) => AsyncGenerator<string, void, unknown>
    }>()
    const out: string[] = []
    for await (const tok of api.ask("hi")) out.push(tok)
    expect(out).toEqual(["hel", "lo ", "world"])
  })

  test("awaiting a streaming handle resolves with the final return value", async () => {
    mountShim({ ask: ["a", "b"] })
    const api = createBackend<{
      ask: (q: string) => AsyncGenerator<string, { fn: string }, unknown>
    }>()
    const final = await api.ask("hi")
    expect(final.fn).toBe("ask")
  })

  test("handle.cancel() stops iteration", async () => {
    mountShim({ ask: ["a", "b", "c", "d"] })
    const api = createBackend<{
      ask: (q: string) => AsyncGenerator<string, void, unknown>
    }>()
    const handle = api.ask("hi")
    const out: string[] = []
    for await (const tok of handle) {
      out.push(tok)
      if (out.length === 2) await handle.cancel()
    }
    expect(out).toEqual(["a", "b"])
  })

  test("once auto-unsubscribes after first call", () => {
    const { events } = mountShim()
    const api = createBackend<Record<string, never>>()
    let hits = 0
    const once = (api as unknown as { once: (n: string, h: () => void) => () => void }).once
    once("fired", () => {
      hits++
    })
    const list = events.get("fired")!
    expect(list).toHaveLength(1)
    const wrapper = list[0]!
    wrapper("payload")
    wrapper("payload") // second invocation must be a no-op
    expect(hits).toBe(1)
    expect(events.get("fired")).toHaveLength(0)
  })
})
