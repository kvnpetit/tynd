/**
 * Frontend plugin client. Mirrors `definePlugin` on the backend: you call
 * `createPluginClient<typeof Plugin>("foo")` once in your plugin's
 * `client.ts`, and consumers get a fully typed proxy that forwards method
 * calls as Tynd RPC.
 *
 * Wire format: method `foo.bar(x, y)` becomes an RPC call with fn
 * `$plugin$foo$bar` and args `[x, y]`. The runtime dispatcher (`full.ts` /
 * `globals.js`) recognizes the prefix and routes to the plugin registry
 * populated by `app.start({ plugins: [...] })`.
 */

import { tynd } from "../logger";
import "./_internal.js";

/**
 * Plugin identity shape — same as the `Plugin<T>` type exported by the
 * backend, duplicated here so `client.ts` files don't need to import from
 * the backend entry.
 */
export interface PluginShape<T extends Record<string, (...args: never[]) => unknown>> {
  readonly __tynd_plugin__: true;
  readonly name: string;
  readonly methods: T;
}

/**
 * Map a plugin's backend method signatures to frontend client signatures.
 * Streaming generators surface as `AsyncIterable` promises; regular
 * functions surface as plain `Promise<Awaited<R>>`.
 */
export type PluginClient<P> =
  P extends PluginShape<infer M>
    ? {
        [K in keyof M]: M[K] extends (...args: infer A) => AsyncGenerator<infer Y, infer R, unknown>
          ? (
              ...args: A
            ) => PromiseLike<Awaited<R>> &
              AsyncIterable<Y> & { cancel(): Promise<IteratorResult<Y>> }
          : M[K] extends (...args: infer A) => AsyncIterable<infer Y>
            ? (
                ...args: A
              ) => PromiseLike<void> & AsyncIterable<Y> & { cancel(): Promise<IteratorResult<Y>> }
            : M[K] extends (...args: infer A) => infer R
              ? (...args: A) => Promise<Awaited<R>>
              : never;
      }
    : never;

/**
 * Build a typed proxy for a plugin's frontend API.
 *
 * @example
 * import type Plugin from "./backend"
 * import { createPluginClient } from "@tynd/core/client"
 * export const terminal = createPluginClient<typeof Plugin>("terminal")
 */
export function createPluginClient<
  P extends PluginShape<Record<string, (...a: never[]) => unknown>>,
>(name: string): PluginClient<P> {
  return new Proxy({} as PluginClient<P>, {
    get(_target, prop: string | symbol) {
      if (typeof prop !== "string") return undefined;
      // Avoid accidental thenable behaviour on the proxy itself.
      if (prop === "then" || prop === "catch" || prop === "finally") return undefined;

      if (!window.__tynd__) {
        tynd.error("window.__tynd__ is not available — are you running outside a tynd app?");
        return () => Promise.reject(new Error("[tynd] not in a tynd app context"));
      }

      return (...args: unknown[]) => window.__tynd__.call(`$plugin$${name}$${prop}`, args);
    },
  }) as PluginClient<P>;
}
