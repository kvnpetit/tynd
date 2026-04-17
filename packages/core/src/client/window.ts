import { osCall } from "./_internal.js"

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
  onMenu(id: string, handler: () => void): () => void {
    return window.__tynd__.os_on("menu:action", (data: unknown) => {
      if (((data as Record<string, unknown>)?.["id"] as string) === id) handler()
    })
  },
}
