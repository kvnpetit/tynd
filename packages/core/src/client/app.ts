import type { MenuItem } from "../types";
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

/**
 * macOS activation policy: `"regular"` shows the Dock icon and menu bar,
 * `"accessory"` hides both (menu-bar / tray-only apps, windows still open),
 * `"prohibited"` also keeps the app from being activated.
 */
export type ActivationPolicy = "regular" | "accessory" | "prohibited";

/** `"informational"` bounces once; `"critical"` bounces until the app is activated. */
export type DockBounceType = "informational" | "critical";

/**
 * macOS Dock controls. Every method rejects with "not supported on
 * <os>" on Windows and Linux; use `window.requestAttention`,
 * `window.setBadge` and `window.setWindowIcon` there.
 */
const dock = {
  /** Show the Dock icon (activation policy `"regular"`). */
  show(): Promise<void> {
    return osCall("app", "setActivationPolicy", { policy: "regular" });
  },
  /** Hide the Dock icon and the menu bar (activation policy `"accessory"`). */
  hide(): Promise<void> {
    return osCall("app", "setActivationPolicy", { policy: "accessory" });
  },
  /**
   * Bounce the Dock icon. Resolves to a request id for `cancelBounce`; `0`
   * when the app is already active (macOS does not bounce then).
   */
  bounce(type: DockBounceType = "informational"): Promise<number> {
    return osCall("app", "dockBounce", { kind: type });
  },
  /** Stop a bounce started by `bounce`. Unknown ids are ignored. */
  cancelBounce(id: number): Promise<void> {
    return osCall("app", "dockCancelBounce", { id });
  },
  /**
   * Replace the Dock icon with an image file (PNG, ICNS…; the FS scope
   * applies). `null` restores the bundle icon. Lasts until the app quits.
   */
  setIcon(path: string | null): Promise<void> {
    return osCall("app", "setDockIcon", { path });
  },
  /**
   * Menu shown when the user right-clicks the Dock icon, using the same item
   * shape as the application menu; clicks emit the usual `menu:action`
   * event. Pass `[]` to remove it.
   */
  setMenu(items: MenuItem[]): Promise<void> {
    return osCall("app", "setDockMenu", { items });
  },
};

/** One link of the Windows jump list. */
export interface JumpListItem {
  /** Text shown in the jump list (1 to 260 characters). */
  title: string;
  /**
   * Arguments the app executable is launched with (quoted for you). With
   * `@tynd/plugin-single-instance` they reach the running instance through
   * `singleInstance.onSecondLaunch`; otherwise a new process starts.
   */
  args?: string[];
  /** Tooltip (up to 1024 characters). */
  description?: string;
  /** `.ico`, `.exe` or `.dll` file holding the icon (FS scope applies). Defaults to the app icon. */
  icon?: string;
  /** Index of the icon inside `icon` (default `0`). */
  iconIndex?: number;
}

/** Named section of the Windows jump list. */
export interface JumpListCategory {
  /** Section title (1 to 260 characters, unique). */
  name: string;
  /** 1 to 32 links. */
  items: JumpListItem[];
}

/** Content of the Windows jump list (right-click on the taskbar button). */
export interface JumpList {
  /** Links in the "Tasks" section (at most 32). */
  tasks?: JumpListItem[];
  /** Custom sections above the tasks (at most 16). */
  categories?: JumpListCategory[];
  /** Keep the OS "Recent" section fed by `addRecentDocument` (default `true`). */
  showRecent?: boolean;
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
  /** macOS Dock controls (show / hide, bounce, icon, menu). */
  dock,
  /** Set the macOS activation policy. Rejects on Windows and Linux. */
  setActivationPolicy(policy: ActivationPolicy): Promise<void> {
    return osCall("app", "setActivationPolicy", { policy });
  },
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
  /**
   * Add a file to the OS recent-documents list: the app's jump-list "Recent"
   * category on Windows (visible once the app is registered for the file
   * type), "Open Recent" on macOS. Rejects on Linux. The path must pass the
   * FS scope and name an existing file.
   */
  addRecentDocument(path: string): Promise<void> {
    return osCall("app", "addRecentDocument", { path });
  },
  /** Clear the app's recent-documents list (Windows jump list, macOS "Open Recent"). Rejects on Linux. */
  clearRecentDocuments(): Promise<void> {
    return osCall("app", "clearRecentDocuments");
  },
  /**
   * Replace the Windows jump list. Links the user removed from a custom
   * section are not added back (Windows refuses them). Rejects on macOS (use
   * `app.dock.setMenu`) and Linux.
   */
  setJumpList(list: JumpList): Promise<void> {
    return osCall("app", "setJumpList", list);
  },
  /** Remove the custom Windows jump list. Rejects on macOS and Linux. */
  clearJumpList(): Promise<void> {
    return osCall("app", "clearJumpList");
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
