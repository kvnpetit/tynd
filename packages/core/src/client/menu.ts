import { osCall } from "./_internal.js"
import type { TrayMenuItem } from "./tray.js"

export type ContextMenuItem = TrayMenuItem

/**
 * Menu event subscriptions. Declarative menu structure still lives in
 * `tynd.config.ts` — this module just lets you react to clicks on items
 * (both app menu bar and tray menu, which share the same `id` namespace).
 */
export const menu = {
  /** Fire `handler` when the user clicks the menu item with the given `id`. */
  onClick(id: string, handler: () => void): () => void {
    return window.__tynd__.os_on("menu:action", (data: unknown) => {
      if (((data as Record<string, unknown>)?.["id"] as string) === id) handler()
    })
  },
  /**
   * Show a native popup menu anchored on the current window. Clicks emit
   * the same `menu:action` event as the menu bar / tray — discriminate by
   * `id`. `x` / `y` are window-relative logical pixels; omitting both uses
   * the cursor position. Windows only for now (macOS / Linux in progress).
   */
  showContextMenu(items: ContextMenuItem[], position?: { x: number; y: number }): Promise<void> {
    return osCall("window", "showContextMenu", {
      items,
      x: position?.x,
      y: position?.y,
    })
  },
}
