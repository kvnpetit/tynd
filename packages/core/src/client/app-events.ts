/**
 * Subscription plumbing for host-originated app events (`app:open-file`,
 * `app:reopen`). A channel attaches to its live source on the first
 * subscription and, when the host buffers events from before the frontend was
 * listening (files a user double-clicked to launch the app), drains that
 * buffer right after attaching.
 */

/** Handle returned by every app event subscription. Call `dispose()` to unsubscribe. */
export interface AppSubscription {
  dispose(): void;
}

export interface ChannelSource<T> {
  /** Attach to the live event stream. Called once, on the first subscription. */
  listen(emit: (value: T) => void): void;
  /**
   * Fetch events the host buffered before `listen` attached. Called on the
   * first subscription, and again on a later one if it failed.
   */
  drain?(): Promise<readonly T[]>;
  /**
   * Keep events that arrive while no handler is registered and give them to
   * the next subscriber (default `false`: such events are dropped).
   */
  holdWhenIdle?: boolean;
}

/** Upper bound on events held while no handler is registered. */
const MAX_HELD = 256;

export function createChannel<T>(
  source: ChannelSource<T>,
): (handler: (value: T) => void) => AppSubscription {
  const handlers = new Set<(value: T) => void>();
  let held: T[] = [];
  let listening = false;
  let draining = false;
  let drained = !source.drain;

  function deliver(handler: (value: T) => void, value: T): void {
    try {
      handler(value);
    } catch (error) {
      // biome-ignore lint/suspicious/noConsole: a handler failure must not break event delivery
      console.error("[tynd] app event handler threw", error);
    }
  }

  function dispatch(value: T): void {
    if (handlers.size === 0) {
      if (source.holdWhenIdle) {
        held.push(value);
        if (held.length > MAX_HELD) held.shift();
      }
      return;
    }
    for (const handler of [...handlers]) deliver(handler, value);
  }

  function startDrain(): void {
    if (drained || draining || !source.drain) return;
    draining = true;
    source
      .drain()
      .then((items) => {
        drained = true;
        for (const item of items) dispatch(item);
      })
      .catch(() => {
        // Host without the buffer API: live events still work. Retry on the
        // next subscription.
      })
      .finally(() => {
        draining = false;
      });
  }

  return (handler) => {
    handlers.add(handler);
    if (!listening) {
      listening = true;
      source.listen(dispatch);
    }
    startDrain();
    if (held.length > 0) {
      const pending = held;
      held = [];
      for (const value of pending) deliver(handler, value);
    }
    return {
      dispose() {
        handlers.delete(handler);
      },
    };
  };
}
