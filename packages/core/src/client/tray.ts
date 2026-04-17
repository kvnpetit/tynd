import "./_internal.js"

export const tray = {
  onClick(handler: () => void): () => void {
    return window.__tynd__.os_on("tray:click", () => handler())
  },
  onRightClick(handler: () => void): () => void {
    return window.__tynd__.os_on("tray:right-click", () => handler())
  },
  onDoubleClick(handler: () => void): () => void {
    return window.__tynd__.os_on("tray:double-click", () => handler())
  },
  onMenu(id: string, handler: () => void): () => void {
    return window.__tynd__.os_on("menu:action", (data: unknown) => {
      if (((data as Record<string, unknown>)?.["id"] as string) === id) handler()
    })
  },
}
