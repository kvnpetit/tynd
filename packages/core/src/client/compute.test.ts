import { afterEach, beforeEach, describe, expect, test } from "bun:test"
import { compute } from "./compute.ts"

type FetchCall = { url: string; method: string; body: Uint8Array | null }
type OsCall = { api: string; method: string; args: unknown }

const originalFetch = globalThis.fetch

function mountShim(options: {
  onFetch: (c: FetchCall) => { status: number; body: Uint8Array | string }
  onOsCall?: (c: OsCall) => unknown
}): { fetches: FetchCall[]; osCalls: OsCall[] } {
  const fetches: FetchCall[] = []
  const osCalls: OsCall[] = []
  ;(globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        const call = { api, method, args }
        osCalls.push(call)
        return Promise.resolve(options.onOsCall?.(call) ?? null)
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
    const res = options.onFetch(call)
    const resBody = typeof res.body === "string" ? new TextEncoder().encode(res.body) : res.body
    return Promise.resolve(
      new Response(resBody as BodyInit, {
        status: res.status,
        statusText: res.status >= 400 ? "error" : "ok",
      }),
    )
  }) as typeof fetch
  return { fetches, osCalls }
}

afterEach(() => {
  globalThis.fetch = originalFetch
})

describe("compute client (tynd-bin scheme)", () => {
  let fetches: FetchCall[]
  let osCalls: OsCall[]
  beforeEach(() => {
    ;({ fetches, osCalls } = mountShim({
      onFetch: (c) => {
        if (c.url.includes("compute/hash")) return { status: 200, body: "deadbeef" }
        if (c.url.includes("compute/compress"))
          return { status: 200, body: new Uint8Array([99, 98, 97]) }
        if (c.url.includes("compute/decompress"))
          return { status: 200, body: new Uint8Array([104, 105]) }
        return { status: 404, body: "missing" }
      },
      onOsCall: (c) => (c.method === "randomBytes" ? "AAECAw==" : null),
    }))
  })

  test("hash encodes string input as UTF-8 request body", async () => {
    const out = await compute.hash("abc")
    expect(out).toBe("deadbeef")
    expect(fetches).toHaveLength(1)
    expect(fetches[0]!.url).toContain("tynd-bin://localhost/compute/hash")
    expect(fetches[0]!.url).toContain("algo=blake3")
    expect(fetches[0]!.url).toContain("encoding=hex")
    expect(new TextDecoder().decode(fetches[0]!.body!)).toBe("abc")
  })

  test("hash forwards raw Uint8Array bytes without re-encoding", async () => {
    const bytes = new Uint8Array([0xde, 0xad, 0xbe, 0xef])
    await compute.hash(bytes, { algo: "sha256", encoding: "base64" })
    expect(fetches[0]!.url).toContain("algo=sha256")
    expect(fetches[0]!.url).toContain("encoding=base64")
    expect(fetches[0]!.body).toEqual(bytes)
  })

  test("compress returns response body as Uint8Array", async () => {
    const out = await compute.compress(new Uint8Array([1, 2, 3]))
    expect(out).toEqual(new Uint8Array([99, 98, 97]))
    expect(fetches[0]!.method).toBe("POST")
  })

  test("decompress returns raw bytes (no JSON envelope)", async () => {
    const out = await compute.decompress(new Uint8Array([1]))
    expect(out).toEqual(new Uint8Array([104, 105]))
  })

  test("randomBytes still routes through the JSON IPC", async () => {
    const out = await compute.randomBytes(4)
    expect(out).toEqual(new Uint8Array([0, 1, 2, 3]))
    expect(osCalls).toHaveLength(1)
    expect(osCalls[0]!.method).toBe("randomBytes")
    expect(fetches).toHaveLength(0)
  })

  test("4xx responses surface as rejected promises", async () => {
    mountShim({ onFetch: () => ({ status: 400, body: "bad algo" }) })
    await expect(compute.hash("x", { algo: "md5" as never })).rejects.toThrow(/bad algo/)
  })
})
