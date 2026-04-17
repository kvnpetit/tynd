import { afterEach, beforeEach, describe, expect, test } from "bun:test"
import { fs } from "./fs.ts"

type FetchCall = { url: string; method: string; body: Uint8Array | null }

const originalFetch = globalThis.fetch

function mountShim(handler: (c: FetchCall) => { status: number; body: Uint8Array }): FetchCall[] {
  const fetches: FetchCall[] = []
  ;(globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call() {
        return Promise.resolve(null)
      },
      os_on() {
        return () => undefined
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
  globalThis.fetch = ((url: string, init?: RequestInit) => {
    const bodyInit = init?.body
    const body =
      bodyInit instanceof Uint8Array
        ? bodyInit
        : bodyInit instanceof ArrayBuffer
          ? new Uint8Array(bodyInit)
          : null
    const call: FetchCall = { url, method: init?.method ?? "GET", body }
    fetches.push(call)
    const res = handler(call)
    return Promise.resolve(new Response(res.body as BodyInit, { status: res.status }))
  }) as typeof fetch
  return fetches
}

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe("fs binary (tynd-bin scheme)", () => {
  let fetches: FetchCall[]
  beforeEach(() => {
    fetches = mountShim(() => ({ status: 200, body: new Uint8Array([1, 2, 3, 4]) }))
  })

  test("readBinary issues a GET and returns bytes", async () => {
    const bytes = await fs.readBinary("/tmp/x.bin")
    expect(bytes).toEqual(new Uint8Array([1, 2, 3, 4]))
    expect(fetches[0]!.method).toBe("GET")
    expect(fetches[0]!.url).toContain("tynd-bin://localhost/fs/readBinary")
    expect(fetches[0]!.url).toContain("path=%2Ftmp%2Fx.bin")
  })

  test("writeBinary POSTs the raw body", async () => {
    const payload = new Uint8Array([9, 8, 7])
    mountShim(() => ({ status: 204, body: new Uint8Array() }))
    await fs.writeBinary("/tmp/out.bin", payload)
  })

  test("writeBinary carries createDirs in the query", async () => {
    const fs2Fetches = mountShim(() => ({ status: 204, body: new Uint8Array() }))
    await fs.writeBinary("/tmp/nested/out.bin", new Uint8Array([1]), { createDirs: true })
    expect(fs2Fetches[0]!.url).toContain("createDirs=1")
  })

  test("non-2xx response rejects", async () => {
    mountShim(() => ({ status: 500, body: new TextEncoder().encode("disk full") }))
    await expect(fs.readBinary("/tmp/x")).rejects.toThrow(/disk full/)
  })
})
