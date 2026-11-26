/**
 * Wire shape shared by the frontend `Channel` and the backend `Channel`: a
 * channel argument travels as `{ __tynd_channel__: "<id>" }` and the backend
 * pushes to it with events named `__tynd_ch__:<id>` sent to the calling
 * webview only.
 */

/** Key of the placeholder object a `Channel` argument is replaced with on the wire. */
export const CHANNEL_KEY = "__tynd_channel__";

/** Event name carrying the messages of channel `id`. */
export function channelEvent(id: string): string {
  return `__tynd_ch__:${id}`;
}

/** One event on a channel: a message (`m`) or the end of the channel (`end`). */
export type ChannelFrame = { m: unknown } | { end: true };

/** Id of a wire placeholder, or `undefined` when `value` is not exactly one. */
export function channelPlaceholderId(value: unknown): string | undefined {
  if (value === null || typeof value !== "object" || Array.isArray(value)) return;
  const keys = Object.keys(value);
  if (keys.length !== 1 || keys[0] !== CHANNEL_KEY) return;
  const id = (value as Record<string, unknown>)[CHANNEL_KEY];
  return typeof id === "string" && id.length > 0 && id.length <= 128 ? id : undefined;
}
