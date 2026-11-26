import { osCall, osOn } from "@tynd/core/client/bridge";

/**
 * Deep-link OS API.
 * System-wide custom URL scheme handler registration (`myapp://...`)
 * plus a runtime URL listener.
 * # Per-OS notes
 * - **Windows**: writes `HKCU\Software\Classes\<scheme>\…` (per-user
 *   registry, no admin needed).
 * - **macOS**: flips the default URL handler via
 *   `LSSetDefaultHandlerForURLScheme`. The app's `Info.plist` must
 *   declare `CFBundleURLTypes` at *build* time for the OS to know it
 *   accepts the scheme; the Tynd CLI handles that injection from
 *   `tynd.config.ts`'s `bundle.macos.urlSchemes` array.
 * - **Linux**: writes `~/.local/share/applications/<app_id>.desktop`
 *   declaring `MimeType=x-scheme-handler/<scheme>`, then runs
 *   `xdg-mime default …`.
 * # Multi-instance forwarding
 * Every supported OS spawns a *fresh* process when a `myapp://...` URL
 * is opened while the app is already running. The host's
 * `os::deep_link::init()` (called during host startup) acquires a per-app
 * file lock + local socket; a second launch detects the existing master,
 * forwards its URL to it, and exits cleanly so the user only ever sees one
 * window.
 */
export const deepLink = {
  /**
   * Register the app as the OS handler for `<scheme>://…`. Persists
   * across sessions, per-user, no admin/root needed.
   */
  register(scheme: string): Promise<void> {
    return osCall("deepLink", "register", { scheme });
  },
  /** Drop the OS-level association for `<scheme>`. */
  unregister(scheme: string): Promise<void> {
    return osCall("deepLink", "unregister", { scheme });
  },
  /** True when *this* binary is the OS-default handler for `<scheme>`. */
  isRegistered(scheme: string): Promise<boolean> {
    return osCall("deepLink", "isRegistered", { scheme });
  },
  /**
   * The URL the OS passed on launch (`argv[1..]`), or null. Drained on
   * first read — subsequent calls return null. Use [`onUrl`] to
   * subscribe to runtime URL events.
   */
  getStartupUrl(): Promise<string | null> {
    return osCall("deepLink", "getStartupUrl");
  },
  /**
   * `true` when this process was an OS-spawned second launch that
   * already forwarded its startup URL to the running master via the
   * native multi-instance socket. Useful for tooling / diagnostics —
   * the host bootstrap polls the same flag right after `init` and
   * exits early when set, so frontend code rarely needs to consult it.
   */
  wasForwardedToPrimary(): Promise<boolean> {
    return osCall("deepLink", "wasForwardedToPrimary");
  },
  /**
   * Subscribe to incoming deep-link URLs at runtime. Returns a
   * `{ dispose() }` handle for symmetry with the rest of the Tynd
   * subscription APIs.
   * Fires for both the cold-start URL (drained from `argv` and
   * delivered after page-ready) and runtime URLs (forwarded from a
   * second launch via the local socket on Win/Linux, or via the
   * `NSAppleEventManager` on macOS).
   * @example
   *   const sub = deepLink.onUrl((url) => console.log("got", url))
   *   // later…
   *   sub.dispose()
   */
  onUrl(handler: (url: string) => void): { dispose(): void } {
    const off = osOn("deep-link:url", (raw) => {
      const url = (raw as { url?: unknown } | null)?.url;
      if (typeof url === "string") handler(url);
    });
    return {
      dispose() {
        off();
      },
    };
  },
};
