import { osCall } from "./_internal.ts"

export interface UpdateInfo {
  /** New version string from the manifest (e.g. `"1.2.3"`). */
  version: string
  /** Release notes from the manifest, if the server included them. */
  notes: string | null
  /** `pub_date` from the manifest (ISO-8601 string), if present. */
  pubDate: string | null
  /** Download URL for the current platform's artifact. */
  url: string
  /** Base64-encoded Ed25519 signature over the artifact bytes. */
  signature: string
  /** Manifest platform key we matched (e.g. `"windows-x86_64"`). */
  platform: string
}

export interface UpdaterCheckOptions {
  /** URL of the `update.json` manifest. */
  endpoint: string
  /** Semver version of the running app (typically read from package.json). */
  currentVersion: string
}

export interface UpdaterProgress {
  /** Matches `id` echoed back in each event. */
  id?: string
  phase: "download" | "verified"
  loaded: number
  total: number | null
}

export interface UpdaterDownloadOptions {
  url: string
  signature: string
  /** Base64 Ed25519 public key the app was built with. */
  pubKey: string
  /** Optional id used to scope progress events for this download. */
  progressId?: string
}

export interface UpdaterDownloadResult {
  /** Absolute path to the verified download on disk. */
  path: string
  /** Size in bytes. */
  size: number
}

export const updater = {
  /**
   * Fetch the update manifest from `endpoint` and return the entry for the
   * running platform if its version is strictly newer than `currentVersion`.
   * Returns `null` when already up to date.
   */
  async check(options: UpdaterCheckOptions): Promise<UpdateInfo | null> {
    const result = await osCall<{ available: boolean } & UpdateInfo>("updater", "check", options)
    return result.available ? result : null
  },
  /**
   * Download the signed artifact, stream progress via `updater:progress`
   * events, and verify the Ed25519 signature against `pubKey` before
   * resolving. Rejects with a "signature check failed" error on tampering.
   */
  downloadAndVerify(options: UpdaterDownloadOptions): Promise<UpdaterDownloadResult> {
    return osCall("updater", "downloadAndVerify", options)
  },
  /** Subscribe to throttled download + verification progress events. */
  onProgress(handler: (p: UpdaterProgress) => void): () => void {
    return window.__tynd__.os_on("updater:progress", (raw) => handler(raw as UpdaterProgress))
  },
}
