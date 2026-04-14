/**
 * Backend-side typed Channel. A command receives a `Channel<T>` when the
 * frontend passes a `Channel` in the payload — the backend can `.send()`
 * values that stream back to the frontend's `channel.onMessage()` listener.
 *
 * This is the out-of-band streaming primitive (equivalent to Tauri's `Channel`).
 * For one-shot request/response, return a value from the command normally.
 */
export class Channel<T = unknown> {
  constructor(private readonly _emit: (payload: T) => void) {}

  /** Push a typed value to the frontend listener. */
  send(payload: T): void {
    this._emit(payload);
  }
}

/**
 * Walk a deserialized payload and replace every `{__bv_channel: id}` marker
 * with a live backend `Channel` that emits on the channel's unique id.
 */
export function materializeChannels(
  value: unknown,
  emit: (channelId: string, payload: unknown) => void,
): unknown {
  if (value == null) return value;
  if (Array.isArray(value)) return value.map((v) => materializeChannels(v, emit));
  if (typeof value === "object") {
    const obj = value as Record<string, unknown>;
    const chId = obj.__bv_channel;
    if (typeof chId === "string" && Object.keys(obj).length === 1) {
      return new Channel<unknown>((payload) => emit(chId, payload));
    }
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(obj)) out[k] = materializeChannels(v, emit);
    return out;
  }
  return value;
}
