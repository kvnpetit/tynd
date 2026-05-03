/**
 * Auto-updater client. Two layers in one module:
 *
 *  1. Low-level dispatch shims (`updaterCall`, `updaterIpc`) — direct
 *     mirrors of the host's `os.updater.*` arms. Same shape as every
 *     other `client/*.ts` module so `osCall` stays the canonical IPC
 *     entry point.
 *  2. High-level `updater` facade — Sparkle / Squirrel / electron-updater
 *     style. Reads `globalThis.__TYND_UPDATER_CONFIG__` (inlined by the
 *     CLI from `tynd.config.ts::updater`) and chooses patch-vs-full,
 *     auto-download, auto-apply, and major-bump prompting based on the
 *     declared user preferences. The host stays a dumb byte-mover; UX
 *     policy lives here so a future `updater.config.set(...)` override
 *     surface stays a single in-process call.
 *
 * Trust model: the manifest itself is unsigned; each platform entry
 * carries an Ed25519 signature over the binary bytes, verified against
 * the pub-key the app bakes in at build time. See `os/updater.rs` for
 * the full flow + double-verify on patch reconstructions.
 */

import { osCall } from "./_internal";

// ---------------------------------------------------------------------------
// Low-level IPC shapes — kept stable for callers that want to drive the host
// directly (tests, custom UX flows that bypass the high-level facade).
// ---------------------------------------------------------------------------

export interface RawUpdateInfo {
  version: string;
  notes: string | null;
  pubDate: string | null;
  url: string;
  signature: string;
  platform: string;
  /**
   * Patch entry (BSDIFF + zstd) when the manifest carries a delta from
   * the running version. `null` falls back to the full-binary path.
   */
  patch: PatchEntry | null;
  /** Optional `size` from the per-platform manifest entry. */
  size?: number;
}

export interface PatchEntry {
  from: string;
  url: string;
  signature: string;
  size?: number;
  blake3?: string;
  compression?: "zstd" | "none";
}

export interface UpdaterCheckOptions {
  endpoint: string;
  currentVersion: string;
  headers?: Record<string, string>;
  proxy?: string;
  allowDowngrade?: boolean;
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
  pubKey: string;
  progressId?: string;
  headers?: Record<string, string>;
  proxy?: string;
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
  path: string;
  size: number;
  /** BLAKE3 hex of the *verified* bytes — required by `install`. */
  hash: string;
  /** Patch byte size when the path went through `downloadAndPatch`. */
  patchSize?: number;
}

export interface UpdaterInstallOptions {
  path: string;
  hash: string;
  relaunch?: boolean;
}

export interface UpdaterInstallResult {
  installed: boolean;
  path: string;
  relaunch: boolean;
}

// ---------------------------------------------------------------------------
// Low-level facade — direct passthrough. Kept exported under the same name
// as before so external callers keep working.
// ---------------------------------------------------------------------------

const lowLevel = {
  async check(options: UpdaterCheckOptions): Promise<RawUpdateInfo | null> {
    const result = await osCall<{ available: boolean } & RawUpdateInfo>(
      "updater",
      "check",
      options,
    );
    return result.available ? result : null;
  },

  downloadAndVerify(options: UpdaterDownloadOptions): Promise<UpdaterDownloadResult> {
    return osCall("updater", "downloadAndVerify", options);
  },

  downloadAndPatch(options: UpdaterPatchDownloadOptions): Promise<UpdaterDownloadResult> {
    return osCall("updater", "downloadAndPatch", options);
  },

  install(options: UpdaterInstallOptions): Promise<UpdaterInstallResult> {
    return osCall("updater", "install", options);
  },

  async startPeriodicCheck(options: UpdaterPeriodicOptions): Promise<number> {
    const { id } = await osCall<{ id: number }>("updater", "startPeriodicCheck", options);
    return id;
  },

  stopPeriodicCheck(): Promise<void> {
    return osCall("updater", "stopPeriodicCheck");
  },

  validateConfig(cfg: UpdaterRuntimeConfig): Promise<unknown> {
    return osCall("updater", "validateConfig", cfg);
  },

  onProgress(handler: (p: UpdaterProgress) => void): () => void {
    return window.__tynd__.os_on("updater:progress", (raw) => handler(raw as UpdaterProgress));
  },

  onCheck(handler: (event: UpdaterCheckEvent) => void): () => void {
    return window.__tynd__.os_on("updater:check", (raw) => handler(raw as UpdaterCheckEvent));
  },
};

// ---------------------------------------------------------------------------
// High-level facade — preference-aware, event-driven. Reads its config from
// `globalThis.__TYND_UPDATER_CONFIG__` (CLI-injected at build time) and
// applies user preferences before falling through to the low-level IPC.
// ---------------------------------------------------------------------------

/** Mirror of `UpdaterSchema` in `cli/src/lib/config/schema.ts` (post-resolve). */
export interface UpdaterRuntimeConfig {
  endpoint: string;
  publicKey: string;
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

interface UpdaterListeners {
  checking: Set<() => void>;
  "update-available": Set<(info: UpdateInfo) => void>;
  "update-not-available": Set<() => void>;
  "download-progress": Set<(p: DownloadProgress) => void>;
  "update-downloaded": Set<(info: UpdateInfo) => void>;
  "update-applied": Set<() => void>;
  error: Set<(err: Error) => void>;
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

  // Events
  on(event: "checking", listener: () => void): () => void;
  on(event: "update-available", listener: (info: UpdateInfo) => void): () => void;
  on(event: "update-not-available", listener: () => void): () => void;
  on(event: "download-progress", listener: (p: DownloadProgress) => void): () => void;
  on(event: "update-downloaded", listener: (info: UpdateInfo) => void): () => void;
  on(event: "update-applied", listener: () => void): () => void;
  on(event: "error", listener: (err: Error) => void): () => void;
}

/**
 * Read the build-time config injected by the CLI. Returns `null` when no
 * `updater` block was declared in `tynd.config.ts` — in that case the
 * facade still exposes its methods but every call is a no-op that emits
 * an `error` event so a misuse surfaces loudly instead of silently
 * succeeding.
 */
function readBakedConfig(): UpdaterRuntimeConfig | null {
  const g = globalThis as { __TYND_UPDATER_CONFIG__?: UpdaterRuntimeConfig };
  return g.__TYND_UPDATER_CONFIG__ ?? null;
}

/** Best-effort major-bump detection (semver core only — `1.x → 2.0` etc.). */
function isMajorBump(current: string, next: string): boolean {
  const trim = (s: string) => s.trim().replace(/^v/i, "");
  const a = trim(current).split(".")[0];
  const b = trim(next).split(".")[0];
  if (!a || !b) return false;
  const an = Number(a);
  const bn = Number(b);
  if (Number.isNaN(an) || Number.isNaN(bn)) return false;
  return bn > an;
}

/**
 * Resolve the running app version. Frontends can override via a `<meta>`
 * tag (`<meta name="tynd:version" content="1.2.3">`) — useful when the
 * backend itself doesn't yet plumb the value through. Falls back to the
 * `TYND_VERSION` global (CLI sets it in dev mode) and finally to the
 * literal string `"0.0.0"` so checks never throw on a missing version.
 */
function detectAppVersion(): string {
  const g = globalThis as { TYND_VERSION?: string; __TYND_VERSION__?: string };
  if (typeof g.TYND_VERSION === "string" && g.TYND_VERSION) return g.TYND_VERSION;
  if (typeof g.__TYND_VERSION__ === "string" && g.__TYND_VERSION__) return g.__TYND_VERSION__;
  if (typeof document !== "undefined") {
    const meta = document.querySelector('meta[name="tynd:version"]');
    const v = meta?.getAttribute("content");
    if (v) return v;
  }
  return "0.0.0";
}

class UpdaterFacade implements Updater {
  /**
   * Config is re-read per call so unit tests (and dev-mode injections
   * via `globalThis.__TYND_UPDATER_CONFIG__ = …`) take effect without a
   * page reload. The CLI's build-time `define` substitution still pins
   * the value at bundle time for production builds.
   */
  get #cfg(): UpdaterRuntimeConfig | null {
    return readBakedConfig();
  }
  get #version(): string {
    return detectAppVersion();
  }
  #listeners: UpdaterListeners = {
    checking: new Set(),
    "update-available": new Set(),
    "update-not-available": new Set(),
    "download-progress": new Set(),
    "update-downloaded": new Set(),
    "update-applied": new Set(),
    error: new Set(),
  };
  /** Most recent `checkForUpdates()` result, used by `download()` / `apply()`. */
  #pending: { info: UpdateInfo; raw: RawUpdateInfo } | null = null;
  /** Result of the most recent download — needed by `apply()`. */
  #staged: UpdaterDownloadResult | null = null;
  #downloadProgress: { downloaded: number; total: number | null } | null = null;
  #lastProgressAt: number = 0;
  #lastProgressBytes: number = 0;
  /** ID returned by `startPeriodicCheck` — kept so `cancel()` can stop it. */
  #periodicId: number | null = null;
  /** Subscription handles for the host-side progress + check streams. */
  #unsubProgress: (() => void) | null = null;

  constructor() {
    // Wire the progress bus eagerly when the config is present at
    // construction; otherwise it auto-wires on first method call (when
    // a dev / test injects the config later).
    if (this.#cfg) this.#wireProgressBus();
  }

  // ---- public API ----------------------------------------------------------

  getConfig(): UpdaterRuntimeConfig | null {
    return this.#cfg;
  }

  async getCurrentVersion(): Promise<string> {
    return this.#version;
  }

  async getAvailableVersion(): Promise<string | null> {
    return this.#pending?.info.version ?? null;
  }

  getDownloadProgress(): { downloaded: number; total: number | null } | null {
    return this.#downloadProgress;
  }

  on<E extends UpdaterEvent>(
    event: E,
    listener: UpdaterListeners[E] extends Set<infer Fn> ? Fn : never,
  ): () => void {
    const set = this.#listeners[event] as Set<unknown>;
    set.add(listener as unknown);
    return () => {
      set.delete(listener as unknown);
    };
  }

  async checkForUpdates(): Promise<UpdateInfo> {
    const cfg = this.#requireConfig("checkForUpdates");
    this.#emit("checking");
    const raw = await lowLevel
      .check({ endpoint: cfg.endpoint, currentVersion: this.#version })
      .catch((e) => {
        this.#emitError(e);
        throw e;
      });
    if (!raw) {
      this.#emit("update-not-available");
      const noUpdate: UpdateInfo = { available: false, currentVersion: this.#version };
      this.#pending = null;
      return noUpdate;
    }
    const major = isMajorBump(this.#version, raw.version);
    const route = this.#chooseRoute(raw);
    const info: UpdateInfo = {
      available: true,
      version: raw.version,
      currentVersion: this.#version,
      isMajorBump: major,
      downloadType: route.type,
      patchSize: route.type === "patch" ? route.size : undefined,
      fullSize: route.type === "full" ? raw.size : undefined,
      fromVersion: route.type === "patch" ? raw.patch?.from : undefined,
      notes: raw.notes,
      pubDate: raw.pubDate,
      raw,
    };
    this.#pending = { info, raw };
    this.#emit("update-available", info);
    return info;
  }

  async download(): Promise<void> {
    const cfg = this.#requireConfig("download");
    if (!this.#pending) {
      throw new Error("updater.download(): no update info — call checkForUpdates() first");
    }
    const { info, raw } = this.#pending;
    this.#downloadProgress = null;
    this.#staged = null;
    try {
      if (info.downloadType === "patch") {
        const patch = raw.patch;
        if (!patch) throw new Error("internal: patch route chosen without patch entry");
        this.#staged = await lowLevel.downloadAndPatch({
          url: patch.url,
          signature: patch.signature,
          releaseSignature: raw.signature,
          pubKey: cfg.publicKey,
          expectedBlake3: patch.blake3,
          compression: patch.compression,
        });
      } else {
        this.#staged = await lowLevel.downloadAndVerify({
          url: raw.url,
          signature: raw.signature,
          pubKey: cfg.publicKey,
        });
      }
    } catch (e) {
      this.#emitError(e);
      throw e;
    }
    this.#emit("update-downloaded", info);
    if (cfg.autoApply && (!info.isMajorBump || !cfg.requireMajorPrompt)) {
      // Silent / autoApply path: chain straight into the install + relaunch.
      // Detached so callers awaiting `download()` don't block on the OS
      // self-replace dance.
      void this.apply().catch((e) => this.#emitError(e));
    }
  }

  async apply(): Promise<void> {
    if (!this.#staged) {
      throw new Error("updater.apply(): nothing staged — call download() first");
    }
    try {
      await lowLevel.install({
        path: this.#staged.path,
        hash: this.#staged.hash,
        relaunch: true,
      });
      this.#emit("update-applied");
    } catch (e) {
      this.#emitError(e);
      throw e;
    }
  }

  async cancel(): Promise<void> {
    if (this.#periodicId !== null) {
      this.#periodicId = null;
      await lowLevel.stopPeriodicCheck();
    }
    // The host's `download_and_verify` doesn't yet expose mid-stream
    // cancellation (the ureq agent has a 60s read timeout floor). Reset
    // local state so the next call doesn't see a stale staged file.
    this.#staged = null;
    this.#downloadProgress = null;
  }

  // ---- internals -----------------------------------------------------------

  /**
   * Decide between patch and full based on user preferences + manifest
   * shape. Returns `{ type, size }` so the caller can populate
   * `UpdateInfo.patchSize` / `fullSize` in one place.
   */
  #chooseRoute(raw: RawUpdateInfo): { type: "patch" | "full"; size?: number } {
    const cfg = this.#cfg;
    if (!cfg) return { type: "full", size: raw.size };
    const patch = raw.patch;
    const patchUsable =
      patch !== null &&
      patch !== undefined &&
      cfg.allowPatch &&
      cfg.preferPatch &&
      (cfg.maxPatchSize === null ||
        patch.size === undefined ||
        patch.size <= cfg.maxPatchSize);

    if (patchUsable && patch) {
      return { type: "patch", size: patch.size };
    }
    if (!cfg.fallbackToFull) {
      // Hard fail: user opted out of full DLs, emit a clear error before
      // the caller gets a confused "downloaded the full binary anyway"
      // surprise.
      throw new Error(
        "updater: no patch available and fallbackToFull is disabled — user must update manually",
      );
    }
    return { type: "full", size: raw.size };
  }

  #wireProgressBus(): void {
    if (typeof window === "undefined" || !window.__tynd__) return;
    this.#unsubProgress = lowLevel.onProgress((p) => {
      const total = p.total;
      const loaded = p.loaded;
      const now = Date.now();
      const dt = (now - this.#lastProgressAt) / 1000;
      const bytesPerSecond = dt > 0 ? Math.max(0, (loaded - this.#lastProgressBytes) / dt) : 0;
      this.#lastProgressAt = now;
      this.#lastProgressBytes = loaded;
      this.#downloadProgress = { downloaded: loaded, total };
      const percent = total ? loaded / total : Number.NaN;
      this.#emit("download-progress", { percent, bytesPerSecond, loaded, total });
    });
  }

  #requireConfig(method: string): UpdaterRuntimeConfig {
    if (!this.#cfg) {
      const err = new Error(
        `updater.${method}(): no updater block in tynd.config.ts — set { endpoint, publicKey } to enable`,
      );
      this.#emitError(err);
      throw err;
    }
    return this.#cfg;
  }

  #emit<E extends UpdaterEvent>(
    event: E,
    ...args: UpdaterListeners[E] extends Set<(...a: infer A) => void> ? A : never
  ): void {
    const set = this.#listeners[event] as Set<(...a: unknown[]) => void>;
    for (const fn of set) {
      try {
        fn(...(args as unknown[]));
      } catch {
        // listeners shouldn't throw — swallow so one bad subscriber
        // doesn't poison the rest of the dispatch.
      }
    }
  }

  #emitError(err: unknown): void {
    const e = err instanceof Error ? err : new Error(String(err));
    this.#emit("error", e);
  }
}

const facade = new UpdaterFacade();

/**
 * High-level updater facade — Sparkle / electron-updater style. Reads
 * preferences from `tynd.config.ts::updater` (inlined by the CLI at
 * build time) and exposes manual + event-driven controls.
 *
 * @example
 *   import { updater } from "@tynd/core/client/updater";
 *
 *   updater.on("update-available", (info) => {
 *     myCustomBanner.show(`v${info.version} ready`);
 *   });
 *   const info = await updater.checkForUpdates();
 *   if (info.available) {
 *     await updater.download();   // verifies + stages
 *     await updater.apply();      // self-replace + relaunch
 *   }
 */
export const updater: Updater = facade;

/**
 * Low-level IPC mirrors. Use these only when you need to drive the host
 * directly (tests, custom flows that bypass the high-level facade).
 */
export const updaterIpc = lowLevel;
