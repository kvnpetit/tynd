import { osCall } from "./_internal";

/**
 * `keys` — disable native browser key combos (Ctrl+R, F5, Ctrl+P, …)
 * inside the webview.
 *
 * The host owns the canonical set of disabled combos so a webview reload
 * doesn't lose state. Every mutation re-broadcasts `keys:set` carrying
 * the full list; the frontend listener installed by {@link install}
 * mirrors the set into `window.__tynd_keys_disabled__` and consults it
 * from a `keydown` capture handler that calls `event.preventDefault()`
 * for matching combos.
 *
 * Combo grammar: `Ctrl+Shift+Alt+Meta+<key>` where `<key>` is the JS
 * `event.key` value (case-sensitive). Modifier order is normalised on
 * the host side so callers can write `"Shift+Ctrl+R"` and
 * `"Ctrl+Shift+R"` interchangeably.
 */
export const keys = {
  /** Add combos to the disabled set (idempotent). */
  disable(combos: string[]): Promise<void> {
    return osCall("keys", "disable", { combos });
  },
  /** Remove combos from the disabled set. Unknown combos are ignored. */
  enable(combos: string[]): Promise<void> {
    return osCall("keys", "enable", { combos });
  },
  /** Snapshot of currently disabled combos (sorted, normalised). */
  list(): Promise<string[]> {
    return osCall("keys", "list", {});
  },
  /** Drop every disabled combo. */
  clear(): Promise<void> {
    return osCall("keys", "clear", {});
  },
  /**
   * Install the frontend keydown listener that consults
   * `window.__tynd_keys_disabled__`. Call once at app boot (typically
   * in your `main.tsx` / `main.ts`). No-op when called outside a
   * browser context (SSR, tests).
   */
  install(): void {
    if (typeof window === "undefined") return;
    const set: Set<string> =
      (window as unknown as { __tynd_keys_disabled__?: Set<string> }).__tynd_keys_disabled__ ??
      new Set<string>();
    (window as unknown as { __tynd_keys_disabled__: Set<string> }).__tynd_keys_disabled__ = set;

    // Sync from backend (initial snapshot).
    void this.list().then((combos) => {
      set.clear();
      for (const c of combos) set.add(c);
    });

    // Re-sync on every host mutation. The host re-broadcasts the full
    // list, so the frontend never has to reconcile diffs.
    window.__tynd__.os_on("keys:set", (raw) => {
      const combos = (raw as { combos?: string[] } | null)?.combos ?? [];
      set.clear();
      for (const c of combos) set.add(c);
    });

    window.addEventListener(
      "keydown",
      (e: KeyboardEvent) => {
        const combo = [
          e.ctrlKey && "Ctrl",
          e.shiftKey && "Shift",
          e.altKey && "Alt",
          e.metaKey && "Meta",
          e.key,
        ]
          .filter(Boolean)
          .join("+");
        if (set.has(combo)) e.preventDefault();
      },
      { capture: true },
    );
  },
};
