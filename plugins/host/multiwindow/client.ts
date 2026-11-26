import {
  type DownloadPolicy,
  type EmitterMap,
  type NewWindowPolicy,
  type TyndWindow,
  tyndWindow,
  type WindowOptions,
} from "@tynd/core/client";
import {
  createWindowHandle,
  currentWebviewLabel,
  currentWindowLabel,
  osCall,
  osOn,
} from "@tynd/core/client/bridge";

export type { DownloadPolicy, NewWindowPolicy, TyndWindow, WindowOptions };

/**
 * Options for `multiwindow.create`: every `app.start({ window })` option
 * (size, position, visibility, frame, theme, kiosk, …) plus the
 * secondary-window ones below.
 */
export interface CreateWindowOptions extends WindowOptions {
  /** Unique label: letter first, then up to 63 letters/digits/`_`/`-`. */
  label: string;
  /** Override URL (e.g. "/settings"). Defaults to the primary window's entry. */
  url?: string;
  /**
   * Persistent, app-scoped data partition for this webview. Names are
   * case-insensitive, limited to 64 safe ASCII characters, and never treated
   * as filesystem paths. One open window may use a partition at a time.
   *
   * Mutually exclusive with `incognito`.
   */
  dataPartition?: string;
  /**
   * Attach this window to a parent (by label) as a modal / owned child.
   * Uses native ownership on Windows/macOS and GTK transient-for on Linux.
   */
  modalTo?: string;
  /** Hide the window automatically when it loses focus (popover pattern). */
  hideOnFocusLost?: boolean;
  /** Hide the window automatically when the user presses Escape. */
  hideOnEscape?: boolean;
}

/** Sender of a window message. */
export interface WindowMessageMeta {
  /** Label of the webview that sent the message. */
  from: string;
}

/** Typed window-to-window event bus, from `multiwindow.events<T>()`. */
export interface WindowEvents<T extends EmitterMap> {
  /** Send to every app-local window and webview, the sender included. */
  emit<K extends keyof T & string>(event: K, payload: T[K]): Promise<void>;
  /** Send to one window or webview by label. Rejects when it does not exist. */
  emitTo<K extends keyof T & string>(label: string, event: K, payload: T[K]): Promise<void>;
  /** Receive `event` from any window. Returns `unsubscribe()`. */
  listen<K extends keyof T & string>(
    event: K,
    handler: (payload: T[K], meta: WindowMessageMeta) => void,
  ): () => void;
}

interface WireWindowMessage {
  event: string;
  payload: unknown;
  from: string;
}

/** Logical-pixel rectangle inside a native parent window. */
export interface WebviewBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Options for `multiwindow.webview.create`. */
export interface CreateWebviewOptions extends WebviewBounds {
  /** Globally unique label: letter first, then up to 63 letters/digits/`_`/`-`. */
  label: string;
  /** Parent native-window label. Defaults to the caller's window. */
  parent?: string;
  /** Override URL. Defaults to the application's frontend entry. */
  url?: string;
  visible?: boolean;
  focused?: boolean;
  /** Fill the parent content area and track future parent resizes. */
  autoResize?: boolean;
  userAgent?: string;
  /** Non-persistent browsing data, discarded when the webview closes. */
  incognito?: boolean;
  /**
   * Persistent app-scoped browsing-data partition.
   * Mutually exclusive with `incognito`; one open webview may own it.
   */
  dataPartition?: string;
  /** Handling of `window.open()` / `target="_blank"`. Defaults to `"allow"`. */
  newWindow?: NewWindowPolicy;
  /**
   * Whether this webview may download files into the OS downloads folder.
   * Unset defaults to `"allow"` for app-local content and `"deny"` for remote
   * content.
   */
  downloads?: DownloadPolicy;
  /**
   * Route this webview through a proxy: `http://host:port` or
   * `socks5://host:port`. Credentials are not supported.
   */
  proxy?: string;
}

export interface WebviewInfo {
  label: string;
  /** `null` for the one webview hosted by a top-level native window. */
  parent: string | null;
  kind: "window" | "child";
  bounds?: WebviewBounds;
  visible?: boolean;
  autoResize?: boolean;
  dataPartition?: string | null;
}

export interface WebviewLifecycleEvent {
  label: string;
  parent: string;
}

/**
 * Secondary windows, window handles, window-to-window messaging and child
 * webviews (host `multiwindow` API).
 *
 * A window handle (`TyndWindow`) has every `tyndWindow` method and event,
 * bound to its window.
 */
export const multiwindow = {
  /**
   * Open a secondary window and return its handle. Labels must be unique and
   * not `"main"`. Accepts every `app.start({ window })` option.
   */
  async create(options: CreateWindowOptions): Promise<TyndWindow> {
    await osCall("window", "create", options);
    return createWindowHandle(options.label);
  },
  /** Handle on an open window by label, or `null` when none has that label. */
  async getWindow(label: string): Promise<TyndWindow | null> {
    const labels = await multiwindow.all();
    if (!labels.includes(label)) return null;
    return label === currentWindowLabel() ? tyndWindow : createWindowHandle(label);
  },
  /** Handles on every open window, primary first. */
  async getAllWindows(): Promise<TyndWindow[]> {
    const self = currentWindowLabel();
    const labels = await multiwindow.all();
    return labels.map((label) => (label === self ? tyndWindow : createWindowHandle(label)));
  },
  /** Handle on the window this code runs in (same object as `tyndWindow`). */
  getCurrentWindow(): TyndWindow {
    return tyndWindow;
  },
  /** Close a secondary window by label. Closing `"main"` is rejected. */
  close(label: string): Promise<void> {
    return osCall("window", "close", { label });
  },
  /** Close this window. No-op on the primary window (use `app.exit()` instead). */
  closeSelf(): Promise<void> {
    const label = currentWindowLabel();
    if (label === "main") return Promise.resolve();
    return osCall("window", "close", { label });
  },
  /** List labels of all open windows (primary first). */
  all(): Promise<string[]> {
    return osCall("window", "all");
  },

  /**
   * Typed window-to-window messages, routed by the host. Only app-local
   * content sends and receives them (never remote pages).
   *
   * @example
   *   const bus = multiwindow.events<{ "doc:saved": { id: string } }>();
   *   bus.listen("doc:saved", ({ id }, { from }) => refresh(id, from));
   *   await bus.emitTo("main", "doc:saved", { id: "42" });
   */
  events<T extends EmitterMap>(): WindowEvents<T> {
    return {
      emit(event, payload) {
        return osCall("window", "emit", { event, payload });
      },
      emitTo(label, event, payload) {
        return osCall("window", "emitTo", { to: label, event, payload });
      },
      listen(event, handler) {
        return osOn("multiwindow:message", (raw) => {
          const message = raw as WireWindowMessage;
          if (message.event !== event) return;
          handler(message.payload as T[typeof event], { from: message.from });
        });
      },
    };
  },

  /**
   * Child webviews share a native window while retaining an independent IPC,
   * navigation, storage, event-routing, and capability identity.
   */
  webview: {
    create(options: CreateWebviewOptions): Promise<void> {
      return osCall("window", "createWebview", options);
    },
    close(label: string): Promise<void> {
      return osCall("window", "closeWebview", { label });
    },
    closeSelf(): Promise<void> {
      return osCall("window", "closeWebview", { label: currentWebviewLabel() });
    },
    all(): Promise<WebviewInfo[]> {
      return osCall("window", "allWebviews");
    },
    setBounds(label: string, bounds: WebviewBounds): Promise<void> {
      return osCall("window", "setWebviewBounds", { label, ...bounds });
    },
    setVisible(label: string, visible: boolean): Promise<void> {
      return osCall("window", "setWebviewVisible", { label, visible });
    },
    setAutoResize(label: string, enabled: boolean): Promise<void> {
      return osCall("window", "setWebviewAutoResize", { label, enabled });
    },
    focus(label: string): Promise<void> {
      return osCall("window", "focusWebview", { label });
    },
    focusParent(label: string): Promise<void> {
      return osCall("window", "focusParentWebview", { label });
    },
    reload(label: string): Promise<void> {
      return osCall("window", "reloadWebview", { label });
    },
    navigate(label: string, url: string): Promise<void> {
      return osCall("window", "navigateWebview", { label, url });
    },
    getUrl(label: string): Promise<string> {
      return osCall("window", "getWebviewUrl", { label });
    },
    setZoom(label: string, level: number): Promise<void> {
      return osCall("window", "setWebviewZoom", { label, level });
    },
    openDevTools(label: string): Promise<void> {
      return osCall("window", "openWebviewDevTools", { label });
    },
    closeDevTools(label: string): Promise<void> {
      return osCall("window", "closeWebviewDevTools", { label });
    },
    print(label: string): Promise<void> {
      return osCall("window", "printWebview", { label });
    },
    clearBrowsingData(label: string): Promise<void> {
      return osCall("window", "clearWebviewData", { label });
    },
    onCreated(handler: (event: WebviewLifecycleEvent) => void): () => void {
      return osOn("webview:created", (raw) => {
        handler(raw as WebviewLifecycleEvent);
      });
    },
    onClosed(handler: (event: WebviewLifecycleEvent) => void): () => void {
      return osOn("webview:closed", (raw) => {
        handler(raw as WebviewLifecycleEvent);
      });
    },
  },
};
