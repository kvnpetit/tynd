import { osCall } from "./_internal";

export interface KeyringEntry {
  /** Service namespace — typically the app's reverse-DNS identifier. */
  service: string;
  /** Distinguishes multiple credentials for the same service (user, label, …). */
  account: string;
}

export const keyring = {
  set(entry: KeyringEntry, password: string): Promise<void> {
    return osCall("keyring", "set", { ...entry, password });
  },
  get(entry: KeyringEntry): Promise<string | null> {
    return osCall("keyring", "get", entry);
  },
  delete(entry: KeyringEntry): Promise<boolean> {
    return osCall("keyring", "delete", entry);
  },
};
