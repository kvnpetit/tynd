import { beforeEach, describe, expect, test } from "bun:test"
import { websocket } from "./websocket.ts"

type Call = { api: string; method: string; args: unknown }
type EventBus = Map<string, Array<(data: unknown) => void>>

function mountShim(resolver: (c: Call) => unknown): { calls: Call[]; events: EventBus } {
  const calls: Call[] = []
  const events: EventBus = new Map()
  ;(globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        const call = { api, method, args }
        calls.push(call)
        return Promise.resolve(resolver(call))
      },
      os_on(name: string, handler: (data: unknown) => void) {
        if (!events.has(name)) events.set(name, [])
        events.get(name)!.push(handler)
        return () => {
          const list = events.get(name)!
          list.splice(list.indexOf(handler), 1)
        }
      },
      call() {
        return Promise.resolve(null)
      },
      on() {
        return () => undefined
      },
      off() {
        return undefined
      },
    },
  }
  return { calls, events }
}

function fire(events: EventBus, name: string, data: unknown) {
  for (const h of events.get(name) ?? []) h(data)
}

describe("websocket client", () => {
  let calls: Call[]
  let events: EventBus
  beforeEach(() => {
    ;({ calls, events } = mountShim((c) => {
      if (c.method === "connect") return { id: 7 }
      return null
    }))
  })

  test("connect returns a handle bound to the assigned id", async () => {
    const ws = await websocket.connect("wss://x")
    expect(ws.id).toBe(7)
    expect(calls[0]).toMatchObject({ method: "connect", args: { url: "wss://x" } })
  })

  test("send string routes as text", async () => {
    const ws = await websocket.connect("wss://x")
    await ws.send("hello")
    const sendCall = calls.find((c) => c.method === "send")!
    expect(sendCall.args).toEqual({ id: 7, kind: "text", data: "hello" })
  })

  test("send Uint8Array routes as binary base64", async () => {
    const ws = await websocket.connect("wss://x")
    await ws.send(new Uint8Array([1, 2, 3]))
    const sendCall = calls.find((c) => c.method === "send")!
    const args = sendCall.args as { id: number; kind: string; data: string }
    expect(args.kind).toBe("binary")
    expect(atob(args.data)).toBe("\x01\x02\x03")
  })

  test("onMessage filters by id and decodes binary frames", async () => {
    const ws = await websocket.connect("wss://x")
    const received: Array<{ kind: string; data: string | Uint8Array }> = []
    ws.onMessage((m) => received.push(m))

    fire(events, "websocket:message", { id: 999, kind: "text", data: "ignored" })
    fire(events, "websocket:message", { id: 7, kind: "text", data: "hi" })
    fire(events, "websocket:message", { id: 7, kind: "binary", data: "AQID" }) // 1,2,3

    expect(received).toHaveLength(2)
    expect(received[0]).toEqual({ kind: "text", data: "hi" })
    expect(received[1]!.kind).toBe("binary")
    expect(received[1]!.data).toEqual(new Uint8Array([1, 2, 3]))
  })

  test("onClose only fires for matching id", async () => {
    const ws = await websocket.connect("wss://x")
    const codes: number[] = []
    ws.onClose((c) => {
      codes.push(c)
    })
    fire(events, "websocket:close", { id: 999, code: 1011 })
    expect(codes).toEqual([])
    fire(events, "websocket:close", { id: 7, code: 1000 })
    expect(codes).toEqual([1000])
  })

  test("close forwards code + reason", async () => {
    const ws = await websocket.connect("wss://x")
    await ws.close(1000, "bye")
    expect(calls.find((c) => c.method === "close")!.args).toEqual({
      id: 7,
      code: 1000,
      reason: "bye",
    })
  })
})
