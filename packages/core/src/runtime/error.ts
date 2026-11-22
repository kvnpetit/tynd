/**
 * Single source of truth for "what does an error look like on the wire".
 * SECURITY: errors crossing the IPC boundary are stringified to
 * `{name, message}` only. Stack frames are dropped on purpose — they often
 * embed absolute filesystem paths, env-resolved tokens, source-map hints,
 * or values from `Error()` constructors that the host should never receive.
 * Bun runtime calls this on every error
 * path that fans out to the host (return frame, yield-fail, frontend emit
 * dispatch). Keeping it centralized means a future change (e.g. add
 * `cause` chain stringification) lands in one place.
 */
export interface SanitizedError {
  /** `Error.name` for `Error` instances; `"Error"` for everything else. */
  name: string;
  /** Short human-readable summary — never contains stack / paths. */
  message: string;
}

export function sanitizeError(e: unknown): SanitizedError {
  if (e instanceof Error) {
    // Intentionally NOT including e.stack — see file header.
    return { name: e.name || "Error", message: e.message };
  }
  if (typeof e === "string") {
    return { name: "Error", message: e };
  }
  // Fallback: stringify so the caller still gets *some* signal. Objects
  // with a `message` field (custom error-likes) flow through here.
  if (e !== null && typeof e === "object") {
    const maybeMessage = (e as { message?: unknown }).message;
    if (typeof maybeMessage === "string") {
      const maybeName = (e as { name?: unknown }).name;
      return {
        name: typeof maybeName === "string" ? maybeName : "Error",
        message: maybeMessage,
      };
    }
  }
  return { name: "Error", message: String(e) };
}

/** Convenience: just the sanitized message, for call sites that only log or forward it. */
export function sanitizedMessage(e: unknown): string {
  return sanitizeError(e).message;
}
