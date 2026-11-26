import { osCall, osOn } from "@tynd/core/client/bridge";

/** Dev-only warning, silent in production builds (same rule as the core logger). */
function devWarn(msg: string): void {
  if ((import.meta as { env?: { DEV?: boolean } }).env?.DEV !== true) return;
  // biome-ignore lint/suspicious/noConsole: dev-only diagnostics
  console.warn(`[tynd] ${msg}`);
}

/**
 * `keys` — disable native browser key combos (Ctrl+R, F5, Ctrl+P, …)
 * inside the webview.
 * The host owns the canonical set of disabled combos so a webview reload
 * doesn't lose state. Every mutation re-broadcasts `keys:set` carrying
 * the full list; the frontend listener installed by {@link install}
 * mirrors the set into `window.__tynd_keys_disabled__` and consults it
 * from a `keydown` capture handler that calls `event.preventDefault()`
 * for matching combos.
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
    const w = window as unknown as {
      __tynd_keys_disabled__?: Set<string>;
      __tynd_keys_installed__?: boolean;
    };
    // Idempotent across calls and HMR re-evaluations (the flag lives on `window`,
    // not in this module): a second install would stack listeners.
    if (w.__tynd_keys_installed__) return;
    w.__tynd_keys_installed__ = true;
    const set: Set<string> = w.__tynd_keys_disabled__ ?? new Set<string>();
    w.__tynd_keys_disabled__ = set;

    // Sync from backend (initial snapshot). A `keys:set` event is always newer
    // than a snapshot still in flight, so once one arrived the snapshot is dropped.
    let sawEvent = false;
    this.list().then(
      (combos) => {
        if (sawEvent) return;
        set.clear();
        for (const c of combos) set.add(c);
      },
      (e: unknown) => {
        devWarn(
          `keys.install(): initial snapshot failed: ${e instanceof Error ? e.message : String(e)}`,
        );
      },
    );

    // Re-sync on every host mutation. The host re-broadcasts the full
    // list, so the frontend never has to reconcile diffs.
    osOn("keys:set", (raw) => {
      sawEvent = true;
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
