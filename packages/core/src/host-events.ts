/**
 * Native OS events the host forwards to the Bun backend (`window:*` and
 * `webview:*`, minus `window:drag-over`). Backend window handles
 * (`getWindow(label).on*`) subscribe here.
 */

import { tynd } from "./logger";

type Handler = (data: unknown) => void;

const listeners = new Map<string, Set<Handler>>();

/** Subscribe to host event `name`; returns the unsubscribe function. */
export function onHostEvent(name: string, handler: Handler): () => void {
  let set = listeners.get(name);
  if (!set) {
    set = new Set();
    listeners.set(name, set);
  }
  set.add(handler);
  return () => {
    const current = listeners.get(name);
    if (!current) return;
    current.delete(handler);
    if (current.size === 0) listeners.delete(name);
  };
}

/** @internal Deliver one forwarded host event. A throwing handler never stops the others. */
export function dispatchHostEvent(name: string, data: unknown): void {
  const set = listeners.get(name);
  if (!set) return;
  for (const handler of [...set]) {
    try {
      handler(data);
    } catch (error) {
      tynd.error(
        `host event '${name}' handler threw: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }
}
