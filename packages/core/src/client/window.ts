import { osCall } from "./_internal.js"

export interface WindowSize {
  width: number
  height: number
}

export interface WindowPosition {
  x: number
  y: number
}

export interface CloseRequestedEvent {
  /** Cancel the pending close. Must be called synchronously inside the handler. */
  preventDefault(): void
}

export interface CreateWindowOptions {
  /** Unique label identifying the window (used by close / event filtering). */
  label: string
  /** Override URL (e.g. "/settings"). Defaults to the primary window's entry. */
  url?: string
  title?: string
  width?: number
  height?: number
  resizable?: boolean
  decorations?: boolean
  alwaysOnTop?: boolean
  /** Custom User-Agent string for requests issued by this webview. */
  userAgent?: string
  /**
   * Attach this window to a parent (by label) as a modal / owned child.
   * Windows + macOS: native owner window; Linux: degrades to a regular
   * always-on-top window (tao exposes no equivalent).
   */
  modalTo?: string
}

export interface NavigationEvent extends WindowEventBase {
  url: string
  /** `false` when the security policy blocked this navigation. */
  allowed: boolean
}

export interface PageLoadEvent extends WindowEventBase {
  phase: "started" | "finished"
  url: string
}

export interface WindowEventBase {
  label: string
}

export interface DragDropEvent extends WindowEventBase {
  /** Native OS file paths. */
  paths: string[]
  /** Cursor position relative to the webview top-left, logical pixels. */
  x: number
  y: number
}

export interface DragOverEvent extends WindowEventBase {
  x: number
  y: number
}

export type CursorIcon =
  | "default"
  | "crosshair"
  | "hand"
  | "arrow"
  | "move"
  | "text"
  | "wait"
  | "help"
  | "progress"
  | "notAllowed"
  | "contextMenu"
  | "cell"
  | "verticalText"
  | "alias"
  | "copy"
  | "noDrop"
  | "grab"
  | "grabbing"
  | "allScroll"
  | "zoomIn"
  | "zoomOut"
  | "eResize"
  | "nResize"
  | "neResize"
  | "nwResize"
  | "sResize"
  | "seResize"
  | "swResize"
  | "wResize"
  | "ewResize"
  | "nsResize"
  | "neswResize"
  | "nwseResize"
  | "colResize"
  | "rowResize"

export type ResizeDirection =
  | "east"
  | "north"
  | "northEast"
  | "northWest"
  | "south"
  | "southEast"
  | "southWest"
  | "west"

declare global {
  interface Window {
    __TYND_WINDOW_LABEL__?: string
    /**
     * Non-standard but supported in Chromium (WebView2) and WKWebView.
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
    ) => boolean
  }
}

/** Label of the current window. Defaults to "main" for the primary window. */
export function getWindowLabel(): string {
  return typeof window !== "undefined" ? (window.__TYND_WINDOW_LABEL__ ?? "main") : "main"
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
  const self = getWindowLabel()
  return window.__tynd__.os_on(`window:${event}`, (raw) => {
    const data = raw as T
    if (!opts?.anyWindow) {
      // Strict match: events without a label (shouldn't happen, but could if
      // the host protocol ever changes) are treated as NOT for this window
      // rather than broadcast to every subscriber.
      const label = (data as WindowEventBase | undefined)?.label
      if (label !== self) return
    }
    handler(data)
  })
}

export const tyndWindow = {
  setTitle(title: string): Promise<void> {
    return osCall("window", "setTitle", { title })
  },
  getTitle(): Promise<string> {
    return osCall("window", "getTitle")
  },
  setSize(width: number, height: number): Promise<void> {
    return osCall("window", "setSize", { width, height })
  },
  /** Inner size (client area) in logical pixels. */
  getSize(): Promise<WindowSize> {
    return osCall("window", "getSize")
  },
  /** Outer size including title bar + borders. */
  getOuterSize(): Promise<WindowSize> {
    return osCall("window", "getOuterSize")
  },
  setPosition(x: number, y: number): Promise<void> {
    return osCall("window", "setPosition", { x, y })
  },
  getPosition(): Promise<WindowPosition> {
    return osCall("window", "getPosition")
  },
  setMinSize(width: number, height: number): Promise<void> {
    return osCall("window", "setMinSize", { width, height })
  },
  setMaxSize(width: number, height: number): Promise<void> {
    return osCall("window", "setMaxSize", { width, height })
  },
  setResizable(resizable: boolean): Promise<void> {
    return osCall("window", "setResizable", { resizable })
  },
  setClosable(closable: boolean): Promise<void> {
    return osCall("window", "setClosable", { closable })
  },
  setMaximizable(maximizable: boolean): Promise<void> {
    return osCall("window", "setMaximizable", { maximizable })
  },
  setMinimizable(minimizable: boolean): Promise<void> {
    return osCall("window", "setMinimizable", { minimizable })
  },
  toggleMaximize(): Promise<void> {
    return osCall("window", "toggleMaximize")
  },
  isResizable(): Promise<boolean> {
    return osCall("window", "isResizable")
  },
  isClosable(): Promise<boolean> {
    return osCall("window", "isClosable")
  },
  isFocused(): Promise<boolean> {
    return osCall("window", "isFocused")
  },
  isDecorated(): Promise<boolean> {
    return osCall("window", "isDecorated")
  },
  scaleFactor(): Promise<number> {
    return osCall("window", "scaleFactor")
  },
  minimize(): Promise<void> {
    return osCall("window", "minimize")
  },
  unminimize(): Promise<void> {
    return osCall("window", "unminimize")
  },
  maximize(): Promise<void> {
    return osCall("window", "maximize")
  },
  unmaximize(): Promise<void> {
    return osCall("window", "unmaximize")
  },
  center(): Promise<void> {
    return osCall("window", "center")
  },
  show(): Promise<void> {
    return osCall("window", "show")
  },
  hide(): Promise<void> {
    return osCall("window", "hide")
  },
  setFullscreen(fullscreen: boolean): Promise<void> {
    return osCall("window", "setFullscreen", { fullscreen })
  },
  setAlwaysOnTop(always: boolean): Promise<void> {
    return osCall("window", "setAlwaysOnTop", { always })
  },
  setDecorations(decorations: boolean): Promise<void> {
    return osCall("window", "setDecorations", { decorations })
  },
  isMaximized(): Promise<boolean> {
    return osCall("window", "isMaximized")
  },
  isMinimized(): Promise<boolean> {
    return osCall("window", "isMinimized")
  },
  isFullscreen(): Promise<boolean> {
    return osCall("window", "isFullscreen")
  },
  isVisible(): Promise<boolean> {
    return osCall("window", "isVisible")
  },

  /**
   * Cancel a pending close. Has effect only during the 500ms window after a
   * `window:close-requested` event fires — after that the app proceeds to exit.
   */
  cancelClose(): Promise<void> {
    return osCall("window", "cancelClose")
  },

  /** Bring this window to the foreground (keyboard focus). */
  setFocus(): Promise<void> {
    return osCall("window", "setFocus")
  },
  /** Flash the taskbar (Windows) / bounce the Dock (macOS) to get attention. */
  requestAttention(): Promise<void> {
    return osCall("window", "requestAttention")
  },

  /** Reload the webview's current URL. */
  reload(): Promise<void> {
    return osCall("window", "reload")
  },
  /** Set the webview's zoom level (`1.0` = 100 %). */
  setZoom(level: number): Promise<void> {
    return osCall("window", "setZoom", { level })
  },
  /** Last zoom passed to `setZoom` (defaults to `1.0`). wry has no getter. */
  getZoom(): Promise<number> {
    return osCall("window", "getZoom")
  },
  /** Open the webview devtools. Only available in debug builds. */
  openDevTools(): Promise<void> {
    return osCall("window", "openDevTools")
  },
  /** Close the webview devtools (no-op if already closed or in release build). */
  closeDevTools(): Promise<void> {
    return osCall("window", "closeDevTools")
  },

  /** Open the OS print dialog for the current webview contents. */
  print(): Promise<void> {
    return osCall("window", "print")
  },
  /** Replace the webview URL at runtime. */
  navigate(url: string): Promise<void> {
    return osCall("window", "navigate", { url })
  },
  /** Replace the webview contents with raw HTML. */
  loadHtml(html: string): Promise<void> {
    return osCall("window", "loadHtml", { html })
  },
  /** Current URL of the webview. */
  getUrl(): Promise<string> {
    return osCall("window", "getUrl")
  },

  /** Set the cursor icon over this window. */
  setCursorIcon(icon: CursorIcon): Promise<void> {
    return osCall("window", "setCursorIcon", { icon })
  },
  /** Warp the cursor to the given window-relative logical position. */
  setCursorPosition(x: number, y: number): Promise<void> {
    return osCall("window", "setCursorPosition", { x, y })
  },
  setCursorVisible(visible: boolean): Promise<void> {
    return osCall("window", "setCursorVisible", { visible })
  },
  /** Click-through: when `true`, mouse events pass through to windows behind. */
  setIgnoreCursorEvents(ignore: boolean): Promise<void> {
    return osCall("window", "setIgnoreCursorEvents", { ignore })
  },
  /** Start a native window drag. Call from a mousedown handler on a drag zone. */
  startDragging(): Promise<void> {
    return osCall("window", "startDragging")
  },
  /** Start a native resize drag from the given edge / corner. */
  startResizeDragging(direction: ResizeDirection = "southEast"): Promise<void> {
    return osCall("window", "startResizeDragging", { direction })
  },

  /** Override the window theme (`system` follows the OS). */
  setTheme(theme: "light" | "dark" | "system"): Promise<void> {
    return osCall("window", "setTheme", { theme })
  },
  /** Set the native window background color (shows through transparent HTML). */
  setBackgroundColor(color: { r: number; g: number; b: number; a?: number } | null): Promise<void> {
    return osCall("window", "setBackgroundColor", { color })
  },
  /** Block screenshots + screen recording of this window (Win/macOS). */
  setContentProtection(enabled: boolean): Promise<void> {
    return osCall("window", "setContentProtection", { enabled })
  },
  /**
   * Taskbar / dock progress indicator. `progress` is 0-100. State: `"normal"`,
   * `"indeterminate"`, `"paused"`, `"error"`, `"none"` (default).
   */
  setProgressBar(
    state: "none" | "normal" | "indeterminate" | "paused" | "error",
    progress?: number,
  ): Promise<void> {
    return osCall("window", "setProgressBar", { state, progress })
  },
  /** Hide the window from the taskbar (Windows/Linux). No-op on macOS. */
  setSkipTaskbar(skip: boolean): Promise<void> {
    return osCall("window", "setSkipTaskbar", { skip })
  },
  /**
   * Dock / taskbar badge. `label` is displayed on macOS; `count` is rendered
   * on Linux (Unity). Windows uses taskbar overlay icons and is unsupported
   * here.
   */
  setBadge(options: { label?: string; count?: number }): Promise<void> {
    return osCall("window", "setBadge", options)
  },
  /** Whether the window can receive keyboard focus. */
  setFocusable(focusable: boolean): Promise<void> {
    return osCall("window", "setFocusable", { focusable })
  },
  /** Windows only — disable mouse + keyboard input. Other OS: no-op. */
  setEnabled(enabled: boolean): Promise<void> {
    return osCall("window", "setEnabled", { enabled })
  },
  /** macOS: show this window on every Space. Other OS: no-op. */
  setVisibleOnAllWorkspaces(visible: boolean): Promise<void> {
    return osCall("window", "setVisibleOnAllWorkspaces", { visible })
  },
  /** macOS: toggle the native drop shadow. Other OS: no-op. */
  setShadow(enabled: boolean): Promise<void> {
    return osCall("window", "setShadow", { enabled })
  },
  /** Replace the window icon at runtime. Pass `null` to clear. */
  setWindowIcon(path: string | null): Promise<void> {
    return osCall("window", "setWindowIcon", { path })
  },
  /** macOS: make the titlebar transparent. No-op on Windows / Linux. */
  setTitlebarTransparent(transparent: boolean): Promise<void> {
    return osCall("window", "setTitlebarTransparent", { transparent })
  },
  /** macOS: let HTML extend under the titlebar. No-op on Windows / Linux. */
  setFullsizeContentView(fullsize: boolean): Promise<void> {
    return osCall("window", "setFullsizeContentView", { fullsize })
  },
  /** macOS: reposition the traffic light buttons. No-op on Windows / Linux. */
  setTrafficLightInset(x: number, y: number): Promise<void> {
    return osCall("window", "setTrafficLightInset", { x, y })
  },
  /**
   * Windows 11+ system backdrop: `"mica"`, `"acrylic"`, `"tabbed"`,
   * `"none"`, or `"auto"`. No-op on other OS. Requires the window to have
   * no decorations for best results (set in config).
   */
  setSystemBackdrop(
    kind: "auto" | "none" | "mica" | "acrylic" | "tabbed",
  ): Promise<void> {
    return osCall("window", "setSystemBackdrop", { kind })
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
    if (typeof window === "undefined" || typeof window.find !== "function") return false
    return window.find(
      query,
      options?.caseSensitive ?? false,
      options?.backwards ?? false,
      true,
      options?.wholeWord ?? false,
      true,
      false,
    )
  },
  /** Clear the active find highlight / selection. */
  stopFindInPage(): void {
    if (typeof window === "undefined") return
    window.getSelection()?.removeAllRanges()
  },

  /** Label of the current window (`"main"` for the primary window). */
  label(): string {
    return getWindowLabel()
  },

  /** Open a secondary window. Labels must be unique and not `"main"`. */
  create(options: CreateWindowOptions): Promise<void> {
    return osCall("window", "create", options)
  },
  /** Close a secondary window by label. Closing `"main"` is rejected. */
  close(label: string): Promise<void> {
    return osCall("window", "close", { label })
  },
  /** Close this window. No-op on the primary window (use `app.exit()` instead). */
  closeSelf(): Promise<void> {
    const label = getWindowLabel()
    if (label === "main") return Promise.resolve()
    return osCall("window", "close", { label })
  },
  /** List labels of all open windows (primary first). */
  all(): Promise<string[]> {
    return osCall("window", "all")
  },

  onResized(handler: (e: WindowSize & WindowEventBase) => void): () => void {
    return onWindow("resized", handler)
  },
  onMoved(handler: (e: WindowPosition & WindowEventBase) => void): () => void {
    return onWindow("moved", handler)
  },
  onFocused(handler: () => void): () => void {
    return onWindow("focused", handler as (e: WindowEventBase) => void)
  },
  onBlurred(handler: () => void): () => void {
    return onWindow("blurred", handler as (e: WindowEventBase) => void)
  },
  onThemeChanged(handler: (e: { theme: "light" | "dark" } & WindowEventBase) => void): () => void {
    return onWindow("theme-changed", handler)
  },
  onDpiChanged(handler: (e: { scale: number } & WindowEventBase) => void): () => void {
    return onWindow("dpi-changed", handler)
  },
  onMinimized(handler: () => void): () => void {
    return onWindow("minimized", handler as (e: WindowEventBase) => void)
  },
  onUnminimized(handler: () => void): () => void {
    return onWindow("unminimized", handler as (e: WindowEventBase) => void)
  },
  onMaximized(handler: () => void): () => void {
    return onWindow("maximized", handler as (e: WindowEventBase) => void)
  },
  onUnmaximized(handler: () => void): () => void {
    return onWindow("unmaximized", handler as (e: WindowEventBase) => void)
  },
  onFullscreen(handler: () => void): () => void {
    return onWindow("fullscreen", handler as (e: WindowEventBase) => void)
  },
  onUnfullscreen(handler: () => void): () => void {
    return onWindow("unfullscreen", handler as (e: WindowEventBase) => void)
  },
  /**
   * Fires when a secondary window closes. Only receives events for other
   * windows (the closed window can't listen for its own destruction).
   */
  onClosed(handler: (e: WindowEventBase) => void): () => void {
    return window.__tynd__.os_on("window:closed", (raw) => handler(raw as WindowEventBase))
  },

  /** Files dragged over the window. `paths` are native OS paths. */
  onDragEnter(handler: (e: DragDropEvent) => void): () => void {
    return onWindow("drag-enter", handler)
  },
  /** Drag cursor moving over the window. Fires frequently — throttle if needed. */
  onDragOver(handler: (e: DragOverEvent) => void): () => void {
    return onWindow("drag-over", handler)
  },
  /** Drag cancelled or left the window. */
  onDragLeave(handler: () => void): () => void {
    return onWindow("drag-leave", handler as (e: WindowEventBase) => void)
  },

  /**
   * Fires before each navigation. Emits `allowed: false` when the active
   * `security` policy blocked the URL. Observation-only — to block a URL,
   * configure `security.configure({ http: { deny: [...] } })` instead.
   */
  onNavigation(handler: (e: NavigationEvent) => void): () => void {
    return window.__tynd__.os_on("webview:navigation", (raw) => {
      const data = raw as NavigationEvent
      if (data.label !== getWindowLabel()) return
      handler(data)
    })
  },
  /** Fires on document start + finish load. */
  onPageLoad(handler: (e: PageLoadEvent) => void): () => void {
    return window.__tynd__.os_on("webview:page-load", (raw) => {
      const data = raw as PageLoadEvent
      if (data.label !== getWindowLabel()) return
      handler(data)
    })
  },
  /** Files dropped onto the window. `paths` are native OS paths. */
  onDrop(handler: (e: DragDropEvent) => void): () => void {
    return onWindow("drop", handler)
  },
  /**
   * Fires when the user tries to close the window (X button, Alt+F4, Cmd+Q…).
   * The close proceeds after 500ms unless `event.preventDefault()` is called
   * synchronously in the handler. Filtered to the current window's label.
   */
  onCloseRequested(handler: (event: CloseRequestedEvent) => void): () => void {
    const self = getWindowLabel()
    return window.__tynd__.os_on("window:close-requested", (raw) => {
      // Strict match — unlabeled events (protocol drift) don't leak to every
      // subscriber and accidentally cancel closes that weren't theirs.
      const label = (raw as WindowEventBase | undefined)?.label
      if (label !== self) return
      let prevented = false
      handler({
        preventDefault() {
          if (prevented) return
          prevented = true
          void osCall("window", "cancelClose")
        },
      })
    })
  },
}
