// Low-level IPC shapes — kept stable for callers that want to drive the host
// directly (tests, custom UX flows that bypass the high-level facade).

export interface RawUpdateInfo {
  version: string;
  notes: string | null;
  pubDate: string | null;
  url: string;
  signature: string;
  platform: string;
  /** Installation payload shape authenticated by the signed manifest. */
  format: "binary" | "app-archive";
  /**
   * Patch entry (BSDIFF + zstd) when the manifest carries a delta from
   * the running version. `null` falls back to the full-binary path.
   */
  patch: PatchEntry | null;
  /** Optional `size` from the per-platform manifest entry. */
  size?: number;
  /** Release channel the manifest was published to (`null` for older manifests). */
  channel?: string | null;
  /** Staged-rollout percentage (0..100) declared by the manifest, if any. */
  rollout?: number | null;
}

export interface PatchEntry {
  from: string;
  url: string;
  signature: string;
  size?: number;
  blake3?: string;
  compression?: "zstd" | "none";
}

/**
 * Options for the low-level `updaterIpc.check()`. Endpoints, channel and the
 * current version always come from the host-trusted config and `app.setInfo`;
 * the only option is the signature requirement, which cannot be relaxed.
 */
export interface UpdaterCheckOptions {
  /** Always enforced; `false` is refused by the host. */
  requireSignature?: true;
}

export interface UpdaterProgress {
  id?: string;
  phase: "download" | "verified" | "verified-patch" | "applying";
  loaded: number;
  total: number | null;
}

export interface UpdaterDownloadOptions {
  url: string;
  signature: string;
  /** Must match the value authorized by the last verified manifest. */
  format?: "binary" | "app-archive";
  progressId?: string;
}

export interface UpdaterPatchDownloadOptions extends UpdaterDownloadOptions {
  /** Ed25519 over the *reconstructed* binary (release-level signature). */
  releaseSignature: string;
  expectedBlake3?: string;
  compression?: "zstd" | "none";
}

export interface UpdaterPeriodicOptions extends UpdaterCheckOptions {
  intervalMs?: number;
}

export type UpdaterCheckEvent =
  | { id: number; ok: true; info: RawUpdateInfo | { available: false } }
  | { id: number; ok: false; error: string };

export interface UpdaterDownloadResult {
  /** Opaque, process-local capability for one install attempt. */
  handle: string;
  size: number;
  /** Patch byte size when the path went through `downloadAndPatch`. */
  patchSize?: number;
}

export interface UpdaterInstallOptions {
  handle: string;
  relaunch?: boolean;
}

export interface UpdaterInstallResult {
  installed: boolean;
  path: string;
  /** `true` when the new version was started and this process is exiting. */
  relaunch: boolean;
  /**
   * Set when the swap succeeded but starting the new version failed. The
   * update is installed and runs on the next launch.
   */
  relaunchError?: string;
}

/** Mirror of Rust `ResolvedUpdaterConfig` after native config resolution. */
export interface UpdaterRuntimeConfig {
  /** Mirrors tried in order, with `{{channel}}` / `{{target}}` / `{{arch}}` / `{{current_version}}` templating. */
  endpoints: string[];
  /** Trusted verifying keys; any one may verify a signature (key rotation). */
  publicKeys: string[];
  /** Default release channel; `"stable"` when unset. */
  channel?: string;
  preferPatch: boolean;
  allowPatch: boolean;
  /** `null` = no cap. Always pre-parsed to bytes by the CLI side. */
  maxPatchSize: number | null;
  fallbackToFull: boolean;
  autoCheck: boolean;
  checkIntervalSecs: number;
  checkOnStartup: boolean;
  autoDownload: boolean;
  autoApply: boolean;
  promptStyle: "modal" | "tray" | "silent" | "manual";
  requireSignature: boolean;
  requireMajorPrompt: boolean;
}

/**
 * What the backend hands the host through `updater.configure` (backend-only; a renderer cannot
 * call it): the resolved updater config plus optional network settings for private update
 * servers. `proxy` and `headers` are never part of the renderer-visible config and any value a
 * renderer passes to `check`/`download*` is ignored.
 */
export interface UpdaterConfigureConfig extends UpdaterRuntimeConfig {
  /**
   * `http://` or `https://` proxy URL for updater traffic. Without it the host falls back to the
   * `HTTPS_PROXY` / `ALL_PROXY` environment variables.
   */
  proxy?: string;
  /**
   * Fixed request headers (e.g. `Authorization`). Sent only to the origins of the configured
   * endpoints, never on a cross-origin redirect. Transport-owned names (`Host`, `Content-Length`,
   * `Transfer-Encoding`, `Connection`, `Proxy-Authorization`, ...) are rejected.
   */
  headers?: Record<string, string>;
}

/** What `updater.checkForUpdates()` resolves with. */
export interface UpdateInfo {
  available: boolean;
  version?: string;
  currentVersion: string;
  isMajorBump?: boolean;
  /** Selected route after applying user preferences. */
  downloadType?: "patch" | "full";
  patchSize?: number;
  fullSize?: number;
  fromVersion?: string;
  notes?: string | null;
  pubDate?: string | null;
  /** Cached raw manifest entry (callers wiring custom UX). */
  raw?: RawUpdateInfo;
}

export type UpdaterEvent =
  | "checking"
  | "update-available"
  | "update-not-available"
  | "download-progress"
  | "update-downloaded"
  | "update-applied"
  | "error";

export interface DownloadProgress {
  /** 0..1 (or NaN when total is unknown). */
  percent: number;
  bytesPerSecond: number;
  loaded: number;
  total: number | null;
}

export interface Updater {
  // Manual control
  checkForUpdates(): Promise<UpdateInfo>;
  download(): Promise<void>;
  apply(): Promise<void>;
  cancel(): Promise<void>;

  // State
  getCurrentVersion(): Promise<string>;
  getAvailableVersion(): Promise<string | null>;
  getDownloadProgress(): { downloaded: number; total: number | null } | null;
  getConfig(): UpdaterRuntimeConfig | null;

  // Channels
  /**
   * Current release channel: the one chosen with `setChannel()`, else the
   * configured default, else `"stable"`.
   */
  getChannel(): Promise<string>;
  /**
   * Switch release channel (letters, digits, `.`, `_`, `-`; max 64 chars).
   * Persisted per install. Discards any pending update found on the old channel.
   */
  setChannel(name: string): Promise<void>;

  // Events
  on(event: "checking", listener: () => void): () => void;
  on(event: "update-available", listener: (info: UpdateInfo) => void): () => void;
  on(event: "update-not-available", listener: () => void): () => void;
  on(event: "download-progress", listener: (p: DownloadProgress) => void): () => void;
  on(event: "update-downloaded", listener: (info: UpdateInfo) => void): () => void;
  on(event: "update-applied", listener: () => void): () => void;
  on(event: "error", listener: (err: Error) => void): () => void;
}
