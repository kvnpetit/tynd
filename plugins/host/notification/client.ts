import { osCall, osOn } from "@tynd/core/client/bridge";

/** An action button: `id` comes back in `onAction`, `label` is shown. */
export interface NotificationAction {
  /** 1-64 chars: letters, digits, `_`, `-`. */
  id: string;
  label: string;
}

/** Inline reply field (Windows and macOS). */
export interface NotificationReplyOptions {
  /** Hint shown in the empty field. */
  placeholder?: string;
  /** Label of the send button (Windows; default `"Send"`). */
  buttonLabel?: string;
}

/** `low` / `normal` / `critical`. Linux sets the D-Bus urgency; Windows shows `critical` as an urgent toast. */
export type NotificationUrgency = "low" | "normal" | "critical";

/** How long the notification stays on screen: the system default, until dismissed, or about `n` ms. */
export type NotificationTimeout = "default" | "never" | number;

export interface NotificationOptions {
  /** Notification body text */
  body?: string;
  /** Absolute path to an icon file (PNG / ICO). */
  icon?: string;
  /** Absolute path to a larger image shown in the notification. */
  image?: string;
  /** System sound name (`default`, `Mail`, `Glass`, ...). Platform-dependent. */
  sound?: string;
  /** Play no sound. */
  silent?: boolean;
  urgency?: NotificationUrgency;
  timeout?: NotificationTimeout;
  /** Up to 5 action buttons. Clicks fire `onAction` with the button `id`. */
  actions?: NotificationAction[];
  /** Use the buttons registered with `registerActions(category, ...)` instead of `actions`. */
  category?: string;
  /** Show an inline reply field; the text arrives in `onReply`. Windows and macOS. */
  reply?: NotificationReplyOptions;
}

/** The user clicked an action button. */
export interface NotificationActionEvent {
  /** Notification id returned by `send` / `schedule`. */
  id: number;
  /** The `id` of the clicked action. */
  action: string;
}

/** The user clicked the notification body. */
export interface NotificationClickEvent {
  id: number;
}

/** The user sent an inline reply. */
export interface NotificationReplyEvent {
  id: number;
  text: string;
}

/** Why a notification closed: dismissed by the user, timed out, removed by the app, or unknown. */
export type NotificationCloseReason = "user" | "timeout" | "app" | "unknown";

export interface NotificationCloseEvent {
  id: number;
  reason: NotificationCloseReason;
}

/** A notification still on screen or in the notification center. */
export interface DeliveredNotification {
  id: number;
  title: string;
  body: string;
}

/** Web-API-shaped permission state for native notifications. */
export type NotificationPermission = "granted" | "denied" | "default";

/** Wire payload shared by `send` and `schedule`. */
function notificationArgs(title: string, opts?: NotificationOptions): Record<string, unknown> {
  return {
    title,
    body: opts?.body ?? "",
    icon: opts?.icon,
    image: opts?.image,
    sound: opts?.sound,
    silent: opts?.silent,
    urgency: opts?.urgency,
    timeout: opts?.timeout,
    actions: opts?.actions,
    category: opts?.category,
    reply: opts?.reply,
  };
}

/** Frontend bindings for the host's `notification` API. */
export const notification = {
  /** Show a notification now. Resolves with its id, carried by every event of it. */
  async send(title: string, opts?: NotificationOptions): Promise<number> {
    const { id } = await osCall<{ id: number }>(
      "notification",
      "send",
      notificationArgs(title, opts),
    );
    return id;
  },
  /**
   * Schedule a notification to fire after `delayMs` milliseconds. Resolves
   * with its id: `cancel(id)` aborts it while pending; once shown, its
   * events and `remove(id)` use the same id.
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
  /**
   * Register (or replace) a set of action buttons under `category`, then
   * pass `{ category }` to `send` / `schedule` instead of `actions`.
   */
  registerActions(category: string, actions: NotificationAction[]): Promise<void> {
    return osCall("notification", "registerActions", { category, actions });
  },
  /** Notifications of this app still on screen or in the notification center. Windows and Linux. */
  getDelivered(): Promise<DeliveredNotification[]> {
    return osCall("notification", "getDelivered");
  },
  /** Remove a delivered notification. Resolves `false` for an unknown id. Windows and Linux. */
  remove(id: number): Promise<boolean> {
    return osCall("notification", "remove", { id });
  },
  /** Remove every delivered notification of this window. Resolves with the count. Windows and Linux. */
  removeAll(): Promise<number> {
    return osCall("notification", "removeAll");
  },
  /** `"denied"` when notifications are turned off for the app (Windows); `"granted"` otherwise. */
  checkPermission(): Promise<NotificationPermission> {
    return osCall("notification", "checkPermission");
  },
  /** No OS prompts for desktop notifications: resolves with the current state. */
  requestPermission(): Promise<NotificationPermission> {
    return osCall("notification", "requestPermission");
  },
  /** Fires when the user clicks an action button. */
  onAction(handler: (event: NotificationActionEvent) => void): () => void {
    return osOn("notification:action", (raw) => handler(raw as NotificationActionEvent));
  },
  /** Fires when the user clicks the notification body. */
  onClick(handler: (event: NotificationClickEvent) => void): () => void {
    return osOn("notification:click", (raw) => handler(raw as NotificationClickEvent));
  },
  /** Fires when the user sends an inline reply. Windows and macOS. */
  onReply(handler: (event: NotificationReplyEvent) => void): () => void {
    return osOn("notification:reply", (raw) => handler(raw as NotificationReplyEvent));
  },
  /** Fires when a notification is dismissed, times out or is removed. */
  onClose(handler: (event: NotificationCloseEvent) => void): () => void {
    return osOn("notification:close", (raw) => handler(raw as NotificationCloseEvent));
  },
};
