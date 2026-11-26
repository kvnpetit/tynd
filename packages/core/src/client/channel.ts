/**
 * Frontend side of `Channel<T>`: pass one as an argument of a backend
 * function and receive every message the handler `send`s to it.
 */

import { CHANNEL_KEY, type ChannelFrame, channelEvent } from "../channel-wire";

/**
 * A push channel passed as a backend function argument. Messages arrive in
 * order on `onmessage`; `onclose` fires when the backend closes it. Call
 * `close()` to stop listening (the backend is not notified: cancel the call
 * itself, or send your own stop signal).
 * @example
 * const progress = new Channel<number>((pct) => bar.value = pct)
 * await api.download(url, progress)
 */
export class Channel<T = unknown> {
  /** Random id the backend's channel is bound to. */
  readonly id: string;
  /** Receives each message. */
  onmessage: ((message: T) => void) | undefined;
  /** Fires once when the backend calls `close()`. */
  onclose: (() => void) | undefined;
  #off: (() => void) | undefined;

  constructor(onmessage?: (message: T) => void) {
    this.id = crypto.randomUUID();
    this.onmessage = onmessage;
    if (typeof window === "undefined" || !window.__tynd__) {
      throw new Error("Channel: window.__tynd__ is not available — are you running in a tynd app?");
    }
    this.#off = window.__tynd__.on(channelEvent(this.id), (raw) => this.#receive(raw));
  }

  /** `true` once closed by either side. */
  get closed(): boolean {
    return this.#off === undefined;
  }

  /** Stop listening. Idempotent. */
  close(): void {
    this.#off?.();
    this.#off = undefined;
  }

  #receive(raw: unknown): void {
    const frame = raw as ChannelFrame | null;
    if (!frame || typeof frame !== "object") return;
    if ("end" in frame) {
      this.close();
      this.onclose?.();
      return;
    }
    if ("m" in frame) this.onmessage?.(frame.m as T);
  }

  /** @internal Wire placeholder; the backend turns it back into a channel. */
  toWire(): Record<string, string> {
    return { [CHANNEL_KEY]: this.id };
  }
}

/** Replace top-level `Channel` arguments with their wire placeholder. */
export function encodeChannelArgs(args: unknown[]): unknown[] {
  if (!args.some((arg) => arg instanceof Channel)) return args;
  return args.map((arg) => (arg instanceof Channel ? arg.toWire() : arg));
}
