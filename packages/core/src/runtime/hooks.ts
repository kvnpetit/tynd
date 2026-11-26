/**
 * Plugin lifecycle hook dispatch (A1).
 * Plugins built with `defineTyndPlugin` register a hook container through
 * `registerPlugin`. This module fires those hooks: `app:*` and `window:*`
 * are fire-and-isolate, `ipc:before` / `ipc:after` ride the middleware
 * chain (a throw from `ipc:before` rejects the call).
 */

import { tynd } from "../logger";
import { PLUGIN_WIRE_PREFIX } from "../plugin-wire";
import { sanitizedMessage } from "./error";
import type { Middleware } from "./middleware";
import { getRegisteredPluginHooks, type RegisteredPluginHooks } from "./plugin-registry";

type HookFn = (name: string, ...args: unknown[]) => unknown;
type LogLevel = "debug" | "info" | "warn" | "error";

function makePluginContext(entry: RegisteredPluginHooks) {
  const prefix = `[plugin:${entry.name}]`;
  const sink = (level: LogLevel) => (msg: string, data?: unknown) => {
    // biome-ignore lint/suspicious/noConsole: plugin-facing logger; console is redirected to stderr by the runtime
    if (data === undefined) console[level](prefix, msg);
    // biome-ignore lint/suspicious/noConsole: plugin-facing logger; console is redirected to stderr by the runtime
    else console[level](prefix, msg, data);
  };
  return {
    name: entry.name,
    log: {
      debug: sink("debug"),
      info: sink("info"),
      warn: sink("warn"),
      error: sink("error"),
    },
    state: entry.state,
  };
}

/** Whether at least one registered plugin declared hooks. */
function hasHookPlugins(): boolean {
  return getRegisteredPluginHooks().length > 0;
}

async function callEach(name: string, args: unknown[], strict: boolean): Promise<void> {
  for (const entry of getRegisteredPluginHooks()) {
    try {
      await (entry.hooks.callHook as unknown as HookFn)(name, makePluginContext(entry), ...args);
    } catch (error) {
      if (strict) throw error;
      tynd.error(`plugin "${entry.name}" hook ${name} failed: ${sanitizedMessage(error)}`);
    }
  }
}

/**
 * Fire a hook on every plugin that declared hooks, in registration order,
 * awaiting each. A failing hook is logged and never blocks the others.
 */
export async function fireHook(name: string, ...args: unknown[]): Promise<void> {
  if (!hasHookPlugins()) return;
  await callEach(name, args, false);
}

/** Like `fireHook` but the first failing hook rejects (used by `ipc:before`). */
async function fireHookStrict(name: string, ...args: unknown[]): Promise<void> {
  if (!hasHookPlugins()) return;
  await callEach(name, args, true);
}

/**
 * Outermost middleware: maps plugin `ipc:before` / `ipc:after` hooks onto
 * the chain. `cmd` is the raw wire name (`$plugin$<name>$<method>` for plugin
 * methods). `ipc:before` may throw to reject the call; `ipc:after` only
 * fires after a successful call and cannot change its outcome.
 */
export const ipcHooksMiddleware: Middleware = async (ctx, next) => {
  if (!hasHookPlugins()) return next();
  const cmd = ctx.plugin ? `${PLUGIN_WIRE_PREFIX}${ctx.plugin}$${ctx.fn}` : ctx.fn;
  await fireHookStrict("ipc:before", cmd, ctx.args);
  const result = await next();
  await fireHook("ipc:after", cmd, result);
  return result;
};
