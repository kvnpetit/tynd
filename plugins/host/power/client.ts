/**
 * Power / battery / idle / lifecycle client. Mirrors the host's
 * `os.power.*` dispatch.
 * `getPowerStatus` exposes battery + AC information collected
 * via platform-native APIs (`GetSystemPowerStatus` on Windows, sysfs
 * on Linux, IOKit on macOS). Fields are nullable — callers must treat `null` as
 * "unavailable on this platform / hardware" instead of zero.
 * `onSuspend` / `onResume` / `onLock` / `onUnlock` mirror Electron's
 * `powerMonitor`: the OS forwards system events to the app so background
 * work can be paused on lock and resumed on unlock. Subscribing on the
 * first call lazily installs the platform listener so apps that never
 * use these events don't pay for the watcher.
 */

import { osCall, osOn } from "@tynd/core/client/bridge";

export interface PowerStatus {
  /** `true` = on battery, `false` = on AC, `null` = unknown. */
  onBattery: boolean | null;
  /** Battery charge in percent (0–100). `null` if no battery / unknown. */
  batteryPercent: number | null;
  /** `true` while the battery is charging. */
  charging: boolean | null;
  /** Estimated seconds remaining on battery. `null` while charging or unknown. */
  secondsRemaining: number | null;
}

/** Detailed battery snapshot from `power.getBattery()`. */
export interface BatteryInfo {
  /** Charge in percent (0–100). */
  percent: number;
  /** `true` while the battery is charging. */
  charging: boolean;
  /** Seconds to full (charging) or to empty (discharging); `null` when the OS has no estimate. */
  secondsRemaining: number | null;
  status: "charging" | "discharging" | "full" | "unknown";
}

/** Thermal pressure reported by macOS (`NSProcessInfo.thermalState`). */
export type ThermalState = "nominal" | "fair" | "serious" | "critical";

/** Payload of `power.onPowerSourceChange`. */
export interface PowerSourceChange {
  /** `true` once the machine runs on battery (or a UPS), `false` on AC. */
  onBattery: boolean;
}

/** Handle returned by every power event subscription. Call `dispose()` to unsubscribe. */
export interface PowerSubscription {
  dispose(): void;
}

// We tell the host once that "this app cares about power events" — it
// installs the OS listener lazily. The pending promise is shared so
// concurrent `on*` calls subscribe once; a failure clears it so a later
// call retries.
let subscription: Promise<void> | null = null;
function ensureSubscribed(): Promise<void> {
  subscription ??= osCall<void>("power", "subscribePowerEvents", null).catch((error: unknown) => {
    subscription = null;
    throw error;
  });
  return subscription;
}

async function subscribePower<T = void>(
  eventName: string,
  handler: (payload: T) => void,
): Promise<PowerSubscription> {
  // Listen first so no event is lost between the host ack and the return.
  const dispose = osOn<T>(eventName, (payload) => handler(payload));
  try {
    await ensureSubscribed();
  } catch (error) {
    dispose();
    throw error;
  }
  return { dispose };
}

/** Options for `power.preventSleep`. */
export interface PreventSleepOptions {
  /** Also keep the display on (default `false`: only the system stays awake). */
  display?: boolean;
}

export const power = {
  /** Seconds since the last keyboard/mouse event. */
  async getIdleTime(): Promise<number> {
    const { seconds } = await osCall<{ seconds: number }>("power", "getIdleTime", null);
    return seconds;
  },

  /** Battery + AC summary. See `PowerStatus` for the field semantics. */
  getPowerStatus(): Promise<PowerStatus> {
    return osCall("power", "getPowerStatus", null);
  },

  /** Detailed battery snapshot, or `null` when the machine has no battery. */
  getBattery(): Promise<BatteryInfo | null> {
    return osCall("power", "battery", null);
  },

  /**
   * Keep the system awake (and the display on with `{ display: true }`) until
   * `allowSleep(id)` is called or the app exits. Returns a request id; each
   * call is an independent request, so sleep is allowed again only once every
   * request has been released. Uses power requests on Windows, IOPM assertions
   * on macOS and logind inhibitors on Linux.
   */
  preventSleep(options: PreventSleepOptions = {}): Promise<number> {
    return osCall("power", "preventSleep", { display: options.display === true });
  },

  /**
   * Thermal pressure on macOS (`"nominal"` to `"critical"`); `null` on
   * Windows and Linux, which expose no public thermal API.
   */
  getThermalState(): Promise<ThermalState | null> {
    return osCall("power", "getThermalState", null);
  },

  /** Release a request made by `preventSleep`. Unknown ids are ignored. */
  allowSleep(id: number): Promise<void> {
    return osCall("power", "allowSleep", { id });
  },

  /**
   * Fired when the system enters sleep / suspend. Save in-flight work
   * here — apps typically have only milliseconds before the OS halts the
   * process. Resolves to a disposer once the OS watcher is installed and
   * rejects when it cannot start (for example a host built without
   * `api-power`). Multiple handlers can be registered.
   */
  onSuspend(handler: () => void): Promise<PowerSubscription> {
    return subscribePower("power:suspend", handler);
  },
  /** Fired when the system resumes from sleep. */
  onResume(handler: () => void): Promise<PowerSubscription> {
    return subscribePower("power:resume", handler);
  },
  /**
   * Fired when the user locks the screen. macOS distinguishes lock from
   * "session resign-active" — both map to this event so the meaning is
   * "user is no longer at the keyboard".
   */
  onLock(handler: () => void): Promise<PowerSubscription> {
    return subscribePower("power:lock", handler);
  },
  /** Fired when the user unlocks the screen. */
  onUnlock(handler: () => void): Promise<PowerSubscription> {
    return subscribePower("power:unlock", handler);
  },
  /**
   * Fired when the machine switches between AC and battery. Windows reports
   * it immediately; macOS and Linux poll every 5 seconds.
   */
  onPowerSourceChange(handler: (change: PowerSourceChange) => void): Promise<PowerSubscription> {
    return subscribePower<PowerSourceChange>("power:source-change", handler);
  },
  /**
   * Fired when the session ends: log off, restart or shut down
   * (`WM_ENDSESSION` on Windows, `NSWorkspaceWillPowerOff` on macOS, logind
   * `PrepareForShutdown` on Linux). Best effort: the OS gives the app very
   * little time, so only flush small amounts of state.
   */
  onShutdown(handler: () => void): Promise<PowerSubscription> {
    return subscribePower("power:shutdown", handler);
  },
  /** Fired when the macOS thermal state changes. Never fires on Windows or Linux. */
  onThermalStateChange(
    handler: (change: { state: ThermalState }) => void,
  ): Promise<PowerSubscription> {
    return subscribePower<{ state: ThermalState }>("power:thermal-change", handler);
  },
};
