/** Response headers on the wire: ordered pairs, so repeated names (Set-Cookie) survive. */
export type HeaderPairs = Array<[string, string]>;

/**
 * Handlers run in the webview, where `Set-Cookie` is a forbidden response-header
 * name: the browser silently drops it from any `Response` a handler builds.
 * Handlers carry cookies through this ordinary header instead, with a JSON array
 * of `Set-Cookie` strings as its value; `headerPairs` turns it back into real
 * `Set-Cookie` pairs for the wire. Build the value with `setCookieHeader`.
 */
export const SET_COOKIE_CARRIER = "x-tynd-set-cookie";

/** Header entry to add to a handler `Response` so the client receives these cookies. */
export function setCookieHeader(cookies: string[]): [string, string] {
  return [SET_COOKIE_CARRIER, JSON.stringify(cookies)];
}

/** CR, LF and NUL are illegal in a header value; the backend `Headers` rejects them. */
const FORBIDDEN_HEADER_CHARS = /[\r\n\0]/;

function decodeCarrier(value: string): string[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value);
  } catch {
    // Malformed carrier: ignore it rather than failing the whole response.
    return [];
  }
  if (!Array.isArray(parsed)) return [];
  const cookies = parsed.filter((c): c is string => typeof c === "string");
  // The JSON carrier lets CR/LF/NUL through the webview `Headers`, but the backend would
  // throw on them after dispatch and leave the request hanging. Fail here so the handler
  // path answers 500 instead.
  if (cookies.some((c) => FORBIDDEN_HEADER_CHARS.test(c))) {
    throw new TypeError(`${SET_COOKIE_CARRIER}: cookie contains CR, LF or NUL`);
  }
  return cookies;
}

/** Every Set-Cookie value of `headers`; `getSetCookie` is missing before Safari 17 / WebKitGTK 2.42. */
function nativeSetCookies(headers: Headers): string[] {
  if (typeof headers.getSetCookie === "function") return headers.getSetCookie();
  const joined = headers.get("set-cookie");
  return joined ? [joined] : [];
}

/** Flatten a `Headers` into wire pairs, keeping every `Set-Cookie` separate. */
export function headerPairs(headers: Headers): HeaderPairs {
  const pairs: HeaderPairs = [];
  const cookies: string[] = [];
  headers.forEach((value, name) => {
    const lower = name.toLowerCase();
    // `forEach` joins every Set-Cookie into one entry; emitted individually below.
    if (lower === "set-cookie") return;
    if (lower === SET_COOKIE_CARRIER) cookies.push(...decodeCarrier(value));
    else pairs.push([name, value]);
  });
  for (const cookie of [...nativeSetCookies(headers), ...cookies]) {
    pairs.push(["set-cookie", cookie]);
  }
  return pairs;
}

/** Build a `Headers` from wire pairs, or from a plain `{name: value}` record. */
export function headersFromWire(
  wire: HeaderPairs | Record<string, string> | null | undefined,
): Headers {
  const headers = new Headers();
  if (!wire) return headers;
  const entries = Array.isArray(wire) ? wire : Object.entries(wire);
  for (const [name, value] of entries) headers.append(name, value);
  return headers;
}
