/**
 * Frontend-side typed Channel. Construct one, pass it in an RPC payload, and
 * the backend receives a live `Channel<T>` it can `.send()` into. Each
 * transmitted value fires `onMessage` on the frontend instance.
 *
 * Serialization: the channel carries a `__bv_channel_id__` that the shared
 * `serialize()` helper detects and turns into a `{__bv_channel: id}` wire
 * marker. The backend's `materializeChannels()` rebuilds a live object from it.
 */
let _nextId = 0;

export class Channel<T = unknown> {
  /** Unique event id used as the transport channel between frontend & backend. */
  readonly __bv_channel_id__: string;
  private listeners = new Set<(msg: T) => void>();
  private unsubscribe: (() => void) | null = null;

  constructor() {
    this.__bv_channel_id__ = `__bv_ch_${++_nextId}_${Date.now().toString(36)}`;

    const api = typeof window !== "undefined" ? window.__bunview_api__ : undefined;
    if (!api) {
      throw new Error(
        "[bunview] Channel requires a Bunview webview context (window.__bunview_api__).",
      );
    }

    this.unsubscribe = api.listen(this.__bv_channel_id__, (msg) => {
      for (const l of this.listeners) l(msg as T);
    });
  }

  /** Subscribe to messages streamed from the backend. Returns an unsubscribe. */
  onMessage(handler: (msg: T) => void): () => void {
    this.listeners.add(handler);
    return () => this.listeners.delete(handler);
  }

  /** Release the underlying event listener. The channel becomes inert. */
  close(): void {
    this.listeners.clear();
    this.unsubscribe?.();
    this.unsubscribe = null;
  }
}
