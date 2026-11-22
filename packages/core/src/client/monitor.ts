import { osCall, osOn } from "./_internal";

/** Rectangle in physical virtual-desktop pixels. */
export interface MonitorRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Description of one connected display. */
export interface Monitor {
  /**
   * Identifier stable while the display stays connected: the adapter device
   * name on Windows (`\\.\DISPLAY1`), the display id on macOS, the model name
   * plus its rank among identical models on Linux.
   */
  id: string;
  /** OS-assigned monitor name (null on some Linux setups). */
  name: string | null;
  /** Top-left of the monitor in the virtual desktop (physical pixels). */
  position: { x: number; y: number };
  /** Raw pixel size. Divide by `scale` for logical pixels. */
  size: { width: number; height: number };
  /**
   * Usable area: the monitor minus the taskbar, menu bar, Dock or panels
   * (physical pixels). Equals the monitor bounds when the OS reports none.
   */
  workArea: MonitorRect;
  /** DPI scale factor (1.0 for 96 DPI, 2.0 for Retina, 1.5 for 150 %). */
  scale: number;
  isPrimary: boolean;
}

/** Payload of `monitors.onChange`. */
export interface MonitorChange {
  /** Displays connected since the previous event. */
  added: Monitor[];
  /** Displays disconnected since the previous event. */
  removed: Monitor[];
  /** Displays whose position, resolution, scale, work area or primary flag changed. */
  changed: Monitor[];
  /** Every display after the change. */
  monitors: Monitor[];
}

/** Handle returned by `monitors.onChange`. Call `dispose()` to unsubscribe. */
export interface MonitorSubscription {
  dispose(): void;
}

// The host watches the displays only once a frontend asks; the pending
// promise is shared so concurrent subscriptions ask once, and a failure
// clears it so a later call retries.
let watching: Promise<void> | null = null;
function ensureWatching(): Promise<void> {
  watching ??= osCall<void>("window", "subscribeMonitorChanges").catch((error: unknown) => {
    watching = null;
    throw error;
  });
  return watching;
}

/** Frontend bindings for monitor enumeration and pointer queries. */
export const monitors = {
  /** List every monitor connected to the system. */
  all(): Promise<Monitor[]> {
    return osCall("window", "monitors");
  },
  /** Primary monitor (the one the taskbar / menu bar is attached to). */
  primary(): Promise<Monitor | null> {
    return osCall("window", "primaryMonitor");
  },
  /** Monitor currently hosting the primary window. */
  current(): Promise<Monitor | null> {
    return osCall("window", "currentMonitor");
  },
  /** Monitor containing a given virtual-desktop point (physical pixels). */
  fromPoint(x: number, y: number): Promise<Monitor | null> {
    return osCall("window", "monitorFromPoint", { x, y });
  },
  /** Global cursor position in the virtual desktop (physical pixels). */
  cursorPosition(): Promise<{ x: number; y: number }> {
    return osCall("window", "globalCursorPosition");
  },
  /**
   * Fires when a display is connected or disconnected, or when one changes
   * resolution, position, scale factor or work area (taskbar / Dock moved or
   * resized). Resolves once the host watches the displays.
   */
  async onChange(handler: (change: MonitorChange) => void): Promise<MonitorSubscription> {
    // Listen first so no event is lost between the host ack and the return.
    const dispose = osOn<MonitorChange>("monitor:change", (change) => handler(change));
    try {
      await ensureWatching();
    } catch (error) {
      dispose();
      throw error;
    }
    return { dispose };
  },
};
