/**
 * Discriminated error type for IPC failures crossing the host boundary.
 * `TyndIpcError` lets frontends `switch` on a stable `code` instead of
 * substring-matching `error.message`. Surfaced everywhere a backend RPC
 * call fails: unknown function, args validation, handler throw, cancel,
 * timeout, security policy, host crash, wire decode error.
 * Wire envelope: the Bun runtime serializes errors as
 *   `__TYND_IPC_ERR__:<json>`
 * where `<json>` is a `{ name, code, message, fn? }` payload — the host
 * shim wraps this verbatim in `new Error(message)`. The client-side call
 * proxy detects the sentinel prefix in `_internal.ts::decodeIpcError` and
 * reconstructs the typed instance. Errors without the sentinel (OS calls)
 * degrade to plain `Error`.
 * @example
 *   try {
 *     await api.greet("Alice")
 *   } catch (e) {
 *     if (e instanceof TyndIpcError && e.code === "HANDLER_THREW") {
 *       toast.error(`Backend crashed: ${e.message}`)
 *     } else throw e
 *   }
 */
export class TyndIpcError extends Error {
  readonly code: TyndIpcErrorCode;
  /** RPC function name (e.g. `"fs.readText"` or `"$plugin$sql$query"`). */
  readonly fn?: string;

  constructor(opts: {
    code: TyndIpcErrorCode;
    message: string;
    fn?: string;
    cause?: unknown;
  }) {
    super(opts.message, opts.cause !== undefined ? { cause: opts.cause } : undefined);
    this.name = "TyndIpcError";
    this.code = opts.code;
    this.fn = opts.fn;
  }
}

/**
 * Stable codes for `TyndIpcError.code`. Kept narrow on purpose — frontends
 * use `switch (e.code)` to react and a churning enum hurts more than it
 * helps. New codes append-only.
 */
export type TyndIpcErrorCode =
  /** RPC function not registered in the backend module / plugin registry. */
  | "UNKNOWN_FN"
  /** Args failed valibot validation or devalue/JSON decode. */
  | "INVALID_ARGS"
  /** Backend handler threw — the most common case. */
  | "HANDLER_THREW"
  /** Call was cancelled by the client (`handle.cancel()` or AbortSignal). */
  | "CANCELLED"
  /** Call exceeded its `withTimeout(...)` window. */
  | "TIMEOUT"
  /** Startup ACL / `app.start({ security })` policy denied the call. */
  | "PERMISSION_DENIED"
  /** Host process died mid-call (for example, Bun crashed). */
  | "BACKEND_DEAD"
  /** Wire-level encode/decode failure (devalue / JSON / base64). */
  | "PROTOCOL_ERROR";

/** Sentinel prefix used on the wire — `_internal.ts` strips it. */
export const TYND_IPC_ERR_SENTINEL = "__TYND_IPC_ERR__:";

/**
 * Serialize a `TyndIpcError` for the wire. The host shim wraps the result
 * in `new Error(message)`, so the client proxy strips the sentinel +
 * reconstructs.
 */
export function encodeIpcError(opts: {
  code: TyndIpcErrorCode;
  message: string;
  fn?: string;
}): string {
  const payload: { name: "TyndIpcError"; code: TyndIpcErrorCode; message: string; fn?: string } = {
    name: "TyndIpcError",
    code: opts.code,
    message: opts.message,
  };
  if (opts.fn !== undefined) payload.fn = opts.fn;
  // JSON.stringify is safe here — code is always a string literal, message
  // came through `sanitizedMessage`, fn is always a string.
  return TYND_IPC_ERR_SENTINEL + JSON.stringify(payload);
}
