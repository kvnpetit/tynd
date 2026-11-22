/**
 * `@tynd/core/plugin` — isomorphic plugin primitives, safe to import from a
 * backend, a frontend or a plugin client (no Bun or node imports).
 * A Tynd plugin is a small npm package that exports a `definePlugin({ name,
 * methods })` value from its `backend.ts` and a matching `createPluginClient`
 * from its `client.ts`. Users never register plugins themselves: the CLI
 * injects the registration at the top of the built backend bundle, driven by
 * the `plugins` field in `tynd.config.ts`.
 * Wire format: plugin methods ride the existing `call` IPC path with a
 * reserved `fn` string of the form `$plugin$<name>$<method>`. The host
 * treats these exactly like normal calls (streaming, credit, cancel); the
 * runtime routes dispatch to the plugin registry instead of the user's
 * backend module.
 */

import { assertPluginName } from "./plugin-wire";

export { createPluginClient, type PluginClient } from "./client/plugin";

/** Compile-time constraint that preserves each plugin method's concrete parameters. */
export type PluginMethod = (...args: never[]) => unknown;

/**
 * Structural view of a hook container (the `hookable` instance built by
 * `defineTyndPlugin` in `@tynd/plugin-sdk`). Kept structural so core carries
 * no dependency on the SDK or on `hookable`.
 */
export interface PluginHooks {
  callHook(name: never, ...args: never[]): unknown;
}

export interface PluginDefinition<T extends Record<string, PluginMethod>> {
  name: string;
  methods: T;
}

/**
 * Identity carrier returned by `definePlugin`. The `__tynd_plugin__` tag
 * lets host + bundler recognize the shape at runtime even after tree
 * shaking renames other fields.
 */
export interface Plugin<T extends Record<string, PluginMethod> = Record<string, PluginMethod>> {
  readonly __tynd_plugin__: true;
  readonly name: string;
  readonly methods: T;
  /** Lifecycle hooks, present on plugins built with `defineTyndPlugin`. */
  readonly hooks?: PluginHooks;
}

/**
 * Declare a Tynd plugin. Call once per plugin, export the result as the
 * default from your `backend.ts`.
 * @example
 * export default definePlugin({
 *   name: "counter",
 *   methods: {
 *     increment() { count += 1; return count },
 *     reset()     { count = 0 },
 *   },
 * })
 */
export function definePlugin<T extends Record<string, PluginMethod>>(
  def: PluginDefinition<T>,
): Plugin<T> {
  assertPluginName(def.name, "definePlugin");
  if (!def.methods || typeof def.methods !== "object") {
    throw new Error(`definePlugin(${def.name}): \`methods\` must be an object`);
  }
  return {
    __tynd_plugin__: true,
    name: def.name,
    methods: def.methods,
  };
}
