/**
 * Backend side of `Channel<T>`: a function argument the handler can push
 * messages to for as long as it likes, delivered to the calling webview only.
 */

import { type ChannelFrame, channelEvent, channelPlaceholderId } from "../channel-wire";
import { getEmitFn } from "../lifecycle";
import type { CallContext } from "./context";

/**
 * A push channel received as an argument of a backend function. The frontend
 * passes `new Channel<T>(onMessage)` from `@tynd/core/client`; every `send`
 * reaches its `onmessage`, in order. Use it for progress reports or
 * server-push feeds that outlive one return value (for a pull-based,
 * back-pressured stream, return an async generator instead).
 * @example
 * export async function download(url: string, progress: Channel<number>) {
 *   for await (const pct of fetchWithProgress(url)) progress.send(pct)
 *   progress.close()
 * }
 */
export interface Channel<T> {
  /** Type marker the frontend client maps to its own `Channel<T>`. */
  readonly __tynd_channel__: true;
  /** Random id chosen by the frontend. */
  readonly id: string;
  /** `true` after `close()`. */
  readonly closed: boolean;
  /** Push one message (any devalue-encodable value). Throws after `close()`. */
  send(message: T): void;
  /** End the channel: the frontend's `onclose` fires and later sends throw. */
  close(): void;
}

function createChannel<T>(id: string, webview: string | undefined): Channel<T> {
  let closed = false;
  const push = (frame: ChannelFrame) => {
    if (webview === undefined) {
      throw new Error("Channel: this call has no calling webview to send to");
    }
    const emit = getEmitFn();
    if (!emit) throw new Error("Channel: send() before app.start()");
    emit(channelEvent(id), frame, webview);
  };
  return {
    __tynd_channel__: true,
    id,
    get closed() {
      return closed;
    },
    send(message: T) {
      if (closed) throw new Error("Channel: send() after close()");
      push({ m: message });
    },
    close() {
      if (closed) return;
      closed = true;
      push({ end: true });
    },
  };
}

/**
 * Replace the top-level `{ __tynd_channel__: id }` placeholders of a call's
 * arguments with live channels bound to the calling webview. The id is
 * untrusted but harmless: a channel only ever sends to the webview that
 * made the call.
 */
export function reviveChannels(args: unknown[], caller: CallContext | undefined): unknown[] {
  if (!args.some((arg) => channelPlaceholderId(arg) !== undefined)) return args;
  return args.map((arg) => {
    const id = channelPlaceholderId(arg);
    return id === undefined ? arg : createChannel(id, caller?.webview);
  });
}
