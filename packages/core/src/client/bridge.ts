/**
 * `@tynd/core/client/bridge` — the contract every plugin client builds on.
 *
 * App code uses `@tynd/core/client` and the plugin packages. A plugin client
 * (first or third party) uses these functions to reach the host: `osCall` for
 * a request to a host API, `osOn` for its events, the `bin*` helpers for raw
 * bytes over the `tynd-bin://` scheme, and the base64 / label helpers for
 * payloads that carry bytes or target the current window.
 *
 * Every rejection goes through `decodeIpcError`, so a dead backend surfaces as
 * a typed `TyndIpcError` instead of a plain `Error`.
 */

export {
  base64ToBytes,
  binExchangeText,
  binFetch,
  binUpload,
  bytesToBase64,
  currentWebviewLabel,
  currentWindowLabel,
  decodeIpcError,
  type OsCallOptions,
  osCall,
  osDeadline,
  osOn,
} from "./_internal";
/** Window handles for plugins that target other windows (`@tynd/plugin-multiwindow`). */
export { createWindowHandle, type WindowTransport } from "./window";
