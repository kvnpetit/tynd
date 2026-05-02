import { osCall } from "./_internal";

/**
 * Monitor descriptor returned by {@link screenshot.listMonitors}.
 *
 * Mirrors the JSON shape produced by `host-rs::os::screenshot::list_monitors`
 * (backed by `xcap`). `id` is the platform monitor handle and is the same
 * value `captureScreen` accepts via positional index.
 */
export interface MonitorInfo {
  id: number;
  name: string;
  width: number;
  height: number;
  /** Backing-scale factor (HiDPI ratio). 1 on standard displays, 2 on Retina. */
  scale: number;
  primary: boolean;
}

/** Window descriptor returned by {@link screenshot.listWindows}. */
export interface WindowInfo {
  /** Stable window id usable with `captureWindow(id)`. */
  id: number;
  title: string;
  app_name: string;
  width: number;
  height: number;
}

/**
 * Captured frame. `data` is base64-encoded PNG bytes — drop into a
 * `<img src="data:image/png;base64,…" />` or `atob` for the raw bytes.
 */
export interface CapturedImage {
  width: number;
  height: number;
  data: string;
}

/**
 * `screenshot` — capture monitors / windows and enumerate the OS surfaces.
 *
 * Privacy-sensitive: `captureScreen` and `captureWindow` are gated by the
 * native `screenshot:capture*` ACL; `listMonitors` / `listWindows` are
 * left ungated so settings UIs can populate pickers before the user has
 * granted capture rights.
 */
export const screenshot = {
  /**
   * Capture the given monitor (defaults to monitor index `0` — typically
   * the primary display). Returns base64 PNG bytes wrapped with the
   * captured dimensions.
   */
  captureScreen(monitorIndex = 0): Promise<CapturedImage> {
    return osCall("screenshot", "captureScreen", { monitorIndex });
  },
  /** Capture a single window by its `WindowInfo.id`. */
  captureWindow(windowId: number): Promise<CapturedImage> {
    return osCall("screenshot", "captureWindow", { windowId });
  },
  /** Enumerate every connected monitor (primary first when supported). */
  listMonitors(): Promise<MonitorInfo[]> {
    return osCall("screenshot", "listMonitors", {});
  },
  /** Enumerate every visible top-level window across all monitors. */
  listWindows(): Promise<WindowInfo[]> {
    return osCall("screenshot", "listWindows", {});
  },
};
