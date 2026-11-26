import { osCall, osOn } from "@tynd/core/client/bridge";
import type {
  RawUpdateInfo,
  UpdaterCheckEvent,
  UpdaterCheckOptions,
  UpdaterDownloadOptions,
  UpdaterDownloadResult,
  UpdaterInstallOptions,
  UpdaterInstallResult,
  UpdaterPatchDownloadOptions,
  UpdaterPeriodicOptions,
  UpdaterProgress,
  UpdaterRuntimeConfig,
} from "./updater-types";

export const updaterIpc = {
  async check(options: UpdaterCheckOptions = {}): Promise<RawUpdateInfo | null> {
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

  async startPeriodicCheck(options: UpdaterPeriodicOptions = {}): Promise<number> {
    const { id } = await osCall<{ id: number }>("updater", "startPeriodicCheck", options);
    return id;
  },

  stopPeriodicCheck(): Promise<void> {
    return osCall("updater", "stopPeriodicCheck");
  },

  async getChannel(defaultChannel?: string): Promise<string> {
    const { channel } = await osCall<{ channel: string }>("updater", "getChannel", {
      default: defaultChannel,
    });
    return channel;
  },

  async setChannel(channel: string): Promise<void> {
    await osCall("updater", "setChannel", { channel });
  },

  validateConfig(cfg: UpdaterRuntimeConfig): Promise<unknown> {
    return osCall("updater", "validateConfig", cfg);
  },

  onProgress(handler: (p: UpdaterProgress) => void): () => void {
    return osOn("updater:progress", (raw) => handler(raw as UpdaterProgress));
  },

  onCheck(handler: (event: UpdaterCheckEvent) => void): () => void {
    return osOn("updater:check", (raw) => handler(raw as UpdaterCheckEvent));
  },
};
