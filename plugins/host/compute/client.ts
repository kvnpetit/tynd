import { base64ToBytes, binExchangeText, osCall } from "@tynd/core/client/bridge";

/** Hash algorithm accepted by `compute.hash`. */
export type HashAlgo = "blake3" | "sha256" | "sha384" | "sha512";

/** Frontend bindings for the host's `compute` API (hashing + crypto-strength randomness). */
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
    );
  },
  /** Generate `n` cryptographically random bytes from the host's CSPRNG. */
  async randomBytes(n: number): Promise<Uint8Array> {
    const b64 = await osCall<string>("compute", "randomBytes", { n });
    return base64ToBytes(b64);
  },
};
