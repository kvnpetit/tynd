import type { NotificationActionEvent, NotificationOptions } from "../types.js"
import { osCall } from "./_internal.js"

export type { NotificationActionEvent, NotificationOptions }

export type NotificationPermission = "granted" | "denied" | "default"

export const notification = {
  send(title: string, opts?: NotificationOptions): Promise<void> {
    return osCall("notification", "send", {
      title,
      body: opts?.body ?? "",
      icon: opts?.icon,
      sound: opts?.sound,
      actions: opts?.actions,
    })
  },
  /**
   * Schedule a notification to fire after `delayMs` milliseconds. Returns a
   * numeric id that `cancel()` accepts to abort delivery.
   */
  async schedule(
    title: string,
    delayMs: number,
    opts?: NotificationOptions,
  ): Promise<number> {
    const { id } = await osCall<{ id: number }>("notification", "schedule", {
      title,
      body: opts?.body ?? "",
      icon: opts?.icon,
      sound: opts?.sound,
      actions: opts?.actions,
      delayMs,
    })
    return id
  },
  /** Cancel a scheduled notification. Returns `false` if already fired. */
  cancel(id: number): Promise<boolean> {
    return osCall("notification", "cancel", { id })
  },
  /** Native OSes grant by default. Kept for web-API parity. */
  checkPermission(): Promise<NotificationPermission> {
    return osCall("notification", "checkPermission")
  },
  /** No-op prompt on native (always granted). Kept for web-API parity. */
  requestPermission(): Promise<NotificationPermission> {
    return osCall("notification", "requestPermission")
  },
  /**
   * Fires when the user clicks an action button. Works on all OS —
   * Linux via libnotify, Windows via WinRT toasts, macOS via
   * NSUserNotification dropdown actions.
   */
  onAction(handler: (event: NotificationActionEvent) => void): () => void {
    return window.__tynd__.os_on("notification:action", (raw) =>
      handler(raw as NotificationActionEvent),
    )
  },
}
