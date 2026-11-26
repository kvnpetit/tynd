/** Convert an unknown thrown/wire value into a stable message without throwing again. */
function errorMessage(value: unknown, fallback = "Unknown error"): string {
  if (value instanceof Error) return value.message;
  if (typeof value === "string") return value || fallback;
  if (value == null) return fallback;
  try {
    const json = JSON.stringify(value);
    if (json) return json;
  } catch {
    // Circular or otherwise non-JSON values still have a useful String form.
  }
  return String(value) || fallback;
}

export function asError(value: unknown, fallback?: string): Error {
  return value instanceof Error ? value : new Error(errorMessage(value, fallback));
}
