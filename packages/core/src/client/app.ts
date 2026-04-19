import { osCall } from "./_internal.ts"

export interface AppInfo {
  name?: string
  version?: string
}

export const app = {
  /** App display name. Defaults to the binary's file stem. */
  getName(): Promise<string> {
    return osCall("app", "getName")
  },
  /** App version. Defaults to `"0.0.0"` unless the backend calls `setInfo`. */
  getVersion(): Promise<string> {
    return osCall("app", "getVersion")
  },
  /**
   * Set app name and/or version at runtime. Typically called once from the
   * backend at startup (e.g. with `package.json` fields) so the frontend can
   * query them via `getName` / `getVersion`.
   */
  setInfo(info: AppInfo): Promise<void> {
    return osCall("app", "setInfo", info)
  },
  /** Exit the process with the given code (default `0`). Fires cleanup first. */
  exit(code?: number): Promise<void> {
    return osCall("app", "exit", code !== undefined ? { code } : {})
  },
  /** Relaunch the current binary, then exit the running process. */
  relaunch(): Promise<void> {
    return osCall("app", "relaunch")
  },
}
