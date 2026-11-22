import { describe, expect, test } from "bun:test";
import {
  decodeMessage,
  encodeFrame,
  encodeRequest,
  FrameDecoder,
  MAX_FRAME_BYTES,
  RpcError,
} from "./protocol.ts";

const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

describe("encodeFrame", () => {
  test("prefixes the body with its little-endian u32 length", () => {
    const frame = encodeFrame(new Uint8Array([1, 2, 3]));
    expect([...frame]).toEqual([3, 0, 0, 0, 1, 2, 3]);
  });

  test("encodes lengths above 255 across multiple bytes", () => {
    const frame = encodeFrame(new Uint8Array(0x1_02_03));
    expect([...frame.subarray(0, 4)]).toEqual([0x03, 0x02, 0x01, 0x00]);
  });

  test("rejects bodies above the host limit", () => {
    expect(() => encodeFrame(new Uint8Array(MAX_FRAME_BYTES + 1))).toThrow(RangeError);
  });
});

describe("encodeRequest", () => {
  test("produces a JSON-RPC 2.0 request with a string id", () => {
    const frame = encodeRequest("7", "webview.click", { target: { ref: "e1" } });
    const length = new DataView(frame.buffer).getUint32(0, true);
    expect(length).toBe(frame.length - 4);
    expect(JSON.parse(text(frame.subarray(4)))).toEqual({
      jsonrpc: "2.0",
      id: "7",
      method: "webview.click",
      params: { target: { ref: "e1" } },
    });
  });

  test("defaults params to an empty object", () => {
    const body = JSON.parse(text(encodeRequest("1", "window.list").subarray(4)));
    expect(body.params).toEqual({});
  });

  test("keeps non-ASCII text intact (length counts bytes)", () => {
    const frame = encodeRequest("1", "webview.type", { text: "héllo ✓" });
    expect(new DataView(frame.buffer).getUint32(0, true)).toBe(frame.length - 4);
    expect(JSON.parse(text(frame.subarray(4))).params.text).toBe("héllo ✓");
  });
});

describe("FrameDecoder", () => {
  const two = new Uint8Array([
    ...encodeFrame(Uint8Array.of(1)),
    ...encodeFrame(Uint8Array.of(2, 3)),
  ]);

  test("returns every complete frame of a chunk", () => {
    const frames = new FrameDecoder().push(two);
    expect(frames.map((frame) => [...frame])).toEqual([[1], [2, 3]]);
  });

  test("reassembles frames split at every possible byte boundary", () => {
    for (let split = 1; split < two.length; split++) {
      const decoder = new FrameDecoder();
      const frames = [
        ...decoder.push(two.subarray(0, split)),
        ...decoder.push(two.subarray(split)),
      ];
      expect(frames.map((frame) => [...frame])).toEqual([[1], [2, 3]]);
      expect(decoder.pending).toBe(0);
    }
  });

  test("handles one byte at a time", () => {
    const decoder = new FrameDecoder();
    const frames: number[][] = [];
    for (const byte of two)
      for (const frame of decoder.push(Uint8Array.of(byte))) frames.push([...frame]);
    expect(frames).toEqual([[1], [2, 3]]);
  });

  test("handles a zero-length body", () => {
    expect(new FrameDecoder().push(encodeFrame(new Uint8Array(0)))).toHaveLength(1);
  });

  test("rejects an oversized length prefix", () => {
    const header = new Uint8Array(4);
    new DataView(header.buffer).setUint32(0, MAX_FRAME_BYTES + 1, true);
    expect(() => new FrameDecoder().push(header)).toThrow(RangeError);
  });
});

describe("decodeMessage", () => {
  test("parses a response", () => {
    const body = new TextEncoder().encode('{"jsonrpc":"2.0","id":"1","result":{"ok":true}}');
    expect(decodeMessage(body)).toEqual({ jsonrpc: "2.0", id: "1", result: { ok: true } });
  });

  test("rejects non-object bodies", () => {
    expect(() => decodeMessage(new TextEncoder().encode("[1]"))).toThrow(TypeError);
  });
});

describe("RpcError", () => {
  test("carries code and method", () => {
    const error = new RpcError({ code: -32601, message: "method not found" }, "nope");
    expect(error.code).toBe(-32601);
    expect(error.message).toBe("nope: method not found (-32601)");
  });
});

describe("FrameDecoder chunking", () => {
  test("a large frame arriving in tiny chunks decodes once, intact", () => {
    const body = new Uint8Array(300_000).map((_, i) => i % 251);
    const frame = encodeFrame(body);
    const decoder = new FrameDecoder();
    const out: Uint8Array[] = [];
    for (let i = 0; i < frame.length; i += 1_000)
      out.push(...decoder.push(frame.subarray(i, i + 1_000)));
    expect(out).toHaveLength(1);
    expect(out[0]).toEqual(body);
    expect(decoder.pending).toBe(0);
  });

  test("a header split across chunks and several frames in one chunk", () => {
    const a = encodeFrame(new Uint8Array([1]));
    const b = encodeFrame(new Uint8Array([2, 3]));
    const joined = new Uint8Array([...a, ...b]);
    const decoder = new FrameDecoder();
    expect(decoder.push(joined.subarray(0, 2))).toEqual([]);
    expect(decoder.pending).toBe(2);
    const frames = decoder.push(joined.subarray(2));
    expect(frames.map((f) => [...f])).toEqual([[1], [2, 3]]);
  });

  test("an oversize length prefix is rejected before the body arrives", () => {
    const header = new Uint8Array(4);
    new DataView(header.buffer).setUint32(0, MAX_FRAME_BYTES + 1, true);
    expect(() => new FrameDecoder().push(header)).toThrow(RangeError);
  });
});
