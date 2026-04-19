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

/** Subscribe to a `window:<event>` stream emitted by the Rust event loop. */
function onWindow<T>(event: string, handler: (data: T) => void): () => void {
  return window.__tynd__.os_on(`window:${event}`, (raw) => handler(raw as T))
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

  onResized(handler: (size: WindowSize) => void): () => void {
    return onWindow("resized", handler)
  },
  onMoved(handler: (pos: WindowPosition) => void): () => void {
    return onWindow("moved", handler)
  },
  onFocused(handler: () => void): () => void {
    return onWindow("focused", handler)
  },
  onBlurred(handler: () => void): () => void {
    return onWindow("blurred", handler)
  },
  onThemeChanged(handler: (data: { theme: "light" | "dark" }) => void): () => void {
    return onWindow("theme-changed", handler)
  },
  onDpiChanged(handler: (data: { scale: number }) => void): () => void {
    return onWindow("dpi-changed", handler)
  },
  onMinimized(handler: () => void): () => void {
    return onWindow("minimized", handler)
  },
  onUnminimized(handler: () => void): () => void {
    return onWindow("unminimized", handler)
  },
  onMaximized(handler: () => void): () => void {
    return onWindow("maximized", handler)
  },
  onUnmaximized(handler: () => void): () => void {
    return onWindow("unmaximized", handler)
  },
  onFullscreen(handler: () => void): () => void {
    return onWindow("fullscreen", handler)
  },
  onUnfullscreen(handler: () => void): () => void {
    return onWindow("unfullscreen", handler)
  },
  /**
   * Fires when the user tries to close the window (X button, Alt+F4, Cmd+Q…).
   * The close proceeds after 500ms unless `event.preventDefault()` is called
   * synchronously in the handler.
   */
  onCloseRequested(handler: (event: CloseRequestedEvent) => void): () => void {
    return window.__tynd__.os_on("window:close-requested", () => {
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
