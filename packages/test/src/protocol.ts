/**
 * Wire format of the Tynd debug protocol: JSON-RPC 2.0 bodies, each prefixed by a
 * 4-byte little-endian length (see crates/debug-protocol/src/protocol.rs).
 */

/** Maximum frame body size accepted by the host (16 MiB). */
export const MAX_FRAME_BYTES = 16 * 1024 * 1024;

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue | undefined };

export interface RpcRequest {
  jsonrpc: "2.0";
  id: string;
  method: string;
  params: unknown;
}

export interface RpcErrorBody {
  code: number;
  message: string;
  data?: unknown;
}

/** A decoded server message: a response (has `id`) or a notification (has `method`, no `id`). */
export interface RpcMessage {
  jsonrpc?: string;
  id?: string | number | null;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: RpcErrorBody;
}

/** Error returned by the host for a request (JSON-RPC `error` member). */
export class RpcError extends Error {
  readonly code: number;
  readonly data: unknown;

  constructor(body: RpcErrorBody, method?: string) {
    super(method ? `${method}: ${body.message} (${body.code})` : `${body.message} (${body.code})`);
    this.name = "RpcError";
    this.code = body.code;
    this.data = body.data;
  }
}

/** Prefix `body` with its 4-byte little-endian length. */
export function encodeFrame(body: Uint8Array): Uint8Array {
  if (body.length > MAX_FRAME_BYTES) {
    throw new RangeError(`debug frame too large: ${body.length} > ${MAX_FRAME_BYTES}`);
  }
  const frame = new Uint8Array(4 + body.length);
  new DataView(frame.buffer).setUint32(0, body.length, true);
  frame.set(body, 4);
  return frame;
}

/** Encode a JSON-RPC request as a ready-to-write frame. */
export function encodeRequest(id: string, method: string, params: unknown = {}): Uint8Array {
  const request: RpcRequest = { jsonrpc: "2.0", id, method, params };
  return encodeFrame(new TextEncoder().encode(JSON.stringify(request)));
}

/**
 * Incremental frame reader: feed arbitrary chunks, get complete frame bodies back.
 * Chunks are queued (never concatenated) and each body is assembled once, so a
 * large frame delivered in small chunks costs O(frame size), not O(chunks x size).
 */
export class FrameDecoder {
  #chunks: Uint8Array[] = [];
  /** Bytes already consumed from the first queued chunk. */
  #offset = 0;
  #size = 0;

  /** Append `chunk` and return every frame body that is now complete. */
  push(chunk: Uint8Array): Uint8Array[] {
    if (chunk.length > 0) {
      this.#chunks.push(chunk);
      this.#size += chunk.length;
    }
    const frames: Uint8Array[] = [];
    for (;;) {
      if (this.#size < 4) break;
      const header = this.#take(4, false);
      const length = new DataView(header.buffer, header.byteOffset, 4).getUint32(0, true);
      if (length > MAX_FRAME_BYTES) {
        throw new RangeError(`debug frame too large: ${length} > ${MAX_FRAME_BYTES}`);
      }
      if (this.#size < 4 + length) break;
      this.#take(4, true);
      frames.push(this.#take(length, true));
    }
    return frames;
  }

  /** Bytes received that do not yet form a complete frame. */
  get pending(): number {
    return this.#size;
  }

  /** Copy the next `count` queued bytes; `consume` also removes them from the queue. */
  #take(count: number, consume: boolean): Uint8Array {
    const out = new Uint8Array(count);
    let written = 0;
    let index = 0;
    let offset = this.#offset;
    while (written < count) {
      const chunk = this.#chunks[index] as Uint8Array;
      const n = Math.min(chunk.length - offset, count - written);
      out.set(chunk.subarray(offset, offset + n), written);
      written += n;
      offset += n;
      if (offset === chunk.length) {
        index++;
        offset = 0;
      }
    }
    if (consume) {
      this.#chunks.splice(0, index);
      this.#offset = offset;
      this.#size -= count;
    }
    return out;
  }
}

/** Parse a frame body into a JSON-RPC message. */
export function decodeMessage(body: Uint8Array): RpcMessage {
  const value: unknown = JSON.parse(new TextDecoder().decode(body));
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new TypeError("debug frame is not a JSON object");
  }
  return value as RpcMessage;
}
