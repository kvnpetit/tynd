import { definePlugin, type Plugin, type PluginMethod } from "@tynd/core/plugin";
import { createHooks, type Hookable } from "hookable";

const TYND_PLUGIN_HOOK_NAMES = new Set<keyof TyndPluginHooks>([
  "app:beforeStart",
  "app:ready",
  "app:beforeQuit",
  "window:created",
  "window:closed",
  "ipc:before",
  "ipc:after",
]);

/**
 * Minimal app-config surface visible to plugin hooks. Mirrors the
 * resolved `tynd.config.ts`. Structural shape so the SDK avoids a
 * build-time dep on `@tynd/cli`.
 */
export interface TyndAppConfig {
  /**
   * Backend entry path. Present on the resolved `tynd.config.ts`, but absent
   * from the runtime config `app:beforeStart` receives (the `app.start()` options).
   */
  backend?: string;
  frontendDir?: string;
  window?: {
    title?: string;
    width?: number;
    height?: number;
  };
  [key: string]: unknown;
}

/** Window handle visible to plugin hooks. Concrete shape provided by the host. */
export interface TyndWindowHandle {
  label: string;
  [key: string]: unknown;
}

/**
 * Plugin context — first argument to every hook. Carries the logger +
 * scratch bag for hook-to-hook state. The host adds more fields
 * (config, window registry, …) at runtime.
 */
export interface PluginContext {
  /** Plugin name — `@tynd/plugin-<name>` from `package.json`. */
  readonly name: string;
  /** Scoped logger; writes to the backend's stderr, prefixed with the plugin name. */
  readonly log: {
    debug(msg: string, data?: unknown): void;
    info(msg: string, data?: unknown): void;
    warn(msg: string, data?: unknown): void;
    error(msg: string, data?: unknown): void;
  };
  /** Per-plugin scratch state — survives across hook calls. */
  readonly state: Record<string, unknown>;
}

/**
 * Hook map — one entry per lifecycle event. Hooks may return a Promise;
 * the runtime awaits each plugin's hook in registration order. Hooks run
 * inside the call's scope, so `getCallContext()` from `@tynd/core` works in
 * `ipc:*` hooks. `ipc:before` may throw to reject the call; `ipc:after` only
 * fires after a successful call. `window:*` payloads carry the window label.
 * Naming: `<scope>:<verb>` where verb is `before<X>` / `after<X>` /
 * `<noun>` (no verb for fire-and-forget events).
 */
export interface TyndPluginHooks {
  /** Fired once before `app.start()` resolves the app config. */
  "app:beforeStart": (cx: PluginContext, config: TyndAppConfig) => void | Promise<void>;
  /** Fired once after the host process is fully initialised. */
  "app:ready": (cx: PluginContext) => void | Promise<void>;
  /** Fired once when the host begins shutdown (before any window closes). */
  "app:beforeQuit": (cx: PluginContext) => void | Promise<void>;
  /** Fired for every window the host creates (primary + spawned). */
  "window:created": (cx: PluginContext, win: TyndWindowHandle) => void | Promise<void>;
  /** Fired for every window the host destroys. */
  "window:closed": (cx: PluginContext, win: TyndWindowHandle) => void | Promise<void>;
  /** Fired before any IPC command dispatch — hooks may inspect / reject. */
  "ipc:before": (cx: PluginContext, cmd: string, args: unknown) => void | Promise<void>;
  /** Fired after a successful IPC command dispatch — hooks may inspect. */
  "ipc:after": (cx: PluginContext, cmd: string, result: unknown) => void | Promise<void>;
}

/** Plugin definition — accepted by `defineTyndPlugin`. */
export interface TyndPluginDefinition<
  M extends Record<string, PluginMethod> = Record<string, never>,
> {
  /** `@tynd/plugin-<name>` from `package.json`. Used in CODEOWNERS, logs, errors. */
  name: string;
  /** Optional IPC commands (same shape as `definePlugin`). */
  methods?: M;
  /** Optional lifecycle hooks. Partial — declare only what you need. */
  hooks?: Partial<TyndPluginHooks>;
}

/** Runtime shape returned by `defineTyndPlugin` — consumed by the dispatcher. */
export interface TyndPlugin<M extends Record<string, PluginMethod>> extends Plugin<M> {
  readonly hooks: Hookable<TyndPluginHooks>;
}

/**
 * Define a Tynd plugin combining IPC methods + lifecycle hooks. Plugins
 * that only need IPC methods can keep using `definePlugin` from
 * `@tynd/core`; reach for `defineTyndPlugin` when the plugin needs to
 * react to `app:ready`, window events or intercept IPC.
 */
export function defineTyndPlugin<M extends Record<string, PluginMethod>>(
  def: TyndPluginDefinition<M>,
): TyndPlugin<M> {
  const hooks = createHooks<TyndPluginHooks>();
  if (def.hooks) {
    for (const [name, fn] of Object.entries(def.hooks) as Array<
      [keyof TyndPluginHooks, TyndPluginHooks[keyof TyndPluginHooks]]
    >) {
      if (!TYND_PLUGIN_HOOK_NAMES.has(name)) {
        throw new Error(`@tynd/plugin-sdk: unknown hook '${String(name)}' on plugin '${def.name}'`);
      }
      if (typeof fn !== "function") {
        throw new TypeError(
          `@tynd/plugin-sdk: hook '${String(name)}' on plugin '${def.name}' must be a function (got ${typeof fn})`,
        );
      }
      hooks.hook(name, fn);
    }
  }
  const descriptor = definePlugin({
    name: def.name,
    methods: (def.methods ?? {}) as M,
  });
  return {
    ...descriptor,
    hooks,
  };
}

export type { Hookable } from "hookable";
export { createHooks } from "hookable";
