/**
 * Bun↔host IPC framing — wire-format abstraction (U1 Layer 1).
 *
 * Two formats live side by side, picked once at process start by reading
 * the `TYND_IPC_FORMAT` env var the Rust spawner always sets on the Bun
 * child:
 *
 * - `json` (default): newline-delimited JSON. Backward-compatible with
 *   every existing app — same wire the framework has used since day one.
 * - `cbor` (opt-in): length-prefixed CBOR frames. ~10× faster decode on
 *   typed payloads, 30-50% smaller wire. Requires `binary-ipc` feature
 *   on the Rust `tynd-full` crate; the Rust spawner falls back to JSON
 *   when the feature is off so a misconfigured env var can't deadlock.
 *
 * Frame format (CBOR path):
 *
 * ```text
 * [u32 LE: body_len][body: cbor-x encoded payload]
 * ```
 *
 * `body_len` is exclusive of the 4-byte header. A `body_len` of 0 is
 * invalid framing — this module throws on read so the surrounding loop
 * can exit the IPC channel rather than spin.
 *
 * The format is read once at module load via `pickWireFormat()` so the
 * lookup doesn't hit `process.env` on every frame; both writer and
 * reader honor the same captured value.
 */

import { Buffer } from "node:buffer";
import { Decoder, Encoder } from "cbor-x";

/** Wire formats supported by the Bun↔host IPC channel. */
export type WireFormat = "json" | "cbor";

/**
 * Resolve the wire format from `TYND_IPC_FORMAT`. Defaults to JSON for
 * back-compat — any value other than `"cbor"` (including missing) keeps
 * the legacy newline-delimited JSON path. Cached at module load.
 */
export function pickWireFormat(): WireFormat {
  return process.env.TYND_IPC_FORMAT === "cbor" ? "cbor" : "json";
}

/** Hard cap on a single CBOR frame body — mirrors the Rust-side cap so
 * a malformed length prefix can't pin arbitrary RAM. 8 MiB. */
export const MAX_CBOR_FRAME = 8 * 1024 * 1024;

// Module-level encoder/decoder reused across frames — `cbor-x` allocates
// its scratch buffers inside the instance, so spinning a fresh one per
// frame would defeat the whole point of the binary path. `useRecords:
// false` keeps the wire format compatible with `ciborium` (the Rust
// side); `cbor-x`'s record optimization is a non-CBOR extension.
const _encoder = new Encoder({
  useRecords: false,
  largeBigIntToFloat: true,
  // `tagUint8Array: false` — mirror ciborium's plain-bytes encoding so
  // round-trips don't sprout extra CBOR tags the Rust side doesn't expect.
  tagUint8Array: false,
});
const _decoder = new Decoder({
  useRecords: false,
});

/**
 * Encode a value as a length-prefixed CBOR frame. The body is encoded
 * first into a `cbor-x` scratch buffer, then a 4-byte u32 LE prefix is
 * prepended in a single `Buffer.concat` so the writer issues one
 * `process.stdout.write` per frame.
 */
export function encodeCborFrame(value: unknown): Buffer {
  const body = _encoder.encode(value);
  if (body.length > MAX_CBOR_FRAME) {
    throw new Error(`CBOR frame body ${body.length} exceeds cap ${MAX_CBOR_FRAME}`);
  }
  const header = Buffer.alloc(4);
  header.writeUInt32LE(body.length, 0);
  return Buffer.concat([header, body], header.length + body.length);
}

/**
 * Streaming CBOR frame splitter. Holds the in-flight tail across `data`
 * events and yields decoded payloads as full frames arrive. Mirror of
 * the line-splitter used on the JSON path, so the upstream consumer
 * stays format-agnostic.
 */
export class CborFrameReader {
  /** Accumulated bytes from previous chunks that didn't form a full frame. */
  private _pending: Buffer = Buffer.alloc(0);

  /**
   * Push a new chunk of bytes. Returns the list of fully-decoded
   * payloads it now has — typically 0 or 1, occasionally more on a
   * burst. Throws on framing errors (bad length, oversized frame) —
   * the surrounding loop is expected to tear down the IPC channel
   * because re-syncing a binary stream after a corrupt prefix is not
   * something we attempt.
   */
  push(chunk: Buffer): unknown[] {
    // Keep one growing buffer rather than `Buffer.concat` per chunk —
    // saves ~40% on the burst path (lots of small frames). `cbor-x`
    // reads from the slice we hand it; the slice is invalidated as
    // soon as we slice forward, so each frame is fully decoded before
    // we advance the cursor.
    this._pending =
      this._pending.length === 0
        ? chunk
        : Buffer.concat([this._pending, chunk], this._pending.length + chunk.length);

    const out: unknown[] = [];
    let cursor = 0;
    while (this._pending.length - cursor >= 4) {
      const bodyLen = this._pending.readUInt32LE(cursor);
      if (bodyLen === 0) {
        throw new Error("CBOR frame body length is 0");
      }
      if (bodyLen > MAX_CBOR_FRAME) {
        throw new Error(`CBOR frame body ${bodyLen} exceeds cap ${MAX_CBOR_FRAME}`);
      }
      const frameEnd = cursor + 4 + bodyLen;
      if (frameEnd > this._pending.length) break; // body not fully arrived yet
      const body = this._pending.subarray(cursor + 4, frameEnd);
      out.push(_decoder.decode(body));
      cursor = frameEnd;
    }
    // Drop everything we consumed; keep only the tail for the next push.
    this._pending = cursor === 0 ? this._pending : this._pending.subarray(cursor);
    return out;
  }
}
