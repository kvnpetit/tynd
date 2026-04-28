/**
 * Auto-updater client. Mirrors the host's `os.updater.*` dispatch table.
 *
 * Trust model: the manifest itself is unsigned; each platform entry carries
 * an Ed25519 signature over the binary bytes, verified against the pub-key
 * the app bakes in at build time. See `os/updater.rs` for the flow.
 */

import { osCall } from "./_internal";

export interface UpdateInfo {
  version: string;
  notes: string | null;
  pubDate: string | null;
  url: string;
  signature: string;
  platform: string;
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
  phase: "download" | "verified";
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

export interface UpdaterPeriodicOptions extends UpdaterCheckOptions {
  intervalMs?: number;
}

export type UpdaterCheckEvent =
  | { id: number; ok: true; info: UpdateInfo | { available: false } }
  | { id: number; ok: false; error: string };

export interface UpdaterDownloadResult {
  path: string;
  size: number;
}

export interface UpdaterInstallOptions {
  path: string;
  relaunch?: boolean;
}

export interface UpdaterInstallResult {
  installed: boolean;
  path: string;
  relaunch: boolean;
}

export const updater = {
  async check(options: UpdaterCheckOptions): Promise<UpdateInfo | null> {
    const result = await osCall<{ available: boolean } & UpdateInfo>(
      "updater",
      "check",
      options,
    );
    return result.available ? result : null;
  },

  downloadAndVerify(options: UpdaterDownloadOptions): Promise<UpdaterDownloadResult> {
    return osCall("updater", "downloadAndVerify", options);
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

  onProgress(handler: (p: UpdaterProgress) => void): () => void {
    return window.__tynd__.os_on("updater:progress", (raw) => handler(raw as UpdaterProgress));
  },

  onCheck(handler: (event: UpdaterCheckEvent) => void): () => void {
    return window.__tynd__.os_on("updater:check", (raw) => handler(raw as UpdaterCheckEvent));
  },
};
