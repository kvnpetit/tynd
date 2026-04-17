import { beforeEach, describe, expect, test } from "bun:test"
import { createBackend } from "./backend.ts"

type Calls = Array<{ fn: string; args: unknown[] }>
type EventHandlers = Map<string, Array<(payload: unknown) => void>>

// Fake the `window.__tynd__` shim so the proxy has something to call into.
function mountShim(): { calls: Calls; events: EventHandlers } {
  const calls: Calls = []
  const events: EventHandlers = new Map()
  ;(globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      call(fn: string, args: unknown[]) {
        calls.push({ fn, args })
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
