/**
 * Types for the command catalog emitted by the Rust plugin macro
 * `#[tynd_plugin::commands]` into a plugin's `*.gen.ts`. The runtime
 * client is `defineTypedPlugin<C>(name)` in `@tynd/plugin-sdk`, which
 * routes every call through `__tynd__.os_call("plugin", "<name>:<method>")`.
 */

/**
 * Shape of a command catalog as emitted by `#[tynd_plugin::commands]`.
 * Each key is a method name; each value is an `{ args, result }` pair
 * carrying type-only phantoms.
 */
export type CommandCatalog = Record<string, { args: unknown; result: unknown }>;

/**
 * Map a catalog to a typed client object: each command becomes a
 * `(args) => Promise<result>` method. `void` / `null` results surface
 * as `Promise<void>` so callers don't have to deal with `null`.
 */
export type TypedClient<C extends CommandCatalog> = {
  [K in keyof C]: C[K]["args"] extends undefined | null
    ? () => C[K]["result"] extends undefined | null ? Promise<void> : Promise<C[K]["result"]>
    : (
        args: C[K]["args"],
      ) => C[K]["result"] extends undefined | null ? Promise<void> : Promise<C[K]["result"]>;
};
