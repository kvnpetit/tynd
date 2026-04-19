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
      const label = (data as WindowEventBase | undefined)?.label
      if (label !== undefined && label !== self) return
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
  /**
   * Fires when the user tries to close the window (X button, Alt+F4, Cmd+Q…).
   * The close proceeds after 500ms unless `event.preventDefault()` is called
   * synchronously in the handler. Filtered to the current window's label.
   */
  onCloseRequested(handler: (event: CloseRequestedEvent) => void): () => void {
    const self = getWindowLabel()
    return window.__tynd__.os_on("window:close-requested", (raw) => {
      const label = (raw as WindowEventBase | undefined)?.label
      if (label !== undefined && label !== self) return
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
