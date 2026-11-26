/**
 * URLs for the app's custom protocols: the `asset://` file protocol
 * (`convertFileSrc`) and the schemes the backend serves with
 * `app.protocol(...)` (`protocolUrl`).
 *
 * WebView2 (Windows) and Android WebView cannot load custom schemes directly:
 * wry serves a scheme `x` as `http://x.localhost` there, `x://localhost`
 * elsewhere. These helpers pick the right spelling at runtime.
 */
import { binOrigin } from "./_internal";

/** Same rule as the host and `app.protocol`: a lowercase DNS label. */
const SCHEME_NAME = /^[a-z](?:[a-z0-9-]{0,30}[a-z0-9])?$/;

/** Origin of scheme `scheme` in the current webview. */
function schemeOrigin(scheme: string): string {
  if (!SCHEME_NAME.test(scheme)) {
    throw new TypeError(
      `'${scheme}' is not a valid protocol name — use lowercase letters, digits or '-', starting with a letter`,
    );
  }
  return binOrigin().startsWith("http://") ? `http://${scheme}.localhost` : `${scheme}://localhost`;
}

/**
 * URL that loads the local file `path` through the `asset://` protocol, for
 * `<img>`, `<video>`, `<audio>`, CSS or `fetch`. Video and audio seek with
 * HTTP Range requests; the file is never read whole.
 *
 * The host serves only files matching `assets.scope` in `app.start()` (and not
 * denied by `security.fs.deny`); everything else answers 403. `path` must be
 * absolute.
 * @example
 * video.src = convertFileSrc("/home/me/Videos/clip.mp4")
 */
export function convertFileSrc(path: string): string {
  if (typeof path !== "string" || path.length === 0) {
    throw new TypeError("convertFileSrc(): path must be a non-empty absolute path");
  }
  return `${schemeOrigin("asset")}/${encodeURIComponent(path)}`;
}

/**
 * URL of `path` on a scheme the backend serves with `app.protocol(scheme, …)`.
 * `path` is used as written (encode dynamic segments yourself) and may carry
 * a query string.
 * @example
 * img.src = protocolUrl("thumbs", `/${encodeURIComponent(id)}?size=128`)
 */
export function protocolUrl(scheme: string, path = "/"): string {
  const origin = schemeOrigin(scheme);
  return `${origin}${path.startsWith("/") ? path : `/${path}`}`;
}
