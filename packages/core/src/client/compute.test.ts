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
        if (c.url.includes("compute/hash")) return { status: 200, body: "ABEiM0RVZneImaq7zN3u/w==" }
        return { status: 404, body: "missing" }
      },
      onOsCall: (c) => (c.method === "randomBytes" ? "AAECAw==" : null),
    }))
  })

  test("hash forwards raw Uint8Array bytes and always requests base64", async () => {
    const bytes = new Uint8Array([0xde, 0xad, 0xbe, 0xef])
    const out = await compute.hash(bytes)
    expect(out).toBe("ABEiM0RVZneImaq7zN3u/w==")
    expect(fetches).toHaveLength(1)
    expect(fetches[0]!.url).toContain("tynd-bin://localhost/compute/hash")
    expect(fetches[0]!.url).toContain("algo=blake3")
    expect(fetches[0]!.url).toContain("encoding=base64")
    expect(fetches[0]!.body).toEqual(bytes)
  })

  test("hash honors the algo option", async () => {
    await compute.hash(new Uint8Array([0x01]), { algo: "sha256" })
    expect(fetches[0]!.url).toContain("algo=sha256")
    expect(fetches[0]!.url).toContain("encoding=base64")
  })

  test("randomBytes routes through the JSON IPC, not the tynd-bin scheme", async () => {
    const out = await compute.randomBytes(4)
    expect(out).toEqual(new Uint8Array([0, 1, 2, 3]))
    expect(osCalls).toHaveLength(1)
    expect(osCalls[0]!.method).toBe("randomBytes")
    expect(fetches).toHaveLength(0)
  })

  test("4xx responses surface as rejected promises", async () => {
    mountShim({ onFetch: () => ({ status: 400, body: "bad algo" }) })
    await expect(compute.hash(new Uint8Array([1]), { algo: "md5" as never })).rejects.toThrow(
      /bad algo/,
    )
  })
})
