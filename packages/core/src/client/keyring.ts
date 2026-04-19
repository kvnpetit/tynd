import { osCall } from "./_internal.ts"

export interface KeyringEntry {
  /** Service namespace — typically the app's reverse-DNS identifier. */
  service: string
  /** Distinguishes multiple credentials for the same service (user, label, …). */
  account: string
}

export const keyring = {
  /** Store a secret in the OS keychain (Keychain / DPAPI / Secret Service). */
  set(entry: KeyringEntry, password: string): Promise<void> {
    return osCall("keyring", "set", { ...entry, password })
  },
  /** Read a secret. Resolves to `null` when the entry doesn't exist. */
  get(entry: KeyringEntry): Promise<string | null> {
    return osCall("keyring", "get", entry)
  },
  /** Remove a secret. Returns `true` if it existed. */
  delete(entry: KeyringEntry): Promise<boolean> {
    return osCall("keyring", "delete", entry)
  },
}
