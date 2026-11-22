import { currentWebviewLabel, currentWindowLabel, osCall } from "./_internal";

/** A cookie as seen by the webview's cookie store. */
export interface WebviewCookie {
  name: string;
  value: string;
  /** `null` when the platform does not report a domain. */
  domain: string | null;
  path: string | null;
  secure: boolean;
  httpOnly: boolean;
  sameSite: "strict" | "lax" | "none" | null;
  /** Expiry as unix seconds; `null` for a session cookie. */
  expires: number | null;
}

/** Cookie accepted by `tyndWebview.cookies.set`. */
export interface WebviewCookieInit {
  name: string;
  value: string;
  /** Required unless `url` is passed to `set`; then the URL host is used. */
  domain?: string;
  /** Defaults to `/`. */
  path?: string;
  secure?: boolean;
  httpOnly?: boolean;
  sameSite?: "strict" | "lax" | "none";
  /** Expiry as unix seconds. Omit for a session cookie. */
  expires?: number;
}

/** Target of a cookie call. */
export interface WebviewCookieTarget {
  /** Webview whose cookie store to use. Defaults to the caller's webview. */
  label?: string;
  /** Restrict to cookies that apply to this `http(s)` URL. */
  url?: string;
}

/** Payload of `webview:download-started`. */
export interface WebviewDownloadStarted {
  type: "started";
  /** Webview that started the download. */
  label: string;
  /** Matches the id of the later completion event. */
  id: number;
  url: string;
  /** Absolute destination inside the OS downloads directory. */
  path: string;
}

/** Payload of `webview:download-completed`. */
export interface WebviewDownloadCompleted {
  type: "completed";
  label: string;
  id: number;
  url: string;
  /** Final file path; `null` on failure, and on macOS where WKWebView does not report it. */
  path: string | null;
  success: boolean;
}

export type WebviewDownloadEvent = WebviewDownloadStarted | WebviewDownloadCompleted;

/** Payload of `webview:new-window`. */
export interface WebviewNewWindowEvent {
  label: string;
  url: string;
  /** Decision taken by the host policy. */
  action: "deny" | "external" | "allow";
}

function cookieArgs(target: WebviewCookieTarget): { label: string; url?: string } {
  const args: { label: string; url?: string } = {
    label: target.label ?? currentWebviewLabel(),
  };
  if (target.url !== undefined) args.url = target.url;
  return args;
}

/**
 * Bindings for the calling webview: identity, cookie store, download and
 * new-window observation. Creating and driving child webviews lives in
 * `@tynd/plugin-multiwindow`.
 */
export const tyndWebview = {
  /** Label of this webview. Differs from `windowLabel()` for a child. */
  label(): string {
    return currentWebviewLabel();
  },
  /** Label of the containing native window. */
  windowLabel(): string {
    return currentWindowLabel();
  },
  /**
   * Cookie store access. Calls are capability-gated as
   * `window:getWebviewCookies`, `window:setWebviewCookie`,
   * `window:deleteWebviewCookie` and `window:clearWebviewCookies`.
   * Not supported on Android.
   */
  cookies: {
    /** Cookies of a webview, optionally only those that apply to `url`. */
    get(target: WebviewCookieTarget = {}): Promise<WebviewCookie[]> {
      return osCall("window", "getWebviewCookies", cookieArgs(target));
    },
    /** Create or replace a cookie. */
    set(cookie: WebviewCookieInit, target: WebviewCookieTarget = {}): Promise<void> {
      return osCall("window", "setWebviewCookie", { ...cookieArgs(target), cookie });
    },
    /** Delete one cookie, identified by name, domain (or `url`) and path. */
    delete(
      cookie: Pick<WebviewCookieInit, "name" | "domain" | "path">,
      target: WebviewCookieTarget = {},
    ): Promise<void> {
      return osCall("window", "deleteWebviewCookie", { ...cookieArgs(target), ...cookie });
    },
    /** Delete every cookie (or those that apply to `url`). Resolves to the count removed. */
    clear(target: WebviewCookieTarget = {}): Promise<number> {
      return osCall("window", "clearWebviewCookies", cookieArgs(target));
    },
  },
  /**
   * Observe downloads. Every download is saved into the OS downloads
   * directory (a numeric suffix avoids overwriting). Receives a `started`
   * event, then a `completed` event with the same `id`.
   */
  onDownload(handler: (event: WebviewDownloadEvent) => void): () => void {
    const offStarted = window.__tynd__.os_on("webview:download-started", (raw) => {
      handler({ type: "started", ...(raw as Omit<WebviewDownloadStarted, "type">) });
    });
    const offCompleted = window.__tynd__.os_on("webview:download-completed", (raw) => {
      handler({ type: "completed", ...(raw as Omit<WebviewDownloadCompleted, "type">) });
    });
    return () => {
      offStarted();
      offCompleted();
    };
  },
  /** Observe `window.open()` / `target="_blank"` requests and the policy decision. */
  onNewWindow(handler: (event: WebviewNewWindowEvent) => void): () => void {
    return window.__tynd__.os_on("webview:new-window", (raw) => {
      handler(raw as WebviewNewWindowEvent);
    });
  },
};
