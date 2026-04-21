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
  /** Custom HTTP headers forwarded to the manifest request. */
  headers?: Record<string, string>
  /** Proxy URL (http / socks5). Default: system proxy. */
  proxy?: string
  /** When `true`, accept manifests that advertise an older version. */
  allowDowngrade?: boolean
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
  /** Custom HTTP headers forwarded to the download request. */
  headers?: Record<string, string>
  /** Proxy URL (http / socks5). Default: system proxy. */
  proxy?: string
}

export interface UpdaterPeriodicOptions extends UpdaterCheckOptions {
  /** Milliseconds between checks. Minimum 1000; default 1 hour. */
  intervalMs?: number
}

export type UpdaterCheckEvent =
  | { id: number; ok: true; info: UpdateInfo | { available: false } }
  | { id: number; ok: false; error: string }

export interface UpdaterDownloadResult {
  /** Absolute path to the verified download on disk. */
  path: string
  /** Size in bytes. */
  size: number
}

export interface UpdaterInstallOptions {
  /** Path returned by `downloadAndVerify`. */
  path: string
  /** Relaunch the app after swapping the binary. Defaults to `true`. */
  relaunch?: boolean
}

export interface UpdaterInstallResult {
  installed: boolean
  /** Final on-disk path of the running binary after the swap. */
  path: string
  relaunch: boolean
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
  /**
   * Swap the downloaded binary for the current one and (by default) relaunch.
   * Windows: delegates to a short cmd script so the running `.exe` can be
   * replaced after it exits. Linux AppImage: `rename` + spawn + exit. macOS:
   * not yet — callers must manage the `.app` swap themselves for now.
   */
  install(options: UpdaterInstallOptions): Promise<UpdaterInstallResult> {
    return osCall("updater", "install", options)
  },

  /**
   * Run `check` on a timer. Emits `updater:check` with either the result
   * or an error. Single task at a time — calling twice returns the
   * existing id. Stop with `stopPeriodicCheck`.
   */
  async startPeriodicCheck(options: UpdaterPeriodicOptions): Promise<number> {
    const { id } = await osCall<{ id: number }>("updater", "startPeriodicCheck", options)
    return id
  },
  stopPeriodicCheck(): Promise<void> {
    return osCall("updater", "stopPeriodicCheck")
  },
  onCheck(handler: (event: UpdaterCheckEvent) => void): () => void {
    return window.__tynd__.os_on("updater:check", (raw) => handler(raw as UpdaterCheckEvent))
  },
}
