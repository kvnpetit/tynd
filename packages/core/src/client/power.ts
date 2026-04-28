/**
 * Power / battery / idle client. Mirrors the host's `os.power.*` dispatch.
 *
 * `getIdleTime` is identical to the v1 `@tynd/plugin-power` surface;
 * `getPowerStatus` is new — exposes battery + AC information collected
 * via platform-native APIs (`GetSystemPowerStatus` on Windows, sysfs
 * on Linux). Fields are nullable — callers must treat `null` as
 * "unavailable on this platform / hardware" instead of zero.
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
};
