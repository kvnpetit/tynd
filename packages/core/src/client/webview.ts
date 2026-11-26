import { base64ToBytes, currentWebviewLabel, currentWindowLabel, osCall } from "./_internal";

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

/** Webview targeted by a session call. */
export interface WebviewTarget {
  /**
   * Webview to act on. Defaults to the caller's webview. Naming another
   * webview needs a caller that shows app content.
   */
  label?: string;
}

/** Named paper sizes accepted by `printToPdf`. */
export type WebviewPageSize = "A3" | "A4" | "A5" | "Letter" | "Legal" | "Tabloid";

/** Options of `tyndWebview.printToPdf`. Lengths are inches. */
export interface WebviewPdfOptions extends WebviewTarget {
  /** Defaults to `false` (portrait). */
  landscape?: boolean;
  /** Print CSS backgrounds. Defaults to `false`. */
  printBackground?: boolean;
  /** `0.1`..`2`, defaults to `1`. */
  scale?: number;
  /** A named size or `{ width, height }` (1..100 in). Defaults to Letter. */
  pageSize?: WebviewPageSize | { width: number; height: number };
  /** One value for every side, or per side (0..10 in, unset sides = 0). */
  margins?: number | { top?: number; right?: number; bottom?: number; left?: number };
}

/**
 * Options of `tyndWebview.clearData`. With no category selected (or
 * `all: true`) every kind of data the engine stores is removed, history and
 * autofill included.
 */
export interface WebviewClearDataOptions extends WebviewTarget {
  cookies?: boolean;
  /** HTTP disk/memory cache and Cache API storage. */
  cache?: boolean;
  localStorage?: boolean;
  indexedDb?: boolean;
  serviceWorkers?: boolean;
  /** Remove everything; overrides the categories. */
  all?: boolean;
}

/** Options of `tyndWebview.print`. */
export interface WebviewPrintOptions extends WebviewTarget {
  /**
   * Print without the OS dialog (Windows, Linux). Only app content may print
   * silently. Defaults to `false`.
   */
  silent?: boolean;
  /** Printer of a silent print (see `getPrinters`). Defaults to the OS default printer. */
  printer?: string;
}

/** An installed printer, as reported by `tyndWebview.getPrinters`. */
export interface WebviewPrinter {
  name: string;
  isDefault: boolean;
}

function targetArgs(target: WebviewTarget): { label?: string } {
  return target.label === undefined ? {} : { label: target.label };
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
  /**
   * Evaluate a JavaScript expression in a webview and resolve to its
   * JSON-serializable result (`undefined` becomes `null`; a returned Promise
   * is not awaited). Wrap statements in an IIFE. A thrown error rejects.
   * Always needs an explicit `window:evaluateScript` capability grant.
   */
  evaluate<T = unknown>(script: string, target: WebviewTarget = {}): Promise<T> {
    return osCall("window", "evaluateScript", { ...targetArgs(target), script });
  },
  /**
   * Delete browsing data of the webview's data store and resolve once the
   * engine is done. Pass categories to clear only those; with none, every
   * cookie, cache entry and storage is removed. Always needs an explicit
   * `window:clearBrowsingData` capability grant.
   */
  clearData(options: WebviewClearDataOptions = {}): Promise<void> {
    const { label, ...kinds } = options;
    return osCall("window", "clearBrowsingData", { ...targetArgs({ label }), ...kinds });
  },
  /**
   * Render the page to PDF and resolve to its bytes. On macOS the whole page
   * becomes one continuous PDF page and the layout options do not apply; on
   * Linux CSS backgrounds are always printed.
   */
  async printToPdf(options: WebviewPdfOptions = {}): Promise<Uint8Array> {
    const { label, ...pdf } = options;
    const data = await osCall<string>("window", "printToPdf", {
      ...targetArgs({ label }),
      options: pdf,
    });
    return base64ToBytes(data);
  },
  /** Capture the visible page as PNG bytes. */
  async capturePage(target: WebviewTarget = {}): Promise<Uint8Array> {
    const data = await osCall<string>("window", "capturePage", targetArgs(target));
    return base64ToBytes(data);
  },
  /** Mute or unmute every sound the page plays. Windows and Linux. */
  setAudioMuted(muted: boolean, target: WebviewTarget = {}): Promise<void> {
    return osCall("window", "setAudioMuted", { ...targetArgs(target), muted });
  },
  /** Whether the page's audio is muted. Windows and Linux. */
  isAudioMuted(target: WebviewTarget = {}): Promise<boolean> {
    return osCall("window", "isAudioMuted", targetArgs(target));
  },
  /**
   * Print the page: the OS print dialog by default, or straight to a
   * printer with `silent: true` (Windows, Linux; resolves once the job is
   * spooled).
   */
  print(options: WebviewPrintOptions = {}): Promise<void> {
    const { label, ...print } = options;
    return osCall("window", "printPage", { ...targetArgs({ label }), ...print });
  },
  /** Installed printers, the default one flagged. Windows only. */
  getPrinters(): Promise<WebviewPrinter[]> {
    return osCall("window", "getPrinters");
  },
  /** Observe `window.open()` / `target="_blank"` requests and the policy decision. */
  onNewWindow(handler: (event: WebviewNewWindowEvent) => void): () => void {
    return window.__tynd__.os_on("webview:new-window", (raw) => {
      handler(raw as WebviewNewWindowEvent);
    });
  },
};
