import { osCall } from "./_internal";

/**
 * Plaintext payload returned by {@link vault.read}. `data` is base64 —
 * decode with `atob` (or `Uint8Array.from(atob(data), c => c.charCodeAt(0))`)
 * to get the raw bytes the file was written with.
 */
export interface VaultReadResult {
  data: string;
}

/**
 * Bytes accepted by {@link vault.write}. Strings go through as base64;
 * `Uint8Array` is base64-encoded on the fly so callers don't have to.
 */
export type VaultWriteData = string | Uint8Array;

function toBase64(data: VaultWriteData): string {
  if (typeof data === "string") return data;
  // Bun + modern browsers expose `btoa`. Use a binary-safe conversion that
  // handles arbitrary byte arrays (the naive String.fromCharCode + btoa
  // approach trips on > 64KB stacks in some engines).
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < data.length; i += chunk) {
    binary += String.fromCharCode(...data.subarray(i, i + chunk));
  }
  return btoa(binary);
}

/**
 * `vault` — encrypted file vault for secrets too big for the OS keychain.
 *
 * Backed by `cocoon` (AES-GCM-SIV / ChaCha20-Poly1305) on the host. The
 * password never leaves the JS heap (we forward it to Rust and the host
 * zeroes it after use) — store it somewhere safer than localStorage if
 * you intend to persist it across sessions.
 *
 * Path resolution goes through the same FS scope policy as `fs.*`, so an
 * unconfigured policy still confines reads/writes to the app's allowed
 * roots.
 */
export const vault = {
  /** Decrypt a vault file. Returns base64-encoded plaintext bytes. */
  read(path: string, password: string): Promise<VaultReadResult> {
    return osCall("vault", "read", { path, password });
  },
  /**
   * Encrypt and write `data` to `path` (atomic via tempfile + rename).
   * Accepts either base64 string or `Uint8Array`.
   */
  write(path: string, password: string, data: VaultWriteData): Promise<void> {
    return osCall("vault", "write", { path, password, data: toBase64(data) });
  },
  /** Whether `path` exists on disk (no decryption attempt). */
  exists(path: string): Promise<boolean> {
    return osCall("vault", "exists", { path });
  },
  /** Idempotent delete — succeeds if the file is already absent. */
  delete(path: string): Promise<void> {
    return osCall("vault", "delete", { path });
  },
};
