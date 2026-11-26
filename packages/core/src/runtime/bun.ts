import * as v from "valibot";
import { encodeIpcError, type TyndIpcErrorCode } from "../errors";
import { dispatchFrontendEmit, FRONTEND_EMIT_FN } from "../frontend-events";
import { hostCall, resolveHostCall, setHostCallTransport } from "../host";
import { dispatchHostEvent } from "../host-events";
import {
  BEFORE_QUIT_TIMEOUT_MS,
  fireBeforeQuit,
  fireBeforeStart,
  fireClose,
  fireReady,
  setEmitFn,
} from "../lifecycle";
import { tynd } from "../logger";
import type { AppConfig } from "../types";
import { reviveChannels } from "./channel";
import { redirectConsoleToStderr } from "./console";
import { type CallContext, parseCallContext, runWithCallContext } from "./context";
import { sanitizedMessage } from "./error";
import { fireHook } from "./hooks";
import { type MiddlewareContext, runMiddleware } from "./middleware";
import { getPluginMethod, parsePluginCall } from "./plugin-registry";
import { handleProtocolMsg, ProtocolMsgSchema, protocolNames } from "./protocol";
import { decodeArgs, encode as encodeWire } from "./wire";

/**
 * Write an outbound IPC frame: the payload is JSON-stringified and
 * `\n`-terminated.
 */
function writeIpcFrame(payload: Record<string, unknown>): void {
  process.stdout.write(`${JSON.stringify(payload)}\n`);
}

const CallMsgSchema = v.object({
  type: v.literal("call"),
  id: v.string(),
  fn: v.string(),
  // `a` carries devalue-encoded args (string), written by the webview shim.
  a: v.string(),
  // Caller identity appended by the host (contract A4). Untrusted shape:
  // validated by `parseCallContext`, so a malformed value degrades to
  // "no context" instead of rejecting the whole call.
  ctx: v.optional(v.unknown()),
});
const CancelMsgSchema = v.object({
  type: v.literal("cancel"),
  id: v.string(),
});
const AckMsgSchema = v.object({
  type: v.literal("ack"),
  id: v.string(),
  n: v.pipe(v.number(), v.integer(), v.minValue(1)),
});
const LifecycleMsgSchema = v.object({
  type: v.union([v.literal("tynd:ready"), v.literal("tynd:close")]),
});
// Window lifecycle notifications from the host, surfaced to plugins as the
// `window:created` / `window:closed` hooks.
const WindowMsgSchema = v.object({
  type: v.literal("tynd:window"),
  event: v.union([v.literal("created"), v.literal("closed")]),
  label: v.string(),
});
// Native window/webview events forwarded by the host (`getWindow(label).on*`).
const OsEventMsgSchema = v.object({
  type: v.literal("tynd:os_event"),
  name: v.string(),
  data: v.unknown(),
});
const HostResultMsgSchema = v.object({
  type: v.literal("host_result"),
  id: v.string(),
  ok: v.boolean(),
  value: v.unknown(),
});
const IpcMsgSchema = v.union([
  CallMsgSchema,
  CancelMsgSchema,
  AckMsgSchema,
  LifecycleMsgSchema,
  WindowMsgSchema,
  HostResultMsgSchema,
  OsEventMsgSchema,
  ProtocolMsgSchema,
]);

/** Per-stream credit cap. Must match ACK_CHUNK on the shim so replenishment beats starvation. */
const STREAM_CREDIT = 64;

interface StreamState {
  iter: AsyncIterator<unknown>;
  /** How many more chunks the backend may yield before it has to wait. */
  credit: number;
  /** Resolved by the next `ack` when credit was 0 at yield time. */
  waiter: (() => void) | null;
}

const _activeStreams = new Map<string, StreamState>();

/**
 * Abort controller of every call between `call` and its return/stream end.
 * `cancel` aborts it (the handler sees `getCallContext().signal`), and its
 * presence lets a `cancel` that races ahead of stream registration be remembered.
 */
const _inflightCalls = new Map<string, AbortController>();
/** Ids cancelled before their stream was registered; consumed by `streamIterable`. */
const _cancelledEarly = new Set<string>();

/**
 * Cap on an unterminated stdin line; larger implies malformed framing since binary goes via bin scheme.
 * The host rejects renderer frames over 8 MiB (`MAX_IPC_FRAME_BYTES`) and then appends the caller
 * context, so a legal frame can be slightly larger: keep headroom so it never kills the backend.
 */
const MAX_STDIN_BUFFER = 8 * 1024 * 1024 + 1024 * 1024;

/**
 * Marker thrown by handler bodies / encode helpers so the outer `catch`
 * knows to surface a non-default `code` (e.g. PROTOCOL_ERROR for an
 * unencodable return). User handlers' own throws stay the default
 * HANDLER_THREW path.
 */
class IpcDispatchError extends Error {
  readonly ipcCode: TyndIpcErrorCode;

  constructor(ipcCode: TyndIpcErrorCode, message: string, cause?: unknown) {
    super(message, cause !== undefined ? { cause } : undefined);
    this.ipcCode = ipcCode;
    this.name = "IpcDispatchError";
  }
}

function _isAsyncIterable(x: unknown): x is AsyncIterable<unknown> {
  return (
    x != null &&
    (typeof x === "object" || typeof x === "function") &&
    typeof (x as { [Symbol.asyncIterator]?: unknown })[Symbol.asyncIterator] === "function"
  );
}

let _moduleCache: Record<string, unknown> | null = null;

async function getModule(entryPath: string): Promise<Record<string, unknown>> {
  if (!_moduleCache) {
    _moduleCache = (await import(entryPath)) as Record<string, unknown>;
  }
  return _moduleCache;
}

/** Bun boot: console→stderr, `app:beforeStart` hooks, first stdout line = config, then emit+IPC loop. */
export function startBun(config: AppConfig): void {
  redirectConsoleToStderr();

  // The native launcher always injects `TYND_ENTRY`; missing it is a launcher
  // contract violation, not a user error worth silently handling.
  const entry = process.env.TYND_ENTRY;
  if (!entry) {
    tynd.error("TYND_ENTRY env var not set — run this app via `tynd dev` or `tynd start`.");
    process.exit(1);
  }

  // Plugins are registered by the CLI-injected header at the top of the
  // built backend bundle, not here — by the time this function runs the
  // registry is already populated. Their `app:beforeStart` hooks run before
  // anything is written: the host expects the config frame first.
  fireBeforeStartBounded(config)
    .catch((e) => tynd.error(`app:beforeStart: ${sanitizedMessage(e)}`))
    .then(() => bootBun(config, entry))
    .catch((e) => {
      tynd.error(`startup failed: ${sanitizedMessage(e)}`);
      process.exit(1);
    });
}

/**
 * @internal exported for tests. Run plugin `app:beforeStart` hooks, giving up (with a warning) after
 * `timeoutMs` so a stuck hook cannot run into the host's generic startup timeout. These hooks run
 * before the host-call transport exists: a hook that calls a host API would never resolve.
 */
export async function fireBeforeStartBounded(
  config: unknown,
  timeoutMs = BEFORE_QUIT_TIMEOUT_MS,
): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timedOut = new Promise<"timeout">((resolve) => {
    timer = setTimeout(() => resolve("timeout"), timeoutMs);
  });
  try {
    const outcome = await Promise.race([
      fireBeforeStart(config).then(() => "done" as const),
      timedOut,
    ]);
    if (outcome === "timeout") {
      // biome-ignore lint/suspicious/noConsole: always-visible warning; console is redirected to stderr by startBun
      console.warn(
        `[tynd] app:beforeStart hooks did not finish within ${timeoutMs} ms — continuing startup (hooks cannot call host APIs yet)`,
      );
    }
  } finally {
    clearTimeout(timer);
  }
}

/**
 * @internal exported for tests. Hand the host the two values it trusts only from the backend:
 * the app version (`app.setInfo`, the updater's downgrade floor) and the resolved updater config
 * (`updater.configure`: public keys, endpoints, channel, optional proxy and headers). Both come from the CLI-generated
 * bootstrap (`__TYND_APP_INFO__`, `__TYND_UPDATER_CONFIG__`), which `tynd dev`, `start` and `build`
 * all emit. Fire-and-forget: the replies are only read once the stdin listener is up, and a
 * failure must not block startup.
 */
export function sendHostTrust(): void {
  const g = globalThis as {
    __TYND_APP_INFO__?: { version?: unknown };
    // Opaque to core: the CLI emits it and the host validates it. Backend-only: may also carry
    // `proxy` / `headers` for private update servers, which the host accepts from
    // `updater.configure` and nowhere else.
    __TYND_UPDATER_CONFIG__?: Record<string, unknown>;
  };
  const report = (what: string) => (e: unknown) =>
    tynd.error(`${what} failed: ${sanitizedMessage(e)}`);
  const version = g.__TYND_APP_INFO__?.version;
  if (typeof version === "string" && version) {
    hostCall("app", "setInfo", { version }).catch(report("app.setInfo"));
  }
  const updater = g.__TYND_UPDATER_CONFIG__;
  if (updater && typeof updater === "object") {
    hostCall("updater", "configure", updater).catch(report("updater.configure"));
  }
}

function bootBun(config: AppConfig, entry: string): void {
  setHostCallTransport(writeIpcFrame);

  // `tynd:config` MUST be the first frame: the host's `read_config` rejects
  // anything else. Host-trust calls go after it.
  writeIpcFrame({
    type: "tynd:config",
    window: config.window ?? {},
    devUrl: config.devUrl ?? process.env.TYND_DEV_URL,
    frontendDir: process.env.TYND_FRONTEND_DIR ?? config.frontendDir,
    menu: config.menu ?? [],
    tray: config.tray ?? null,
    security: config.security ?? null,
    // Send an explicit empty policy so deny-by-default holds without
    // relying only on host initialization.
    capabilities: config.capabilities ?? [],
    quitOnLastWindowClosed: config.quitOnLastWindowClosed ?? false,
    assets: config.assets ?? null,
    // `app.protocol(...)` schemes: the host registers them before it builds
    // the first webview.
    protocols: protocolNames(),
  });
  sendHostTrust();

  setEmitFn((name, payload, to) => {
    // Reserved framework event — `evalInFrontend` ships a raw JS script
    // string the Rust host needs to read natively. Bypasses devalue so
    // Rust doesn't have to ship a parser; the wire frame carries the
    // script in plain.
    if (name === "__tynd:eval__") {
      const script = (payload as { script?: unknown } | null)?.script;
      if (typeof script !== "string") {
        tynd.error("__tynd:eval__: payload.script must be a string — dropped");
        return;
      }
      writeIpcFrame({ type: "evalInFrontend", script, to });
      return;
    }
    let p: string;
    try {
      p = encodeWire(payload);
    } catch (e) {
      tynd.error(`emit: payload for '${name}' is not encodable (${sanitizedMessage(e)}) — dropped`);
      return;
    }
    // Frame envelope is a JSON line; the user payload rides as a devalue
    // string in `p` so the webview shim's parser keeps working unchanged.
    writeIpcFrame({ type: "event", name, p, to });
  });

  startListener(entry);
}

/** Newline-delimited JSON reader. */
function startListener(entryPath: string): void {
  let buffer = "";

  process.stdin.setEncoding("utf8");

  process.stdin.on("data", (chunk: string) => {
    buffer += chunk;
    const lines = buffer.split("\n");
    buffer = lines.pop() ?? "";
    // Guard against a host that sends bytes without newlines — we'd
    // otherwise grow the buffer until OOM. Only the unterminated tail counts:
    // a chunk holding several complete frames is not a framing problem.
    if (buffer.length > MAX_STDIN_BUFFER) {
      tynd.error(
        `IPC stdin buffer exceeded ${MAX_STDIN_BUFFER} bytes without a newline — aborting`,
      );
      process.exit(1);
    }

    for (const line of lines) {
      if (!line.trim()) continue;
      handleLine(line, entryPath).catch((e) => tynd.error(`IPC error: ${e}`));
    }
  });

  process.stdin.on("end", () => {
    // Same fan-out as the explicit `tynd:close` lifecycle message — the
    // stdin-end path (parent host went away) is just a less polite close.
    void shutdown();
  });
  process.stdin.on("error", (err) => {
    tynd.error(`ipc transport: ${err instanceof Error ? err.message : String(err)}`);
    process.exit(2);
  });
  process.stdin.resume();
}

let quitting: Promise<never> | undefined;

/** @internal exported for tests. Forget the memoized shutdown. */
export function resetShutdownForTests(): void {
  quitting = undefined;
}

/**
 * Orderly exit: plugin `app:beforeQuit` hooks, user `onClose` handlers, then `process.exit(0)`.
 * Memoized: `tynd:close` and the stdin `end` that follows it must not run the hooks twice.
 */
function shutdown(): Promise<never> {
  quitting ??= doShutdown();
  return quitting;
}

async function doShutdown(): Promise<never> {
  try {
    await fireBeforeQuit();
  } catch {
    /* best-effort: never block exit on a hook */
  }
  fireClose();
  return process.exit(0);
}

/** Keep log lines terminal-friendly; a multi-MiB malformed payload would
 * otherwise blow up the log itself. */
const LOG_PREVIEW = 200;
function preview(s: string): string {
  return s.length > LOG_PREVIEW ? `${s.slice(0, LOG_PREVIEW)}… (${s.length} chars)` : s;
}

async function handleLine(line: string, entryPath: string): Promise<void> {
  let raw: unknown;
  try {
    raw = JSON.parse(line);
  } catch {
    tynd.error(`Invalid JSON from Rust: ${preview(line)}`);
    return;
  }
  return handleParsedMsg(raw, entryPath);
}

/** Credit a stream with `n` consumed chunks and wake its producer. */
function ackStream(id: string, n: number): void {
  const stream = _activeStreams.get(id);
  if (!stream) return;
  // Cap credit at STREAM_CREDIT — a buggy / hostile host sending a huge
  // ack value could otherwise grow credit without bound and defeat the
  // back-pressure cap. Floor `n` at 0 so a negative ack can't drive
  // credit below zero and deadlock the next step.
  stream.credit = Math.min(stream.credit + Math.max(0, n), STREAM_CREDIT);
  if (stream.waiter) {
    const wake = stream.waiter;
    stream.waiter = null;
    wake();
  }
}

/** @internal exported for tests. Dispatch a parsed IPC message: `raw` comes
 *  from `JSON.parse` and is validated against the valibot schema before the
 *  switch dispatch. */
export async function handleParsedMsg(raw: unknown, entryPath: string): Promise<void> {
  const parsed = v.safeParse(IpcMsgSchema, raw);
  if (!parsed.success) {
    tynd.error(`Invalid IPC message shape: ${preview(JSON.stringify(raw) ?? String(raw))}`);
    return;
  }
  // Exhaustive dispatch over the IPC message union. The `never` assertion in
  // the default arm makes the TS compiler refuse the build if a new variant
  // is added to `IpcMsgSchema` without a matching case here.
  const m = parsed.output;
  switch (m.type) {
    case "call":
      await handleCall(m, entryPath);
      return;
    case "cancel":
      cancelCall(m.id);
      return;
    case "ack":
      ackStream(m.id, m.n);
      return;
    case "tynd:ready":
      await fireReady();
      return;
    case "tynd:close":
      setHostCallTransport(null);
      await shutdown();
      return;
    case "tynd:window":
      await fireHook(m.event === "created" ? "window:created" : "window:closed", {
        label: m.label,
      });
      return;
    case "host_result":
      resolveHostCall(m.id, m.ok, m.value);
      return;
    case "tynd:os_event":
      dispatchHostEvent(m.name, m.data);
      return;
    case "protocol":
      await handleProtocolMsg(m, writeIpcFrame);
      return;
    default: {
      const _exhaustive: never = m;
      void _exhaustive;
    }
  }
}

/** Stop a streaming call, or remember the cancel when it raced ahead of the stream registration. */
function cancelCall(id: string): void {
  const controller = _inflightCalls.get(id);
  if (controller && !controller.signal.aborted) {
    controller.abort(new DOMException("call cancelled", "AbortError"));
  }
  const stream = _activeStreams.get(id);
  if (!stream) {
    // Not streaming (yet): the handler / middleware may still be running.
    // Remember the cancel so `streamIterable` can bail out on registration.
    if (_inflightCalls.has(id)) _cancelledEarly.add(id);
    return;
  }
  _activeStreams.delete(id);
  stream.waiter?.();
  if (typeof stream.iter.return === "function") {
    stream.iter.return(undefined).catch(() => {
      /* swallow: cancel is best-effort */
    });
  }
}

function decodeCallArgs(msg: { id: string; fn: string; a: string }): unknown[] | null {
  try {
    return decodeArgs(msg.a);
  } catch (error) {
    writeIpcError(
      msg.id,
      "INVALID_ARGS",
      `invalid args for '${msg.fn}': ${sanitizedMessage(error)}`,
      msg.fn,
    );
    return null;
  }
}

function handleFrontendEmit(id: string, fn: string, args: unknown[]): boolean {
  if (fn !== FRONTEND_EMIT_FN) return false;

  const eventName = args[0];
  if (typeof eventName !== "string") {
    throw new IpcDispatchError(
      "INVALID_ARGS",
      `${FRONTEND_EMIT_FN}: expected args[0] to be an event name string, got ${typeof eventName}`,
    );
  }
  dispatchFrontendEmit(eventName, args[1]);
  writeIpcFrame({ type: "return", id, ok: true, v: encodeWire(null) });
  return true;
}

/** Framework-reserved RPC names: `__tynd_<anything>__`. */
function isReservedFnName(fn: string): boolean {
  return fn.length > "__tynd___".length && fn.startsWith("__tynd_") && fn.endsWith("__");
}

async function invokeHandler(
  fn: string,
  args: unknown[],
  entryPath: string,
  pluginCall: { plugin: string; method: string } | null,
): Promise<unknown> {
  if (pluginCall) {
    const handler = getPluginMethod(pluginCall.plugin, pluginCall.method);
    if (!handler) {
      throw new IpcDispatchError(
        "UNKNOWN_FN",
        `plugin "${pluginCall.plugin}" has no method "${pluginCall.method}" (missing from tynd.config.ts::plugins?)`,
      );
    }
    return handler(...args);
  }

  // `__tynd_*__` names belong to the framework (`__tynd_fe_emit__` is handled
  // before this point): the host skips its `backend:` ACL for them, so they must
  // never resolve to a user-exported function.
  if (isReservedFnName(fn)) {
    throw new IpcDispatchError("UNKNOWN_FN", `"${fn}" is a reserved framework name`);
  }
  const mod = await getModule(entryPath);
  const handler = mod[fn];
  if (typeof handler !== "function") {
    throw new IpcDispatchError("UNKNOWN_FN", `"${fn}" is not an exported function`);
  }
  return (handler as (...callArgs: unknown[]) => unknown)(...args);
}

/** Resolve a call through the middleware chain; unknown targets still fail inside it. */
function resolveCall(
  fn: string,
  args: unknown[],
  entryPath: string,
  caller?: CallContext,
): Promise<unknown> {
  const pluginCall = parsePluginCall(fn);
  const ctx: MiddlewareContext = pluginCall
    ? { fn: pluginCall.method, args, caller, plugin: pluginCall.plugin }
    : { fn, args, caller };
  // `ctx.args` (not the local `args`) so middleware can replace arguments.
  return runMiddleware(ctx, () => invokeHandler(fn, ctx.args, entryPath, pluginCall));
}

function writeCallReturn(id: string, fn: string, resolved: unknown): void {
  try {
    writeIpcFrame({ type: "return", id, ok: true, v: encodeWire(resolved) });
  } catch (error) {
    throw new IpcDispatchError(
      "PROTOCOL_ERROR",
      `return value for '${fn}' is not encodable: ${sanitizedMessage(error)}`,
    );
  }
}

async function writeResolvedCall(id: string, fn: string, resolved: unknown): Promise<void> {
  if (_isAsyncIterable(resolved)) {
    await streamIterable(id, resolved, fn);
    return;
  }
  writeCallReturn(id, fn, resolved);
}

function callErrorCode(error: unknown, message: string): TyndIpcErrorCode {
  if (error instanceof IpcDispatchError) return error.ipcCode;
  return looksLikePermissionDenial(message) ? "PERMISSION_DENIED" : "HANDLER_THREW";
}

async function handleCall(
  msg: { id: string; fn: string; a: string; ctx?: unknown },
  entryPath: string,
): Promise<void> {
  const { id, fn } = msg;
  const decoded = decodeCallArgs(msg);
  if (!decoded) return;
  const controller = new AbortController();
  const caller = parseCallContext(msg.ctx, controller.signal);
  // `Channel` arguments arrive as placeholders bound to the calling webview.
  const args = reviveChannels(decoded, caller);
  _inflightCalls.set(id, controller);

  // The whole call (handler, middleware, plugin hooks and stream iteration)
  // runs inside the caller's AsyncLocalStorage scope for `getCallContext()`.
  await runWithCallContext(caller, async () => {
    try {
      if (handleFrontendEmit(id, fn, args)) return;
      const resolved = await resolveCall(fn, args, entryPath, caller);
      // A cancelled call was already settled by the client and forgotten by
      // the host: its result has nowhere to go.
      if (controller.signal.aborted) {
        await discardResult(resolved);
        return;
      }
      await writeResolvedCall(id, fn, resolved);
    } catch (error) {
      _activeStreams.delete(id);
      if (controller.signal.aborted) return;
      const message = sanitizedMessage(error);
      writeIpcError(id, callErrorCode(error, message), message, fn);
    } finally {
      _inflightCalls.delete(id);
      _cancelledEarly.delete(id);
    }
  });
}

/** Close a stream the client no longer wants (it was cancelled before its first chunk). */
async function discardResult(resolved: unknown): Promise<void> {
  if (_isAsyncIterable(resolved)) {
    await closeStreamIterator(resolved[Symbol.asyncIterator](), false);
  }
}

/**
 * Heuristic for distinguishing security-policy denials from regular
 * handler throws. Host APIs that reject through `security::check_*`
 * surface "denied"-shaped strings; ACL gates use "permission denied".
 * Mapping to `PERMISSION_DENIED` lets frontends `switch` on the code
 * without substring-matching `error.message`.
 */
function looksLikePermissionDenial(message: string): boolean {
  const m = message.toLowerCase();
  return (
    m.includes("denied") ||
    m.includes("not permitted") ||
    m.includes("permission denied") ||
    m.includes("not within allowed scopes") ||
    m.includes("no allow rule")
  );
}

/** Single emitter for IPC error-return frames — keeps the wire envelope (sentinel + JSON payload) DRY. */
function writeIpcError(id: string, code: TyndIpcErrorCode, message: string, fn?: string): void {
  writeIpcFrame({
    type: "return",
    id,
    ok: false,
    error: encodeIpcError({ code, message, fn }),
  });
}

async function consumeStreamCredit(id: string, state: StreamState): Promise<boolean> {
  if (state.credit <= 0) {
    await new Promise<void>((resolve) => {
      state.waiter = resolve;
    });
    if (!_activeStreams.has(id)) return false;
  }
  state.credit -= 1;
  return true;
}

function encodeStreamYield(id: string, value: unknown): string {
  try {
    return encodeWire(value);
  } catch (error) {
    throw new IpcDispatchError(
      "PROTOCOL_ERROR",
      `yielded value is not encodable on stream ${id}: ${sanitizedMessage(error)}`,
    );
  }
}

async function closeStreamIterator(
  iter: AsyncIterator<unknown>,
  completed: boolean,
): Promise<void> {
  if (completed || typeof iter.return !== "function") return;
  try {
    await iter.return(undefined);
  } catch {
    /* intentional: generator cleanup is best-effort */
  }
}

function writeStreamReturn(id: string, fn: string, value: unknown): void {
  try {
    writeIpcFrame({ type: "return", id, ok: true, v: encodeWire(value) });
  } catch (error) {
    writeIpcError(
      id,
      "PROTOCOL_ERROR",
      `final value is not encodable: ${sanitizedMessage(error)}`,
      fn,
    );
  }
}

async function streamIterable(
  id: string,
  iterable: AsyncIterable<unknown>,
  fn: string,
): Promise<void> {
  const iter = iterable[Symbol.asyncIterator]();
  const state: StreamState = { iter, credit: STREAM_CREDIT, waiter: null };
  if (_cancelledEarly.delete(id)) {
    // The client cancelled before the stream existed: never start it.
    await closeStreamIterator(iter, false);
    return;
  }
  _activeStreams.set(id, state);
  let final: unknown;
  let completed = false;

  try {
    while (true) {
      const step = await iter.next();
      if (step.done) {
        final = step.value;
        completed = true;
        break;
      }
      if (!(await consumeStreamCredit(id, state))) break;
      writeIpcFrame({ type: "yield", id, v: encodeStreamYield(id, step.value) });
      if (!_activeStreams.has(id)) break;
    }
  } finally {
    _activeStreams.delete(id);
    await closeStreamIterator(iter, completed);
  }

  if (completed) writeStreamReturn(id, fn, final);
}
