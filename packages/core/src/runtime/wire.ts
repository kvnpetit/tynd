/**
 * IPC wire helpers — type-preserving (de)serialization for the user payload
 * fields of `call` / `return` / `yield` / `event` messages.
 *
 * The IPC framing itself stays plain JSON so the Rust host can route messages
 * without deserializing user data (it inspects `type`, `id`, etc. only).
 * Inside the framing the user-payload fields (`v`, `p`, `a`) carry a
 * devalue-encoded string instead of raw JSON, which lets `Date`, `Map`,
 * `Set`, `BigInt`, `RegExp`, `Error`, `URL`, and cyclic references survive
 * the round trip — JSON.stringify silently flattens or drops all of those.
 *
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
  if (s == null || s === "") return undefined;
  return parse(s);
}

/** Decode an `args` array with each element re-hydrated. Used on the Bun side
 * to materialize `call` payloads before invoking the user's handler. */
export function decodeArgs(s: string | undefined | null): unknown[] {
  const v = decode(s);
  return Array.isArray(v) ? v : [];
}
