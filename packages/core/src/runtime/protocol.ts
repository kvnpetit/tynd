/**
 * Backend-served custom protocols (`app.protocol(name, handler)`).
 *
 * The host forwards each webview request to `name://...` as a `protocol` frame;
 * the handler's `Response` goes back as a `protocol_response` frame. Bodies are
 * buffered and base64-encoded inside one JSON frame, so both directions are
 * capped at {@link MAX_PROTOCOL_BODY_BYTES}.
 */
import * as v from "valibot";
import { tynd } from "../logger";
import { sanitizedMessage } from "./error";

/** Request and response body cap (the host enforces the same value). */
export const MAX_PROTOCOL_BODY_BYTES = 4 * 1024 * 1024;

/** Where a protocol request comes from. */
export interface ProtocolContext {
  /** Label of the webview that issued the request. */
  webview: string;
}

/** Answers one request to an `app.protocol(...)` scheme. */
export type ProtocolHandler = (
  request: Request,
  context: ProtocolContext,
) => Response | Promise<Response>;

/**
 * Schemes Tynd, the asset protocol or the web platform own. Kept in sync with
 * `RESERVED_SCHEMES` in `crates/host/src/webview/protocols.rs`.
 */
const RESERVED_SCHEMES = new Set([
  "tynd",
  "tynd-bin",
  "asset",
  "ipc",
  "http",
  "https",
  "ws",
  "wss",
  "file",
  "ftp",
  "about",
  "blob",
  "data",
  "javascript",
  "mailto",
  "tel",
  "view-source",
  "chrome",
  "edge",
  "webkit",
  "localhost",
]);

/** A DNS label: WebView2 serves the scheme as `http://<name>.localhost`. */
const SCHEME_NAME = /^[a-z](?:[a-z0-9-]{0,30}[a-z0-9])?$/;

const handlers = new Map<string, ProtocolHandler>();

/** Reject a name the host would refuse, with the fix in the message. */
export function validateProtocolName(name: string): void {
  if (typeof name !== "string" || !SCHEME_NAME.test(name)) {
    throw new TypeError(
      `app.protocol(): '${String(name)}' is not a valid protocol name — use 1-32 lowercase letters, digits or '-', starting with a letter (e.g. "media")`,
    );
  }
  if (RESERVED_SCHEMES.has(name)) {
    throw new TypeError(
      `app.protocol(): '${name}' is reserved — pick an app-specific name such as "media"`,
    );
  }
}

/** Register `handler` for `name://` requests. */
export function registerProtocol(name: string, handler: ProtocolHandler): void {
  validateProtocolName(name);
  if (typeof handler !== "function") {
    throw new TypeError(`app.protocol("${name}"): the handler must be a function`);
  }
  if (handlers.has(name)) {
    throw new Error(`app.protocol(): '${name}' is already registered`);
  }
  handlers.set(name, handler);
}

/** Registered scheme names, sent in the `tynd:config` frame. */
export function protocolNames(): string[] {
  return [...handlers.keys()];
}

/** @internal tests only. */
export function resetProtocolsForTests(): void {
  handlers.clear();
}

export const ProtocolMsgSchema = v.object({
  type: v.literal("protocol"),
  id: v.string(),
  scheme: v.string(),
  method: v.string(),
  url: v.string(),
  headers: v.array(v.tuple([v.string(), v.string()])),
  body: v.nullish(v.string()),
  webview: v.string(),
});
export type ProtocolMsg = v.InferOutput<typeof ProtocolMsgSchema>;

interface ProtocolReply {
  type: "protocol_response";
  id: string;
  status: number;
  headers: [string, string][];
  body: string | null;
}

/** Headers that describe the transfer, not the body the host forwards. */
const DROPPED_HEADERS = new Set(["content-length", "transfer-encoding", "connection"]);

function errorReply(id: string, status: number, message: string): ProtocolReply {
  return {
    type: "protocol_response",
    id,
    status,
    headers: [["content-type", "text/plain; charset=utf-8"]],
    body: Buffer.from(message).toString("base64"),
  };
}

/** Run the handler of one forwarded request and write its reply frame. */
export async function handleProtocolMsg(
  msg: ProtocolMsg,
  write: (frame: Record<string, unknown>) => void,
): Promise<void> {
  write({ ...(await answer(msg)) });
}

async function answer(msg: ProtocolMsg): Promise<ProtocolReply> {
  const handler = handlers.get(msg.scheme);
  if (!handler) return errorReply(msg.id, 404, `no handler for protocol '${msg.scheme}'`);
  let response: unknown;
  try {
    const method = msg.method.toUpperCase();
    const body =
      msg.body && method !== "GET" && method !== "HEAD" ? Buffer.from(msg.body, "base64") : null;
    const request = new Request(msg.url, { method, headers: msg.headers, body });
    response = await handler(request, { webview: msg.webview });
  } catch (e) {
    tynd.error(`app.protocol("${msg.scheme}") handler threw: ${sanitizedMessage(e)}`);
    return errorReply(msg.id, 500, `protocol handler failed: ${sanitizedMessage(e)}`);
  }
  if (!(response instanceof Response)) {
    tynd.error(`app.protocol("${msg.scheme}") handler must return a Response`);
    return errorReply(msg.id, 500, "protocol handler did not return a Response");
  }
  let bytes: Uint8Array;
  try {
    bytes = new Uint8Array(await response.arrayBuffer());
  } catch (e) {
    return errorReply(msg.id, 500, `protocol response body failed: ${sanitizedMessage(e)}`);
  }
  if (bytes.byteLength > MAX_PROTOCOL_BODY_BYTES) {
    tynd.error(
      `app.protocol("${msg.scheme}") response exceeds ${MAX_PROTOCOL_BODY_BYTES} bytes — serve large files with convertFileSrc`,
    );
    return errorReply(msg.id, 502, "protocol response body is too large");
  }
  // `fetch()` already decoded the body it hands back: forwarding its
  // `content-encoding` would make the webview decode it a second time.
  const fetched = response.url !== "";
  const headers: [string, string][] = [];
  response.headers.forEach((value, name) => {
    if (DROPPED_HEADERS.has(name)) return;
    if (fetched && name === "content-encoding") return;
    headers.push([name, value]);
  });
  return {
    type: "protocol_response",
    id: msg.id,
    status: response.status,
    headers,
    body: bytes.byteLength ? Buffer.from(bytes).toString("base64") : null,
  };
}
