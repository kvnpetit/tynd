import { osCall } from "./_internal";
import { type AppSubscription, createChannel } from "./app-events";

export type { AppSubscription } from "./app-events";

/**
 * App metadata a renderer may set via `app.setInfo`. `version` is deliberately absent: it is the
 * updater's downgrade floor, so the host accepts it only from the CLI-generated backend bootstrap,
 * which reads it from `package.json` (no app code sets it).
 */
export interface AppInfo {
  name?: string;
  /** Reverse-DNS identifier, e.g. `com.acme.myapp`. */
  bundleId?: string;
}

/** Payload of `app.onReopen`. */
export interface AppReopenInfo {
  /** `false` when the user clicked the dock icon while every window was closed or hidden. */
  hasVisibleWindows: boolean;
}

// Documents opened before the frontend subscribes (macOS double-click launch,
// argv on Windows/Linux) are buffered by the host; the first `onOpenFile`
// subscription drains them.
const subscribeOpenFile = createChannel<string>({
  listen(emit) {
    window.__tynd__.os_on("app:open-file", (raw) => {
      const path = (raw as { path?: unknown } | null)?.path;
      if (typeof path === "string") emit(path);
    });
  },
  async drain() {
    const result = await osCall<{ paths?: unknown } | null>("app", "subscribeOpenFiles");
    const paths = result?.paths;
    return Array.isArray(paths) ? paths.filter((p): p is string => typeof p === "string") : [];
  },
  holdWhenIdle: true,
});

const subscribeReopen = createChannel<AppReopenInfo>({
  listen(emit) {
    window.__tynd__.os_on("app:reopen", (raw) => {
      const flag = (raw as { hasVisibleWindows?: unknown } | null)?.hasVisibleWindows;
      emit({ hasVisibleWindows: flag === true });
    });
  },
});

/** Frontend bindings for the host's `app` API: name/version, bundle id, lifecycle. */
export const app = {
  /** App display name. Defaults to the binary's file stem. */
  getName(): Promise<string> {
    return osCall("app", "getName");
  },
  /** App version, from `package.json` via the CLI-generated bootstrap (`"0.0.0"` when absent). */
  getVersion(): Promise<string> {
    return osCall("app", "getVersion");
  },
  /** Tynd host version baked in at compile time (e.g. `"1.0.0"`). */
  getFrameworkVersion(): Promise<string> {
    return osCall("app", "getFrameworkVersion");
  },
  /** Reverse-DNS bundle identifier set via `setInfo({ bundleId })`. */
  getBundleIdentifier(): Promise<string | null> {
    return osCall("app", "getBundleIdentifier");
  },
  /**
   * Set the app name and/or bundle identifier at runtime so other windows can
   * query them via `getName` / `getBundleIdentifier`. The version cannot be set
   * here (the host rejects it): it comes from `package.json` through the
   * CLI-generated bootstrap.
   */
  setInfo(info: AppInfo): Promise<void> {
    return osCall("app", "setInfo", info);
  },
  /** Exit the process with the given code (default `0`). Fires cleanup first. */
  exit(code?: number): Promise<void> {
    return osCall("app", "exit", code !== undefined ? { code } : {});
  },
  /** Relaunch the current binary, then exit the running process. */
  relaunch(): Promise<void> {
    return osCall("app", "relaunch");
  },
  /**
   * Fires with the absolute path of each document the OS asks the app to open:
   * a file-association double-click or "Open with" on macOS, a path on the
   * command line on Windows/Linux (at launch, and when a second instance
   * forwards its argv through `singleInstance`).
   *
   * Paths received before the first subscription (typically the file that
   * launched the app) are buffered by the host and delivered to the first
   * handler registered. Custom-scheme URLs are not reported here; use
   * `singleInstance.onOpenUrl`.
   */
  onOpenFile(handler: (path: string) => void): AppSubscription {
    return subscribeOpenFile(handler);
  },
  /**
   * macOS only: fires when the user re-activates the running app (dock icon
   * click) so it can reopen a window when none is visible. Never fires on
   * Windows/Linux.
   */
  onReopen(handler: (info: AppReopenInfo) => void): AppSubscription {
    return subscribeReopen(handler);
  },
};
