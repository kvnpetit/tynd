import type { NotificationActionEvent, NotificationOptions } from "../types.js"
import { osCall } from "./_internal.js"

export type { NotificationActionEvent, NotificationOptions }

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
   * Fires when the user clicks an action button. Linux only — Windows /
   * macOS don't surface notification clicks through this API.
   */
  onAction(handler: (event: NotificationActionEvent) => void): () => void {
    return window.__tynd__.os_on("notification:action", (raw) =>
      handler(raw as NotificationActionEvent),
    )
  },
}
