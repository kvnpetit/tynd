import { osCall, osOn } from "@tynd/core/client/bridge";

export interface NotificationOptions {
  /** Notification body text */
  body?: string;
  /** Absolute path to an icon file (PNG / ICO). */
  icon?: string;
  /** System sound name (`default`, `alarm`, ...). Platform-dependent. */
  sound?: string;
  /**
   * Action buttons. Click emits `notification:action` with the clicked
   * `id`. Works on all OS — Linux via libnotify, Windows via WinRT
   * toasts, macOS via NSUserNotification (single button = direct action,
   * 2+ = native dropdown).
   */
  actions?: { id: string; label: string }[];
}

export interface NotificationActionEvent {
  /** The `id` of the clicked action. */
  action: string;
}

/** Web-API-shaped permission state for native notifications. */
export type NotificationPermission = "granted" | "denied" | "default";

/** Wire payload shared by `send` and `schedule`. */
function notificationArgs(title: string, opts?: NotificationOptions): Record<string, unknown> {
  return {
    title,
    body: opts?.body ?? "",
    icon: opts?.icon,
    sound: opts?.sound,
    actions: opts?.actions,
  };
}

/** Frontend bindings for the host's `notification` API (toasts + scheduled + actions). */
export const notification = {
  /** Show a notification with an optional body, icon, sound, and action buttons. */
  send(title: string, opts?: NotificationOptions): Promise<void> {
    return osCall("notification", "send", notificationArgs(title, opts));
  },
  /**
   * Schedule a notification to fire after `delayMs` milliseconds. Returns a
   * numeric id that `cancel()` accepts to abort delivery.
   */
  async schedule(title: string, delayMs: number, opts?: NotificationOptions): Promise<number> {
    const { id } = await osCall<{ id: number }>("notification", "schedule", {
      ...notificationArgs(title, opts),
      delayMs,
    });
    return id;
  },
  /** Cancel a scheduled notification. Returns `false` if already fired. */
  cancel(id: number): Promise<boolean> {
    return osCall("notification", "cancel", { id });
  },
  /** Native OSes grant by default. Kept for web-API parity. */
  checkPermission(): Promise<NotificationPermission> {
    return osCall("notification", "checkPermission");
  },
  /** No-op prompt on native (always granted). Kept for web-API parity. */
  requestPermission(): Promise<NotificationPermission> {
    return osCall("notification", "requestPermission");
  },
  /**
   * Fires when the user clicks an action button. Works on all OS —
   * Linux via libnotify, Windows via WinRT toasts, macOS via
   * NSUserNotification dropdown actions.
   */
  onAction(handler: (event: NotificationActionEvent) => void): () => void {
    return osOn("notification:action", (raw) => handler(raw as NotificationActionEvent));
  },
};
