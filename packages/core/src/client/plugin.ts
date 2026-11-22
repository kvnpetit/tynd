/**
 * Frontend plugin client. Mirrors `definePlugin` on the backend: you call
 * `createPluginClient<typeof Plugin>("foo")` once in your plugin's
 * `client.ts`, and consumers get a fully typed proxy that forwards method
 * calls as Tynd RPC.
 * Wire format: method `foo.bar(x, y)` becomes an RPC call with fn
 * `$plugin$foo$bar` and args `[x, y]`. The runtime dispatcher (`bun.ts` /
 * `globals.js`) recognizes the prefix and routes to the plugin registry
 * populated by the CLI from `tynd.config.ts::plugins`.
 */

import type { Plugin, PluginMethod } from "../plugin";
import { createCallProxy } from "./_internal";

/**
 * Map a plugin's backend method signatures to frontend client signatures.
 * Streaming generators surface as `AsyncIterable` promises; regular
 * functions surface as plain `Promise<Awaited<R>>`.
 */
export type PluginClient<P> =
  P extends Plugin<infer M>
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
 * @example
 * import type Plugin from "./backend"
 * import { createPluginClient } from "@tynd/core/plugin"
 * export const terminal = createPluginClient<typeof Plugin>("terminal")
 */
export function createPluginClient<P extends Plugin<Record<string, PluginMethod>>>(
  name: string,
): PluginClient<P> {
  // Plugin RPC: prop name is wrapped in the `$plugin$<name>$<method>`
  // wire envelope so the runtime dispatcher routes to the plugin registry.
  return createCallProxy<PluginClient<P> & object>(
    (prop) => `$plugin$${name}$${prop}`,
  ) as PluginClient<P>;
}
