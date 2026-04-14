/**
 * Transparent binary encoding for IPC payloads.
 *
 * Walks any JSON-serializable value and replaces `Uint8Array` / `ArrayBuffer`
 * / `Buffer` instances with `{ __bv_bytes: "<base64>" }` markers.
 * `deserialize()` reverses the transform.
 *
 * This lets `client.rpc.X(bytes)` and handlers that return `Uint8Array`
 * work transparently — callers see typed arrays on both ends.
 *
 * NOTE: base64 adds ~33% overhead. For very large transfers (>10 MB)
 * prefer writing to disk and sending the file path instead.
 */

const MARKER = "__bv_bytes";

function toBase64(bytes: Uint8Array): string {
  if (typeof Buffer !== "undefined") {
    return Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength).toString("base64");
  }
  let bin = "";
  for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]!);
  return btoa(bin);
}

function fromBase64(b64: string): Uint8Array {
  if (typeof Buffer !== "undefined") {
    return new Uint8Array(Buffer.from(b64, "base64"));
  }
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function isBinary(v: unknown): v is Uint8Array | ArrayBuffer {
  return v instanceof Uint8Array || v instanceof ArrayBuffer
    || (typeof Buffer !== "undefined" && v instanceof Buffer);
}

/** Encode typed arrays / buffers in `value` as base64 markers. */
export function serialize<T>(value: T): T {
  if (value == null) return value;
  if (isBinary(value)) {
    const bytes = value instanceof ArrayBuffer ? new Uint8Array(value) : value as Uint8Array;
    return { [MARKER]: toBase64(bytes) } as unknown as T;
  }
  if (Array.isArray(value)) {
    return value.map(serialize) as unknown as T;
  }
  if (typeof value === "object") {
    // Channel instances carry a well-known id — forward the marker only so the
    // backend can rebuild a live Channel pointing at the same event name.
    const chId = (value as { __bv_channel_id__?: unknown }).__bv_channel_id__;
    if (typeof chId === "string") return { __bv_channel: chId } as unknown as T;

    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(value)) out[k] = serialize(v);
    return out as unknown as T;
  }
  return value;
}

/** Decode base64 markers back to `Uint8Array` in-place. */
export function deserialize<T>(value: T): T {
  if (value == null) return value;
  if (Array.isArray(value)) return value.map(deserialize) as unknown as T;
  if (typeof value === "object") {
    const obj = value as Record<string, unknown>;
    const b64 = obj[MARKER];
    if (typeof b64 === "string" && Object.keys(obj).length === 1) {
      return fromBase64(b64) as unknown as T;
    }
    const out: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(obj)) out[k] = deserialize(v);
    return out as unknown as T;
  }
  return value;
}
