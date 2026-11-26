import { osCall } from "@tynd/core/client/bridge";

/** Identifier pair used to locate a credential in the OS keyring. */
export interface KeyringEntry {
  /** Service namespace — typically the app's reverse-DNS identifier. */
  service: string;
  /** Distinguishes multiple credentials for the same service (user, label, …). */
  account: string;
}

/** Frontend bindings for the host's `keyring` API (OS-level secret store). */
export const keyring = {
  /** Store or overwrite the password for `entry`. */
  set(entry: KeyringEntry, password: string): Promise<void> {
    return osCall("keyring", "set", { ...entry, password });
  },
  /** Retrieve the password for `entry`, or `null` when none. */
  get(entry: KeyringEntry): Promise<string | null> {
    return osCall("keyring", "get", entry);
  },
  /** Remove the password for `entry`; returns whether anything was deleted. */
  delete(entry: KeyringEntry): Promise<boolean> {
    return osCall("keyring", "delete", entry);
  },
};

export default keyring;
