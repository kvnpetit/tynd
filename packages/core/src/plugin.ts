/**
 * Plugin runtime primitives.
 *
 * A Tynd plugin is a small npm package that exports a `definePlugin({ name,
 * methods })` value from its `backend.ts` and a matching `createPluginClient`
 * from its `client.ts`. Users never call `registerPlugin` themselves — the
 * CLI injects calls at the top of the built backend bundle, driven by the
 * `plugins` field in `tynd.config.ts` (or the bundled-plugins list for
 * preset runtimes).
 *
 * Wire format: plugin methods ride the existing `call` IPC path with a
 * reserved `fn` string of the form `$plugin$<name>$<method>`. The host
 * treats these exactly like normal calls (streaming, credit, cancel); the
 * runtime just routes dispatch to the plugin registry instead of the
 * user's backend module.
 */

/** Wire prefix for plugin-invoked methods. See dispatch in `runtime/*.ts`. */
export const PLUGIN_WIRE_PREFIX = "$plugin$"

// biome-ignore lint/suspicious/noExplicitAny: generic plugin methods
export type PluginMethod = (...args: any[]) => unknown

export interface PluginDefinition<T extends Record<string, PluginMethod>> {
  name: string
  methods: T
}

/**
 * Identity carrier returned by `definePlugin`. The `__tynd_plugin__` tag
 * lets host + bundler recognize the shape at runtime even after tree
 * shaking renames other fields.
 */
export interface Plugin<T extends Record<string, PluginMethod> = Record<string, PluginMethod>> {
  readonly __tynd_plugin__: true
  readonly name: string
  readonly methods: T
}

/**
 * Declare a Tynd plugin. Call once per plugin, export the result as the
 * default from your `backend.ts`.
 *
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
  if (!def.name) throw new Error("definePlugin: `name` is required")
  if (!def.methods || typeof def.methods !== "object") {
    throw new Error(`definePlugin(${def.name}): \`methods\` must be an object`)
  }
  return {
    __tynd_plugin__: true,
    name: def.name,
    methods: def.methods,
  }
}

/** Narrow an arbitrary value to a `Plugin`. */
export function isPlugin(x: unknown): x is Plugin {
  return (
    typeof x === "object" &&
    x !== null &&
    (x as { __tynd_plugin__?: boolean }).__tynd_plugin__ === true
  )
}

/**
 * Plugin methods live on a well-known global so the lite runtime's dispatch
 * shim (a separate JS file evaluated into the same QuickJS context) can
 * route calls without importing user modules. Full-mode code also reads
 * through the same map for symmetry — one source of truth across runtimes.
 */
interface PluginGlobals {
  __tynd_plugin_methods__?: Record<string, Record<string, PluginMethod>>
}

function getGlobalRegistry(): Record<string, Record<string, PluginMethod>> {
  const g = globalThis as unknown as PluginGlobals
  if (!g.__tynd_plugin_methods__) g.__tynd_plugin_methods__ = {}
  return g.__tynd_plugin_methods__
}

/**
 * Register a plugin with the current runtime. Invoked by the header the
 * CLI injects at the top of the built backend bundle — one call per
 * plugin listed in `tynd.config.ts::plugins` (or per bundled default
 * for preset runtimes).
 *
 * Idempotent: re-registering the same name is a no-op. Also validates
 * that the name doesn't clash with the wire-format delimiter so the
 * dispatcher can parse `$plugin$<name>$<method>` unambiguously.
 *
 * Writes once to the registry entry (no read-then-write gap) so
 * parallel registrations of the same plugin resolve deterministically
 * rather than racing over partial state.
 */
export function registerPlugin(plugin: Plugin): void {
  if (plugin.name.includes("$")) {
    throw new Error(`Plugin name "${plugin.name}" contains "$" — reserved for the IPC wire format`)
  }
  const registry = getGlobalRegistry()
  if (registry[plugin.name]) return
  const methods: Record<string, PluginMethod> = {}
  for (const [key, fn] of Object.entries(plugin.methods)) {
    if (typeof fn !== "function") {
      // Silently dropping a misconfigured export makes every "why is my
      // method undefined on the client?" debugging session impossible.
      throw new Error(
        `definePlugin(${plugin.name}): method "${key}" is not a function (got ${typeof fn})`,
      )
    }
    if (key.includes("$")) {
      throw new Error(
        `definePlugin(${plugin.name}): method "${key}" contains "$" — reserved for the IPC wire format`,
      )
    }
    methods[key] = fn
  }
  // Single assignment — the JS engine writes the property atomically,
  // so a concurrent read either sees the old value (undefined) or the
  // final map, never a half-populated object.
  registry[plugin.name] = methods
}

/**
 * Look up a plugin method by name/method pair. Returns `null` when unknown;
 * callers translate that into an IPC error.
 */
export function getPluginMethod(plugin: string, method: string): PluginMethod | null {
  const registry = getGlobalRegistry()
  return registry[plugin]?.[method] ?? null
}

/**
 * Split a wire-format plugin fn (`$plugin$<name>$<method>`) into its parts.
 * Returns `null` when the string isn't a plugin call — lets dispatchers
 * fall through to the normal module-export path.
 */
export function parsePluginCall(wireFn: string): { plugin: string; method: string } | null {
  if (!wireFn.startsWith(PLUGIN_WIRE_PREFIX)) return null
  const rest = wireFn.slice(PLUGIN_WIRE_PREFIX.length)
  const sep = rest.indexOf("$")
  if (sep <= 0 || sep === rest.length - 1) return null
  return { plugin: rest.slice(0, sep), method: rest.slice(sep + 1) }
}
