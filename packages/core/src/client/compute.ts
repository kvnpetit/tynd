import { base64ToBytes, binExchangeText, osCall } from "./_internal.js"

export type HashAlgo = "blake3" | "sha256" | "sha384" | "sha512"

export const compute = {
  /**
   * Hash raw bytes. Always returns the digest as a base64 string.
   * Convert to hex in userland if needed (one-liner):
   *   `[...atob(digest)].map(c => c.charCodeAt(0).toString(16).padStart(2,'0')).join('')`
   */
  hash(data: Uint8Array, opts?: { algo?: HashAlgo }): Promise<string> {
    return binExchangeText(
      "compute/hash",
      { algo: opts?.algo ?? "blake3", encoding: "base64" },
      data,
    )
  },
  async randomBytes(n: number): Promise<Uint8Array> {
    const b64 = await osCall<string>("compute", "randomBytes", { n })
    return base64ToBytes(b64)
  },
}
