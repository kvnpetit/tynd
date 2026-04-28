import { tynd } from "../logger";
import type { Emitter } from "../types";
import { createCallProxy } from "./_internal";

type UnionToIntersection<U> = (U extends unknown ? (k: U) => void : never) extends (
  k: infer I,
) => void
  ? I
  : never;

type ModuleEvents<T> = UnionToIntersection<
  { [K in keyof T]: T[K] extends Emitter<infer E> ? E : never }[keyof T]
>;

/**
 * Awaitable + async-iterable handle for a streaming backend call.
 * - `await call(...)` -> generator's `return` value (or `undefined`)
 * - `for await (const x of call(...))` -> each `yield`ed chunk
 * - `handle.cancel()` -> stops the backend generator via `iterator.return()`
 */
export type StreamCall<Y, R> = PromiseLike<Awaited<R>> &
  AsyncIterable<Y> & {
    cancel(): Promise<IteratorResult<Y>>;
  };

type ModuleFunctions<T> = {
  [K in keyof T as T[K] extends (...args: infer _A) => infer _R ? K : never]: T[K] extends (
    ...args: infer A
  ) => AsyncGenerator<infer Y, infer R, unknown>
    ? (...args: A) => StreamCall<Y, R>
    : T[K] extends (...args: infer A) => AsyncIterable<infer Y>
      ? (...args: A) => StreamCall<Y, void>
      : T[K] extends (...args: infer A) => infer R
        ? (...args: A) => Promise<Awaited<R>>
        : never;
};

export type BackendClient<T> = ModuleFunctions<T> & {
  on<K extends keyof ModuleEvents<T> & string>(
    event: K,
    handler: (payload: ModuleEvents<T>[K]) => void,
  ): () => void;
  once<K extends keyof ModuleEvents<T> & string>(
    event: K,
    handler: (payload: ModuleEvents<T>[K]) => void,
  ): () => void;
};

/**
 * Create a fully type-safe proxy to the backend.
 *
 * @example
 * import type * as backend from "../backend/main"
 * const api = createBackend<typeof backend>()
 * await api.greet("Alice")
 */
export function createBackend<T>(): BackendClient<T> {
  return createCallProxy<BackendClient<T>>(
    // Backend RPC: prop name == wire fn name.
    (prop) => prop,
    // Subscription helpers (`on`/`once`) are not RPC calls — handled here
    // before the default `(...args) => window.__tynd__.call(...)` path runs.
    (prop) => {
      if (prop === "on") {
        return (event: string, handler: (p: unknown) => void) => window.__tynd__.on(event, handler);
      }
      if (prop === "once") {
        return (event: string, handler: (p: unknown) => void) => {
          let called = false;
          const wrapper = (p: unknown) => {
            if (called) {
              // Extra fires usually mean emitter/receiver disagree on multiplicity — surface them.
              tynd.warn(
                `on.once(${JSON.stringify(event)}): handler fired more than once — extra invocation ignored`,
              );
              return;
            }
            called = true;
            window.__tynd__.off(event, wrapper);
            handler(p);
          };
          window.__tynd__.on(event, wrapper);
          return () => window.__tynd__.off(event, wrapper);
        };
      }
      return undefined;
    },
  );
}
