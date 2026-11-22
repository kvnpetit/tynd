/**
 * `@tynd/plugin-http-server` — single-import entry. Wraps request /
 * response and WebSocket events into a `Bun.serve`-shaped API.
 */

import { createPluginClient } from "@tynd/core/client";
import { base64ToBytes, bytesToBase64 } from "@tynd/core/client/bridge";
import { composePluginClient, createResourceCloser, onHostEvent } from "@tynd/plugin-sdk";
import type httpServerBackend from "./backend";
import { headerPairs } from "./headers";
import type {
  HttpRequestChunkPayload,
  HttpRequestPayload,
  WsClosePayload,
  WsMessagePayload,
  WsOpenPayload,
} from "./types";

const _api = createPluginClient<typeof httpServerBackend>("http-server");

export { SET_COOKIE_CARRIER, setCookieHeader } from "./headers";

/** Wire payloads of the host-emitted HTTP and WebSocket events. */
export type {
  HttpRequestChunkPayload,
  HttpRequestPayload,
  WsClosePayload,
  WsMessagePayload,
  WsOpenPayload,
} from "./types";

/** Per-connection WebSocket handle exposed to user `websocket` handlers. */
export interface ServerWebSocket {
  readonly id: number;
  readonly remote: string;
  send(data: string | Uint8Array): Promise<void>;
  close(): Promise<void>;
}

/** User-supplied request and websocket handlers, modelled on `Bun.serve`. */
export interface ServeHandlers {
  fetch?(req: Request, info: { remote: string }): Response | Promise<Response>;
  websocket?: {
    open?(ws: ServerWebSocket): void | Promise<void>;
    message?(ws: ServerWebSocket, data: string | Uint8Array): void | Promise<void>;
    close?(ws: ServerWebSocket): void | Promise<void>;
  };
}

/** Bind options for `httpServer.serve`. */
export interface ServeOptions {
  port?: number;
  host?: string;
  /** Resource overrides forwarded to the host-side `ServeOptions`. */
  maxPendingRequests?: number;
  maxWsConnections?: number;
  responseTimeoutSecs?: number;
  bodyCapBytes?: number;
  /** When `true`, request bodies are not buffered host-side. The user
   *  `fetch` handler receives a streaming `Request` whose `body` is a
   *  `ReadableStream<Uint8Array>` reassembled from `http:request-chunk`
   *  events. Default `false`. */
  streaming?: boolean;
}

/** Handle to a server bound by `httpServer.serve`; call `stop()` to drain. */
export interface RunningServer {
  readonly id: number;
  readonly host: string;
  readonly port: number;
  /** Stop accepting new requests; in-flight handlers run to completion. */
  stop(): Promise<void>;
}

type ChunkStreams = Map<number, ReadableStreamDefaultController<Uint8Array>>;

function finishChunkStream(
  controller: ReadableStreamDefaultController<Uint8Array>,
  error: string | undefined,
): void {
  try {
    if (error) controller.error(new Error(`http-server: ${error}`));
    else controller.close();
  } catch {
    // Already errored / closed.
  }
}

function handleRequestChunk(
  serverId: number,
  streams: ChunkStreams,
  raw: HttpRequestChunkPayload | null | undefined,
): void {
  if (raw?.serverId !== serverId) return;
  const controller = streams.get(raw.reqId);
  if (!controller) return;
  if (raw.chunk) {
    try {
      controller.enqueue(base64ToBytes(raw.chunk));
    } catch {
      streams.delete(raw.reqId);
      return;
    }
  }
  if (!raw.done) return;
  streams.delete(raw.reqId);
  finishChunkStream(controller, raw.error);
}

interface ServerBinding {
  id: number;
  host: string;
  port: number;
}

class ClientServerSession {
  private readonly chunkStreams: ChunkStreams = new Map();
  private readonly connections = new Map<number, ServerWebSocket>();
  private offChunk = (): void => undefined;
  private offRequest = (): void => undefined;
  private offWsOpen = (): void => undefined;
  private offWsMessage = (): void => undefined;
  private offWsClose = (): void => undefined;
  private disposed = false;

  private readonly serverId: number;
  private readonly handlers: ServeHandlers;

  constructor(serverId: number, handlers: ServeHandlers) {
    this.serverId = serverId;
    this.handlers = handlers;
  }

  subscribe(): void {
    try {
      this.offChunk = onHostEvent<HttpRequestChunkPayload>("http:request-chunk", (raw) => {
        if (this.disposed) return;
        handleRequestChunk(this.serverId, this.chunkStreams, raw);
      });
      this.offRequest = onHostEvent<HttpRequestPayload>("http:request", (raw) => {
        this.handleRequest(raw);
      });
      this.offWsOpen = onHostEvent<WsOpenPayload>("ws:open", (raw) => this.handleWsOpen(raw));
      this.offWsMessage = onHostEvent<WsMessagePayload>("ws:message", (raw) =>
        this.handleWsMessage(raw),
      );
      this.offWsClose = onHostEvent<WsClosePayload>("ws:close", (raw) => this.handleWsClose(raw));
    } catch (error) {
      this.dispose();
      throw error;
    }
  }

  running(binding: ServerBinding): RunningServer {
    if (binding.id !== this.serverId) {
      throw new Error(
        `http-server.activate: expected reservation ${this.serverId}, received ${binding.id}`,
      );
    }
    return {
      ...binding,
      stop: this.createStop(),
    };
  }

  private handleRequest(raw: HttpRequestPayload | null | undefined): void {
    if (this.disposed) return;
    if (raw?.serverId !== this.serverId) return;
    const stream = raw.streaming ? this.createRequestStream(raw.reqId) : undefined;
    void dispatchRequest(this.serverId, raw, this.handlers, stream);
  }

  private createRequestStream(reqId: number): ReadableStream<Uint8Array> {
    let controller: ReadableStreamDefaultController<Uint8Array> | undefined;
    const stream = new ReadableStream<Uint8Array>({
      start: (current) => {
        controller = current;
        this.chunkStreams.set(reqId, current);
      },
      cancel: () => {
        this.chunkStreams.delete(reqId);
      },
    });
    // ReadableStream.start is synchronous today; retain guard for future runtimes.
    if (!this.chunkStreams.has(reqId)) {
      if (!controller) {
        throw new Error("http-server: stream controller was not initialized synchronously");
      }
      this.chunkStreams.set(reqId, controller);
    }
    return stream;
  }

  private handleWsOpen(raw: WsOpenPayload | null | undefined): void {
    if (this.disposed) return;
    if (raw?.serverId !== this.serverId) return;
    const socket = makeServerSocket(this.serverId, raw.connId, raw.remote);
    this.connections.set(raw.connId, socket);
    runWebSocketHandler(() => this.handlers.websocket?.open?.(socket));
  }

  private handleWsMessage(raw: WsMessagePayload | null | undefined): void {
    if (this.disposed) return;
    if (raw?.serverId !== this.serverId) return;
    const socket = this.connections.get(raw.connId);
    if (!socket) return;
    const data = raw.text != null ? raw.text : base64ToBytes(raw.binary ?? "");
    runWebSocketHandler(() => this.handlers.websocket?.message?.(socket, data));
  }

  private handleWsClose(raw: WsClosePayload | null | undefined): void {
    if (this.disposed) return;
    if (raw?.serverId !== this.serverId) return;
    const socket = this.connections.get(raw.connId);
    this.connections.delete(raw.connId);
    if (socket) runWebSocketHandler(() => this.handlers.websocket?.close?.(socket));
  }

  private createStop(): RunningServer["stop"] {
    return createResourceCloser(
      async () => {
        await _api.stop({ serverId: this.serverId });
      },
      () => this.dispose(),
    );
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    for (const controller of this.chunkStreams.values()) {
      try {
        controller.error(new Error("http-server: server stopped"));
      } catch {
        // Consumer already closed or errored stream.
      }
    }
    this.chunkStreams.clear();
    this.connections.clear();
    for (const unsubscribe of [
      this.offRequest,
      this.offChunk,
      this.offWsOpen,
      this.offWsMessage,
      this.offWsClose,
    ]) {
      try {
        unsubscribe();
      } catch {
        // Every remaining listener must still be detached after one failure.
      }
    }
  }
}

async function startClientServer(
  opts: ServeOptions,
  handlers: ServeHandlers,
): Promise<RunningServer> {
  const { id } = await _api.reserve({
    ...opts,
    websocket: handlers.websocket != null,
  });
  const session = new ClientServerSession(id, handlers);
  try {
    // No socket is bound until every request/WebSocket listener exists.
    session.subscribe();
    const binding = await _api.activate({ serverId: id });
    return session.running(binding);
  } catch (error) {
    session.dispose();
    await _api.stop({ serverId: id }).catch(() => undefined);
    throw error;
  }
}

/** Frontend bindings for the `@tynd/plugin-http-server` plugin. */
export const httpServer = composePluginClient(_api, {
  /** Bind a host HTTP server and dispatch `fetch`/`websocket` events through user handlers. */
  async serve(opts: ServeOptions, handlers: ServeHandlers): Promise<RunningServer> {
    return startClientServer(opts, handlers);
  },
  /** List ids of every running server bound by this app. */
  list(): Promise<number[]> {
    return _api.list();
  },
});

export default httpServer;

async function dispatchRequest(
  serverId: number,
  raw: HttpRequestPayload,
  handlers: ServeHandlers,
  streamingBody?: ReadableStream<Uint8Array>,
): Promise<void> {
  if (!handlers.fetch) {
    await _api.respond({ serverId, reqId: raw.reqId, status: 404, body: "" });
    return;
  }
  const url = `http://${raw.headers.host ?? "localhost"}${raw.path}`;
  const acceptsBody = raw.method !== "GET" && raw.method !== "HEAD";
  const init: RequestInit = {
    method: raw.method,
    headers: stripUpgradeHeaders(raw.headers),
    body: acceptsBody ? requestBody(raw, streamingBody) : undefined,
    // `Request` rejects a `ReadableStream` body without `duplex: "half"`
    // on every spec-compliant runtime; ignore on platforms that don't
    // know the field.
    ...(streamingBody && acceptsBody ? ({ duplex: "half" } as unknown as RequestInit) : {}),
  };
  let responseArgs: Parameters<typeof _api.respond>[0];
  try {
    const res = await handlers.fetch(new Request(url, init), { remote: raw.remote });
    const headers = headerPairs(res.headers);
    const buf = new Uint8Array(await res.arrayBuffer());
    responseArgs = {
      serverId,
      reqId: raw.reqId,
      status: res.status,
      headers,
      body: buf.length ? bytesToBase64(buf) : "",
    };
  } catch (err) {
    await _api.respond({
      serverId,
      reqId: raw.reqId,
      status: 500,
      headers: { "content-type": "text/plain" },
      body: bytesToBase64(new TextEncoder().encode(String(err))),
    });
    return;
  }
  await _api.respond(responseArgs);
}

function requestBody(
  raw: HttpRequestPayload,
  streamingBody: ReadableStream<Uint8Array> | undefined,
): BodyInit | undefined {
  if (streamingBody) return streamingBody as unknown as BodyInit;
  return raw.body ? (base64ToBytes(raw.body) as unknown as BodyInit) : undefined;
}

function makeServerSocket(serverId: number, connId: number, remote: string): ServerWebSocket {
  return {
    id: connId,
    remote,
    async send(data) {
      if (typeof data === "string") {
        await _api.wsSend({ serverId, connId, text: data });
      } else {
        await _api.wsSend({ serverId, connId, binary: bytesToBase64(data) });
      }
    },
    async close() {
      await _api.wsClose({ serverId, connId });
    },
  };
}

function runWebSocketHandler(handler: () => void | Promise<void> | undefined): void {
  try {
    void Promise.resolve(handler()).catch(() => undefined);
  } catch {
    // A user callback must not surface as an unhandled host-event error.
  }
}

/** `Request` rejects connection-control headers; strip them. */
function stripUpgradeHeaders(headers: Record<string, string>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [k, v] of Object.entries(headers)) {
    const lc = k.toLowerCase();
    if (
      lc === "connection" ||
      lc === "upgrade" ||
      lc === "host" ||
      lc.startsWith("sec-websocket-")
    ) {
      continue;
    }
    out[k] = v;
  }
  return out;
}
