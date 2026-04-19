import { osCall } from "./_internal.ts"

export interface ShortcutHandle {
  /** User id (custom or derived from the accelerator). */
  id: string
  unregister(): Promise<boolean>
}

export const shortcuts = {
  /**
   * Register a system-wide keyboard shortcut. The handler fires whenever the
   * accelerator is pressed, even when the app doesn't have focus. Returns a
   * handle for unregistering later (plus the auto-derived id when omitted).
   */
  async register(accelerator: string, handler: () => void, id?: string): Promise<ShortcutHandle> {
    const result = await osCall<{ id: string }>("shortcuts", "register", {
      accelerator,
      ...(id === undefined ? {} : { id }),
    })
    const off = window.__tynd__.os_on("shortcut:triggered", (raw) => {
      if ((raw as { id?: string } | undefined)?.id === result.id) handler()
    })
    return {
      id: result.id,
      async unregister() {
        off()
        return osCall<boolean>("shortcuts", "unregister", { id: result.id })
      },
    }
  },
  /** Unregister a shortcut by id. Returns `true` if it existed. */
  unregister(id: string): Promise<boolean> {
    return osCall("shortcuts", "unregister", { id })
  },
  /** Remove every registered shortcut. */
  unregisterAll(): Promise<void> {
    return osCall("shortcuts", "unregisterAll")
  },
  /** Whether an id is currently registered. */
  isRegistered(id: string): Promise<boolean> {
    return osCall("shortcuts", "isRegistered", { id })
  },
}
