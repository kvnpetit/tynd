import { definePlugin } from "@tynd/core";
import {
  type BackendDependencies,
  base64,
  type CheckBind,
  decodeBase64,
  defaultCheckBind,
  defaultEmit,
  type Emit,
  handleRequest,
  isNullBodyStatus,
  nextId,
  type ResolvedOptions,
  resolveOptions,
  type ServerState,
  validateHost,
  validatePort,
  validateRespondArgs,
  type WebSocketData,
} from "./backend-transport";
import { headersFromWire } from "./headers";
import type {
  ReserveResult,
  RespondArgs,
  ServeArgs,
  ServeResult,
  ServerIdArgs,
  WsConnectionArgs,
  WsSendArgs,
} from "./types";

const MAX_SERVERS = 32;
const STOP_DRAIN_TIMEOUT_MS = 5_000;

interface ServerReservation {
  id: number;
  host: string;
  requestedPort: number;
  options: ResolvedOptions;
  activation: Promise<ServeResult> | null;
  stopped: boolean;
}

interface HttpServerBackendState {
  emit: Emit;
  checkBind: CheckBind;
  reservations: Map<number, ServerReservation>;
  servers: Map<number, ServerState>;
  nextServerId: number;
}

interface ServerStateRef {
  current?: ServerState;
}

export function createHttpServerMethods(dependencies: BackendDependencies = {}) {
  const state: HttpServerBackendState = {
    emit: dependencies.emit ?? defaultEmit,
    checkBind: dependencies.checkBind ?? defaultCheckBind,
    reservations: new Map(),
    servers: new Map(),
    nextServerId: 0,
  };

  return {
    reserve: (args: ServeArgs = {}): Promise<ReserveResult> => reserveHttpServer(state, args),
    activate: ({ serverId }: ServerIdArgs): Promise<ServeResult> =>
      activateHttpServer(state, serverId),
    respond: (args: RespondArgs): void => respondToRequest(state.servers, args),
    wsSend: (args: WsSendArgs): void => sendWebSocketMessage(state.servers, args),
    wsClose: (args: WsConnectionArgs): void => closeWebSocket(state.servers, args),
    stop: ({ serverId }: ServerIdArgs): Promise<{ stopped: boolean }> =>
      stopHttpServer(state, serverId),
    list: (): number[] => [...state.servers.keys()].sort((a, b) => a - b),
  };
}

function ensureServerCapacity(backend: HttpServerBackendState): void {
  if (backend.servers.size + backend.reservations.size >= MAX_SERVERS) {
    throw new Error(`http-server.serve: server limit reached (max ${MAX_SERVERS})`);
  }
}

async function reserveHttpServer(
  backend: HttpServerBackendState,
  args: ServeArgs,
): Promise<ReserveResult> {
  ensureServerCapacity(backend);
  const host = args.host ?? "127.0.0.1";
  const requestedPort = args.port ?? 0;
  validateHost(host);
  validatePort(requestedPort);
  const options = resolveOptions(args);
  await backend.checkBind(host, requestedPort);
  // Capacity may have changed while the async policy check ran.
  ensureServerCapacity(backend);

  backend.nextServerId = nextId(
    backend.nextServerId,
    (id) => backend.servers.has(id) || backend.reservations.has(id),
  );
  const id = backend.nextServerId;
  backend.reservations.set(id, {
    id,
    host,
    requestedPort,
    options,
    activation: null,
    stopped: false,
  });
  return { id };
}

async function activateHttpServer(
  backend: HttpServerBackendState,
  serverId: number,
): Promise<ServeResult> {
  const active = backend.servers.get(serverId);
  if (active) return { id: active.id, host: active.host, port: active.port };

  const reservation = backend.reservations.get(serverId);
  if (!reservation || reservation.stopped) {
    throw new Error(`http-server.activate: reservation ${serverId} not found`);
  }
  if (!reservation.activation) {
    reservation.activation = bindReservedServer(backend, reservation).catch((error) => {
      if (backend.reservations.get(serverId) === reservation && !reservation.stopped) {
        reservation.activation = null;
      }
      throw error;
    });
  }
  return reservation.activation;
}

async function bindReservedServer(
  backend: HttpServerBackendState,
  reservation: ServerReservation,
): Promise<ServeResult> {
  const { id, host, requestedPort, options } = reservation;
  const stateRef: ServerStateRef = {};
  const server = createBunHttpServer({ backend, stateRef, host, port: requestedPort, options });
  const port = server.port ?? requestedPort;
  await checkResolvedPort({
    checkBind: backend.checkBind,
    server,
    host,
    requestedPort,
    actualPort: port,
  });

  if (reservation.stopped || backend.reservations.get(id) !== reservation) {
    await server.stop(true);
    throw new Error(`http-server.activate: reservation ${id} was stopped`);
  }

  const state = createServerState({ id, host, port, server, options });
  backend.servers.set(id, state);
  backend.reservations.delete(id);
  stateRef.current = state;
  return { id, host, port };
}

function createBunHttpServer({
  backend,
  stateRef,
  host,
  port,
  options,
}: {
  backend: HttpServerBackendState;
  stateRef: ServerStateRef;
  host: string;
  port: number;
  options: ResolvedOptions;
}): Bun.Server<WebSocketData> {
  return Bun.serve<WebSocketData>({
    hostname: host,
    port,
    maxRequestBodySize: options.bodyCapBytes,
    fetch(request, server) {
      const state = stateRef.current;
      if (!state) return new Response("server starting", { status: 503 });
      return handleRequest(state, request, server, backend.emit);
    },
    websocket: {
      data: {} as WebSocketData,
      open: (socket) => handleWebSocketOpen(backend.servers, backend.emit, socket),
      message: (socket, message) => handleWebSocketMessage(backend.emit, socket, message),
      close: (socket) => handleWebSocketClose(backend.servers, backend.emit, socket),
      maxPayloadLength: options.bodyCapBytes,
      closeOnBackpressureLimit: true,
    },
  });
}

function handleWebSocketOpen(
  servers: Map<number, ServerState>,
  emit: Emit,
  socket: Bun.ServerWebSocket<WebSocketData>,
): void {
  const current = servers.get(socket.data.serverId);
  if (!current) {
    socket.close(1012, "server stopped");
    return;
  }
  current.pendingWsConnections = Math.max(0, current.pendingWsConnections - 1);
  current.sockets.set(socket.data.connId, socket);
  emit("ws:open", socket.data);
}

function handleWebSocketMessage(
  emit: Emit,
  socket: Bun.ServerWebSocket<WebSocketData>,
  message: string | Buffer,
): void {
  emit(
    "ws:message",
    typeof message === "string"
      ? { ...socket.data, text: message }
      : { ...socket.data, binary: base64(message) },
  );
}

function handleWebSocketClose(
  servers: Map<number, ServerState>,
  emit: Emit,
  socket: Bun.ServerWebSocket<WebSocketData>,
): void {
  servers.get(socket.data.serverId)?.sockets.delete(socket.data.connId);
  emit("ws:close", {
    serverId: socket.data.serverId,
    connId: socket.data.connId,
  });
}

async function checkResolvedPort({
  checkBind,
  server,
  host,
  requestedPort,
  actualPort,
}: {
  checkBind: CheckBind;
  server: Bun.Server<WebSocketData>;
  host: string;
  requestedPort: number;
  actualPort: number;
}): Promise<void> {
  if (actualPort === requestedPort) return;
  try {
    await checkBind(host, actualPort);
  } catch (error) {
    await server.stop(true);
    throw error;
  }
}

function createServerState({
  id,
  host,
  port,
  server,
  options,
}: {
  id: number;
  host: string;
  port: number;
  server: Bun.Server<WebSocketData>;
  options: ResolvedOptions;
}): ServerState {
  return {
    id,
    host,
    port,
    server,
    options,
    pending: new Map(),
    sockets: new Map(),
    pendingWsConnections: 0,
    nextRequestId: 0,
    nextConnectionId: 0,
  };
}

function respondToRequest(servers: Map<number, ServerState>, args: RespondArgs): void {
  const state = servers.get(args.serverId);
  const pending = state?.pending.get(args.reqId);
  if (!state || !pending) return;
  const { status, body } = validateRespondArgs(args);
  if (isNullBodyStatus(status) && body.byteLength > 0) {
    throw new TypeError(`http-server.respond: status ${status} cannot have a response body`);
  }
  const headers = headersFromWire(args.headers);
  state.pending.delete(args.reqId);
  clearTimeout(pending.timer);
  pending.resolve(
    new Response(isNullBodyStatus(status) ? null : (body as unknown as BodyInit), {
      status,
      headers,
    }),
  );
}

function sendWebSocketMessage(servers: Map<number, ServerState>, args: WsSendArgs): void {
  const socket = servers.get(args.serverId)?.sockets.get(args.connId);
  if (!socket) return;
  const hasText = args.text != null;
  const hasBinary = args.binary != null;
  if (hasText === hasBinary) {
    throw new TypeError("http-server.wsSend: pass exactly one of text or binary");
  }
  socket.send(hasText ? (args.text as string) : decodeBase64(args.binary as string));
}

function closeWebSocket(servers: Map<number, ServerState>, args: WsConnectionArgs): void {
  servers.get(args.serverId)?.sockets.get(args.connId)?.close();
}

async function stopHttpServer(
  backend: HttpServerBackendState,
  serverId: number,
): Promise<{ stopped: boolean }> {
  const reservation = backend.reservations.get(serverId);
  if (reservation) {
    backend.reservations.delete(serverId);
    reservation.stopped = true;
    if (reservation.activation) {
      try {
        await reservation.activation;
      } catch {
        // Activation observes `stopped` and closes any partially bound socket.
      }
    }
    const activated = backend.servers.get(serverId);
    if (activated) return stopRunningServer(backend.servers, activated);
    return { stopped: true };
  }

  const state = backend.servers.get(serverId);
  if (!state) return { stopped: false };
  return stopRunningServer(backend.servers, state);
}

async function stopRunningServer(
  servers: Map<number, ServerState>,
  state: ServerState,
): Promise<{ stopped: boolean }> {
  servers.delete(state.id);
  const hadPendingResponses = state.pending.size > 0;
  for (const pending of state.pending.values()) {
    clearTimeout(pending.timer);
    pending.resolve(new Response("server stopped", { status: 503 }));
  }
  state.pending.clear();
  for (const socket of state.sockets.values()) socket.close(1012, "server stopped");
  state.sockets.clear();
  await stopBunServer(state.server, hadPendingResponses);
  return { stopped: true };
}

async function stopBunServer(
  server: Bun.Server<WebSocketData>,
  hadPendingResponses: boolean,
): Promise<void> {
  if (!hadPendingResponses) {
    await server.stop(true);
    return;
  }
  // Flush resolved 503s, but never let keep-alive peers hang stop forever.
  await Promise.race([
    server.stop(false),
    Bun.sleep(STOP_DRAIN_TIMEOUT_MS).then(() => server.stop(true)),
  ]);
}

export const httpServerBackend = definePlugin({
  name: "http-server",
  methods: createHttpServerMethods(),
});

export default httpServerBackend;
