import { describe, expect, test } from "bun:test";
import { decodeIpcError } from "./client/_internal";
import { encodeIpcError, TYND_IPC_ERR_SENTINEL, TyndIpcError } from "./errors";

describe("TyndIpcError class", () => {
  test("instanceof Error and TyndIpcError", () => {
    const e = new TyndIpcError({ code: "HANDLER_THREW", message: "boom" });
    expect(e).toBeInstanceOf(Error);
    expect(e).toBeInstanceOf(TyndIpcError);
    expect(e.name).toBe("TyndIpcError");
  });

  test("preserves code and fn", () => {
    const e = new TyndIpcError({ code: "UNKNOWN_FN", message: "no", fn: "fs.readText" });
    expect(e.code).toBe("UNKNOWN_FN");
    expect(e.fn).toBe("fs.readText");
    expect(e.message).toBe("no");
  });

  test("preserves cause when provided", () => {
    const root = new Error("root cause");
    const e = new TyndIpcError({ code: "PROTOCOL_ERROR", message: "wire", cause: root });
    expect(e.cause).toBe(root);
  });

  test("omits cause when not provided", () => {
    const e = new TyndIpcError({ code: "TIMEOUT", message: "slow" });
    // Per the constructor, we only forward `{ cause }` when defined — older
    // engines that don't recognize the option then leave `cause` `undefined`.
    expect(e.cause).toBeUndefined();
  });
});

describe("encodeIpcError -> decodeIpcError round-trip", () => {
  test("round-trips through the wire envelope", () => {
    const wire = encodeIpcError({ code: "HANDLER_THREW", message: "boom", fn: "greet" });
    expect(wire.startsWith(TYND_IPC_ERR_SENTINEL)).toBe(true);

    // Simulate the host shim wrapping the wire error message in `new Error(...)`.
    const hostError = new Error(wire);
    const decoded = decodeIpcError(hostError);
    expect(decoded).toBeInstanceOf(TyndIpcError);
    const tipc = decoded as TyndIpcError;
    expect(tipc.code).toBe("HANDLER_THREW");
    expect(tipc.message).toBe("boom");
    expect(tipc.fn).toBe("greet");
  });

  test("legacy plain Error (no sentinel) passes through unchanged", () => {
    const plain = new Error("just a regular error");
    expect(decodeIpcError(plain)).toBe(plain);
  });

  test("non-Error inputs pass through unchanged", () => {
    expect(decodeIpcError("string")).toBe("string");
    expect(decodeIpcError(42)).toBe(42);
    expect(decodeIpcError(null)).toBe(null);
    expect(decodeIpcError(undefined)).toBe(undefined);
  });

  test("malformed sentinel payload becomes PROTOCOL_ERROR", () => {
    const broken = new Error(`${TYND_IPC_ERR_SENTINEL}{not valid json`);
    const decoded = decodeIpcError(broken);
    expect(decoded).toBeInstanceOf(TyndIpcError);
    expect((decoded as TyndIpcError).code).toBe("PROTOCOL_ERROR");
    expect((decoded as TyndIpcError).cause).toBe(broken);
  });

  test("unknown code in payload falls back to HANDLER_THREW", () => {
    const wire = `${TYND_IPC_ERR_SENTINEL}${JSON.stringify({
      name: "TyndIpcError",
      code: "MADE_UP_CODE",
      message: "weird",
    })}`;
    const decoded = decodeIpcError(new Error(wire));
    expect(decoded).toBeInstanceOf(TyndIpcError);
    // Defensive: an enum we don't recognize must not be trusted as a stable code.
    expect((decoded as TyndIpcError).code).toBe("HANDLER_THREW");
    expect((decoded as TyndIpcError).message).toBe("weird");
  });

  test("uses fnHint when payload omits fn", () => {
    const wire = encodeIpcError({ code: "INVALID_ARGS", message: "bad" });
    const decoded = decodeIpcError(new Error(wire), "$plugin$sql$query");
    expect((decoded as TyndIpcError).fn).toBe("$plugin$sql$query");
  });

  test("payload fn wins over fnHint", () => {
    const wire = encodeIpcError({ code: "UNKNOWN_FN", message: "x", fn: "real.fn" });
    const decoded = decodeIpcError(new Error(wire), "hint.fn");
    expect((decoded as TyndIpcError).fn).toBe("real.fn");
  });

  test("each known code surfaces verbatim", () => {
    const codes = [
      "UNKNOWN_FN",
      "INVALID_ARGS",
      "HANDLER_THREW",
      "CANCELLED",
      "TIMEOUT",
      "PERMISSION_DENIED",
      "BACKEND_DEAD",
      "PROTOCOL_ERROR",
    ] as const;
    for (const code of codes) {
      const wire = encodeIpcError({ code, message: code });
      const decoded = decodeIpcError(new Error(wire));
      expect((decoded as TyndIpcError).code).toBe(code);
    }
  });
});
