/**
 * Power / battery / idle / lifecycle client. Mirrors the host's
 * `os.power.*` dispatch.
 *
 * `getIdleTime` is identical to the v1 `@tynd/plugin-power` surface;
 * `getPowerStatus` is new — exposes battery + AC information collected
 * via platform-native APIs (`GetSystemPowerStatus` on Windows, sysfs
 * on Linux). Fields are nullable — callers must treat `null` as
 * "unavailable on this platform / hardware" instead of zero.
 *
 * `onSuspend` / `onResume` / `onLock` / `onUnlock` mirror Electron's
 * `powerMonitor`: the OS forwards system events to the app so background
 * work can be paused on lock and resumed on unlock. Subscribing on the
 * first call lazily installs the platform listener so apps that never
 * use these events don't pay for the watcher.
 */

import { osCall } from "./_internal";

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

/** Handle returned by every power event subscription. Call `dispose()` to unsubscribe. */
export interface PowerSubscription {
  dispose(): void;
}

// We tell the host once that "this app cares about power events" — it
// installs the OS listener lazily. Tracked here so we don't fire the
// dispatch on every `onSuspend` call.
let subscribed = false;
function ensureSubscribed(): void {
  if (subscribed) return;
  subscribed = true;
  void osCall("power", "subscribePowerEvents", null).catch(() => {
    // Subscription failure is non-fatal — `os_on` still works, the
    // events just won't fire. Reset the flag so a later call retries.
    subscribed = false;
  });
}

function subscribePower(eventName: string, handler: () => void): PowerSubscription {
  ensureSubscribed();
  const dispose = window.__tynd__.os_on(eventName, () => handler());
  return { dispose };
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

  /**
   * Fired when the system enters sleep / suspend. Save in-flight work
   * here — apps typically have only milliseconds before the OS halts the
   * process. Returns a disposer; multiple handlers can be registered.
   */
  onSuspend(handler: () => void): PowerSubscription {
    return subscribePower("power:suspend", handler);
  },
  /** Fired when the system resumes from sleep. */
  onResume(handler: () => void): PowerSubscription {
    return subscribePower("power:resume", handler);
  },
  /**
   * Fired when the user locks the screen. macOS distinguishes lock from
   * "session resign-active" — both map to this event so the meaning is
   * "user is no longer at the keyboard".
   */
  onLock(handler: () => void): PowerSubscription {
    return subscribePower("power:lock", handler);
  },
  /** Fired when the user unlocks the screen. */
  onUnlock(handler: () => void): PowerSubscription {
    return subscribePower("power:unlock", handler);
  },
};
