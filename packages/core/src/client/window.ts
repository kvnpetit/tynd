import { tynd } from "../logger";
import { currentWebviewLabel, currentWindowLabel, osCall } from "./_internal";
import { positionWindowAt } from "./window-position";
import type {
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

/**
 * Subscribe to a `window:<event>`. OS events are broadcast to every webview;
 * the helper filters by the current window's label so handlers only fire for
 * their own window. Pass `{ anyWindow: true }` to receive events from all.
 */
function onWindow<T extends WindowEventBase | undefined>(
  event: string,
  handler: (data: T) => void,
  opts?: { anyWindow?: boolean },
): () => void {
  const self = currentWindowLabel();
  return window.__tynd__.os_on(`window:${event}`, (raw) => {
    const data = raw as T;
    if (!opts?.anyWindow) {
      // Strict match: events without a label (shouldn't happen, but could if
      // the host protocol ever changes) are treated as NOT for this window
      // rather than broadcast to every subscriber.
      const label = (data as WindowEventBase | undefined)?.label;
      if (label !== self) return;
    }
    handler(data);
  });
}

/** Subscribe to a `webview:<event>`, filtered to the current webview's label. */
function onWebview<T extends { label: string }>(
  event: string,
  handler: (data: T) => void,
): () => void {
  return window.__tynd__.os_on(`webview:${event}`, (raw) => {
    const data = raw as T;
    if (data.label !== currentWebviewLabel()) return;
    handler(data);
  });
}

/** Frontend bindings for the host's `window` API (sizing, focus, fullscreen, drag, events). */
export const tyndWindow = {
  setTitle(title: string): Promise<void> {
    return osCall("window", "setTitle", { title });
  },
  getTitle(): Promise<string> {
    return osCall("window", "getTitle");
  },
  setSize(width: number, height: number): Promise<void> {
    return osCall("window", "setSize", { width, height });
  },
  /** Inner size (client area) in logical pixels. */
  getSize(): Promise<WindowSize> {
    return osCall("window", "getSize");
  },
  /** Outer size including title bar + borders. */
  getOuterSize(): Promise<WindowSize> {
    return osCall("window", "getOuterSize");
  },
  setPosition(x: number, y: number): Promise<void> {
    return osCall("window", "setPosition", { x, y });
  },
  getPosition(): Promise<WindowPosition> {
    return osCall("window", "getPosition");
  },
  setMinSize(width: number, height: number): Promise<void> {
    return osCall("window", "setMinSize", { width, height });
  },
  setMaxSize(width: number, height: number): Promise<void> {
    return osCall("window", "setMaxSize", { width, height });
  },
  setResizable(resizable: boolean): Promise<void> {
    return osCall("window", "setResizable", { resizable });
  },
  setClosable(closable: boolean): Promise<void> {
    return osCall("window", "setClosable", { closable });
  },
  setMaximizable(maximizable: boolean): Promise<void> {
    return osCall("window", "setMaximizable", { maximizable });
  },
  setMinimizable(minimizable: boolean): Promise<void> {
    return osCall("window", "setMinimizable", { minimizable });
  },
  toggleMaximize(): Promise<void> {
    return osCall("window", "toggleMaximize");
  },
  isResizable(): Promise<boolean> {
    return osCall("window", "isResizable");
  },
  isClosable(): Promise<boolean> {
    return osCall("window", "isClosable");
  },
  isFocused(): Promise<boolean> {
    return osCall("window", "isFocused");
  },
  isDecorated(): Promise<boolean> {
    return osCall("window", "isDecorated");
  },
  scaleFactor(): Promise<number> {
    return osCall("window", "scaleFactor");
  },
  minimize(): Promise<void> {
    return osCall("window", "minimize");
  },
  unminimize(): Promise<void> {
    return osCall("window", "unminimize");
  },
  maximize(): Promise<void> {
    return osCall("window", "maximize");
  },
  unmaximize(): Promise<void> {
    return osCall("window", "unmaximize");
  },
  center(): Promise<void> {
    return osCall("window", "center");
  },
  show(): Promise<void> {
    return osCall("window", "show");
  },
  hide(): Promise<void> {
    return osCall("window", "hide");
  },
  setFullscreen(fullscreen: boolean): Promise<void> {
    return osCall("window", "setFullscreen", { fullscreen });
  },
  setAlwaysOnTop(always: boolean): Promise<void> {
    return osCall("window", "setAlwaysOnTop", { always });
  },
  setDecorations(decorations: boolean): Promise<void> {
    return osCall("window", "setDecorations", { decorations });
  },
  isMaximized(): Promise<boolean> {
    return osCall("window", "isMaximized");
  },
  isMinimized(): Promise<boolean> {
    return osCall("window", "isMinimized");
  },
  isFullscreen(): Promise<boolean> {
    return osCall("window", "isFullscreen");
  },
  isVisible(): Promise<boolean> {
    return osCall("window", "isVisible");
  },

  /**
   * Cancel a pending close. Has effect only during the 500ms window after a
   * `window:close-requested` event fires — after that the app proceeds to exit.
   */
  cancelClose(): Promise<void> {
    return osCall("window", "cancelClose");
  },

  /** Bring this window to the foreground (keyboard focus). */
  setFocus(): Promise<void> {
    return osCall("window", "setFocus");
  },
  /** Flash the taskbar (Windows) / bounce the Dock (macOS) to get attention. */
  requestAttention(): Promise<void> {
    return osCall("window", "requestAttention");
  },

  /** Reload the webview's current URL. */
  reload(): Promise<void> {
    return osCall("window", "reload");
  },
  /** Set the webview's zoom level (`1.0` = 100 %). */
  setZoom(level: number): Promise<void> {
    return osCall("window", "setZoom", { level });
  },
  /** Last zoom passed to `setZoom` (defaults to `1.0`). wry has no getter. */
  getZoom(): Promise<number> {
    return osCall("window", "getZoom");
  },
  /** Open the webview devtools. Only available in debug builds. */
  openDevTools(): Promise<void> {
    return osCall("window", "openDevTools");
  },
  /** Close the webview devtools (no-op if already closed or in release build). */
  closeDevTools(): Promise<void> {
    return osCall("window", "closeDevTools");
  },

  /** Open the OS print dialog for the current webview contents. */
  print(): Promise<void> {
    return osCall("window", "print");
  },
  /** Replace the webview URL at runtime. */
  navigate(url: string): Promise<void> {
    return osCall("window", "navigate", { url });
  },
  /** Replace the webview contents with raw HTML. */
  loadHtml(html: string): Promise<void> {
    return osCall("window", "loadHtml", { html });
  },
  /** Current URL of the webview. */
  getUrl(): Promise<string> {
    return osCall("window", "getUrl");
  },

  /** Set the cursor icon over this window. */
  setCursorIcon(icon: CursorIcon): Promise<void> {
    return osCall("window", "setCursorIcon", { icon });
  },
  /** Warp the cursor to the given window-relative logical position. */
  setCursorPosition(x: number, y: number): Promise<void> {
    return osCall("window", "setCursorPosition", { x, y });
  },
  setCursorVisible(visible: boolean): Promise<void> {
    return osCall("window", "setCursorVisible", { visible });
  },
  /** Click-through: when `true`, mouse events pass through to windows behind. */
  setIgnoreCursorEvents(ignore: boolean): Promise<void> {
    return osCall("window", "setIgnoreCursorEvents", { ignore });
  },
  /** Start a native window drag. Call from a mousedown handler on a drag zone. */
  startDragging(): Promise<void> {
    return osCall("window", "startDragging");
  },
  /** Start a native resize drag from the given edge / corner. */
  startResizeDragging(direction: ResizeDirection = "southEast"): Promise<void> {
    return osCall("window", "startResizeDragging", { direction });
  },

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
    return positionWindowAt(preset, padding);
  },

  /** Override the window theme (`system` follows the OS). */
  setTheme(theme: "light" | "dark" | "system"): Promise<void> {
    return osCall("window", "setTheme", { theme });
  },
  /** Set the native window background color (shows through transparent HTML). */
  setBackgroundColor(color: { r: number; g: number; b: number; a?: number } | null): Promise<void> {
    return osCall("window", "setBackgroundColor", { color });
  },
  /** Block screenshots + screen recording of this window (Win/macOS). */
  setContentProtection(enabled: boolean): Promise<void> {
    return osCall("window", "setContentProtection", { enabled });
  },
  /**
   * Taskbar / dock progress indicator. `progress` is 0-100. State: `"normal"`,
   * `"indeterminate"`, `"paused"`, `"error"`, `"none"` (default).
   */
  setProgressBar(
    state: "none" | "normal" | "indeterminate" | "paused" | "error",
    progress?: number,
  ): Promise<void> {
    return osCall("window", "setProgressBar", { state, progress });
  },
  /** Hide the window from the taskbar (Windows/Linux). No-op on macOS. */
  setSkipTaskbar(skip: boolean): Promise<void> {
    return osCall("window", "setSkipTaskbar", { skip });
  },
  /**
   * Dock / launcher badge. macOS shows `label` (or `count` as text when no
   * label is given); Linux (Unity) shows `count` and rejects a label-only
   * call. Rejects on Windows. Call with `{}` to clear.
   */
  setBadge(options: { label?: string; count?: number }): Promise<void> {
    return osCall("window", "setBadge", options);
  },
  /** Whether the window can receive keyboard focus. */
  setFocusable(focusable: boolean): Promise<void> {
    return osCall("window", "setFocusable", { focusable });
  },
  /** Windows only — disable mouse + keyboard input. Rejects on other OS. */
  setEnabled(enabled: boolean): Promise<void> {
    return osCall("window", "setEnabled", { enabled });
  },
  /** macOS: show this window on every Space. Other OS: no-op. */
  setVisibleOnAllWorkspaces(visible: boolean): Promise<void> {
    return osCall("window", "setVisibleOnAllWorkspaces", { visible });
  },
  /** macOS: toggle the native drop shadow. Other OS: no-op. */
  setShadow(enabled: boolean): Promise<void> {
    return osCall("window", "setShadow", { enabled });
  },
  /** Replace the window icon at runtime. Pass `null` to clear. */
  setWindowIcon(path: string | null): Promise<void> {
    return osCall("window", "setWindowIcon", { path });
  },
  /** macOS: make the titlebar transparent. No-op on Windows / Linux. */
  setTitlebarTransparent(transparent: boolean): Promise<void> {
    return osCall("window", "setTitlebarTransparent", { transparent });
  },
  /** macOS: let HTML extend under the titlebar. No-op on Windows / Linux. */
  setFullsizeContentView(fullsize: boolean): Promise<void> {
    return osCall("window", "setFullsizeContentView", { fullsize });
  },
  /** macOS: reposition the traffic light buttons. No-op on Windows / Linux. */
  setTrafficLightInset(x: number, y: number): Promise<void> {
    return osCall("window", "setTrafficLightInset", { x, y });
  },
  /**
   * Windows 11+ system backdrop: `"mica"`, `"acrylic"`, `"tabbed"`,
   * `"none"`, or `"auto"`. No-op on other OS. Requires the window to have
   * no decorations for best results (set in config).
   */
  setSystemBackdrop(kind: "auto" | "none" | "mica" | "acrylic" | "tabbed"): Promise<void> {
    return osCall("window", "setSystemBackdrop", { kind });
  },

  /**
   * Cmd+F-style in-page search. Uses the built-in `window.find()` present
   * in Chromium-based WebView2 and WKWebView. `true` = match found and the
   * viewport was scrolled to the hit.
   */
  findInPage(
    query: string,
    options?: { backwards?: boolean; caseSensitive?: boolean; wholeWord?: boolean },
  ): boolean {
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
    if (typeof window === "undefined") return;
    window.getSelection()?.removeAllRanges();
  },

  /** Label of the current window (`"main"` for the primary window). */
  label(): string {
    return currentWindowLabel();
  },

  onResized(handler: (e: WindowSize & WindowEventBase) => void): () => void {
    return onWindow("resized", handler);
  },
  onMoved(handler: (e: WindowPosition & WindowEventBase) => void): () => void {
    return onWindow("moved", handler);
  },
  onFocused(handler: () => void): () => void {
    return onWindow("focused", handler as (e: WindowEventBase) => void);
  },
  onBlurred(handler: () => void): () => void {
    return onWindow("blurred", handler as (e: WindowEventBase) => void);
  },
  onThemeChanged(handler: (e: { theme: "light" | "dark" } & WindowEventBase) => void): () => void {
    return onWindow("theme-changed", handler);
  },
  onDpiChanged(handler: (e: { scale: number } & WindowEventBase) => void): () => void {
    return onWindow("dpi-changed", handler);
  },
  onMinimized(handler: () => void): () => void {
    return onWindow("minimized", handler as (e: WindowEventBase) => void);
  },
  onUnminimized(handler: () => void): () => void {
    return onWindow("unminimized", handler as (e: WindowEventBase) => void);
  },
  onMaximized(handler: () => void): () => void {
    return onWindow("maximized", handler as (e: WindowEventBase) => void);
  },
  onUnmaximized(handler: () => void): () => void {
    return onWindow("unmaximized", handler as (e: WindowEventBase) => void);
  },
  onFullscreen(handler: () => void): () => void {
    return onWindow("fullscreen", handler as (e: WindowEventBase) => void);
  },
  onUnfullscreen(handler: () => void): () => void {
    return onWindow("unfullscreen", handler as (e: WindowEventBase) => void);
  },
  /**
   * Fires when a secondary window closes. Only receives events for other
   * windows (the closed window can't listen for its own destruction).
   */
  onClosed(handler: (e: WindowEventBase) => void): () => void {
    return window.__tynd__.os_on("window:closed", (raw) => handler(raw as WindowEventBase));
  },

  /** Files dragged over the window. `paths` are native OS paths. */
  onDragEnter(handler: (e: DragDropEvent) => void): () => void {
    return onWindow("drag-enter", handler);
  },
  /** Drag cursor moving over the window. Fires frequently — throttle if needed. */
  onDragOver(handler: (e: DragOverEvent) => void): () => void {
    return onWindow("drag-over", handler);
  },
  /** Drag cancelled or left the window. */
  onDragLeave(handler: () => void): () => void {
    return onWindow("drag-leave", handler as (e: WindowEventBase) => void);
  },

  /**
   * Fires before each navigation. Emits `allowed: false` when the active
   * `security` policy blocked the URL. Observation-only — to block a URL,
   * configure `app.start({ security: { http: { deny: [...] } } })` in the
   * trusted backend instead.
   */
  onNavigation(handler: (e: NavigationEvent) => void): () => void {
    return onWebview("navigation", handler);
  },
  /** Fires on document start + finish load. */
  onPageLoad(handler: (e: PageLoadEvent) => void): () => void {
    return onWebview("page-load", handler);
  },
  /** Files dropped onto the window. `paths` are native OS paths. */
  onDrop(handler: (e: DragDropEvent) => void): () => void {
    return onWindow("drop", handler);
  },
  /**
   * Fires when the user tries to close the window (X button, Alt+F4, Cmd+Q…).
   * The close proceeds after 500ms unless `event.preventDefault()` is called
   * synchronously in the handler. Filtered to the current window's label.
   */
  onCloseRequested(handler: (event: CloseRequestedEvent) => void): () => void {
    const self = currentWindowLabel();
    return window.__tynd__.os_on("window:close-requested", (raw) => {
      // Strict match — unlabeled events (protocol drift) don't leak to every
      // subscriber and accidentally cancel closes that weren't theirs.
      const label = (raw as WindowEventBase | undefined)?.label;
      if (label !== self) return;
      let prevented = false;
      handler({
        preventDefault() {
          if (prevented) return;
          prevented = true;
          osCall("window", "cancelClose").catch((e: unknown) => {
            tynd.warn(`window.cancelClose failed: ${e instanceof Error ? e.message : String(e)}`);
          });
        },
      });
    });
  },
};
