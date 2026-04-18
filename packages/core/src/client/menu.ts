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
}
