import { base64ToBytes, bytesToBase64, osCall } from "./_internal.js"

export type HashAlgo = "blake3" | "sha256" | "sha512"
export type CompressAlgo = "zstd"

export const compute = {
  async hash(
    data: string | Uint8Array,
    opts?: { algo?: HashAlgo; encoding?: "hex" | "base64" },
  ): Promise<string> {
    const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data
    return osCall("compute", "hash", {
      data: bytesToBase64(bytes),
      algo: opts?.algo ?? "blake3",
      encoding: opts?.encoding ?? "hex",
    }) as Promise<string>
  },
  async compress(
    data: Uint8Array,
    opts?: { algo?: CompressAlgo; level?: number },
  ): Promise<Uint8Array> {
    const b64 = await osCall<string>("compute", "compress", {
      data: bytesToBase64(data),
      algo: opts?.algo ?? "zstd",
      level: opts?.level,
    })
    return base64ToBytes(b64)
  },
  async decompress(data: Uint8Array, opts?: { algo?: CompressAlgo }): Promise<Uint8Array> {
    const res = await osCall<{ data: string; bytes: number }>("compute", "decompress", {
      data: bytesToBase64(data),
      algo: opts?.algo ?? "zstd",
    })
    return base64ToBytes(res.data)
  },
  async randomBytes(n: number): Promise<Uint8Array> {
    const b64 = await osCall<string>("compute", "randomBytes", { n })
    return base64ToBytes(b64)
  },
}
