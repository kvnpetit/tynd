import { isIP } from "node:net";
import { createEmitter, hostCall } from "@tynd/core";
import type { HttpRequestPayload, HttpServerEventMap, RespondArgs, ServeArgs } from "./types";

const DEFAULT_MAX_PENDING_REQUESTS = 4096;
const DEFAULT_MAX_WS_CONNECTIONS = 4096;
const DEFAULT_RESPONSE_TIMEOUT_SECS = 60;
const DEFAULT_BODY_CAP_BYTES = 16 * 1024 * 1024;
const SANITY_MAX_PENDING_REQUESTS = 1_000_000;
const SANITY_MAX_WS_CONNECTIONS = 1_000_000;
const SANITY_MAX_RESPONSE_TIMEOUT_SECS = 3600;
const SANITY_MAX_BODY_CAP_BYTES = 1024 * 1024 * 1024;

export type Emit = <K extends keyof HttpServerEventMap>(
  event: K,
  payload: HttpServerEventMap[K],
) => void;
export type CheckBind = (host: string, port: number) => Promise<void>;

export interface BackendDependencies {
  emit?: Emit;
  checkBind?: CheckBind;
}

export interface ResolvedOptions {
  maxPendingRequests: number;
  maxWsConnections: number;
  responseTimeoutSecs: number;
  bodyCapBytes: number;
  streaming: boolean;
  websocket: boolean;
}

interface PendingResponse {
  resolve(response: Response): void;
  timer: ReturnType<typeof setTimeout>;
}

export interface WebSocketData {
  serverId: number;
  connId: number;
  remote: string;
}

export interface ServerState {
  id: number;
  host: string;
  port: number;
  server: Bun.Server<WebSocketData>;
  options: ResolvedOptions;
  pending: Map<number, PendingResponse>;
  sockets: Map<number, Bun.ServerWebSocket<WebSocketData>>;
  pendingWsConnections: number;
  nextRequestId: number;
  nextConnectionId: number;
}

const events = createEmitter<HttpServerEventMap>();

export function defaultEmit<K extends keyof HttpServerEventMap>(
  event: K,
  payload: HttpServerEventMap[K],
): void {
  events.emit(event, payload);
}

/**
 * Loopback means `localhost` or a literal loopback IP. DNS names are never
 * loopback, even when they look like one ("127.0.0.1.nip.io"): Bun.serve would
 * resolve them to whatever address the resolver returns.
 */
export function isLoopbackHost(host: string): boolean {
  const normalized = host
    .trim()
    .replace(/^\[|\]$/g, "")
    .toLowerCase();
  if (normalized === "localhost") return true;
  const family = isIP(normalized);
  if (family === 4) return normalized.startsWith("127.");
  if (family === 6) return isLoopbackIPv6(normalized);
  return false;
}

function isLoopbackIPv6(address: string): boolean {
  // The URL parser canonicalises every spelling: "::1", "0:0:0:0:0:0:0:1", "::ffff:127.0.0.1".
  let canonical: string;
  try {
    canonical = new URL(`http://[${address}]`).hostname;
  } catch {
    return false;
  }
  return canonical === "[::1]" || /^\[::ffff:7f[0-9a-f]{2}:[0-9a-f]{1,4}\]$/.test(canonical);
}

export async function defaultCheckBind(host: string, port: number): Promise<void> {
  if (isLoopbackHost(host)) {
    await hostCall("network-loopback", "bind", { port });
    return;
  }
  await hostCall("network-public", "bind", { host, port });
}

function integer(
  value: number | null | undefined,
  { fallback, min, max, field }: { fallback: number; min: number; max: number; field: string },
): number {
  if (value == null) return fallback;
  if (!Number.isSafeInteger(value) || value < min) {
    throw new TypeError(`http-server.serve: ${field} must be an integer >= ${min}`);
  }
  return Math.min(value, max);
}

export function resolveOptions(args: ServeArgs): ResolvedOptions {
  return {
    maxPendingRequests: integer(args.maxPendingRequests, {
      fallback: DEFAULT_MAX_PENDING_REQUESTS,
      min: 1,
      max: SANITY_MAX_PENDING_REQUESTS,
      field: "maxPendingRequests",
    }),
    maxWsConnections: integer(args.maxWsConnections, {
      fallback: DEFAULT_MAX_WS_CONNECTIONS,
      min: 1,
      max: SANITY_MAX_WS_CONNECTIONS,
      field: "maxWsConnections",
    }),
    responseTimeoutSecs: integer(args.responseTimeoutSecs, {
      fallback: DEFAULT_RESPONSE_TIMEOUT_SECS,
      min: 1,
      max: SANITY_MAX_RESPONSE_TIMEOUT_SECS,
      field: "responseTimeoutSecs",
    }),
    bodyCapBytes: integer(args.bodyCapBytes, {
      fallback: DEFAULT_BODY_CAP_BYTES,
      min: 1,
      max: SANITY_MAX_BODY_CAP_BYTES,
      field: "bodyCapBytes",
    }),
    streaming: args.streaming === true,
    websocket: args.websocket === true,
  };
}

export function validatePort(port: number): void {
  if (!Number.isInteger(port) || port < 0 || port > 65_535) {
    throw new TypeError(`http-server.serve: port must be an integer between 0 and 65535`);
  }
}

export function validateHost(host: string): void {
  if (!host || host.length > 253 || host.includes("\0")) {
    throw new TypeError("http-server.serve: host must be 1-253 characters without NUL");
  }
}

function headersRecord(headers: Headers): Record<string, string> {
  const out: Record<string, string> = Object.create(null) as Record<string, string>;
  headers.forEach((value, key) => {
    out[key] = value;
  });
  return out;
}

function remoteAddress(server: Bun.Server<WebSocketData>, request: Request): string {
  return server.requestIP(request)?.address ?? "";
}

export function nextId(current: number, used: (id: number) => boolean): number {
  let candidate = current;
  for (let attempts = 0; attempts < Number.MAX_SAFE_INTEGER; attempts++) {
    candidate = candidate >= Number.MAX_SAFE_INTEGER ? 1 : candidate + 1;
    if (!used(candidate)) return candidate;
  }
  throw new Error("http-server: no handle available");
}

export function base64(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString("base64");
}

export function decodeBase64(value: string): Uint8Array {
  if (value.length === 0) return new Uint8Array();
  if (value.length > Math.ceil((SANITY_MAX_BODY_CAP_BYTES * 4) / 3) + 4) {
    throw new RangeError("http-server.respond: response body exceeds 1 GiB cap");
  }
  if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) {
    throw new TypeError("http-server: binary payload must be canonical base64");
  }
  const decoded = Buffer.from(value, "base64");
  if (decoded.byteLength > SANITY_MAX_BODY_CAP_BYTES) {
    throw new RangeError("http-server.respond: response body exceeds 1 GiB cap");
  }
  return decoded;
}

async function readBufferedBody(request: Request, cap: number): Promise<Uint8Array> {
  const declared = Number(request.headers.get("content-length") ?? 0);
  if (Number.isFinite(declared) && declared > cap) {
    throw new RangeError(`request body exceeds ${cap}-byte cap`);
  }
  const bytes = new Uint8Array(await request.arrayBuffer());
  if (bytes.byteLength > cap) {
    throw new RangeError(`request body exceeds ${cap}-byte cap`);
  }
  return bytes;
}

async function pumpBody(
  state: ServerState,
  requestId: number,
  request: Request,
  emit: Emit,
): Promise<void> {
  const reader = request.body?.getReader();
  if (!reader) {
    emit("http:request-chunk", { serverId: state.id, reqId: requestId, done: true });
    return;
  }
  let received = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      received += value.byteLength;
      if (received > state.options.bodyCapBytes) {
        throw new RangeError(`request body exceeds ${state.options.bodyCapBytes}-byte cap`);
      }
      emit("http:request-chunk", {
        serverId: state.id,
        reqId: requestId,
        chunk: base64(value),
      });
    }
    emit("http:request-chunk", { serverId: state.id, reqId: requestId, done: true });
  } catch (error) {
    emit("http:request-chunk", {
      serverId: state.id,
      reqId: requestId,
      done: true,
      error: error instanceof Error ? error.message : String(error),
    });
  } finally {
    reader.releaseLock();
  }
}

function pendingResponse(
  state: ServerState,
  requestId: number,
  emitRequest: () => void,
): Promise<Response> {
  return new Promise<Response>((resolve) => {
    const timer = setTimeout(() => {
      state.pending.delete(requestId);
      resolve(new Response("gateway timeout", { status: 504 }));
    }, state.options.responseTimeoutSecs * 1000);
    state.pending.set(requestId, { resolve, timer });
    emitRequest();
  });
}

export function validateRespondArgs(args: RespondArgs): { status: number; body: Uint8Array } {
  const status = args.status ?? 200;
  if (!Number.isInteger(status) || status < 200 || status > 599) {
    throw new TypeError("http-server.respond: status must be an integer between 200 and 599");
  }
  return { status, body: decodeBase64(args.body ?? "") };
}

export function isNullBodyStatus(status: number): boolean {
  return status === 204 || status === 205 || status === 304;
}

function isWebSocketUpgrade(state: ServerState, request: Request): boolean {
  return state.options.websocket && request.headers.get("upgrade")?.toLowerCase() === "websocket";
}

function upgradeWebSocket(
  state: ServerState,
  request: Request,
  server: Bun.Server<WebSocketData>,
  remote: string,
): Response | undefined {
  if (state.sockets.size + state.pendingWsConnections >= state.options.maxWsConnections) {
    return new Response("websocket connection limit reached", { status: 503 });
  }
  state.nextConnectionId = nextId(state.nextConnectionId, (id) => state.sockets.has(id));
  state.pendingWsConnections += 1;
  const upgraded = server.upgrade(request, {
    data: { serverId: state.id, connId: state.nextConnectionId, remote },
  });
  if (!upgraded) state.pendingWsConnections -= 1;
  return upgraded ? undefined : new Response("websocket upgrade failed", { status: 400 });
}

function requestPayload(
  state: ServerState,
  request: Request,
  requestId: number,
  remote: string,
): HttpRequestPayload {
  const url = new URL(request.url);
  return {
    serverId: state.id,
    reqId: requestId,
    method: request.method,
    path: `${url.pathname}${url.search}`,
    headers: headersRecord(request.headers),
    body: "",
    remote,
    streaming: state.options.streaming,
  };
}

async function bufferRequestBody(
  state: ServerState,
  request: Request,
  payload: HttpRequestPayload,
): Promise<Response | null> {
  if (state.options.streaming) return null;
  try {
    payload.body = base64(await readBufferedBody(request, state.options.bodyCapBytes));
    return null;
  } catch (error) {
    return new Response(error instanceof Error ? error.message : String(error), { status: 413 });
  }
}

export async function handleRequest(
  state: ServerState,
  request: Request,
  server: Bun.Server<WebSocketData>,
  emit: Emit,
): Promise<Response | undefined> {
  const remote = remoteAddress(server, request);
  if (isWebSocketUpgrade(state, request)) {
    return upgradeWebSocket(state, request, server, remote);
  }
  if (state.pending.size >= state.options.maxPendingRequests) {
    return new Response("pending request limit reached", { status: 503 });
  }

  state.nextRequestId = nextId(state.nextRequestId, (id) => state.pending.has(id));
  const requestId = state.nextRequestId;
  const payload = requestPayload(state, request, requestId, remote);
  const bodyError = await bufferRequestBody(state, request, payload);
  if (bodyError) return bodyError;

  return pendingResponse(state, requestId, () => {
    emit("http:request", payload);
    if (state.options.streaming) void pumpBody(state, requestId, request, emit);
  });
}
