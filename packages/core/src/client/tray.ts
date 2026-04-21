import { osCall } from "./_internal.js"

export interface TrayMenuItem {
  type?: "separator" | "submenu" | "checkbox" | "radio" | "item"
  id?: string
  label?: string
  enabled?: boolean
  role?: string
  accelerator?: string
  checked?: boolean
  items?: TrayMenuItem[]
}

export const tray = {
  /** Replace the tray icon. Accepts the same image formats as the config. */
  setIcon(path: string): Promise<void> {
    return osCall("tray", "setIcon", { path })
  },
  /** Set (or clear, pass `undefined`) the hover tooltip. */
  setTooltip(text?: string): Promise<void> {
    return osCall("tray", "setTooltip", { text })
  },
  /** macOS: text next to the icon in the menu bar. No-op elsewhere. */
  setTitle(text?: string): Promise<void> {
    return osCall("tray", "setTitle", { text })
  },
  setVisible(visible: boolean): Promise<void> {
    return osCall("tray", "setVisible", { visible })
  },
  /**
   * macOS: render the tray icon as a template so it adapts to light / dark
   * menu bar automatically. No-op on Windows / Linux.
   */
  setIconAsTemplate(template: boolean): Promise<void> {
    return osCall("tray", "setIconAsTemplate", { template })
  },
  /** Replace the entire tray menu. Pass `[]` to clear. */
  setMenu(items: TrayMenuItem[]): Promise<void> {
    return osCall("tray", "setMenu", { items })
  },

  onClick(handler: () => void): () => void {
    return window.__tynd__.os_on("tray:click", () => handler())
  },
  onRightClick(handler: () => void): () => void {
    return window.__tynd__.os_on("tray:right-click", () => handler())
  },
  onDoubleClick(handler: () => void): () => void {
    return window.__tynd__.os_on("tray:double-click", () => handler())
  },
  /** Mouse entered the tray icon's bounds. `x`/`y` are physical pixels. */
  onEnter(handler: (pos: { x: number; y: number }) => void): () => void {
    return window.__tynd__.os_on("tray:enter", (raw) =>
      handler(raw as { x: number; y: number }),
    )
  },
  /** Mouse moved over the tray icon. */
  onMove(handler: (pos: { x: number; y: number }) => void): () => void {
    return window.__tynd__.os_on("tray:move", (raw) =>
      handler(raw as { x: number; y: number }),
    )
  },
  /** Mouse left the tray icon's bounds. */
  onLeave(handler: (pos: { x: number; y: number }) => void): () => void {
    return window.__tynd__.os_on("tray:leave", (raw) =>
      handler(raw as { x: number; y: number }),
    )
  },
  onMenu(id: string, handler: () => void): () => void {
    return window.__tynd__.os_on("menu:action", (data: unknown) => {
      if (((data as Record<string, unknown>)?.["id"] as string) === id) handler()
    })
  },
}
