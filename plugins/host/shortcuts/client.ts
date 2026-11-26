/**
 * Global shortcut client. Mirrors the host's `os.shortcuts.*` dispatch.
 * The host owns one `GlobalHotKeyManager` and one OS-level event handler;
 * frontend handlers are wired through `os_on("shortcut:triggered")` and
 * routed by id. The TS layer keeps an `id -> dispose` map so the same id
 * can't leak listeners across re-registers.
 */

import { osCall, osOn } from "@tynd/core/client/bridge";

export interface ShortcutHandle {
  /** User id (custom or derived from the accelerator). */
  id: string;
  unregister(): Promise<boolean>;
}

const disposers = new Map<string, () => void>();

/** Detach the renderer listener registered for `id`, if any. */
function dropListener(id: string): void {
  disposers.get(id)?.();
  disposers.delete(id);
}

export const shortcuts = {
  async register(accelerator: string, handler: () => void, id?: string): Promise<ShortcutHandle> {
    const result = (await osCall("shortcuts", "register", {
      accelerator,
      ...(id === undefined ? {} : { id }),
    })) as { id: string };

    // One os_on listener per shortcut, filtered by id at dispatch time.
    const off = osOn("shortcut:triggered", (raw) => {
      const evt = raw as { id?: string };
      if (evt.id === result.id) handler();
    });

    // Replace any prior dispose for the same id so we don't leak listeners.
    disposers.get(result.id)?.();
    disposers.set(result.id, off);

    return {
      id: result.id,
      async unregister() {
        dropListener(result.id);
        return osCall<boolean>("shortcuts", "unregister", { id: result.id });
      },
    };
  },

  unregister(id: string): Promise<boolean> {
    dropListener(id);
    return osCall("shortcuts", "unregister", { id });
  },

  async unregisterAll(): Promise<void> {
    for (const dispose of disposers.values()) dispose();
    disposers.clear();
    await osCall("shortcuts", "unregisterAll", null);
  },

  isRegistered(id: string): Promise<boolean> {
    return osCall("shortcuts", "isRegistered", { id });
  },
};
