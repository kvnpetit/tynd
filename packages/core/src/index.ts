/**
 * @tynd/core — backend API
 * Import from "@tynd/core" in your Bun/Node backend only.
 * For frontend use: import from "@tynd/core/client"
 */

export type {
  AppConfig,
  Capability,
  DownloadPolicy,
  Emitter,
  EmitterMap,
  GpuPreference,
  MenuAction,
  MenuCheckbox,
  MenuItem,
  MenuRadio,
  MenuRole,
  MenuSeparator,
  MenuSubmenu,
  NewWindowPolicy,
  ObservabilityConfig,
  SecurityPolicy,
  SecurityScope,
  TrayConfig,
  TrayMenuItem,
  WindowConfig,
} from "./types";

import * as v from "valibot";
import { getEmitFn, lifecycleFlags, onCloseFns, onReadyFns, runHandler } from "./lifecycle";
import { tynd } from "./logger";
import { startBun } from "./runtime/bun";
import { type Middleware, useMiddleware } from "./runtime/middleware";
import {
  type AppConfig,
  AppConfigSchema,
  configIssues,
  type Emitter,
  type EmitterMap,
} from "./types";

export { TyndIpcError, type TyndIpcErrorCode } from "./errors";
export { clearFrontendListeners, onFrontendEmit } from "./frontend-events";
export { hostCall } from "./host";
export {
  definePlugin,
  type Plugin,
  type PluginDefinition,
  type PluginHooks,
  type PluginMethod,
} from "./plugin";
export { type CallContext, getCallContext } from "./runtime/context";
export type { Middleware, MiddlewareContext } from "./runtime/middleware";
/**
 * @internal Called by the `bootstrap.ts` the CLI generates (one call per
 * plugin in `tynd.config.ts::plugins`); apps and plugins never call it.
 */
export { registerPlugin } from "./runtime/plugin-registry";
export { type BunWorkerHandle, bunWorkers } from "./workers";

/**
 * Evaluate raw JavaScript inside a webview. Useful for DevTools-style
 * tools or forcing a frontend module to re-init without a reload. When
 * `label` is omitted the script runs on every open window.
 * @example
 *   evalInFrontend(`document.body.classList.add('debug')`)
 *   evalInFrontend(`window.location.reload()`, "settings")
 */
export function evalInFrontend(script: string, label?: string): void {
  const fn = getEmitFn();
  if (!fn) {
    tynd.warn("evalInFrontend() before app.start() — dropped");
    return;
  }
  fn("__tynd:eval__", { script }, label);
}

/**
 * Create a typed event emitter. Export the result from your backend module.
 * The frontend subscribes via `api.on("eventName", handler)`.
 * @example
 * export const events = createEmitter<{ userCreated: User }>()
 * events.emit("userCreated", newUser)
 */
export function createEmitter<T extends EmitterMap>(): Emitter<T> {
  return {
    __tynd_emitter__: true,
    __tynd_event_types__: undefined as unknown as T,
    emit<K extends keyof T>(event: K & string, payload: T[K]) {
      const fn = getEmitFn();
      if (!fn) {
        tynd.warn(`emit("${event}") before app.start() — event dropped`);
        return;
      }
      fn(event, payload);
    },
    emitTo<K extends keyof T>(label: string, event: K & string, payload: T[K]) {
      const fn = getEmitFn();
      if (!fn) {
        tynd.warn(`emitTo("${event}") before app.start() — event dropped`);
        return;
      }
      fn(event, payload, label);
    },
  };
}

let _started = false;

export const app = {
  /**
   * Start the Tynd app. Call this once at the end of your backend entry file.
   * Starts the Bun IPC listener (stdin/stdout).
   * @example
   * app.start({ window: { title: "My App" } })
   */
  start(config: AppConfig = {}): void {
    if (_started) throw new Error("app.start() was already called — call it once per process");
    const result = v.safeParse(AppConfigSchema, config);
    if (!result.success) {
      const issues = configIssues(result.issues)
        .map((i) => (i.path ? `  • ${i.path}: ${i.message}` : `  • ${i.message}`))
        .join("\n");
      tynd.error(`app.start() received invalid config:\n${issues}`);
      throw new Error(`Invalid app.start() config:\n${issues}`);
    }
    _started = true;
    startBun(result.output);
  },

  /**
   * Register a middleware that runs around every backend function and JS
   * plugin method, in registration order. Call `next()` to continue; a throw
   * rejects the call with the regular IPC error. Register before `app.start()`.
   * @example
   * app.use(async (ctx, next) => {
   *   if (ctx.fn === "deleteAll" && ctx.caller?.window !== "admin") throw new Error("denied")
   *   return next()
   * })
   */
  use(mw: Middleware): void {
    useMiddleware(mw);
  },

  /** Fires when the WebView window is ready (page fully loaded) */
  onReady(fn: () => void): void {
    if (lifecycleFlags.readyFired) {
      queueMicrotask(() => runHandler("ready", fn));
      return;
    }
    onReadyFns.push(fn);
  },

  /** Fires when the window is about to close */
  onClose(fn: () => void): void {
    if (lifecycleFlags.closeFired) {
      queueMicrotask(() => runHandler("close", fn));
      return;
    }
    onCloseFns.push(fn);
  },
};
