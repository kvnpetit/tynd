import { osCall } from "./_internal.ts"

export interface AutolaunchOptions {
  /** Display name (Windows registry key / .plist label / .desktop Name). */
  name?: string
  /** Extra CLI args appended when the OS relaunches the app at boot. */
  args?: string[]
}

export const autolaunch = {
  /** Register the running binary to start at system login. */
  enable(options?: AutolaunchOptions): Promise<{ enabled: boolean }> {
    return osCall("autolaunch", "enable", options ?? {})
  },
  /** Remove the login entry. */
  disable(options?: AutolaunchOptions): Promise<{ enabled: boolean }> {
    return osCall("autolaunch", "disable", options ?? {})
  },
  /** Whether the current binary is registered to launch at login. */
  isEnabled(options?: AutolaunchOptions): Promise<boolean> {
    return osCall("autolaunch", "isEnabled", options ?? {})
  },
}
