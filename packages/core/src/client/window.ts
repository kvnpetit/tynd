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

declare global {
  interface Window {
    __TYND_WINDOW_LABEL__?: string
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
