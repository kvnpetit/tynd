import { osCall } from "./_internal.ts"

export interface ShortcutHandle {
  /** User id (custom or derived from the accelerator). */
  id: string
  unregister(): Promise<boolean>
}

// Every `register()` attaches an `os_on` listener. Track them globally so
// `unregisterAll()` and the bare `unregister(id)` path don't leave stale
// listeners behind that would fire if the id were ever reused.
const offHandlers = new Map<string, () => void>()

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
    // Replace any prior off for the same id (e.g. repeated register with the
    // same custom id after a previous unregister) so we don't leak the old.
    offHandlers.get(result.id)?.()
    offHandlers.set(result.id, off)
    return {
      id: result.id,
      async unregister() {
        offHandlers.get(result.id)?.()
        offHandlers.delete(result.id)
        return osCall<boolean>("shortcuts", "unregister", { id: result.id })
      },
    }
  },
  /** Unregister a shortcut by id. Returns `true` if it existed. */
  unregister(id: string): Promise<boolean> {
    offHandlers.get(id)?.()
    offHandlers.delete(id)
    return osCall("shortcuts", "unregister", { id })
  },
  /** Remove every registered shortcut. */
  async unregisterAll(): Promise<void> {
    for (const off of offHandlers.values()) off()
    offHandlers.clear()
    return osCall("shortcuts", "unregisterAll")
  },
  /** Whether an id is currently registered. */
  isRegistered(id: string): Promise<boolean> {
    return osCall("shortcuts", "isRegistered", { id })
  },
}
