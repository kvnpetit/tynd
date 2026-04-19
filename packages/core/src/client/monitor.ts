import { osCall } from "./_internal.ts"

export interface Monitor {
  /** OS-assigned monitor name (null on some Linux setups). */
  name: string | null
  /** Top-left of the monitor in the virtual desktop (physical pixels). */
  position: { x: number; y: number }
  /** Raw pixel size. Divide by `scale` for logical pixels. */
  size: { width: number; height: number }
  /** DPI scale factor (1.0 for 96 DPI, 2.0 for Retina, 1.5 for 150 %). */
  scale: number
  isPrimary: boolean
}

export const monitors = {
  /** List every monitor connected to the system. */
  all(): Promise<Monitor[]> {
    return osCall("window", "monitors")
  },
  /** Primary monitor (the one the taskbar / menu bar is attached to). */
  primary(): Promise<Monitor | null> {
    return osCall("window", "primaryMonitor")
  },
  /** Monitor currently hosting the primary window. */
  current(): Promise<Monitor | null> {
    return osCall("window", "currentMonitor")
  },
}
