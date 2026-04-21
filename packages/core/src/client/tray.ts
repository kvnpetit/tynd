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
  onMenu(id: string, handler: () => void): () => void {
    return window.__tynd__.os_on("menu:action", (data: unknown) => {
      if (((data as Record<string, unknown>)?.["id"] as string) === id) handler()
    })
  },
}
