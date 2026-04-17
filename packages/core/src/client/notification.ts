import type { NotificationOptions } from "../types.js"
import { osCall } from "./_internal.js"

export type { NotificationOptions }

export const notification = {
  send(title: string, opts?: NotificationOptions): Promise<void> {
    return osCall("notification", "send", { title, body: opts?.body ?? "" })
  },
}
