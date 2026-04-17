import { base64ToBytes, binExchange, binExchangeText, osCall } from "./_internal.js"

export type HashAlgo = "blake3" | "sha256" | "sha512"
export type CompressAlgo = "zstd"

export const compute = {
  hash(
    data: string | Uint8Array,
    opts?: { algo?: HashAlgo; encoding?: "hex" | "base64" },
  ): Promise<string> {
    const bytes = typeof data === "string" ? new TextEncoder().encode(data) : data
    return binExchangeText(
      "compute/hash",
      { algo: opts?.algo ?? "blake3", encoding: opts?.encoding ?? "hex" },
      bytes,
    )
  },
  compress(data: Uint8Array, opts?: { algo?: CompressAlgo; level?: number }): Promise<Uint8Array> {
    const query: Record<string, string> = { algo: opts?.algo ?? "zstd" }
    if (opts?.level !== undefined) query["level"] = String(opts.level)
    return binExchange("compute/compress", query, data)
  },
  decompress(data: Uint8Array, opts?: { algo?: CompressAlgo }): Promise<Uint8Array> {
    return binExchange("compute/decompress", { algo: opts?.algo ?? "zstd" }, data)
  },
  async randomBytes(n: number): Promise<Uint8Array> {
    const b64 = await osCall<string>("compute", "randomBytes", { n })
    return base64ToBytes(b64)
  },
}
