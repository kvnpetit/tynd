import { osCall } from "./_internal.ts"

export interface SingleInstanceResult {
  acquired: boolean
  already: boolean
}

export interface SecondLaunchPayload {
  /** argv of the duplicate launch (including argv[0] = exe path). */
  argv: string[]
  /** Working directory the duplicate was launched from. */
  cwd: string
}

export const singleInstance = {
  acquire(id: string): Promise<SingleInstanceResult> {
    return osCall("singleInstance", "acquire", { id })
  },
  isAcquired(): Promise<boolean> {
    return osCall("singleInstance", "isAcquired")
  },
  /**
   * Fires in the primary instance whenever a duplicate launch is detected.
   * The duplicate forwards its argv + cwd and exits — the primary window is
   * auto-focused by the host; use this handler to act on deep-link arguments
   * or reopen a specific document.
   */
  onSecondLaunch(handler: (payload: SecondLaunchPayload) => void): () => void {
    return window.__tynd__.os_on("app:second-launch", (raw) => handler(raw as SecondLaunchPayload))
  },
  /**
   * Fires when the app is opened via a registered URL scheme (declared in
   * `tynd.config.ts::protocols`). Fires both for a cold start (argv contains
   * the URL) and for a duplicate launch (URL forwarded then auto-focused).
   */
  onOpenUrl(handler: (url: string) => void): () => void {
    return window.__tynd__.os_on("app:open-url", (raw) => {
      const url = (raw as { url?: string } | undefined)?.url
      if (typeof url === "string") handler(url)
    })
  },
}
