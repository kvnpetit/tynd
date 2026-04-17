import { beforeEach, describe, expect, test } from "bun:test"
import { compute } from "./compute.ts"

type Call = { api: string; method: string; args: unknown }

function mountShim(handler: (c: Call) => unknown): Call[] {
  const calls: Call[] = []
  ;(globalThis as unknown as { window: unknown }).window = {
    __tynd__: {
      os_call(api: string, method: string, args: unknown) {
        const call = { api, method, args }
        calls.push(call)
        return Promise.resolve(handler(call))
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
  return calls
}

describe("compute client", () => {
  let calls: Call[]
  beforeEach(() => {
    calls = mountShim((c) => {
      if (c.method === "hash") return "deadbeef"
      if (c.method === "compress") return "Y29tcHJlc3NlZA==" // "compressed"
      if (c.method === "decompress") return { data: "aGk=", bytes: 2 }
      if (c.method === "randomBytes") return "AAECAw==" // 0,1,2,3
      return null
    })
  })

  test("hash encodes string input as UTF-8 base64", async () => {
    await compute.hash("abc")
    expect(calls).toHaveLength(1)
    const args = calls[0]!.args as { data: string; algo: string; encoding: string }
    expect(args.algo).toBe("blake3")
    expect(args.encoding).toBe("hex")
    expect(atob(args.data)).toBe("abc")
  })

  test("hash forwards raw Uint8Array without double-encoding", async () => {
    const bytes = new Uint8Array([0xde, 0xad, 0xbe, 0xef])
    await compute.hash(bytes, { algo: "sha256", encoding: "base64" })
    const args = calls[0]!.args as { data: string; algo: string; encoding: string }
    expect(args.algo).toBe("sha256")
    expect(args.encoding).toBe("base64")
    const decoded = Uint8Array.from(atob(args.data), (c) => c.charCodeAt(0))
    expect(decoded).toEqual(bytes)
  })

  test("compress returns decoded bytes", async () => {
    const out = await compute.compress(new Uint8Array([1, 2, 3]))
    expect(out).toBeInstanceOf(Uint8Array)
    expect(new TextDecoder().decode(out)).toBe("compressed")
  })

  test("decompress unwraps the .data field", async () => {
    const out = await compute.decompress(new Uint8Array([1]))
    expect(new TextDecoder().decode(out)).toBe("hi")
  })

  test("randomBytes returns Uint8Array of requested length", async () => {
    const out = await compute.randomBytes(4)
    expect(out).toEqual(new Uint8Array([0, 1, 2, 3]))
    const args = calls[0]!.args as { n: number }
    expect(args.n).toBe(4)
  })
})
