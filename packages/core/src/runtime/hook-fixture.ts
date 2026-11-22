/** Test fixtures: a minimal serial hook container and a recording plugin. */

import type { Plugin } from "../plugin";

type Listener = (...args: unknown[]) => unknown;

export interface RecordedHook {
  plugin: string;
  name: string;
  args: unknown[];
}

/** Serial `callHook`, same contract as `hookable` (awaits listeners in order). */
export function makeHooks(listeners: Record<string, Listener>) {
  return {
    async callHook(name: string, ...args: unknown[]): Promise<void> {
      const fn = listeners[name];
      if (fn) await fn(...args);
    },
  };
}

function expectContext(cx: unknown, name: string): void {
  const c = cx as { name?: unknown; state?: unknown; log?: { info?: unknown } };
  if (c?.name !== name || typeof c.state !== "object" || typeof c.log?.info !== "function") {
    throw new Error(`hook for '${name}' received a malformed PluginContext`);
  }
}

/**
 * Plugin whose every listed hook records `(plugin, name, args-after-cx)` into
 * `log` and then runs the optional per-hook behaviour.
 */
export function recordingPlugin(
  name: string,
  hookNames: string[],
  log: RecordedHook[],
  behaviour: Record<string, Listener> = {},
): Plugin {
  const listeners: Record<string, Listener> = {};
  for (const hook of hookNames) {
    listeners[hook] = async (cx: unknown, ...args: unknown[]) => {
      expectContext(cx, name);
      log.push({ plugin: name, name: hook, args });
      await behaviour[hook]?.(...args);
    };
  }
  return {
    __tynd_plugin__: true,
    name,
    methods: {},
    hooks: makeHooks(listeners) as unknown as Plugin["hooks"],
  };
}
