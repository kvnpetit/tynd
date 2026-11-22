/**
 * Runtime plugin registry: where `registerPlugin` stores each plugin's
 * methods and hooks, and where the dispatcher looks them up. Backend only.
 * `registerPlugin` is re-exported from `@tynd/core` for the bootstrap the CLI
 * generates; the rest is internal to the runtime.
 */

import type { Plugin, PluginHooks } from "../plugin";
import {
  assertPluginName,
  PLUGIN_METHOD_PATTERN,
  PLUGIN_NAME_PATTERN,
  PLUGIN_WIRE_PREFIX,
} from "../plugin-wire";

/** Runtime registry boundary: decoded IPC arguments are not typed until invocation. */
type RegisteredPluginMethod = (...args: unknown[]) => unknown;

/**
 * Plugin methods live on a well-known global so runtime dispatch can route
 * calls without importing user modules.
 */
interface PluginGlobals {
  __tynd_plugin_methods__?: Record<string, Record<string, RegisteredPluginMethod>>;
}

/** A registered plugin's hook container plus its per-plugin scratch state. */
export interface RegisteredPluginHooks {
  readonly name: string;
  readonly hooks: PluginHooks;
  readonly state: Record<string, unknown>;
}

interface PluginHookGlobals {
  __tynd_plugin_hooks__?: Map<string, RegisteredPluginHooks>;
}

/** Registration-ordered hook registry, shared via `globalThis` like the method registry. */
function getHookRegistry(): Map<string, RegisteredPluginHooks> {
  const g = globalThis as unknown as PluginHookGlobals;
  if (!g.__tynd_plugin_hooks__) g.__tynd_plugin_hooks__ = new Map();
  return g.__tynd_plugin_hooks__;
}

/** Plugins that declared lifecycle hooks, in registration order. */
export function getRegisteredPluginHooks(): RegisteredPluginHooks[] {
  return Array.from(getHookRegistry().values());
}

/** @internal Test helper: forget every registered plugin and hook. */
export function resetPluginRegistry(): void {
  const g = globalThis as unknown as PluginGlobals & PluginHookGlobals;
  g.__tynd_plugin_methods__ = undefined;
  g.__tynd_plugin_hooks__ = undefined;
}

function getGlobalRegistry(): Record<string, Record<string, RegisteredPluginMethod>> {
  const g = globalThis as unknown as PluginGlobals;
  // `Object.create(null)` so a webview-side caller can't reach
  // `Object.prototype` methods via crafted plugin/method names like
  // `$plugin$__proto__$valueOf`.
  if (!g.__tynd_plugin_methods__)
    g.__tynd_plugin_methods__ = Object.create(null) as Record<
      string,
      Record<string, RegisteredPluginMethod>
    >;
  return g.__tynd_plugin_methods__;
}

/** Original (unbound) method functions behind each registered method map, to recognise a repeat registration. */
const registrationSources = new WeakMap<object, Record<string, unknown>>();

function isSameRegistration(
  existing: Record<string, RegisteredPluginMethod>,
  plugin: Plugin,
): boolean {
  const sources = registrationSources.get(existing);
  // Registered by another copy of this module: cannot tell, keep the old no-op behaviour.
  if (!sources) return true;
  const incoming = Object.entries(plugin.methods);
  if (incoming.length !== Object.keys(sources).length) return false;
  return incoming.every(([key, fn]) => sources[key] === fn);
}

/**
 * Register a plugin with the current runtime. Invoked by the header the
 * CLI injects at the top of the built backend bundle — one call per
 * plugin listed in `tynd.config.ts::plugins`.
 * Idempotent: re-registering the same plugin is a no-op; a different plugin
 * under an already-registered name throws. Also validates
 * that the name doesn't clash with the wire-format delimiter so the
 * dispatcher can parse `$plugin$<name>$<method>` unambiguously.
 * Writes once to the registry entry (no read-then-write gap) so
 * parallel registrations of the same plugin resolve deterministically
 * rather than racing over partial state.
 */
export function registerPlugin(plugin: Plugin): void {
  assertPluginName(plugin.name, "registerPlugin");
  const registry = getGlobalRegistry();
  const existing = registry[plugin.name];
  if (existing) {
    // Same plugin registered again (header run twice): no-op. A different
    // plugin under the same name would silently shadow its methods with
    // UNKNOWN_FN errors, so refuse it.
    if (!isSameRegistration(existing, plugin)) {
      throw new Error(
        `registerPlugin: a different plugin is already registered under the name "${plugin.name}"`,
      );
    }
    return;
  }
  const methods: Record<string, RegisteredPluginMethod> = Object.create(null);
  const sources: Record<string, unknown> = Object.create(null);
  for (const [key, fn] of Object.entries(plugin.methods)) {
    if (typeof fn !== "function") {
      // Silently dropping a misconfigured export makes every "why is my
      // method undefined on the client?" debugging session impossible.
      throw new Error(
        `definePlugin(${plugin.name}): method "${key}" is not a function (got ${typeof fn})`,
      );
    }
    if (!PLUGIN_METHOD_PATTERN.test(key)) {
      throw new Error(
        `definePlugin(${plugin.name}): method "${key}" must match ^[A-Za-z_][A-Za-z0-9_-]{0,63}$`,
      );
    }
    // Bind to the plugin's `methods` object so `this.other()` keeps working.
    methods[key] = (fn as RegisteredPluginMethod).bind(plugin.methods);
    sources[key] = fn;
  }
  registrationSources.set(methods, sources);
  // Single assignment — the JS engine writes the property atomically,
  // so a concurrent read either sees the old value (undefined) or the
  // final map, never a half-populated object.
  registry[plugin.name] = methods;
  if (plugin.hooks && typeof plugin.hooks.callHook === "function") {
    getHookRegistry().set(plugin.name, { name: plugin.name, hooks: plugin.hooks, state: {} });
  }
}

/**
 * Look up a plugin method by name/method pair. Returns `null` when unknown;
 * callers translate that into an IPC error.
 */
export function getPluginMethod(plugin: string, method: string): RegisteredPluginMethod | null {
  const registry = getGlobalRegistry();
  if (!Object.hasOwn(registry, plugin)) return null;
  const methods = registry[plugin];
  if (!methods || !Object.hasOwn(methods, method)) return null;
  return methods[method] ?? null;
}

/**
 * Split a wire-format plugin fn (`$plugin$<name>$<method>`) into its parts.
 * Returns `null` when the string isn't a plugin call — lets dispatchers
 * fall through to the normal module-export path.
 */
export function parsePluginCall(wireFn: string): { plugin: string; method: string } | null {
  if (!wireFn.startsWith(PLUGIN_WIRE_PREFIX)) return null;
  const rest = wireFn.slice(PLUGIN_WIRE_PREFIX.length);
  const sep = rest.indexOf("$");
  if (sep <= 0 || sep === rest.length - 1) return null;
  const plugin = rest.slice(0, sep);
  const method = rest.slice(sep + 1);
  if (!PLUGIN_NAME_PATTERN.test(plugin) || !PLUGIN_METHOD_PATTERN.test(method)) return null;
  return { plugin, method };
}
