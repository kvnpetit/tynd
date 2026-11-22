import { osCall } from "@tynd/core/client/bridge";

/** Per-call options for the autolaunch API. */
export interface AutolaunchOptions {
  /** Display name (Windows registry key / .plist label / .desktop Name). */
  name?: string;
  /** Extra CLI args appended when the OS relaunches the app at boot. */
  args?: string[];
}

/** Frontend bindings for the host's `autolaunch` API (start-at-login). */
export const autolaunch = {
  /** Register the running binary to start at system login. Resolves to the new state (`true`). */
  enable(options?: AutolaunchOptions): Promise<boolean> {
    return osCall("autolaunch", "enable", options ?? {});
  },
  /** Remove the login entry. Resolves to the new state (`false`). */
  disable(options?: AutolaunchOptions): Promise<boolean> {
    return osCall("autolaunch", "disable", options ?? {});
  },
  /** Whether the current binary is registered to launch at login. */
  isEnabled(options?: AutolaunchOptions): Promise<boolean> {
    return osCall("autolaunch", "isEnabled", options ?? {});
  },
};
