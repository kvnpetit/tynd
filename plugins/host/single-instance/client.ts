import { osCall, osOn } from "@tynd/core/client/bridge";

/** Result of `singleInstance.acquire`. */
export interface SingleInstanceResult {
  acquired: boolean;
  already: boolean;
}

/** Event payload delivered to `onSecondLaunch`. */
export interface SecondLaunchPayload {
  /** argv of the duplicate launch (including argv[0] = exe path). */
  argv: string[];
  /** Working directory the duplicate was launched from. */
  cwd: string;
}

/** Frontend bindings for the host's `singleInstance` API (process locking + URL forward). */
export const singleInstance = {
  /** Try to claim the named lock; subsequent acquires return `acquired: false`. */
  acquire(id: string): Promise<SingleInstanceResult> {
    return osCall("singleInstance", "acquire", { id });
  },
  /** Whether this process currently holds a single-instance lock. */
  isAcquired(): Promise<boolean> {
    return osCall("singleInstance", "isAcquired");
  },
  /**
   * Fires in the primary instance whenever a duplicate launch is detected.
   * The duplicate forwards its argv + cwd and exits — the primary window is
   * auto-focused by the host; use this handler to act on deep-link arguments
   * or reopen a specific document.
   */
  onSecondLaunch(handler: (payload: SecondLaunchPayload) => void): () => void {
    return osOn("app:second-launch", (raw) => handler(raw as SecondLaunchPayload));
  },
  /**
   * Fires when the app is opened via a registered URL scheme (declared in
   * `tynd.config.ts::protocols`). Fires both for a cold start (argv contains
   * the URL) and for a duplicate launch (URL forwarded then auto-focused).
   */
  onOpenUrl(handler: (url: string) => void): () => void {
    return osOn("app:open-url", (raw) => {
      const url = (raw as { url?: string } | undefined)?.url;
      if (typeof url === "string") handler(url);
    });
  },
};
