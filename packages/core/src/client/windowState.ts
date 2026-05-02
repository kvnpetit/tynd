/**
 * Window-state persistence — saves per-window size, position and
 * maximized / fullscreen flags between sessions.
 *
 * Native promotion of the previous `@tynd/plugin-window-state` plugin.
 * The Rust side (`host-rs/src/os/window_persistence.rs`) is a storage
 * primitive: an in-memory `HashMap<label, WindowState>` plus an atomic
 * JSON file on disk. Capturing the live geometry and applying it on
 * restore live here because the `tyndWindow.*` setters are reachable
 * only from a window context.
 *
 * Boot pattern:
 *
 *   import { windowState } from "@tynd/core/client"
 *   await windowState.init()      // load disk → memory, attach listeners
 *   await windowState.restore()   // apply saved state to current window
 *
 * After `init()` returns, every resize / move / maximize toggle is
 * auto-persisted (debounced 500ms by default) and the disk write
 * happens in the background.
 */

import { osCall } from "./_internal";
import { os } from "./os";
import { tyndWindow } from "./window";

/** Persisted window geometry + flags. Matches the on-disk JSON shape. */
export interface WindowStateSnapshot {
  /** Outer-position X in logical pixels. `undefined` when never reported. */
  x?: number;
  /** Outer-position Y in logical pixels. `undefined` when never reported. */
  y?: number;
  /** Inner-size width in logical pixels. `undefined` when never reported. */
  width?: number;
  /** Inner-size height in logical pixels. `undefined` when never reported. */
  height?: number;
  /** Window was maximized at save time. */
  maximized: boolean;
  /** Window was fullscreen at save time. */
  fullscreen: boolean;
}

/** Options for [`windowState.init`]. */
export interface WindowStateInitOptions {
  /**
   * App identifier used when computing the default storage path:
   * `<dataDir>/<appId>/window-states.json`. Defaults to `"tynd-app"`
   * — set this to your app's slug so multiple apps don't share state.
   */
  appId?: string;
  /**
   * Override the storage path entirely. When set, `appId` is ignored
   * and the path is used verbatim. Must be absolute.
   */
  storagePath?: string;
  /** Auto-save debounce in milliseconds. Defaults to 500. */
  debounceMs?: number;
}

interface InternalState {
  initPromise: Promise<void> | null;
  appId: string;
  debounceMs: number;
  /** Pending debounce timers, keyed by label. */
  timers: Map<string, ReturnType<typeof setTimeout>>;
  /** Detach functions for the per-window event listeners. */
  unsubs: Array<() => void>;
}

const internal: InternalState = {
  initPromise: null,
  appId: "tynd-app",
  debounceMs: 500,
  timers: new Map(),
  unsubs: [],
};

function call<T>(method: string, args: unknown = null): Promise<T> {
  return osCall("windowState", method, args) as Promise<T>;
}

/**
 * Compute the default storage path:
 * `<dataDir>/<appId>/window-states.json`. Returns `null` when the host
 * can't report a data dir (rare — locked-down sandboxes).
 */
async function defaultStoragePath(appId: string): Promise<string | null> {
  const root = await os.dataDir();
  if (!root) return null;
  // Use the same separator as the data dir so the resulting path
  // round-trips cleanly through Rust's `Path` on every OS.
  const sep = root.includes("\\") ? "\\" : "/";
  return `${root}${sep}${appId}${sep}window-states.json`;
}

/**
 * Capture the current window's geometry. Reads outer position + inner
 * size + maximized + fullscreen flags in parallel; failures degrade to
 * the field being `undefined` (or `false` for booleans) so a partial
 * platform doesn't break persistence.
 */
async function captureCurrent(): Promise<WindowStateSnapshot> {
  const [pos, size, maximized, fullscreen] = await Promise.all([
    tyndWindow.getPosition().catch(() => null),
    tyndWindow.getSize().catch(() => null),
    tyndWindow.isMaximized().catch(() => false),
    tyndWindow.isFullscreen().catch(() => false),
  ]);
  return {
    x: pos?.x,
    y: pos?.y,
    width: size?.width,
    height: size?.height,
    maximized,
    fullscreen,
  };
}

/** Apply a snapshot to the current window, in restore-friendly order. */
async function applySnapshot(snap: WindowStateSnapshot): Promise<void> {
  // Order matters: size + position first, then state flags. Maximizing
  // before sizing would clobber the requested geometry.
  if (snap.width !== undefined && snap.height !== undefined) {
    await tyndWindow.setSize(snap.width, snap.height);
  }
  if (snap.x !== undefined && snap.y !== undefined) {
    await tyndWindow.setPosition(snap.x, snap.y);
  }
  if (snap.maximized) await tyndWindow.maximize();
  if (snap.fullscreen) await tyndWindow.setFullscreen(true);
}

/** Schedule a save for `label`, coalescing rapid bursts inside one window. */
function scheduleSave(label: string): void {
  const existing = internal.timers.get(label);
  if (existing !== undefined) clearTimeout(existing);
  const t = setTimeout(() => {
    internal.timers.delete(label);
    void flushSave(label);
  }, internal.debounceMs);
  internal.timers.set(label, t);
}

/** Capture + persist the current state for `label`. */
async function flushSave(label: string): Promise<void> {
  try {
    const snap = await captureCurrent();
    await call("set", { label, state: snap });
    await call("saveToDisk");
  } catch {
    // Persisting window state must never crash the app. Failures are
    // typically transient (disk full, permissions changed mid-session)
    // and the next resize/move event will retry on its own.
  }
}

function attachListeners(): void {
  const label = tyndWindow.label();
  internal.unsubs.push(
    tyndWindow.onResized(() => scheduleSave(label)),
    tyndWindow.onMoved(() => scheduleSave(label)),
    tyndWindow.onMaximized(() => scheduleSave(label)),
    tyndWindow.onUnmaximized(() => scheduleSave(label)),
    tyndWindow.onFullscreen(() => scheduleSave(label)),
    tyndWindow.onUnfullscreen(() => scheduleSave(label)),
    tyndWindow.onClosed((e) => {
      // `onClosed` fires for *other* windows — flush any pending
      // debounce for that label so the geometry isn't lost. The current
      // window's own close goes through `window:close-requested`; users
      // wanting that covered should call `save()` from `onCloseRequested`.
      const other = e.label;
      const t = internal.timers.get(other);
      if (t !== undefined) {
        clearTimeout(t);
        internal.timers.delete(other);
      }
      void flushSave(other);
    }),
  );
}

export const windowState = {
  /**
   * One-shot setup. Loads any previously saved states from disk into
   * the host's in-memory map and attaches the auto-save listeners for
   * the current window. Idempotent — safe to await from multiple boot
   * sites.
   */
  init(opts: WindowStateInitOptions = {}): Promise<void> {
    if (internal.initPromise) return internal.initPromise;
    if (opts.appId !== undefined) internal.appId = opts.appId;
    if (opts.debounceMs !== undefined) internal.debounceMs = opts.debounceMs;
    internal.initPromise = (async () => {
      const path = opts.storagePath ?? (await defaultStoragePath(internal.appId));
      if (path !== null) {
        await call("setStoragePath", { path });
        await call("loadFromDisk", { path });
      }
      attachListeners();
    })();
    return internal.initPromise;
  },

  /**
   * Capture the current window's state and write it to disk
   * **immediately**, bypassing the debounce. Useful from
   * `tyndWindow.onCloseRequested` so the last geometry survives the
   * session even if the user quits during a debounce window.
   */
  async save(label?: string): Promise<void> {
    const target = label ?? tyndWindow.label();
    const t = internal.timers.get(target);
    if (t !== undefined) {
      clearTimeout(t);
      internal.timers.delete(target);
    }
    await flushSave(target);
  },

  /**
   * Apply the last saved state to the named window (defaults to the
   * current one). Returns `true` when something was applied, `false`
   * when no snapshot existed for that label.
   */
  async restore(label?: string): Promise<boolean> {
    const target = label ?? tyndWindow.label();
    const snap = await call<WindowStateSnapshot | null>("get", { label: target });
    if (snap === null) return false;
    if (target === tyndWindow.label()) {
      await applySnapshot(snap);
    }
    // For other labels we can't apply from here — the snapshot lives
    // in the host map until that window itself calls `restore()`.
    return true;
  },

  /** Latest saved snapshot for `label`, or `null` when nothing was stored. */
  get(label?: string): Promise<WindowStateSnapshot | null> {
    return call<WindowStateSnapshot | null>("get", { label: label ?? tyndWindow.label() });
  },

  /**
   * Drop the saved state for one window, or every window when
   * `label` is omitted. Persists the change to disk on the next
   * scheduled save (call `windowState.save()` to flush immediately).
   */
  async clear(label?: string): Promise<void> {
    await call("clear", label === undefined ? {} : { label });
    // Best-effort flush — if storage path isn't configured yet, ignore.
    try {
      await call("saveToDisk");
    } catch {
      /* no path configured yet */
    }
  },

  /**
   * Override the on-disk JSON file location. Default is
   * `<dataDir>/<appId>/window-states.json`. Pass an absolute path;
   * relative paths resolve against the host's CWD which is rarely
   * what callers expect.
   */
  async setStoragePath(path: string): Promise<void> {
    await call("setStoragePath", { path });
  },
};
