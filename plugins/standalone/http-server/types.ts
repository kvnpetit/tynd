import type { HeaderPairs } from "./headers";

export interface HttpServerOptions {
  maxPendingRequests?: number | null;
  maxWsConnections?: number | null;
  responseTimeoutSecs?: number | null;
  bodyCapBytes?: number | null;
  streaming?: boolean | null;
}

export interface ServeArgs extends HttpServerOptions {
  port?: number | null;
  host?: string | null;
  websocket?: boolean;
}

export interface ReserveResult {
  id: number;
}

export interface ServeResult {
  id: number;
  host: string;
  port: number;
}

export interface RespondArgs {
  serverId: number;
  reqId: number;
  status?: number | null;
  headers?: HeaderPairs | Record<string, string> | null;
  body?: string | null;
}

export interface ServerIdArgs {
  serverId: number;
}

export interface WsConnectionArgs extends ServerIdArgs {
  connId: number;
}

export interface WsSendArgs extends WsConnectionArgs {
  text?: string | null;
  binary?: string | null;
}

export interface HttpRequestPayload {
  serverId: number;
  reqId: number;
  method: string;
  path: string;
  headers: Record<string, string>;
  body: string;
  remote: string;
  streaming?: boolean;
}

export interface HttpRequestChunkPayload {
  serverId: number;
  reqId: number;
  chunk?: string;
  error?: string;
  done?: boolean;
}

export interface WsOpenPayload {
  serverId: number;
  connId: number;
  remote: string;
}

export interface WsMessagePayload {
  serverId: number;
  connId: number;
  text?: string;
  binary?: string;
}

export interface WsClosePayload {
  serverId: number;
  connId: number;
}

export type HttpServerEventMap = {
  "http:request": HttpRequestPayload;
  "http:request-chunk": HttpRequestChunkPayload;
  "ws:open": WsOpenPayload;
  "ws:message": WsMessagePayload;
  "ws:close": WsClosePayload;
};
