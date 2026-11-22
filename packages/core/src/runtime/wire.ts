/**
 * IPC wire helpers — type-preserving (de)serialization for the user payload
 * fields of `call` / `return` / `yield` / `event` messages.
 * The IPC framing itself stays plain JSON so the Rust host can route messages
 * without deserializing user data (it inspects `type`, `id`, etc. only).
 * Inside the framing the user-payload fields (`v`, `p`, `a`) carry a
 * devalue-encoded string instead of raw JSON, which lets `Date`, `Map`,
 * `Set`, `BigInt`, `RegExp`, `URL`, typed arrays, `undefined`, and cyclic
 * references survive the round trip — JSON.stringify silently flattens or
 * drops all of those.
 * `Error` instances (and other class instances) are NOT encodable: `encode`
 * throws, which the dispatcher reports as a `PROTOCOL_ERROR`. A handler that
 * wants to signal failure must `throw`; the throw travels as a structured
 * `TyndIpcError` (see `errors.ts`), not through devalue.
 * `os_call` deliberately keeps plain JSON args because the Rust host inspects
 * them directly (window dimensions, menu IDs, etc.); devalue-encoding those
 * would break native dispatch.
 */

import { parse, stringify } from "devalue";

/** Encode a JS value for the wire. Throws if the value is unencodable. */
export function encode(value: unknown): string {
  return stringify(value);
}

/**
 * Decode a wire string back into a JS value. Returns `undefined` for empty /
 * missing strings so callers don't have to special-case `null` / `""`.
 */
function decode(s: string | undefined | null): unknown {
  if (s == null || s === "") return;
  return parse(s);
}

/** Decode an `args` array with each element re-hydrated. Used on the Bun side
 * to materialize `call` payloads before invoking the user's handler. */
export function decodeArgs(s: string | undefined | null): unknown[] {
  const v = decode(s);
  if (v === undefined) return [];
  if (!Array.isArray(v)) {
    throw new TypeError("IPC args payload must decode to an array");
  }
  return v;
}
