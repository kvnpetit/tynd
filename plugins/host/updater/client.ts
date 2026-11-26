/**
 * High-level updater facade. Reads build-time policy, chooses patch/full routes,
 * tracks state, and emits UX events. Host IPC transport lives in `updater-ipc.ts`;
 * shared public contracts live in `updater-types.ts`.
 *
 * Manifest and payload signatures are verified by native host. See
 * `crates/host/src/api/updater.rs` for manifest verification and patch reconstruction checks.
 */

import { app } from "@tynd/core/client";
import { assertValidChannel } from "./updater-config";
import { updaterIpc as lowLevel } from "./updater-ipc";
import type {
  DownloadProgress,
  RawUpdateInfo,
  UpdateInfo,
  Updater,
  UpdaterCheckEvent,
  UpdaterDownloadResult,
  UpdaterEvent,
  UpdaterRuntimeConfig,
} from "./updater-types";

export { assertValidChannel, DEFAULT_UPDATER_CHANNEL } from "./updater-config";
export { updaterIpc } from "./updater-ipc";
export type {
  DownloadProgress,
  PatchEntry,
  RawUpdateInfo,
  UpdateInfo,
  Updater,
  UpdaterCheckEvent,
  UpdaterCheckOptions,
  UpdaterConfigureConfig,
  UpdaterDownloadOptions,
  UpdaterDownloadResult,
  UpdaterEvent,
  UpdaterInstallOptions,
  UpdaterInstallResult,
  UpdaterPatchDownloadOptions,
  UpdaterPeriodicOptions,
  UpdaterProgress,
  UpdaterRuntimeConfig,
} from "./updater-types";

// High-level facade — preference-aware, event-driven. Reads its config from
// `globalThis.__TYND_UPDATER_CONFIG__` (CLI-injected at build time) and
// applies user preferences before falling through to the low-level IPC.

interface UpdaterListeners {
  checking: Set<() => void>;
  "update-available": Set<(info: UpdateInfo) => void>;
  "update-not-available": Set<() => void>;
  "download-progress": Set<(p: DownloadProgress) => void>;
  "update-downloaded": Set<(info: UpdateInfo) => void>;
  "update-applied": Set<() => void>;
  error: Set<(err: Error) => void>;
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
 * Local fallback for the running app version, used when the host did not report
 * one (`app.getVersion()` is authoritative). Frontends can override via a `<meta>`
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
  /**
   * Version the host reports (`app.getVersion()`, declared by the backend via
   * `app.setInfo`). Refreshed by `#refreshVersion()` before every check; `null`
   * until the host reported a real one.
   */
  #hostVersion: string | null = null;
  get #version(): string {
    return this.#hostVersion ?? detectAppVersion();
  }
  /**
   * Ask the host for the app version so packaged apps (where no `TYND_VERSION`
   * global or `<meta>` tag exists) do not fall back to `0.0.0`. The host's
   * unset default (`"0.0.0"`) and transport errors keep the local detection.
   */
  async #refreshVersion(): Promise<string> {
    try {
      const v = await app.getVersion();
      if (typeof v === "string" && v.trim() !== "" && v !== "0.0.0") this.#hostVersion = v;
    } catch {
      /* host unreachable: keep the locally detected version */
    }
    return this.#version;
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
  #unsubCheck: (() => void) | null = null;
  /** Delay before the periodic checks begin when `checkOnStartup` is off. */
  #periodicTimer: ReturnType<typeof setTimeout> | null = null;
  /** Version of the update currently staged by `download()`. */
  #stagedVersion: string | null = null;
  #automationStarted = false;

  constructor() {
    // Wire the progress bus eagerly when the config is present at
    // construction; otherwise it auto-wires on first method call (when
    // a dev / test injects the config later).
    if (this.#cfg) {
      this.#wireProgressBus();
      this.#startAutomation();
    }
  }

  // ---- public API ----------------------------------------------------------

  getConfig(): UpdaterRuntimeConfig | null {
    return this.#cfg;
  }

  async getChannel(): Promise<string> {
    return lowLevel.getChannel(this.#cfg?.channel);
  }

  async setChannel(name: string): Promise<void> {
    assertValidChannel(name);
    await lowLevel.setChannel(name);
    // An update found on the previous channel no longer applies.
    this.#pending = null;
    this.#staged = null;
  }

  async getCurrentVersion(): Promise<string> {
    return this.#refreshVersion();
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
    this.#requireConfig("checkForUpdates");
    this.#emit("checking");
    await this.#refreshVersion();
    const raw = await lowLevel.check().catch((e) => {
      this.#emitError(e);
      throw e;
    });
    return this.#settleCheck(raw);
  }

  /**
   * Record the outcome of a check (manual, startup or periodic): refresh the
   * pending update and emit the matching event. The previous pending update is
   * always dropped first, so a failure here can never leave a stale one that a
   * later `download()` would install.
   */
  #settleCheck(raw: RawUpdateInfo | null): UpdateInfo {
    this.#pending = null;
    if (!raw) {
      this.#emit("update-not-available");
      return { available: false, currentVersion: this.#version };
    }
    const major = isMajorBump(this.#version, raw.version);
    let route: { type: "patch" | "full"; size?: number };
    try {
      route = this.#chooseRoute(raw);
    } catch (e) {
      this.#emitError(e);
      throw e;
    }
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
    this.#stagedVersion = null;
    try {
      if (info.downloadType === "patch") {
        const patch = raw.patch;
        if (!patch) throw new Error("internal: patch route chosen without patch entry");
        this.#staged = await lowLevel.downloadAndPatch({
          url: patch.url,
          signature: patch.signature,
          releaseSignature: raw.signature,
          expectedBlake3: patch.blake3,
          compression: patch.compression,
        });
      } else {
        this.#staged = await lowLevel.downloadAndVerify({
          url: raw.url,
          signature: raw.signature,
          format: raw.format,
        });
      }
    } catch (e) {
      this.#emitError(e);
      throw e;
    }
    this.#stagedVersion = info.version ?? null;
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
        handle: this.#staged.handle,
        relaunch: true,
      });
      this.#emit("update-applied");
    } catch (e) {
      this.#emitError(e);
      throw e;
    }
  }

  async cancel(): Promise<void> {
    if (this.#periodicTimer !== null) {
      clearTimeout(this.#periodicTimer);
      this.#periodicTimer = null;
    }
    if (this.#unsubCheck) {
      this.#unsubCheck();
      this.#unsubCheck = null;
    }
    if (this.#periodicId !== null) {
      this.#periodicId = null;
      await lowLevel.stopPeriodicCheck();
    }
    if (this.#unsubProgress) {
      this.#unsubProgress();
      this.#unsubProgress = null;
    }
    // The host's `download_and_verify` doesn't yet expose mid-stream
    // cancellation (the ureq agent has a 60s read timeout floor). Reset
    // local state so the next call doesn't see a stale staged file.
    this.#staged = null;
    this.#stagedVersion = null;
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
      (cfg.maxPatchSize === null || patch.size === undefined || patch.size <= cfg.maxPatchSize);

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

  /**
   * Honour `autoCheck` / `checkInterval` / `checkOnStartup` / `autoDownload`.
   * Runs once, in the main window only (secondary windows would repeat the
   * checks and downloads), as soon as the updater module is loaded.
   * - `autoCheck`: the host runs a periodic check every `checkIntervalSecs`; the
   *   host's first check happens immediately, so it is delayed by one interval
   *   unless `checkOnStartup` is also set.
   * - `checkOnStartup` alone: a single check now.
   * - `autoDownload`: every check that finds an update chains into `download()`
   *   (skipped for a major bump while `requireMajorPrompt` is set).
   */
  #startAutomation(): void {
    const cfg = this.#cfg;
    if (this.#automationStarted || !cfg) return;
    if (typeof window === "undefined" || !window.__tynd__) return;
    const label = window.__TYND_WINDOW_LABEL__;
    if (label !== undefined && label !== "main") return;
    if (!cfg.autoCheck && !cfg.checkOnStartup) return;
    this.#automationStarted = true;

    const intervalMs = cfg.checkIntervalSecs * 1000;
    if (cfg.autoCheck && intervalMs > 0) {
      this.#unsubCheck = lowLevel.onCheck((event) => this.#onPeriodicCheck(event));
      const begin = () => {
        this.#periodicTimer = null;
        // The version is refreshed first: periodic results are settled against it.
        this.#refreshVersion()
          .then(() => lowLevel.startPeriodicCheck({ intervalMs }))
          .then(
            (id) => {
              this.#periodicId = id;
            },
            (e) => this.#emitError(e),
          );
      };
      if (cfg.checkOnStartup) begin();
      else this.#periodicTimer = setTimeout(begin, Math.min(intervalMs, MAX_TIMER_MS));
    } else if (cfg.checkOnStartup) {
      this.checkForUpdates().then(
        (info) => this.#maybeAutoDownload(info),
        () => {
          /* already reported through the "error" event */
        },
      );
    }
  }

  #onPeriodicCheck(event: UpdaterCheckEvent): void {
    if (!event.ok) {
      this.#emitError(new Error(event.error));
      return;
    }
    const raw =
      "available" in event.info && event.info.available === false
        ? null
        : (event.info as RawUpdateInfo);
    try {
      this.#maybeAutoDownload(this.#settleCheck(raw));
    } catch {
      /* already reported through the "error" event */
    }
  }

  #maybeAutoDownload(info: UpdateInfo): void {
    const cfg = this.#cfg;
    if (!cfg?.autoDownload || !info.available) return;
    if (info.isMajorBump && cfg.requireMajorPrompt) return;
    if (this.#staged && this.#stagedVersion === (info.version ?? null)) return;
    this.download().catch(() => {
      /* already reported through the "error" event */
    });
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
      // Cap to [0, 1] and avoid `NaN` for unknown-total — consumers
      // binding `percent * 100` to a CSS width otherwise see a `NaN%`
      // (which renders as 0) and any `percent >= 1` finishing logic
      // never fires.
      const percent =
        total && total > 0 ? Math.min(1, Math.max(0, loaded / total)) : loaded > 0 ? 1 : 0;
      this.#emit("download-progress", { percent, bytesPerSecond, loaded, total });
    });
  }

  #requireConfig(method: string): UpdaterRuntimeConfig {
    if (!this.#cfg) {
      const err = new Error(
        `updater.${method}(): no updater block in tynd.config.ts — set { endpoints, publicKeys } to enable`,
      );
      this.#emitError(err);
      throw err;
    }
    if (!this.#unsubProgress) this.#wireProgressBus();
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

/** Largest delay `setTimeout` accepts (2^31 - 1 ms); longer values fire immediately. */
const MAX_TIMER_MS = 2_147_483_647;

/** @internal Fresh facade (the exported `updater` is a module singleton). For tests. */
export function createUpdater(): Updater {
  return new UpdaterFacade();
}

const facade = new UpdaterFacade();

/**
 * High-level updater facade — Sparkle / electron-updater style. Reads
 * preferences from `tynd.config.ts::updater` (inlined by the CLI at
 * build time) and exposes manual + event-driven controls.
 *
 * @example
 *   import { updater } from "@tynd/plugin-updater/client";
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
