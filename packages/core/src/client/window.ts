import { tynd } from "../logger";
import type { MenuItem } from "../types";
import { currentWebviewLabel, currentWindowLabel, osCall } from "./_internal";
import type { WindowEffectsOptions } from "./window-effects";
import { positionWindowAt } from "./window-position";
import type {
  AlwaysOnTopLevel,
  AttentionKind,
  CloseRequestedEvent,
  CursorIcon,
  DragDropEvent,
  DragOverEvent,
  NavigationEvent,
  PageLoadEvent,
  ResizeDirection,
  WindowEventBase,
  WindowPosition,
  WindowPreset,
  WindowSize,
} from "./window-types";

export type {
  AlwaysOnTopLevel,
  AttentionKind,
  CloseRequestedEvent,
  CursorIcon,
  DownloadPolicy,
  DragDropEvent,
  DragOverEvent,
  NavigationEvent,
  NewWindowPolicy,
  PageLoadEvent,
  ResizeDirection,
  WindowEventBase,
  WindowOptions,
  WindowPosition,
  WindowPreset,
  WindowSize,
} from "./window-types";

declare global {
  interface Window {
    /**
     * Chromium/WebKit-specific but supported in WebView2 and WKWebView.
     * Returns `true` when a match was found and scrolled into view.
     */
    find?: (
      query: string,
      caseSensitive?: boolean,
      backwards?: boolean,
      wrap?: boolean,
      wholeWord?: boolean,
      searchInFrames?: boolean,
      showDialog?: boolean,
    ) => boolean;
  }
}

/** Sends one `window.<method>` command to the host. */
export type WindowTransport = <T>(method: string, args?: Record<string, unknown>) => Promise<T>;

const rendererTransport: WindowTransport = (method, args) => osCall("window", method, args);

/**
 * Subscribes to one host OS event (`window:resized`…) and returns the
 * unsubscribe function. Renderers use the webview bridge; the Bun backend
 * passes its own host-event source.
 */
export type WindowEventSource = (name: string, handler: (data: unknown) => void) => () => void;

const rendererEvents: WindowEventSource = (name, handler) => {
  if (typeof window === "undefined" || !window.__tynd__) {
    throw new Error("window events need the tynd bridge: subscribe from a renderer or the backend");
  }
  return window.__tynd__.os_on(name, handler);
};

/**
 * Subscribe to a `window:<event>`. OS events are broadcast to every webview;
 * the helper filters by the handle's window label so handlers only fire for
 * their own window.
 */
function onWindow<T extends WindowEventBase | undefined>(
  events: WindowEventSource,
  self: string,
  event: string,
  handler: (data: T) => void,
): () => void {
  return events(`window:${event}`, (raw) => {
    const data = raw as T;
    // Strict match: events without a label (shouldn't happen, but could if
    // the host protocol ever changes) are treated as NOT for this window
    // rather than broadcast to every subscriber.
    const label = (data as WindowEventBase | undefined)?.label;
    if (label !== self) return;
    handler(data);
  });
}

/** Subscribe to a `webview:<event>`, filtered to one webview label. */
function onWebview<T extends { label: string }>(
  events: WindowEventSource,
  self: string,
  event: string,
  handler: (data: T) => void,
): () => void {
  return events(`webview:${event}`, (raw) => {
    const data = raw as T;
    if (data.label !== self) return;
    handler(data);
  });
}

/** Options for {@link TyndWindow.startDrag}. */
export interface StartDragOptions {
  /** Absolute paths of the files to drag; each must pass the `fs` policy. */
  files: string[];
  /** Drag image: a PNG path (checked against the `fs` policy) or PNG bytes. */
  icon?: string | Uint8Array;
}

interface HandleContext {
  call: WindowTransport;
  transport: WindowTransport;
  windowLabel: () => string;
  webviewLabel: () => string;
  currentPageOnly: (method: string) => void;
  events: WindowEventSource;
}

/** Title, size and position. */
function geometryMethods({ call }: HandleContext) {
  return {
    setTitle(title: string): Promise<void> {
      return call("setTitle", { title });
    },
    getTitle(): Promise<string> {
      return call("getTitle");
    },
    setSize(width: number, height: number): Promise<void> {
      return call("setSize", { width, height });
    },
    /** Inner size (client area) in logical pixels. */
    getSize(): Promise<WindowSize> {
      return call("getSize");
    },
    /** Outer size including title bar + borders. */
    getOuterSize(): Promise<WindowSize> {
      return call("getOuterSize");
    },
    setPosition(x: number, y: number): Promise<void> {
      return call("setPosition", { x, y });
    },
    /** Outer position (top-left of the frame) in logical pixels. */
    getPosition(): Promise<WindowPosition> {
      return call("getPosition");
    },
    /** Position of the client area (below the title bar) in logical pixels. */
    getInnerPosition(): Promise<WindowPosition> {
      return call("getInnerPosition");
    },
    setMinSize(width: number, height: number): Promise<void> {
      return call("setMinSize", { width, height });
    },
    setMaxSize(width: number, height: number): Promise<void> {
      return call("setMaxSize", { width, height });
    },
  };
}

/** Resizable / closable / movable flags and simple getters. */
function capabilityMethods({ call }: HandleContext) {
  return {
    setResizable(resizable: boolean): Promise<void> {
      return call("setResizable", { resizable });
    },
    setClosable(closable: boolean): Promise<void> {
      return call("setClosable", { closable });
    },
    setMaximizable(maximizable: boolean): Promise<void> {
      return call("setMaximizable", { maximizable });
    },
    setMinimizable(minimizable: boolean): Promise<void> {
      return call("setMinimizable", { minimizable });
    },
    toggleMaximize(): Promise<void> {
      return call("toggleMaximize");
    },
    isResizable(): Promise<boolean> {
      return call("isResizable");
    },
    isClosable(): Promise<boolean> {
      return call("isClosable");
    },
    isMinimizable(): Promise<boolean> {
      return call("isMinimizable");
    },
    isMaximizable(): Promise<boolean> {
      return call("isMaximizable");
    },
    /**
     * Lock or unlock moving the window (title bar drag, `data-tynd-drag`,
     * `startDragging`). Windows and macOS; rejects on Linux.
     */
    setMovable(movable: boolean): Promise<void> {
      return call("setMovable", { movable });
    },
    isFocused(): Promise<boolean> {
      return call("isFocused");
    },
    isDecorated(): Promise<boolean> {
      return call("isDecorated");
    },
    scaleFactor(): Promise<number> {
      return call("scaleFactor");
    },
  };
}

/** Minimize, maximize, visibility, fullscreen and kiosk. */
function stateMethods({ call }: HandleContext) {
  return {
    minimize(): Promise<void> {
      return call("minimize");
    },
    unminimize(): Promise<void> {
      return call("unminimize");
    },
    maximize(): Promise<void> {
      return call("maximize");
    },
    unmaximize(): Promise<void> {
      return call("unmaximize");
    },
    center(): Promise<void> {
      return call("center");
    },
    /** Show the window. With `window: { visible: false }`, call it once the page is ready. */
    show(): Promise<void> {
      return call("show");
    },
    hide(): Promise<void> {
      return call("hide");
    },
    setFullscreen(fullscreen: boolean): Promise<void> {
      return call("setFullscreen", { fullscreen });
    },
    /**
     * Fullscreen without a new Space or the macOS fullscreen animation (the
     * menu bar and Dock auto-hide). Windows and Linux use borderless
     * fullscreen, which never switches workspaces.
     */
    setSimpleFullscreen(fullscreen: boolean): Promise<void> {
      return call("setSimpleFullscreen", { fullscreen });
    },
    /**
     * Kiosk mode: borderless fullscreen pinned above every other window.
     * Leaving it restores the previous always-on-top state.
     */
    setKiosk(enabled: boolean): Promise<void> {
      return call("setKiosk", { enabled });
    },
    isKiosk(): Promise<boolean> {
      return call("isKiosk");
    },
  };
}

/** Z-order, opacity, aspect ratio and state getters. */
function stackingMethods({ call }: HandleContext) {
  return {
    /**
     * Pin the window above other windows. `level` picks the macOS band
     * (default `"floating"`; `"screen-saver"` stays above fullscreen apps);
     * Windows and Linux have a single topmost band.
     */
    setAlwaysOnTop(always: boolean, options?: { level?: AlwaysOnTopLevel }): Promise<void> {
      return call(
        "setAlwaysOnTop",
        options?.level === undefined ? { always } : { always, level: options.level },
      );
    },
    isAlwaysOnTop(): Promise<boolean> {
      return call("isAlwaysOnTop");
    },
    /** Keep the window below every other window (desktop widgets). */
    setAlwaysOnBottom(bottom: boolean): Promise<void> {
      return call("setAlwaysOnBottom", { bottom });
    },
    /**
     * Window opacity from `0` (invisible) to `1` (opaque), including the page.
     * Linux needs a compositing window manager.
     */
    setOpacity(opacity: number): Promise<void> {
      return call("setOpacity", { opacity });
    },
    /**
     * Keep the client area at `width / height` while the user resizes
     * (e.g. `16 / 9`). `null` removes the constraint. Windows and macOS;
     * rejects on Linux.
     */
    setAspectRatio(ratio: number | null): Promise<void> {
      return call("setAspectRatio", { ratio });
    },
    setDecorations(decorations: boolean): Promise<void> {
      return call("setDecorations", { decorations });
    },
    isMaximized(): Promise<boolean> {
      return call("isMaximized");
    },
    isMinimized(): Promise<boolean> {
      return call("isMinimized");
    },
    isFullscreen(): Promise<boolean> {
      return call("isFullscreen");
    },
    isVisible(): Promise<boolean> {
      return call("isVisible");
    },
  };
}

/** Close cancellation, focus and attention. */
function focusMethods({ call }: HandleContext) {
  return {
    /**
     * Cancel a pending close. Has effect only during the 500ms window after a
     * `window:close-requested` event fires — after that the app proceeds to exit.
     */
    cancelClose(): Promise<void> {
      return call("cancelClose");
    },

    /** Bring this window to the foreground (keyboard focus). */
    setFocus(): Promise<void> {
      return call("setFocus");
    },
    /**
     * Flash the taskbar button (Windows) / bounce the Dock icon (macOS) /
     * mark the window urgent (Linux). `"informational"` (default) flashes
     * briefly; `"critical"` keeps going until the window gets focus.
     */
    requestAttention(kind: AttentionKind = "informational"): Promise<void> {
      return call("requestAttention", { kind });
    },
    /** Stop a pending `requestAttention`. */
    cancelAttention(): Promise<void> {
      return call("cancelAttention");
    },
  };
}

/** Operations on the window's webview. */
function webviewMethods({ call }: HandleContext) {
  return {
    /** Reload the webview's current URL. */
    reload(): Promise<void> {
      return call("reload");
    },
    /** Set the webview's zoom level (`1.0` = 100 %). */
    setZoom(level: number): Promise<void> {
      return call("setZoom", { level });
    },
    /** Last zoom passed to `setZoom` (defaults to `1.0`). wry has no getter. */
    getZoom(): Promise<number> {
      return call("getZoom");
    },
    /** Open the webview devtools. Only available in debug builds. */
    openDevTools(): Promise<void> {
      return call("openDevTools");
    },
    /** Close the webview devtools (no-op if already closed or in release build). */
    closeDevTools(): Promise<void> {
      return call("closeDevTools");
    },

    /** Open the OS print dialog for the current webview contents. */
    print(): Promise<void> {
      return call("print");
    },
    /** Replace the webview URL at runtime. */
    navigate(url: string): Promise<void> {
      return call("navigate", { url });
    },
    /** Replace the webview contents with raw HTML. */
    loadHtml(html: string): Promise<void> {
      return call("loadHtml", { html });
    },
    /** Current URL of the webview. */
    getUrl(): Promise<string> {
      return call("getUrl");
    },
  };
}

/** Cursor and native drag / resize. */
function pointerMethods({ call }: HandleContext) {
  return {
    /** Set the cursor icon over this window. */
    setCursorIcon(icon: CursorIcon): Promise<void> {
      return call("setCursorIcon", { icon });
    },
    /** Warp the cursor to the given window-relative logical position. */
    setCursorPosition(x: number, y: number): Promise<void> {
      return call("setCursorPosition", { x, y });
    },
    setCursorVisible(visible: boolean): Promise<void> {
      return call("setCursorVisible", { visible });
    },
    /** Confine the cursor to the window (games, drawing tools). */
    setCursorGrab(grab: boolean): Promise<void> {
      return call("setCursorGrab", { grab });
    },
    /** Click-through: when `true`, mouse events pass through to windows behind. */
    setIgnoreCursorEvents(ignore: boolean): Promise<void> {
      return call("setIgnoreCursorEvents", { ignore });
    },
    /**
     * Start a native window drag. Call from a mousedown handler; a
     * `data-tynd-drag` attribute does this without any script.
     */
    startDragging(): Promise<void> {
      return call("startDragging");
    },
    /** Start a native resize drag from the given edge / corner. */
    startResizeDragging(direction: ResizeDirection = "southEast"): Promise<void> {
      return call("startResizeDragging", { direction });
    },
  };
}

/** Placement presets, theme, background and progress. */
function appearanceMethods({ call }: HandleContext) {
  return {
    /**
     * Move + resize to one of 13 presets, anchored on the current monitor.
     * Matches Tauri's WindowPositioner — useful for HUDs, quick pickers, or
     * panels that want consistent placement on every display.
     * All coordinates are in LOGICAL pixels (CSS pixels, divided by
     * devicePixelRatio). The host's `Monitor.size` / `Monitor.position` are
     * in physical pixels and we divide by `Monitor.scale` here.
     * The four `tray*` presets are *approximations* of the OS tray icon
     * area — there is no cross-OS API that reports the real tray bounds, so
     * we anchor to the nearest monitor corner with a heuristic taskbar
     * inset. Apps that need exact tray-icon placement should track the
     * cursor inside their own `tray:click` handler.
     */
    async positionAt(preset: WindowPreset, padding = 20): Promise<void> {
      return positionWindowAt(call, preset, padding);
    },

    /** Override the window theme (`system` follows the OS). */
    setTheme(theme: "light" | "dark" | "system"): Promise<void> {
      return call("setTheme", { theme });
    },
    /** Set the native window background color (shows through transparent HTML). */
    setBackgroundColor(
      color: { r: number; g: number; b: number; a?: number } | null,
    ): Promise<void> {
      return call("setBackgroundColor", { color });
    },
    /** Block screenshots + screen recording of this window (Win/macOS). */
    setContentProtection(enabled: boolean): Promise<void> {
      return call("setContentProtection", { enabled });
    },
    /**
     * Taskbar / dock progress indicator. `progress` is 0-100. State: `"normal"`,
     * `"indeterminate"`, `"paused"`, `"error"`, `"none"` (default).
     */
    setProgressBar(
      state: "none" | "normal" | "indeterminate" | "paused" | "error",
      progress?: number,
    ): Promise<void> {
      return call("setProgressBar", { state, progress });
    },
  };
}

/** Taskbar, badge, focusability and workspaces. */
function taskbarMethods({ call }: HandleContext) {
  return {
    /** Hide the window from the taskbar (Windows/Linux). No-op on macOS. */
    setSkipTaskbar(skip: boolean): Promise<void> {
      return call("setSkipTaskbar", { skip });
    },
    /**
     * Dock / taskbar / launcher badge. macOS shows `label` (or `count` as text
     * when no label is given); Windows draws `label` (at most 3 characters) or
     * `count` (`99+` above 99) as a taskbar overlay icon; Linux (Unity) shows
     * `count` and rejects a label-only call. Call with `{}` to clear.
     */
    setBadge(options: { label?: string; count?: number }): Promise<void> {
      return call("setBadge", options);
    },
    /** Whether the window can receive keyboard focus. */
    setFocusable(focusable: boolean): Promise<void> {
      return call("setFocusable", { focusable });
    },
    /** Windows only — disable mouse + keyboard input. Rejects on other OS. */
    setEnabled(enabled: boolean): Promise<void> {
      return call("setEnabled", { enabled });
    },
    /** macOS: show this window on every Space. Other OS: no-op. */
    setVisibleOnAllWorkspaces(visible: boolean): Promise<void> {
      return call("setVisibleOnAllWorkspaces", { visible });
    },
  };
}

/** Shadow, icon and macOS title bar. */
function chromeMethods({ call }: HandleContext) {
  return {
    /**
     * Toggle the native drop shadow: macOS for every window, Windows for
     * frameless windows (decorated ones always keep theirs). No-op on Linux.
     */
    setShadow(enabled: boolean): Promise<void> {
      return call("setShadow", { enabled });
    },
    /** Replace the window icon at runtime. Pass `null` to clear. */
    setWindowIcon(path: string | null): Promise<void> {
      return call("setWindowIcon", { path });
    },
    /** macOS: make the titlebar transparent. No-op on Windows / Linux. */
    setTitlebarTransparent(transparent: boolean): Promise<void> {
      return call("setTitlebarTransparent", { transparent });
    },
    /** macOS: let HTML extend under the titlebar. No-op on Windows / Linux. */
    setFullsizeContentView(fullsize: boolean): Promise<void> {
      return call("setFullsizeContentView", { fullsize });
    },
    /** macOS: reposition the traffic light buttons. No-op on Windows / Linux. */
    setTrafficLightInset(x: number, y: number): Promise<void> {
      return call("setTrafficLightInset", { x, y });
    },
    /**
     * Native window material. `effects` lists materials in order of
     * preference and the first one the running OS supports is applied:
     * `mica` / `tabbed` (Windows 11), `acrylic` / `blur` (Windows 10+),
     * `sidebar`, `hudWindow`, `popover`, `underWindowBackground`… (macOS).
     * `{ effects: [] }` clears it. Rejects when no listed effect runs on the
     * current OS (always on Linux, unless the list is empty). The material
     * only shows through a transparent window (`transparent: true`) whose page
     * background is transparent.
     */
    setEffects(options: WindowEffectsOptions): Promise<void> {
      return call("setEffects", { ...options });
    },
  };
}

/** In-page search (current page only). */
function findMethods({ currentPageOnly }: HandleContext) {
  return {
    /**
     * Cmd+F-style in-page search. Uses the built-in `window.find()` present
     * in Chromium-based WebView2 and WKWebView. `true` = match found and the
     * viewport was scrolled to the hit.
     */
    findInPage(
      query: string,
      options?: { backwards?: boolean; caseSensitive?: boolean; wholeWord?: boolean },
    ): boolean {
      currentPageOnly("findInPage");
      if (typeof window === "undefined" || typeof window.find !== "function") return false;
      return window.find(
        query,
        options?.caseSensitive ?? false,
        options?.backwards ?? false,
        true,
        options?.wholeWord ?? false,
        true,
        false,
      );
    },
    /** Clear the active find highlight / selection. */
    stopFindInPage(): void {
      currentPageOnly("stopFindInPage");
      if (typeof window === "undefined") return;
      window.getSelection()?.removeAllRanges();
    },
  };
}

/** Close, per-window menu bar, file drag-out and label. */
function lifecycleMethods({ call, transport, windowLabel }: HandleContext) {
  return {
    /**
     * Close the window. Secondary windows only (needs `@tynd/plugin-multiwindow`);
     * the primary window rejects — use `app.exit()`.
     */
    close(): Promise<void> {
      return transport("close", { label: windowLabel() });
    },
    /**
     * Give the window its own menu bar (Windows / Linux), or remove it with
     * `null`. On the primary window this replaces the application bar, like
     * `menu.setMenu`. Top-level entries are submenus; clicks emit
     * `menu:action { id }`. macOS has one app-wide bar: secondary windows reject.
     */
    setMenu(items: MenuItem[] | null): Promise<void> {
      return call("setMenu", { items });
    },
    /**
     * Drag files out of the window into another app (file manager, mail
     * client…). Call it from a `mousedown` / `dragstart` handler while the
     * button is held. Resolves once the drag has started (on Windows, once it
     * ends).
     */
    startDrag(options: StartDragOptions): Promise<void> {
      const { files, icon } = options;
      if (icon === undefined) return call("startDrag", { files });
      return call(
        "startDrag",
        typeof icon === "string"
          ? { files, iconPath: icon }
          : { files, iconBytes: Array.from(icon) },
      );
    },

    /** Label of the current window (`"main"` for the primary window). */
    label(): string {
      return windowLabel();
    },
  };
}

/** Window state events. */
function windowEventMethods({ windowLabel, events }: HandleContext) {
  return {
    onResized(handler: (e: WindowSize & WindowEventBase) => void): () => void {
      return onWindow(events, windowLabel(), "resized", handler);
    },
    onMoved(handler: (e: WindowPosition & WindowEventBase) => void): () => void {
      return onWindow(events, windowLabel(), "moved", handler);
    },
    onFocused(handler: () => void): () => void {
      return onWindow(events, windowLabel(), "focused", handler as (e: WindowEventBase) => void);
    },
    onBlurred(handler: () => void): () => void {
      return onWindow(events, windowLabel(), "blurred", handler as (e: WindowEventBase) => void);
    },
    onThemeChanged(
      handler: (e: { theme: "light" | "dark" } & WindowEventBase) => void,
    ): () => void {
      return onWindow(events, windowLabel(), "theme-changed", handler);
    },
    onDpiChanged(handler: (e: { scale: number } & WindowEventBase) => void): () => void {
      return onWindow(events, windowLabel(), "dpi-changed", handler);
    },
    onMinimized(handler: () => void): () => void {
      return onWindow(events, windowLabel(), "minimized", handler as (e: WindowEventBase) => void);
    },
    onUnminimized(handler: () => void): () => void {
      return onWindow(
        events,
        windowLabel(),
        "unminimized",
        handler as (e: WindowEventBase) => void,
      );
    },
    onMaximized(handler: () => void): () => void {
      return onWindow(events, windowLabel(), "maximized", handler as (e: WindowEventBase) => void);
    },
    onUnmaximized(handler: () => void): () => void {
      return onWindow(
        events,
        windowLabel(),
        "unmaximized",
        handler as (e: WindowEventBase) => void,
      );
    },
    onFullscreen(handler: () => void): () => void {
      return onWindow(events, windowLabel(), "fullscreen", handler as (e: WindowEventBase) => void);
    },
    onUnfullscreen(handler: () => void): () => void {
      return onWindow(
        events,
        windowLabel(),
        "unfullscreen",
        handler as (e: WindowEventBase) => void,
      );
    },
    /**
     * Fires when a secondary window closes. Only receives events for other
     * windows (the closed window can't listen for its own destruction).
     */
    onClosed(handler: (e: WindowEventBase) => void): () => void {
      return events("window:closed", (raw) => handler(raw as WindowEventBase));
    },
  };
}

/** Drag & drop and webview events. */
function dragEventMethods({ windowLabel, webviewLabel, events }: HandleContext) {
  return {
    /** Files dragged over the window. `paths` are native OS paths. */
    onDragEnter(handler: (e: DragDropEvent) => void): () => void {
      return onWindow(events, windowLabel(), "drag-enter", handler);
    },
    /** Drag cursor moving over the window. Fires frequently — throttle if needed. */
    onDragOver(handler: (e: DragOverEvent) => void): () => void {
      return onWindow(events, windowLabel(), "drag-over", handler);
    },
    /** Drag cancelled or left the window. */
    onDragLeave(handler: () => void): () => void {
      return onWindow(events, windowLabel(), "drag-leave", handler as (e: WindowEventBase) => void);
    },

    /**
     * Fires before each navigation. Emits `allowed: false` when the active
     * `security` policy blocked the URL. Observation-only — to block a URL,
     * configure `app.start({ security: { http: { deny: [...] } } })` in the
     * trusted backend instead.
     */
    onNavigation(handler: (e: NavigationEvent) => void): () => void {
      return onWebview(events, webviewLabel(), "navigation", handler);
    },
    /** Fires on document start + finish load. */
    onPageLoad(handler: (e: PageLoadEvent) => void): () => void {
      return onWebview(events, webviewLabel(), "page-load", handler);
    },
    /** Files dropped onto the window. `paths` are native OS paths. */
    onDrop(handler: (e: DragDropEvent) => void): () => void {
      return onWindow(events, windowLabel(), "drop", handler);
    },
  };
}

/** Preventable close. */
function closeEventMethods({ call, windowLabel, events }: HandleContext) {
  return {
    /**
     * Fires when the user tries to close the window (X button, Alt+F4, Cmd+Q…),
     * for the main window and every secondary window, in renderers and in the
     * backend (`getWindow(label).onCloseRequested`). The close proceeds after
     * 500ms unless a handler calls `event.preventDefault()` synchronously.
     * Filtered to this handle's window label.
     */
    onCloseRequested(handler: (event: CloseRequestedEvent) => void): () => void {
      const self = windowLabel();
      return events("window:close-requested", (raw) => {
        // Strict match — unlabeled events (protocol drift) don't leak to every
        // subscriber and accidentally cancel closes that weren't theirs.
        const label = (raw as WindowEventBase | undefined)?.label;
        if (label !== self) return;
        let prevented = false;
        handler({
          preventDefault() {
            if (prevented) return;
            prevented = true;
            call("cancelClose").catch((e: unknown) => {
              tynd.warn(`window.cancelClose failed: ${e instanceof Error ? e.message : String(e)}`);
            });
          },
        });
      });
    },
  };
}

/**
 * Build a window handle. `target` is a window label; omit it for the window
 * the code runs in. `transport` defaults to the renderer bridge (the Bun
 * backend passes `hostCall` and its host-event source). Every method and event is the same on every
 * window: `tyndWindow` is the handle of the current window.
 */
export function createWindowHandle(
  target?: string,
  transport: WindowTransport = rendererTransport,
  events: WindowEventSource = rendererEvents,
) {
  const call: WindowTransport = (method, args) =>
    transport(method, target === undefined ? args : { ...args, window: target });
  const windowLabel = () => target ?? currentWindowLabel();
  const webviewLabel = () => target ?? currentWebviewLabel();
  const currentPageOnly = (method: string) => {
    if (target !== undefined && target !== currentWindowLabel()) {
      throw new Error(`window.${method}() only searches the page it runs in`);
    }
  };
  const ctx: HandleContext = {
    call,
    transport,
    windowLabel,
    webviewLabel,
    currentPageOnly,
    events,
  };

  return {
    ...geometryMethods(ctx),
    ...capabilityMethods(ctx),
    ...stateMethods(ctx),
    ...stackingMethods(ctx),
    ...focusMethods(ctx),
    ...webviewMethods(ctx),
    ...pointerMethods(ctx),
    ...appearanceMethods(ctx),
    ...taskbarMethods(ctx),
    ...chromeMethods(ctx),
    ...findMethods(ctx),
    ...lifecycleMethods(ctx),
    ...windowEventMethods(ctx),
    ...dragEventMethods(ctx),
    ...closeEventMethods(ctx),
  };
}

/** A native window: the same methods and events for the main window and every secondary window. */
export type TyndWindow = ReturnType<typeof createWindowHandle>;

/** Frontend bindings for the host's `window` API, bound to the window the code runs in. */
export const tyndWindow: TyndWindow = createWindowHandle();
