import { describe, expect, test } from "bun:test"
import { mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import { keygen, sign } from "./signer.ts"

describe("signer", () => {
  test("keygen writes pkcs8 private + base64 public, sign round-trips via WebCrypto", async () => {
    const dir = mkdtempSync(path.join(tmpdir(), "tynd-signer-"))
    try {
      const base = path.join(dir, "key")
      await keygen({ out: base, force: true })

      const privDer = await Bun.file(`${base}.key`).bytes()
      const pubB64 = (await Bun.file(`${base}.pub`).text()).trim()
      const pubRaw = Buffer.from(pubB64, "base64")
      expect(pubRaw.length).toBe(32)

      // Sign a fixed payload and verify with the exported public key — same
      // code path the Rust host would exercise at runtime.
      const payload = path.join(dir, "payload.bin")
      await Bun.write(payload, new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]))
      const sigPath = path.join(dir, "payload.sig")
      await sign({ file: payload, key: `${base}.key`, out: sigPath })

      const sigBytes = Buffer.from((await Bun.file(sigPath).text()).trim(), "base64")
      expect(sigBytes.length).toBe(64)

      const pubKey = await crypto.subtle.importKey("raw", pubRaw, "Ed25519", false, ["verify"])
      const ok = await crypto.subtle.verify(
        "Ed25519",
        pubKey,
        sigBytes,
        await Bun.file(payload).bytes(),
      )
      expect(ok).toBe(true)

      // Confirm PKCS#8 imports cleanly too.
      await crypto.subtle.importKey("pkcs8", privDer, "Ed25519", false, ["sign"])
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
