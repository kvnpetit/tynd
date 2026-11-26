export type { CommandCatalog, TypedClient } from "@tynd/core/client";
export {
  createPluginClient,
  definePlugin,
  type Plugin,
  type PluginClient,
  type PluginDefinition,
  type PluginMethod,
} from "@tynd/core/plugin";

import type { StandardSchemaV1 } from "@standard-schema/spec";
import type {
  CommandCatalog as _CommandCatalog,
  TypedClient as _TypedClient,
} from "@tynd/core/client";
import { osCall, osOn } from "@tynd/core/client/bridge";
import { definePlugin, type Plugin, type PluginMethod } from "@tynd/core/plugin";

/** Single-consumer queue that adapts push-based plugin events to an async iterable. */
export class AsyncEventQueue<T> implements AsyncIterable<T> {
  private readonly buffer: T[] = [];
  private pending: ((value: IteratorResult<T>) => void) | null = null;
  private ended = false;
  private error: string | null = null;

  get done(): boolean {
    return this.ended;
  }

  push(value: T): void {
    if (this.ended) return;
    if (this.pending) {
      const resolve = this.pending;
      this.pending = null;
      resolve({ value, done: false });
      return;
    }
    this.buffer.push(value);
  }

  finish(error?: string): void {
    if (this.ended) return;
    this.ended = true;
    if (error !== undefined) this.error = error;
    if (!this.pending) return;
    const resolve = this.pending;
    this.pending = null;
    resolve({ value: undefined, done: true });
  }

  async *[Symbol.asyncIterator](): AsyncIterator<T> {
    while (true) {
      if (this.buffer.length > 0) {
        yield this.buffer.shift() as T;
        continue;
      }
      if (this.ended) {
        if (this.error !== null) throw new Error(this.error);
        return;
      }
      const next = await new Promise<IteratorResult<T>>((resolve) => {
        this.pending = resolve;
      });
      if (next.done) {
        if (this.error !== null) throw new Error(this.error);
        return;
      }
      yield next.value;
    }
  }
}

const PLUGIN_FIELDS = ["name", "methods", "__tynd_plugin__"] as const;
const PROMISE_FIELDS = ["then", "catch", "finally"] as const;
const NOOP = (): void => undefined;

function isPromiseField(prop: string): boolean {
  return (PROMISE_FIELDS as readonly string[]).includes(prop);
}

function pluginField<T extends Record<string, PluginMethod>>(
  descriptor: Plugin<T>,
  prop: string,
): unknown {
  switch (prop) {
    case "__tynd_plugin__":
      return descriptor.__tynd_plugin__;
    case "name":
      return descriptor.name;
    case "methods":
      return descriptor.methods;
    default:
      return;
  }
}

function hasOsOn(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof (window as unknown as { __tynd__?: { os_on?: unknown } }).__tynd__?.os_on === "function"
  );
}

function hasOsCall(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof (window as unknown as { __tynd__?: { os_call?: unknown } }).__tynd__?.os_call ===
      "function"
  );
}

/**
 * Single-import plugin entry. Returns one object that is **both** the
 * registration descriptor and typed native client.
 * Reads the catalog *type* from the plugin's `*.gen.ts` and turns every
 * command into a typed RPC method. The plugin's `client.ts` exports it;
 * the Rust side lives in `native/`.
 * @example
 *   // plugins/sql/client.ts
 *   import { defineTypedPlugin } from "@tynd/plugin-sdk";
 *   import type { CommandsCatalog } from "./sql.gen";
 *   export const sql = defineTypedPlugin<CommandsCatalog>("sql");
 *   export default sql;
 *   // userland
 *   import { sql } from "@tynd/plugin-sql";
 *   // tynd.config.ts: plugins: ["@tynd/plugin-sql"]
 *   await sql.open({ path: ":memory:" });
 * For plugins that need rich wrappers (handles, async iterables), compose
 * the typed client with explicit overrides:
 * @example
 *   const _api = defineTypedPlugin<CommandsCatalog>("sql");
 *   export const sql = composePluginClient(_api, {
 *     async open(path: string): Promise<SqlConnection> {
 *       const { id } = await _api.open({ path });
 *       return makeConn(id, _api);
 *     },
 *   });
 */
export function defineTypedPlugin<C extends _CommandCatalog>(
  name: string,
): _TypedClient<C> & Plugin<Record<string, never>> {
  // Backend lives in Rust: `methods` stays empty and frontend calls go
  // directly through the native plugin dispatcher.
  const descriptor = definePlugin({ name, methods: {} as Record<string, never> });

  return new Proxy(descriptor as _TypedClient<C> & Plugin<Record<string, never>>, {
    get(_, prop: string | symbol) {
      if (typeof prop === "symbol") return;
      const field = pluginField(descriptor, prop);
      if (field !== undefined) return field;
      // Keep the proxy non-thenable: `await typedPlugin` must not fire
      // an IPC call against `${name}:then`.
      if (isPromiseField(prop)) return;
      return (args: unknown = null) => {
        if (!hasOsCall()) {
          return Promise.reject(
            new Error(
              `@tynd/plugin-${name}: __tynd__.os_call missing — are you running inside a Tynd app?`,
            ),
          );
        }
        // Through the core bridge so a dead backend rejects with a typed `TyndIpcError`.
        return osCall("plugin", `${name}:${prop}`, args);
      };
    },
  });
}

type ReservedPluginOverrides = {
  readonly __tynd_plugin__?: never;
  readonly name?: never;
  readonly methods?: never;
  readonly then?: never;
  readonly catch?: never;
  readonly finally?: never;
};

/**
 * Add ergonomic methods to a plugin client without losing its dynamic RPC
 * surface or, for native clients, its registration descriptor.
 *
 * Typed RPC methods do not exist as enumerable JavaScript properties: the
 * proxy creates them on demand. `Object.assign({}, plugin)` therefore cannot
 * copy them. This compositor keeps overrides as real own properties and
 * delegates every other lookup to the typed RPC proxy.
 */
export function composePluginClient<Base extends object, Overrides extends object>(
  client: Base,
  overrides: Overrides & ReservedPluginOverrides,
): Omit<Base, keyof Overrides> & Overrides {
  for (const reserved of [...PLUGIN_FIELDS, ...PROMISE_FIELDS]) {
    if (Object.hasOwn(overrides, reserved)) {
      throw new TypeError(`composePluginClient: override "${reserved}" is reserved`);
    }
  }
  const target = Object.assign({}, client, overrides);
  for (const field of PLUGIN_FIELDS) {
    const descriptor = Reflect.getOwnPropertyDescriptor(client, field);
    if (descriptor) Reflect.defineProperty(target, field, descriptor);
  }

  return new Proxy(target, {
    get(current, prop: string | symbol, receiver) {
      if (typeof prop === "string" && isPromiseField(prop)) return;
      if (Object.hasOwn(current, prop)) {
        return Reflect.get(current, prop, receiver);
      }
      return Reflect.get(client, prop);
    },
  }) as Omit<Base, keyof Overrides> & Overrides;
}

/**
 * Subscribe to an event a native plugin emits (`emit("terminal:data", …)`
 * on the Rust side). The host delivers plugin events on the OS event bus,
 * so this listens through `osOn` (`__tynd__.os_on`) under the exact name
 * the plugin emitted. Returns an unsubscribe function; when the host bridge
 * is missing (e.g. unit test) returns a no-op.
 * @example
 *   const off = onHostEvent("shortcut:triggered", (raw) => {
 *     if ((raw as { id?: string }).id === handle.id) handler()
 *   })
 */
export function onHostEvent<T = unknown>(event: string, handler: (payload: T) => void): () => void {
  if (!hasOsOn()) return NOOP;
  return osOn<T>(event, handler);
}

/**
 * Helper for plugin clients that return a handle with id-bound event
 * listeners (terminal sessions, sql connections, shortcut handles, …).
 * Tracks subscriptions created via `on()` and exposes `disposeAll()`
 * to detach them in one shot.
 * Filtering is payload-shape agnostic: pass the field name that carries
 * the handle id (`"id"`, `"connId"`, `"sessionId"`, …). Events whose
 * payload doesn't match are ignored.
 * @example
 *   const events = createHandleEvents(handle.id, "terminal");
 *   const offData = events.on<{ data: string }>("terminal:data", "id", (p) => onData(p.data));
 *   handle.kill = createResourceCloser(
 *     () => api.kill({ id: handle.id }),
 *     () => events.disposeAll(),
 *   );
 */
export function createHandleEvents<Id>(filterId: Id, label?: string) {
  const offs: (() => void)[] = [];
  // `label` tags diagnostic output so a plugin owning many handle types
  // (sql connections, terminal sessions, …) can grep its console output
  // without resorting to stack traces. Disposal-error paths use it too.
  const tag = label ?? "handle-events";
  const disposeAll = (): void => {
    for (const off of [...offs]) {
      try {
        off();
      } catch (err) {
        // best-effort: a broken unsubscribe must not block the rest
        // biome-ignore lint/suspicious/noConsole: diagnostic surface for handle-events disposal failures
        console.debug(`[${tag}] dispose error`, err);
      }
    }
    offs.length = 0;
  };
  const track = (off: () => void): (() => void) => {
    let active = true;
    const dispose = (): void => {
      if (!active) return;
      active = false;
      try {
        off();
      } finally {
        const index = offs.indexOf(dispose);
        if (index >= 0) offs.splice(index, 1);
      }
    };
    offs.push(dispose);
    return dispose;
  };
  return {
    /**
     * Subscribe to a host event, calling `handler` only when
     * `payload[idKey] === filterId`. Returned unsubscribe is idempotent.
     */
    on<P>(event: string, idKey: string, handler: (p: P) => void): () => void {
      const off = onHostEvent<Record<string, unknown> | null>(event, (raw) => {
        if (raw == null || raw[idKey] !== filterId) return;
        handler(raw as P);
      });
      return track(off);
    },
    /** Dispose all tracked listeners after a matching terminal lifecycle event. */
    disposeOn(event: string, idKey: string): void {
      const off = onHostEvent<Record<string, unknown> | null>(event, (raw) => {
        if (raw == null || raw[idKey] !== filterId) return;
        queueMicrotask(disposeAll);
      });
      track(off);
    },
    /** Detach every subscription created via `on()`. Idempotent. */
    disposeAll,
  };
}

/**
 * Build an idempotent resource close function. Remote close completes
 * before local listeners/state are disposed, so a transport failure leaves
 * a live resource usable and retryable. Concurrent calls share one attempt;
 * local cleanup failure retries cleanup without closing remotely twice.
 */
export function createResourceCloser(
  closeRemote: () => Promise<unknown>,
  cleanupLocal: () => void,
): () => Promise<void> {
  let remoteClosed = false;
  let closed = false;
  let pending: Promise<void> | null = null;

  return () => {
    if (closed) return Promise.resolve();
    if (pending) return pending;

    const attempt = async () => {
      if (!remoteClosed) {
        await closeRemote();
        remoteClosed = true;
      }
      cleanupLocal();
      closed = true;
    };
    pending = attempt().finally(() => {
      if (!closed) pending = null;
    });
    return pending;
  };
}

/**
 * Validate a plugin method's `args` against a schema before the
 * round-trip to the native side. Returns the parsed, typed value on
 * success; throws a [`PluginArgsError`] listing every validation issue
 * on failure.
 * Works with any Standard Schema v1 validator (valibot, zod, arktype).
 * Async schemas are rejected.
 * @example
 *   import * as v from "valibot"
 *   import { composePluginClient, defineTypedPlugin, validate } from "@tynd/plugin-sdk"
 *   import type { CommandsCatalog } from "./foo.gen"
 *   const HelloArgs = v.object({ who: v.pipe(v.string(), v.minLength(1)) })
 *   const api = defineTypedPlugin<CommandsCatalog>("foo")
 *   export const foo = composePluginClient(api, {
 *     hello(args: v.InferInput<typeof HelloArgs>) {
 *       return api.hello(validate(HelloArgs, args))
 *     },
 *   })
 */
export function validate<S extends StandardSchemaV1>(
  schema: S,
  args: unknown,
): StandardSchemaV1.InferOutput<S> {
  const res = schema["~standard"].validate(args);
  if (typeof res === "object" && res !== null && "then" in res && typeof res.then === "function") {
    throw new PluginArgsError(["async schemas are not supported for plugin args"]);
  }
  const syncResult = res as StandardSchemaV1.Result<StandardSchemaV1.InferOutput<S>>;
  if (syncResult.issues !== undefined) {
    throw new PluginArgsError(syncResult.issues.map((issue) => issue.message));
  }
  return syncResult.value;
}

/** Error thrown by [`validate`] when args fail schema checks. */
export class PluginArgsError extends Error {
  readonly issues: readonly string[];
  constructor(issues: readonly string[]) {
    const normalizedIssues =
      issues.length > 0 ? issues : ["validation failed without issue details"];
    super(
      normalizedIssues.length === 1
        ? `invalid plugin args: ${normalizedIssues[0]}`
        : `invalid plugin args: ${normalizedIssues.join("; ")}`,
    );
    this.name = "PluginArgsError";
    this.issues = normalizedIssues;
  }
}
